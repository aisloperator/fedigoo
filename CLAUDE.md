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
- `src/model.rs`: typed structs parsed from `json::Value` (`Status`, `Account`, …).
  `Status::shown()` gives the boosted status for boosts.
- `src/ui/app.rs`: `App` (one `Rc<App>` for the process): window, sidebar `Root`s,
  navigation stack (`push`/`push_timeline`/`pop`), toasts, account switching,
  streaming WebSocket, link routing (`activate_link`).
- `src/ui/timeline.rs`: generic paged list driven by a `Source` enum; it also takes live
  stream events.
- `src/ui/pages.rs`: profile, thread, hashtag, lists, search, explore, report, edit profile.
- `src/ui/status.rs`: post, notification and conversation widgets. `compose.rs`,
  `login.rs` (OAuth OOB code flow), `media.rs`, `prefs.rs`, `style.css`.
- `data/`: app icon SVG and `fedigoo.gresource.xml`. `build.rs` runs
  `glib-compile-resources` into `OUT_DIR`; `main.rs` embeds and registers the bundle, and
  GtkApplication finds the icon under `/com/aisloperator/FediGoo/icons` via the app ID.
  Use `crate::APP_ID` as the icon name.

## Conventions and gotchas

- Async work: `app.spawn(async move { …; Ok(()) })` reports `Err(String)` as a toast.
  Use `glib::spawn_future_local` directly only when errors are handled inline.
- Avoid reference cycles: closures stored on widgets capture `Weak<Timeline>`,
  `glib::clone!(#[weak] widget, …)` or `widget.downgrade()`. Capturing `Rc<App>`
  strongly is fine, since it lives for the whole process.
- `App.generation` is bumped on account switch; async results compare against it and
  bail if stale.
- CSS: use GTK theme colours such as `@theme_selected_bg_color`. libadwaita names like
  `@accent_bg_color` don't resolve here (no libadwaita).
- Don't indent widgets inside list rows with `set_margin_start`: it triggers
  "GtkBox adjusted size … must not decrease" warnings. Use CSS padding classes
  (see `.status.indentN`).
- Custom emoji shortcodes render as text; video/audio open externally.

## Visual testing without a display

Run on GTK's Broadway backend with a throwaway config dir (never the user's real one):

    gtk4-broadwayd :7 &
    XDG_CONFIG_HOME=<scratch>/cfg GDK_BACKEND=broadway BROADWAY_DISPLAY=:7 ./target/debug/fedigoo

To get screenshots, add a temporary hook that renders the window with
`gtk::WidgetPaintable` → `Snapshot` → `renderer().render_texture()` → `save_to_png`.
Remove the hook afterwards. Public endpoints work with `Api::new("mastodon.social", None)`.
Clean stderr (no Gtk-WARNING/CRITICAL) is part of "works".

## Git

- Branch `main`; remote `origin` is `git@github.com:aisloperator/fedigoo.git` (SSH,
  because HTTPS has no credentials in this environment).
- Commit or push only when asked.
- README must keep the AI Sloperator credit line and "No license is given." at the top.
