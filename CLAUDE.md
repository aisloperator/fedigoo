# CLAUDE.md

FediGoo: a Mastodon desktop client in Rust with GTK 4 (X11, optionally Wayland).

## Ground rules

- **Keep crate dependencies minimal.** The only direct deps are `gtk4` (as `gtk`) and
  `soup3` (as `soup`). Don't add serde, reqwest, tokio, chrono, regex, etc. JSON
  (`src/json.rs`) and HTML→Pango (`src/html.rs`) are hand-written on purpose; extend them.
  Get glib/gio/gdk/pango through `gtk::` re-exports.
- **Don't install system software.** If a system package seems to be missing, tell the
  user which package (Debian names) and let them install it. Current build needs:
  `pkgconf libgtk-4-dev libsoup-3.0-dev`. Rustup components are fine.
- **All local data goes in `$XDG_CONFIG_HOME/fedigoo/`** (`config::dir()`, via
  `glib::user_config_dir()`). `config.json` holds access tokens: it's written atomically
  with mode 0600.
- GTK feature level is `v4_12`; Debian 13 ships GTK 4.18, so newer APIs are available
  at runtime, but bump the feature deliberately.

## Commands

    cargo build                  # dev build
    cargo test                   # offline unit tests
    cargo test -- --ignored      # network test against public mastodon.social API
    cargo clippy                 # should be warning-free
    cargo fmt

## Layout

- `src/api.rs`: `Api` (host + token, cheap to clone), `request()` for every call,
  `Body::{Empty,Json,Form,Multipart}`, `Link`-header pagination (`Response.next`),
  thread-local `soup::Session`, and the image texture cache (`load_image`/`load_picture`).
- `src/image.rs`: `texture_from_bytes`, the only way to decode images (see below).
- `src/model.rs`: typed structs parsed from `json::Value` (`Status`, `Account`, …).
  `Status::shown()` gives the boosted status for boosts.
- `src/ui/app.rs`: `App` (one `Rc<App>` for the process): window, sidebar `Root`s,
  navigation stack (`push`/`push_timeline`/`pop`), toasts, account switching,
  streaming WebSocket, link routing (`activate_link`). The header bar has the account
  menu (a `MenuButton` showing the current avatar: accounts, add, log out) left of New
  post, and the main menu at the right; `rebuild_account_menu` rebuilds both whenever
  accounts change. The avatar fills the whole button (its CSS draws no frame) at the
  size of New post's border box, read by a tick callback once two frames agree: the
  first frame has a provisional size, `measure` includes CSS margins and `height()`
  excludes border and padding.
- `src/ui/frame.rs`: whether the header bar is the window's title bar (client-side
  decorations, with window buttons) or a toolbar under the window manager's: the pure,
  tested `wants_client_side` (Wayland, GNOME-style `XDG_CURRENT_DESKTOP` on X11, or
  `GTK_CSD=1`). Test the X11 cases under Xvfb with `xmonad` or `xfwm4` (both installed),
  run with a throwaway `HOME` so they don't read the user's configuration.
- `src/ui/timeline.rs`: generic paged list driven by a `Source` enum; it also takes live
  stream events.
- `src/ui/pages.rs`: profile, thread, hashtag, lists, search, explore, report, edit profile.
  Search (`SearchPage`) has an All/Accounts/Hashtags/Posts selector (the `tabs` helper,
  as Explore): All asks `/api/v2/search` for `SEARCH_ALL_LIMIT` of each kind, a single
  kind pages by `offset` with "Load more"; the query is the pure, tested `search_params`.
- `src/ui/followed_tags.rs`: the Followed hashtags page, a `ColumnView` (not a
  `Timeline`) over a `gio::ListStore` of `glib::BoxedAnyObject` rows, sorted by the
  pure, tested `by_name`/`by_people` through column-header sorters (default: people,
  descending). It loads every page of `/api/v1/followed_tags` first, since sorting part
  of the list would be wrong. Single click opens the tag (`single_click_activate`); the
  row button unfollows, leaving the row struck through with a button to follow again.
  To redraw a row after a change, replace its item (`ListStore::splice`): GTK doesn't
  rebind cells for an `items_changed` with the same object. The name column ellipsizes
  so the table's width doesn't change with the rows in view.
