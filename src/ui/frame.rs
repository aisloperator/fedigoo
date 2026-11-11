//! Who draws the main window's title bar: the app (client-side decorations,
//! with the header bar as the title bar and its own minimize, maximize and
//! close buttons) or the window manager (the X11 tradition).
//!
//! GTK 4 always draws its own decorations on Wayland, and GNOME-style desktops
//! expect it on X11 too.  Elsewhere on X11 (KDE, Xfce, MATE, Cinnamon, LXQt,
//! tiling window managers such as XMonad or i3, …) the window manager provides
//! the title bar, or none, and its own key bindings, so FediGoo leaves the
//! window to it and shows the header bar as an ordinary toolbar without window
//! buttons.  `GTK_CSD=1`, GTK's own switch, asks for client-side decorations
//! anywhere.

use gtk::{gdk, prelude::*};

/// Whether the main window should draw its own title bar, given
/// `$XDG_CURRENT_DESKTOP` and `$GTK_CSD`, on an X11 display or not.
pub fn wants_client_side(x11: bool, current_desktop: Option<&str>, gtk_csd: Option<&str>) -> bool {
    if gtk_csd == Some("1") || !x11 {
        return true;
    }
    // A colon-separated list, most specific first ("ubuntu:GNOME").
    current_desktop.is_some_and(|desktops| {
        desktops.split(':').any(|d| {
            let d = d.trim().to_ascii_lowercase();
            // GNOME, GNOME-Classic, GNOME-Flashback; elementary; Budgie.
            d == "gnome" || d.starts_with("gnome-") || d == "pantheon" || d == "budgie"
        })
    })
}

/// `wants_client_side` for this session.
pub fn client_side(display: &gdk::Display) -> bool {
    // The X11 display's type, without depending on the gdk4-x11 crate.
    let x11 = display.type_().name() == "GdkX11Display";
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").ok();
    let csd = std::env::var("GTK_CSD").ok();
    wants_client_side(x11, desktop.as_deref(), csd.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn x11_leaves_the_frame_to_the_window_manager() {
        for desktop in [
            None,
            Some(""),
            Some("KDE"),
            Some("XFCE"),
            Some("X-Cinnamon"),
            Some("MATE"),
        ] {
            assert!(!wants_client_side(true, desktop, None), "{desktop:?}");
        }
        // Tiling window managers usually set nothing, or their own name.
        assert!(!wants_client_side(true, Some("i3"), None));
    }

    #[test]
    fn gnome_style_desktops_get_client_side_on_x11() {
        for desktop in [
            "GNOME",
            "ubuntu:GNOME",
            "GNOME-Flashback:GNOME",
            "GNOME-Classic",
            "Pantheon",
            "Budgie:GNOME",
        ] {
            assert!(wants_client_side(true, Some(desktop), None), "{desktop}");
        }
    }

    #[test]
    fn wayland_and_gtk_csd_always_get_client_side() {
        assert!(wants_client_side(false, Some("KDE"), None));
        assert!(wants_client_side(false, None, None));
        assert!(wants_client_side(true, Some("KDE"), Some("1")));
        assert!(!wants_client_side(true, Some("KDE"), Some("0")));
    }
}
