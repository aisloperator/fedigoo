//! Preferences window.

use super::app::App;
use crate::config;
use gtk::{glib, prelude::*};
use std::rc::Rc;

pub fn open(app: &Rc<App>) {
    let win = gtk::Window::builder()
        .title("Preferences")
        .transient_for(&app.window)
        .modal(true)
        .default_width(420)
        .resizable(false)
        .build();
    let vb = gtk::Box::new(gtk::Orientation::Vertical, 10);
    vb.set_margin_top(18);
    vb.set_margin_bottom(18);
    vb.set_margin_start(18);
    vb.set_margin_end(18);
    win.set_child(Some(&vb));

    let s = app.config.borrow().settings.clone();
    let opts: [(&str, bool, fn(&mut config::Settings, bool)); 3] = [
        (
            "Always show media marked as sensitive",
            s.show_sensitive_media,
            |s, v| s.show_sensitive_media = v,
        ),
        (
            "Always expand posts with content warnings",
            s.expand_content_warnings,
            |s, v| s.expand_content_warnings = v,
        ),
        ("Live updates (streaming)", s.stream_updates, |s, v| {
            s.stream_updates = v
        }),
    ];
    for (label, value, set) in opts {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let l = gtk::Label::new(Some(label));
        l.set_xalign(0.0);
        l.set_hexpand(true);
        l.set_wrap(true);
        let sw = gtk::Switch::new();
        sw.set_active(value);
        sw.set_valign(gtk::Align::Center);
        let a = app.clone();
        sw.connect_active_notify(move |sw| {
            let streaming_changed = {
                let mut c = a.config.borrow_mut();
                let before = c.settings.stream_updates;
                set(&mut c.settings, sw.is_active());
                c.save();
                before != c.settings.stream_updates
            };
            if streaming_changed {
                a.restart_stream();
            }
        });
        row.append(&l);
        row.append(&sw);
        vb.append(&row);
    }
    let note = gtk::Label::new(Some(&format!(
        "Settings and accounts are stored in {}",
        config::dir().display()
    )));
    note.add_css_class("dim-label");
    note.add_css_class("caption");
    note.set_wrap(true);
    note.set_xalign(0.0);
    note.set_margin_top(8);
    vb.append(&note);
    let close = gtk::Button::with_label("Close");
    close.set_halign(gtk::Align::End);
    close.connect_clicked(glib::clone!(
        #[weak]
        win,
        move |_| win.close()
    ));
    vb.append(&close);
    win.present();
}