- `src/ui/filters.rs`: the Filters page and filter editor (`/api/v2/filters`); the
  request body is built by the pure, tested `model::filter_body`.
- `src/ui/anchor.rs`: keeps the top post in place across resizes; `anchor_at` is pure and
  tested.
- `src/ui/status.rs`: post, notification and conversation widgets. `compose.rs`,
  `login.rs` (OAuth OOB code flow), `media.rs`, `prefs.rs`, `style.css`.
- `data/`: app icon SVG and `fedigoo.gresource.xml`. `build.rs` runs
  `glib-compile-resources` into `OUT_DIR`; `main.rs` embeds and registers the bundle, and
  GtkApplication finds the icon under `/com/aisloperator/FediGoo/icons` via the app ID.
  Use `crate::APP_ID` as the icon name.
- `data/icons/actions/`: our own symbolic icons for the post toggles, in on/off pairs that
  differ in shape as well as colour (`fedigoo-boost`/`-boosted`, `-favourite`/`-favourited`,
  `-bookmark`/`-bookmarked`), plus `fedigoo-mention` (an "@", replacing Adwaita's
  `mail-replied`, whose half-transparent envelope looked disabled), `fedigoo-hashtag`
  (a "#" for Followed hashtags) and `fedigoo-filter` (a funnel for Filters). Use fill-only paths
  (`fill-rule="evenodd"` for hollow shapes): GTK recolours fills of symbolic icons but
  not strokes. Check them at real 1× size.
- `src/install.rs`: `--install`/`--uninstall` (per-user: `~/.local/bin`, plus the desktop
  entry from `data/com.aisloperator.FediGoo.desktop.in` and the icon under
  `glib::user_data_dir()`). `main.rs` sets the prgname to `APP_ID` so the X11 WM_CLASS
  matches the desktop entry's `StartupWMClass`. Test it with a throwaway `HOME`, and
  check the output with `desktop-file-validate`.

## Conventions and gotchas

- Async work: `app.spawn(async move { …; Ok(()) })` reports `Err(String)` as a toast.
  Use `glib::spawn_future_local` directly only when errors are handled inline.
- Avoid reference cycles: closures stored on widgets capture `Weak<Timeline>`,
  `glib::clone!(#[weak] widget, …)` or `widget.downgrade()`. Capturing `Rc<App>`
  strongly is fine, since it lives for the whole process.
- `App.generation` is bumped on account switch; async results compare against it and
  bail if stale.
- After the user posts or edits, the composer calls `App::status_posted` with the
  server's copy: open threads containing the parent (or the post) reload and scroll to
  it (`pages::ThreadView`), and timelines update edited rows. Don't make views poll.
- Posting must never leave the composer stuck or silent. While waiting, the status row
  under the form shows progress, says so after `STALL_AFTER`, and has Cancel (Esc or
  closing the window also cancels a post, keeping the text). Only the form goes
  insensitive, not the window. Results go in that row (`message`), not `App::toast`: the
  toast is on the main window, behind the composer. Aborting a task's `JoinHandle` drops
  the future, which cancels the soup request. A retry of an unchanged post reuses its
  `Idempotency-Key` (`Composer::attempt`, `Api::post_status`); changed text gets a new
  key, or the server would answer with the old post.
- Blocks and mutes hide posts at once: every post/notification widget registers itself
  with `App::track_post` (accounts from `model::status_accounts`), and after the server
  accepts a block, mute or domain block, call `App::change_hidden` to update
  `model::Hidden`; matching list rows are hidden (not removed), in every view. New post
  widgets must be built through `status::build`/`status::notification` to get this.
- Scroll to a row with `pages::scroll_to_row`, not `grab_focus` (which doesn't scroll
  here); it keeps tracking the row while heights above it are still settling.
