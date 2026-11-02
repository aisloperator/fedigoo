#!/usr/bin/env python3
"""Send real pointer and keyboard input to a GTK app on the Broadway backend.

Broadway is GTK's HTML5 backend: `gtk4-broadwayd :N` serves a web page on
port 8080+N whose JavaScript client draws the windows and sends input back
over a WebSocket.  This script speaks that same protocol in place of the
browser, so tests can drive an app with real press/motion/release and key
events that go through GTK's normal input handling (gestures, focus,
grabs), unlike emitting widget signals directly.  Standard library only.

Usage:
    python3 -I tools/broadway-input.py [--display :N | --port P] SCRIPT

SCRIPT is a file of commands, one per line (or "-" for standard input).
Coordinates are in pixels, relative to the current target window's surface
(which includes any client-side decoration shadow around the window).

    target dialog|main   choose the window: "dialog" is a visible window that
                         is transient for another (e.g. the image viewer),
                         "main" a visible top-level one; waits up to 30 s
                         for it to appear.  The default is "main".
    surfaces             print the visible windows' surfaces
    wait SECONDS         sleep
    move X Y             move the pointer
    click X Y [BUTTON]   press and release (button 1 by default)
    drag X1 Y1 X2 Y2 [STEPS]
                         press at X1,Y1, move in STEPS steps, release at X2,Y2
    key [MOD+]...NAME    press and release a key, e.g. "key Right",
                         "key ctrl+w", "key space", "key a".  Keys go to the
                         window that Broadway has focused (normally the one
                         most recently shown or clicked).
    # ...                a comment

Each command is followed by a short pause (PAUSE seconds) so the app can
react.  Only one client can be connected to a Broadway server at a time;
this script takes the place of the browser.

Example (FediGoo's image viewer, with the app showing an image):
    target dialog
    wait 3
    drag 700 300 300 300
    key Left
    key ctrl+w
"""
import base64
import os
import socket
import struct
import sys
import threading
import time

PAUSE = 0.6

# Client-to-server event types (broadway.js BROADWAY_EVENT_*).
EV_ENTER, EV_LEAVE, EV_MOVE, EV_PRESS, EV_RELEASE = 0, 1, 2, 3, 4
EV_KEY_PRESS, EV_KEY_RELEASE = 7, 8
EV_GRAB_NOTIFY, EV_UNGRAB_NOTIFY = 9, 10
EV_SCREEN_SIZE = 12
EV_ROUNDTRIP_NOTIFY = 14

# GdkModifierType bits.
MODIFIERS = {"shift": 1 << 0, "ctrl": 1 << 2, "control": 1 << 2, "alt": 1 << 3}
BUTTON_MASKS = {1: 1 << 8, 2: 1 << 9, 3: 1 << 10}

# X keysyms for keys without a printable character.
KEYSYMS = {
    "BackSpace": 0xFF08, "Tab": 0xFF09, "Return": 0xFF0D, "Escape": 0xFF1B,
    "Delete": 0xFFFF, "Home": 0xFF50, "Left": 0xFF51, "Up": 0xFF52,
    "Right": 0xFF53, "Down": 0xFF54, "Page_Up": 0xFF55, "Page_Down": 0xFF56,
    "End": 0xFF57, "space": 0x20,
}
KEYSYMS.update({"F%d" % n: 0xFFBD + n for n in range(1, 13)})


def keysym(name):
    if name in KEYSYMS:
        return KEYSYMS[name]
    if len(name) == 1:
        return ord(name)
    raise SystemExit("unknown key name %r" % name)


