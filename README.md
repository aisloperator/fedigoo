# FediGoo

Created by [AI Sloperator](https://www.aisloperator.com/) with Claude Code.

No license is given.

A Mastodon client for the Linux desktop, written in Rust with GTK 4.

## Building

System packages (Debian/Ubuntu names):

    sudo apt install pkgconf libgtk-4-dev libsoup-3.0-dev

Then, with a stable Rust toolchain from rustup:

    cargo build --release
    ./target/release/fedigoo

The only crate dependencies are the GTK bindings (`gtk4`) and the libsoup
bindings (`soup3`) plus their own dependencies. JSON handling and the HTML
conversion are built in. HTTPS uses libsoup and glib-networking.

## Running

    fedigoo            # GTK picks the backend (honours GDK_BACKEND)
    fedigoo --x11      # force X11
    fedigoo --wayland  # force Wayland

Log in through the welcome screen. FediGoo opens your instance's
authorisation page in the browser; paste the code it shows back into
FediGoo.

All local data (accounts, access tokens, OAuth client registrations,
preferences) is in `$XDG_CONFIG_HOME/fedigoo/config.json`, normally
`~/.config/fedigoo/config.json`. The file is created with owner-only
permissions.

## Features

- Multiple accounts, with OAuth login, switching and logout (which revokes the token)
- Timelines: home, local, federated, hashtags, lists, trending posts, bookmarks and favourites
- Notifications, mentions and private conversations
- Live updates over the streaming API, with an unread badge and a "new items" button
- Posts: content warnings, sensitive-media covers, image viewer with alt text,
  link cards, polls (with voting), threads with indented replies, translation
- Actions: reply, boost, favourite, bookmark, edit, delete, delete & re-draft,
  pin, mute conversation, copy link or text, report
- Composer: character counter using the instance limit, content warning,
  visibility, language, media upload (file picker or paste) with alt text,
  polls, and Ctrl+Enter to send
- Profiles: bio, verified fields, follow or follow request, notify on new posts,
  hide boosts, mute, block, domain block, list membership, followers and following
- Search for accounts, hashtags and posts (URLs from other servers are resolved)
- Explore (trending posts, hashtags and news), followed hashtags, follow requests,
  list management, mutes and blocks, profile editing (including avatar and header)
- Links to profiles, posts and hashtags on any server open inside the app

## Keyboard shortcuts

| Key               | Action      |
|-------------------|-------------|
| Ctrl+N            | New post    |
| F5 / Ctrl+R       | Refresh     |
| Alt+Left / Esc    | Back        |
| Ctrl+F            | Search      |
| Ctrl+Enter        | Send post   |
| Ctrl+Q            | Quit        |

## Tests

    cargo test                   # offline unit tests
    cargo test -- --ignored      # also hits the public mastodon.social API