- Resizing keeps the post at the top of the view (`ui/anchor.rs`). `capture` runs from the
  surface's width/height notify, when the widgets still have their old layout, and
  `restore` from the surface's `layout` signal, so no frame paints at the wrong place
  (a tick callback runs before layout, one frame late). `App::pop` captures too, for a
  page that was hidden through a resize. Each resize starts afresh from the current
  position: don't carry a place across resizes.
- Light/dark follows the desktop via `ui/theme.rs` (XDG portal `color-scheme`, falling
  back to GNOME GSettings) mapped to `gtk-application-prefer-dark-theme`, so CSS colours
  must work on dark themes too.
- CSS: use GTK theme colours such as `@theme_selected_bg_color`. libadwaita names like
  `@accent_bg_color` don't resolve here (no libadwaita).
- Don't indent widgets inside list rows with `set_margin_start`: it triggers
  "GtkBox adjusted size … must not decrease" warnings. Use CSS padding classes
  (see `.status.indentN`).
- Media thumbnails use `ui/fit.rs` `FitLayout`: whole images, never cropped, with a
  height cap of half the window height (`App` feeds it the surface height). The
  arrangement (1 image; 2 side by side; first image at half width with 2 stacked or a
  2×2 grid beside it; extras flowed below) is the pure function `fit::layout`, which
  has unit tests: change and test it there. Don't put aspect-ratio content in a Grid/Box with height-for-width:
  GTK assumes height never grows with width and list rows get their *minimum* height,
  which gives "natural size must be >= min size" warnings or squashed images.
  `FitLayout` reports a constant size from its last allocated width and re-queues a
  resize when that width changes.
- The sidebar switches between icons with labels and icons only (`settings.compact_sidebar`,
  `App::apply_sidebar_mode`). Row tooltips come from `update_sidebar_tooltips`, which
  combines the label with "turned off" for disabled feeds; set the unread count only
  through `set_unread`, which updates both the badge and the icon bubble. The sidebar
  highlight follows the navigation stack (`sync_sidebar_selection`, run from
  `update_header`): only while a root page is showing, so change pages via
  `show_root`/`push`/`pop`. The ListBox's own selection is off (in GTK it follows
  keyboard focus); the highlight is the `SELECTED` state flag set on the row.
