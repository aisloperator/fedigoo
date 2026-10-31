//! FediGoo — a GTK 4 Mastodon client for the Linux desktop.

// Callback-heavy GTK code reads better with inline closure types.
#![allow(clippy::type_complexity)]

mod api;
mod config;
mod html;
mod json;
mod model;
mod ui;
mod util;

use gtk::{glib, prelude::*};
use std::cell::RefCell;
use std::rc::Rc;

pub const APP_ID: &str = "com.aisloperator.FediGoo";

fn usage() {
    println!(
        "Usage: fedigoo [--x11 | --wayland]\n\n  \
         --x11       use the X11 backend\n  \
         --wayland   use the Wayland backend\n\n\
         Without an option GTK picks the backend (GDK_BACKEND is honoured).\n\
         Data is stored in {}",
        config::dir().display()
    );
}

fn main() -> glib::ExitCode {
    let mut gtk_args = Vec::new();
    for arg in std::env::args() {
        let backend = match arg.as_str() {
            "--x11" => Some("x11"),
            "--wayland" => Some("wayland"),
            "-h" | "--help" => {
                usage();
                return glib::ExitCode::SUCCESS;
            }
            _ => None,
        };
        match backend {
            // SAFETY: still single-threaded; GTK has not been initialised.
            Some(b) => unsafe { std::env::set_var("GDK_BACKEND", b) },
            None => gtk_args.push(arg),
        }
    }

    // Embedded resources (the app icon); GtkApplication picks up the icons
    // under /com/aisloperator/FediGoo/icons from the application ID.
    let resource = gtk::gio::Resource::from_data(&glib::Bytes::from_static(include_bytes!(
        concat!(env!("OUT_DIR"), "/fedigoo.gresource")
    )))
    .expect("embedded resources are valid");
    gtk::gio::resources_register(&resource);

    let app = gtk::Application::builder().application_id(APP_ID).build();
    let state: Rc<RefCell<Option<Rc<ui::app::App>>>> = Rc::default();
    app.connect_activate(move |gtk_app| {
        // A second launch just raises the existing window.
        if let Some(a) = state.borrow().as_ref() {
            a.window.present();
            return;
        }
        gtk::Window::set_default_icon_name(APP_ID);
        let a = ui::app::App::new(gtk_app);
        a.window.present();
        *state.borrow_mut() = Some(a);
    });
    app.run_with_args(&gtk_args)
}