class Client:
    def __init__(self, port):
        self.sock = socket.create_connection(("127.0.0.1", port))
        key = base64.b64encode(os.urandom(16)).decode()
        self.sock.sendall(
            (
                "GET /socket HTTP/1.1\r\nHost: 127.0.0.1\r\nUpgrade: websocket\r\n"
                "Connection: Upgrade\r\nSec-WebSocket-Key: %s\r\n"
                "Sec-WebSocket-Version: 13\r\nSec-WebSocket-Protocol: broadway\r\n\r\n"
                % key
            ).encode()
        )
        resp = b""
        while b"\r\n\r\n" not in resp:
            byte = self.sock.recv(1)
            if not byte:
                raise SystemExit("connection closed during the WebSocket handshake")
            resp += byte
        if b" 101 " not in resp.split(b"\r\n")[0]:
            raise SystemExit("WebSocket handshake failed: %r" % resp)
        self.surfaces = {}  # id -> dict(x, y, w, h, visible, parent)
        self.serial = 0
        self.lock = threading.Lock()
        self.t0 = time.monotonic()
        self.target_kind = "main"
        self.entered = None  # surface the pointer is over
        threading.Thread(target=self.reader, daemon=True).start()
        self.send(EV_SCREEN_SIZE, [1600, 1000, 1])

    def now(self):
        return int((time.monotonic() - self.t0) * 1000) + 1000

    # --- WebSocket framing ---

    def recv_exact(self, n):
        buf = b""
        while len(buf) < n:
            chunk = self.sock.recv(n - len(buf))
            if not chunk:
                raise EOFError
            buf += chunk
        return buf

    def read_frame(self):
        b0, b1 = self.recv_exact(2)
        n = b1 & 0x7F
        if n == 126:
            n = struct.unpack(">H", self.recv_exact(2))[0]
        elif n == 127:
            n = struct.unpack(">Q", self.recv_exact(8))[0]
        if b1 & 0x80:
            mask = self.recv_exact(4)
            data = bytes(c ^ mask[i % 4] for i, c in enumerate(self.recv_exact(n)))
        else:
            data = self.recv_exact(n)
        return b0 & 0x0F, data

    def send_frame(self, payload, opcode=2):
        # Client frames must be masked.
        mask = os.urandom(4)
        n = len(payload)
        head = bytes([0x80 | opcode])
        if n < 126:
            head += bytes([0x80 | n])
        else:
            head += bytes([0x80 | 126]) + struct.pack(">H", n)
        body = bytes(c ^ mask[i % 4] for i, c in enumerate(payload))
        with self.lock:
            self.sock.sendall(head + mask + body)

    def send(self, event, args):
        # Every input message: event type, last server serial, timestamp,
        # then the event's own fields, all big-endian 32-bit integers.
        vals = [event, self.serial, self.now()] + list(args)
        self.send_frame(struct.pack(">%di" % len(vals), *vals))

    # --- server messages (little-endian; broadway.js handleCommands) ---

    def reader(self):
        try:
            while True:
                opcode, data = self.read_frame()
                if opcode == 8:
                    return
                if opcode in (1, 2):
                    self.parse(data)
        except (EOFError, OSError):
            return

    def parse(self, d):
        p = 0
        s = self.surfaces
        while p < len(d):
            op = d[p]
            self.serial = struct.unpack_from("<I", d, p + 1)[0]
            p += 5
            if op == 0:  # grab pointer: acknowledge, as the web client does
                p += 3
                self.send(EV_GRAB_NOTIFY, [])
            elif op == 1:  # ungrab pointer
                self.send(EV_UNGRAB_NOTIFY, [])
            elif op == 2:  # new surface
                i, x, y, w, h = struct.unpack_from("<HhhHH", d, p)
                p += 10
                s[i] = dict(x=x, y=y, w=w, h=h, visible=False, parent=0)
            elif op in (3, 4):  # show / hide surface
                (i,) = struct.unpack_from("<H", d, p)
                p += 2
                if i in s:
                    s[i]["visible"] = op == 3
            elif op in (5, 6, 7, 11):  # raise, lower, destroy, update
                (i,) = struct.unpack_from("<H", d, p)
                p += 2
                if op == 7:
                    s.pop(i, None)
            elif op == 8:  # move / resize
                i, flags = struct.unpack_from("<HB", d, p)
                p += 3
                if flags & 1:
                    x, y = struct.unpack_from("<hh", d, p)
                    p += 4
                    if i in s:
                        s[i].update(x=x, y=y)
                if flags & 2:
                    w, h = struct.unpack_from("<HH", d, p)
                    p += 4
                    if i in s:
                        s[i].update(w=w, h=h)
            elif op == 9:  # transient for
                i, parent = struct.unpack_from("<HH", d, p)
                p += 4
                if i in s:
                    s[i]["parent"] = parent
            elif op == 10:  # disconnected
                return
            elif op == 12:  # show on-screen keyboard
                p += 2
            elif op == 13:  # upload texture
                _, size = struct.unpack_from("<II", d, p)
                p += 8 + size
            elif op == 14:  # release texture
                p += 4
            elif op == 15:  # set render nodes
                _, n = struct.unpack_from("<HI", d, p)
                p += 6 + n * 4
            elif op == 16:  # roundtrip: GTK waits for the answer
                i, tag = struct.unpack_from("<HI", d, p)
                p += 6
                self.send(EV_ROUNDTRIP_NOTIFY, [i, tag])
            else:
                print("broadway-input: unknown server op %d; skipping the rest of "
                      "this message" % op, file=sys.stderr, flush=True)
                return

    # --- targets ---

    def find_target(self):
        """The largest visible surface of the chosen kind, as (id, surface)."""
        want_dialog = self.target_kind == "dialog"
        found = [
            (s["w"] * s["h"], i, s)
            for i, s in list(self.surfaces.items())
            # Tiny surfaces are tooltips, popovers and drag icons.
            if s["visible"] and bool(s["parent"]) == want_dialog and s["w"] > 100
        ]
        if not found:
            return None
        _, i, s = max(found, key=lambda t: t[0])
        return i, s

    def set_target(self, kind):
        if kind not in ("dialog", "main"):
            raise SystemExit("target must be 'dialog' or 'main', not %r" % kind)
        self.target_kind = kind
        self.entered = None
        deadline = time.monotonic() + 30
        while self.find_target() is None:
            if time.monotonic() > deadline:
                raise SystemExit("no %s window appeared; surfaces: %r" % (kind, self.surfaces))
            time.sleep(0.2)
        i, s = self.find_target()
        print("target %s: surface %d at %d,%d, %dx%d" % (kind, i, s["x"], s["y"], s["w"], s["h"]),
              flush=True)

    # --- input ---

    def pointer(self, event, x, y, state, button=None):
        target = self.find_target()
        if target is None:
            raise SystemExit("the %s window has gone" % self.target_kind)
        i, s = target
        if self.entered != i:
            # Like a browser: one enter as the pointer comes over the window.
            self.send(EV_ENTER, [i, i, s["x"] + x, s["y"] + y, x, y, 0, 0])
            self.entered = i
        args = [i, i, s["x"] + x, s["y"] + y, x, y, state]
        if button is not None:
            args.append(button)
        self.send(event, args)

    def move(self, x, y):
        self.pointer(EV_MOVE, x, y, 0)

    def click(self, x, y, button=1):
        mask = BUTTON_MASKS.get(button, 0)
        self.move(x, y)
        time.sleep(0.05)
        # As in broadway.js, a press's state already includes its button.
        self.pointer(EV_PRESS, x, y, mask, button)
        time.sleep(0.08)
        self.pointer(EV_RELEASE, x, y, 0, button)

    def drag(self, x1, y1, x2, y2, steps=12):
        mask = BUTTON_MASKS[1]
        self.move(x1, y1)
        time.sleep(0.05)
        self.pointer(EV_PRESS, x1, y1, mask, 1)
        for k in range(1, steps + 1):
            time.sleep(0.02)
            self.pointer(EV_MOVE, x1 + (x2 - x1) * k // steps, y1 + (y2 - y1) * k // steps, mask)
        time.sleep(0.03)
        self.pointer(EV_RELEASE, x2, y2, 0, 1)

    def key(self, spec):
        *mods, name = spec.split("+")
        state = 0
        for m in mods:
            if m.lower() not in MODIFIERS:
                raise SystemExit("unknown modifier %r in %r" % (m, spec))
            state |= MODIFIERS[m.lower()]
        sym = keysym(name)
        self.send(EV_KEY_PRESS, [sym, state])
        time.sleep(0.03)
        self.send(EV_KEY_RELEASE, [sym, state])


def main(argv):
    port = None
    args = argv[1:]
    while args and args[0].startswith("--"):
        opt = args.pop(0)
        if opt == "--port" and args:
            port = int(args.pop(0))
        elif opt == "--display" and args:
            port = 8080 + int(args.pop(0).lstrip(":"))
        else:
            raise SystemExit(__doc__)
    if len(args) != 1:
        raise SystemExit(__doc__)
    if port is None:
        port = 8080 + int(os.environ.get("BROADWAY_DISPLAY", ":0").lstrip(":") or 0)
    lines = sys.stdin if args[0] == "-" else open(args[0])

    c = Client(port)
    c.set_target("main")
    for line in lines:
        parts = line.split()
        if not parts or parts[0].startswith("#"):
            continue
        cmd, rest = parts[0], parts[1:]
        print("input:", " ".join(parts), flush=True)
        if cmd == "target":
            c.set_target(rest[0])
            continue
        elif cmd == "surfaces":
            for i, s in sorted(c.surfaces.items()):
                if s["visible"]:
                    print("  surface %d at %d,%d, %dx%d%s" % (
                        i, s["x"], s["y"], s["w"], s["h"],
                        ", transient for %d" % s["parent"] if s["parent"] else ""), flush=True)
            continue
        elif cmd == "wait":
            time.sleep(float(rest[0]))
            continue
        elif cmd == "move":
            c.move(int(rest[0]), int(rest[1]))
        elif cmd == "click":
            c.click(*[int(v) for v in rest])
        elif cmd == "drag":
            c.drag(*[int(v) for v in rest])
        elif cmd == "key":
            c.key(rest[0])
        else:
            raise SystemExit("unknown command %r" % cmd)
        time.sleep(PAUSE)


if __name__ == "__main__":
    main(sys.argv)
