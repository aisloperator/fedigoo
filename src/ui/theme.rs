//! Follows the desktop's light/dark preference.
//!
//! GTK 4.18 without libadwaita doesn't read the modern "color-scheme"
//! setting itself, so we read it from the XDG desktop portal (works on any
//! desktop, X11 or Wayland), falling back to GNOME's GSettings key, and map
//! it to `gtk-application-prefer-dark-theme`.  Both are watched, so the app
//! switches when the desktop does.  "No preference" keeps GTK's own
//! setting (e.g. from settings.ini).

use gtk::{gio, glib, prelude::*};
use std::cell::{Cell, RefCell};

const PORTAL_NS: &str = "org.freedesktop.appearance";
const GNOME_SCHEMA: &str = "org.gnome.desktop.interface";

thread_local! {
    /// GTK's own dark preference at startup, restored for "no preference".
    static INITIAL_DARK: Cell<bool> = const { Cell::new(false) };
    /// Keeps the portal proxy or GSettings object (and so its signal) alive.
    static WATCH: RefCell<Option<glib::Object>> = const { RefCell::new(None) };
}

/// Starts following the desktop preference.  Call once, after GTK init.
pub fn follow_desktop() {
    if let Some(settings) = gtk::Settings::default() {
        INITIAL_DARK.with(|d| d.set(settings.is_gtk_application_prefer_dark_theme()));
    }
    gio::DBusProxy::for_bus(
        gio::BusType::Session,
        // Starting the portal here would block for the D-Bus default
        // timeout if it never comes up; the read below has its own timeout.
        gio::DBusProxyFlags::DO_NOT_LOAD_PROPERTIES
            | gio::DBusProxyFlags::DO_NOT_AUTO_START_AT_CONSTRUCTION,
        None,
        "org.freedesktop.portal.Desktop",
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.Settings",
        None::<&gio::Cancellable>,
        |res| match res {
            Ok(proxy) => read_portal(proxy),
            Err(_) => follow_gsettings(),
        },
    );
}

fn read_portal(proxy: gio::DBusProxy) {
    read_portal_with(&proxy.clone(), "ReadOne", move |res| match res {
        Ok(()) => watch_portal(proxy),
        // Portals before version 2 only have the deprecated Read.
        Err(e) if e.matches(gio::DBusError::UnknownMethod) => {
            read_portal_with(&proxy.clone(), "Read", move |res| match res {
                Ok(()) => watch_portal(proxy),
                Err(_) => follow_gsettings(),
            })
        }
        Err(_) => follow_gsettings(),
    });
}

/// Calls `method` for the colour scheme and applies it.
fn read_portal_with(
    proxy: &gio::DBusProxy,
    method: &str,
    done: impl FnOnce(Result<(), glib::Error>) + 'static,
) {
    proxy.call(
        method,
        Some(&(PORTAL_NS, "color-scheme").to_variant()),
        gio::DBusCallFlags::NONE,
        2000,
        None::<&gio::Cancellable>,
        move |r| {
            done(r.and_then(|v| {
                let scheme = scheme_of(&v).ok_or_else(|| {
                    glib::Error::new(gio::IOErrorEnum::InvalidData, "bad color-scheme value")
                })?;
                apply(scheme);
                Ok(())
            }))
        },
    );
}

fn watch_portal(proxy: gio::DBusProxy) {
    proxy.connect_g_signal(|_, _, signal, params| {
        if signal == "SettingChanged"
            && let Some((ns, key, value)) = params.get::<(String, String, glib::Variant)>()
            && ns == PORTAL_NS
            && key == "color-scheme"
            && let Some(scheme) = scheme_of(&value)
        {
            apply(scheme);
        }
    });
    WATCH.with(|w| *w.borrow_mut() = Some(proxy.upcast()));
}

/// The portal's value (0 = no preference, 1 = dark, 2 = light), unwrapping
/// the variant boxes that `Read`/`ReadOne` put around it.
fn scheme_of(v: &glib::Variant) -> Option<u32> {
    let mut v = v.clone();
    loop {
        if let Some(n) = v.get::<u32>() {
            return Some(n);
        }
        v = if v.is_container() && v.n_children() == 1 {
            v.child_value(0)
        } else {
            v.as_variant()?
        };
    }
}

fn follow_gsettings() {
    let has_key = gio::SettingsSchemaSource::default()
        .and_then(|src| src.lookup(GNOME_SCHEMA, true))
        .is_some_and(|schema| schema.has_key("color-scheme"));
    if !has_key {
        return;
    }
    let settings = gio::Settings::new(GNOME_SCHEMA);
    let update = |s: &gio::Settings| {
        apply(match s.string("color-scheme").as_str() {
            "prefer-dark" => 1,
            "prefer-light" => 2,
            _ => 0,
        })
    };
    update(&settings);
    settings.connect_changed(Some("color-scheme"), move |s, _| update(s));
    WATCH.with(|w| *w.borrow_mut() = Some(settings.upcast()));
}

fn apply(scheme: u32) {
    let Some(settings) = gtk::Settings::default() else {
        return;
    };
    let dark = match scheme {
        1 => true,
        2 => false,
        _ => INITIAL_DARK.with(Cell::get),
    };
    if settings.is_gtk_application_prefer_dark_theme() != dark {
        settings.set_gtk_application_prefer_dark_theme(dark);
    }
}