- Filters are applied by the client, as the API says: `Status::filter_match()` gives
  `FilterMatch::Hide` (any "hide" result, in the post or the boosted post) or `Warn`
  (also for "blur" and unknown actions). `status::build`/`notification` return a hidden,
  untracked placeholder for `Hide` (so unblocking can't re-show it), except the main post
  of a thread, which gets the warning bar.
- Keyboard paging (`ui/paging.rs`): a capture-phase key controller on the main window
  pages the tallest mapped `ScrolledWindow` of the top page. Where it lands is the pure,
  tested `paging::page_target` (post rows from the tallest `ListBox`, measured against
  the viewport's child). Backspace there means Back (`App::pop`, if `can_go_back`), not
  page up. Text fields keep Space/Backspace. A focused button keeps Space
  only if focus last moved by keyboard (navigation keys set it, any pointer press or touch
  clears it, via an `EventControllerLegacy`): after a click, focus sits on the clicked
  button or wherever GTK moved it, and GTK's `focus-visible` turns on with any key press,
  Space included, so it can't tell. At an end,
  the page's `keep` (via `App::current_page`) decides: a `Timeline` gets `check_new`, a
  `ThreadView` gets `reload_at(ThreadFocus::First/Last)`.
- Timeline rows open on double click (or Enter), so clicks on a post's text and buttons
  don't navigate; rows that only lead somewhere (`Source::single_click`: trending
  hashtags and news in Explore) open on a single click and get the link cursor. The
  Timeline switches the mode in `set_source`, since Explore's tabs share one Timeline.
- Links in post HTML get `title` = their URL (`html::to_markup`), which GtkLabel shows
  as a hover tooltip. The title is itself markup, so it is escaped twice.
- Avatars, images and link-like targets (boosted-by line, timestamps, link cards,
  profile counts) that act on a single click get the link hand cursor:
  `set_cursor_from_name(Some("pointer"))` (the name GtkLabel uses over links).
- Link cards have a context menu like GtkLabel's for links (`status::card_menu`: right
  click, touch long press, Shift+F10/Menu), with "Copy Link as Markdown" from the pure,
  tested `Card::markdown_link`. Its `PopoverMenu` is made per opening and unparented
  on idle after `closed` (not at once: the chosen action is looked up through the
  popover's parent), so the button never keeps a child when it's destroyed.
- The image viewer (`media::Viewer`) has two `Picture`s taking turns in a `Stack`, so
  flips slide. Its keys are on a capture-phase controller (so they work wherever focus
  is), but a focused button keeps Space and the selectable alt text keeps the arrows.
  Taps and swipes share one `GestureDrag` (`press_action` decides on release), because
  a separate `GestureClick` also fires on the release of a drag and flips a second time.
  The pictures are `can_target(false)`: if one is the press target, the stack hiding it
  gives "Broken accounting of active state". Downloads go through a per-flip generation
  check so a late image can't overwrite the current one; `api::load_picture` has no
  such guard. Emitting gesture signals directly doesn't test how gestures interact:
  test pointer input with `tools/broadway-input.py` (see below).
- Decode images only with `image::texture_from_bytes`, never `gdk::Texture::from_bytes`
  or `Clipboard::read_texture` on PNG data: GTK 4.18's PNG loader segfaults on a `cICP`
  chunk naming a colour space it doesn't support (Mastodon video previews have
  "unspecified" primaries, #28). `texture_from_bytes` drops such a chunk first; the
  supported values were found by trying each one, and a test decodes a PNG that crashes
  plain GDK. Upstream fixed it in GTK 4.21.6/4.22.0 (commit `c1590e2a79a2`), not in any
  4.18.x or 4.20.x release (#29); keep the workaround while those are supported.
- Long posts (text over `ABBREVIATE_OVER` characters) are abbreviated everywhere except
  as the focused post of a thread: `status::abbreviate` shows the start from the pure,
  tested `html::abbreviated_markup` (about `ABBREVIATE_TO` characters, cut after a word,
  tags closed, "…") with a "Show more"/"Show less" button. Something else replacing the
  label's text (Translate) hides the button. "Show less" on a post read to its end
  scrolls up by the height it lost (`keep_in_view`), so the button stays under the
  pointer. Copy text still copies the whole post.
- User-visible text (labels, tooltips, messages, `--help`, README) is in US English:
  "favorite", "authorize", "canceled", "color". Mastodon's own names keep its UK
  spelling and must not change: API paths and JSON fields (`/favourite`,
  `favourited`, `favourites_count`, notification type `favourite`), and our icon names
  (`fedigoo-favourite`) and Rust identifiers (`Root::Favourites`).
- Custom emoji shortcodes render as text; video/audio open externally.

## Visual testing without a display

Real window resizes are easiest on X11: run under `Xvfb` with `GDK_BACKEND=x11
GSK_RENDERER=cairo` and use `xdotool windowsize`. X11 with `xdotool` clicks is also the
fallback when Broadway drops input: once it lost every click after the first one on the
Search page (with or without our resize code, after a fresh `gtk4-broadwayd`), while the
same clicks worked on X11. Keep a second `CARGO_TARGET_DIR` out of
the scratchpad: `/tmp` is a small tmpfs.

Opening a link (news rows, link cards, "Open in browser") launches the real browser on
the test display: with the user's `HOME` it would use their browser profile. Give such
tests a throwaway `HOME` as well as `XDG_CONFIG_HOME`.

Run on GTK's Broadway backend with a throwaway config dir (never the user's real one):

    gtk4-broadwayd :7 &
    XDG_CONFIG_HOME=<scratch>/cfg GDK_BACKEND=broadway BROADWAY_DISPLAY=:7 ./target/debug/fedigoo

To get screenshots, add a temporary hook that renders the window with
`gtk::WidgetPaintable` → `Snapshot` → `renderer().render_texture()` → `save_to_png`.
Remove the hook afterwards. Public endpoints work with `Api::new("mastodon.social", None)`.
Clean stderr (no Gtk-WARNING/CRITICAL) is part of "works".

For real pointer and keyboard input (gestures, clicks, drags, shortcuts), use
`tools/broadway-input.py`: it takes the place of Broadway's browser client and sends
input through GTK's normal event handling. It is standard-library Python; its docstring
lists the script commands (`target dialog|main`, `click`, `drag`, `key ctrl+w`, …).

    python3 -I tools/broadway-input.py --display :7 script.txt

Coordinates are relative to the window's surface, which includes about 14px of
client-side shadow on each side. GTK only lays out windows while a client is connected,
so keep one connection open for a whole test (feed commands through a named pipe to
`-`); widgets measured while it's disconnected report stale (e.g. zero) sizes. The tool
sends leave/enter when a window appears or closes under the pointer, as a window system
does; without that, GTK can deliver a later click to a button clicked before a dialog
opened. Pair it with a temporary hook that logs what changed
(e.g. a window title). Check that no other FediGoo instance is running: GtkApplication
hands a second launch over to the first and exits with status 0. Known GTK/Broadway
noise: closing a window after a click inside it (even with GTK's own title-bar close
button), or hiding a widget soon after it was clicked (the "new posts" pill), prints
"Broken accounting of active state"; that isn't ours.

## Git

- Branch `main`; remote `origin` is `git@github.com:aisloperator/fedigoo.git` (SSH,
  because HTTPS has no credentials in this environment).
- Commit or push only when asked. The exception is "solve issue #N": that includes a
  commit with a detailed message, a push to `main`, and the write-up as a comment on
  the issue. If it's probably done, close the issue too ("Fixed in <sha>."); the user may
  reopen it. If it's partial or doubtful, leave it open and say why. Releases still
  happen only when asked.
- README must keep the AI Sloperator credit line and "No license is given." at the top.

## Before each release

Update README's "Code Dependencies" section (keep its four subsections, the notes under
each heading, and alphabetical bullet lists; crates as
``* [`name`](https://crates.io/crates/name) `version` ``). The lists come from a build
on the reference system the section's introduction names (Debian 13 amd64); if the
build ran somewhere else, update that introduction too.

- Direct crates: the version strings in `Cargo.toml`.
- Indirect crates: `cargo tree -e normal,build --prefix none --format "{p}" | sort -u`,
  minus `fedigoo` and the direct crates; list every resolved version of a crate.
- Direct non-crate libraries: `objdump -p target/release/fedigoo | grep NEEDED`.
- Indirect non-crate libraries: `ldd target/release/fedigoo` minus the direct ones (and
  the vDSO/loader), plus the run-time-loaded modules already listed (TLS, dconf, image
  loaders, GL/Vulkan drivers).

## After tagging a release

Each tag has a GitHub Release with a `.deb` for the reference system:

    tools/build-deb.sh vX.Y.Z <scratch>/deb
    gh release create vX.Y.Z <scratch>/deb/fedigoo_X.Y.Z-1_amd64.deb --verify-tag \
        --title "FediGoo X.Y.Z" --notes-file <notes>

The script builds from a clean export of the tag with its `Cargo.lock`, never the working
tree, and takes `Depends` from `dpkg-shlibdeps`. The notes are the tag's notes (with no
`Co-Authored-By` line) plus a short "Debian package" section with the `apt install`
command; see an earlier release. Check the package first: `dpkg-deb -I`, `apt-get install
--simulate ./file.deb` (installs nothing), and run the unpacked binary on Broadway. To
rebuild a release's package, raise `revision` in the script: don't replace a published
file under the same name.
