//! Compact rows for accounts, follow requests, hashtags and trending links.

use super::{app::App, pages, status::avatar};
use crate::api;
use crate::html;
use crate::json::Value;
use crate::model::{Account, Tag};
use crate::util;
use gtk::{glib, pango, prelude::*};
use std::rc::Rc;

fn base_row(acc: &Account) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    b.add_css_class("account-row");
    b.append(&avatar(&acc.avatar, 40));
    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    let name = gtk::Label::new(None);
    name.set_markup(&format!(
        "<b>{}</b>{}",
        glib::markup_escape_text(acc.name()),
        if acc.locked { " 🔒" } else { "" }
    ));
    name.set_xalign(0.0);
    name.set_ellipsize(pango::EllipsizeMode::End);
    let handle = gtk::Label::new(Some(&format!("@{}", acc.acct)));
    handle.set_xalign(0.0);
    handle.add_css_class("dim-label");
    handle.set_ellipsize(pango::EllipsizeMode::End);
    text.append(&name);
    text.append(&handle);
    let note = html::to_text(&acc.note_html);
    if !note.is_empty() {
        let n = gtk::Label::new(Some(&note.replace('\n', " ")));
        n.set_xalign(0.0);
        n.set_ellipsize(pango::EllipsizeMode::End);
        n.set_lines(1);
        n.add_css_class("caption");
        text.append(&n);
    }
    b.append(&text);
    b
}

pub fn row(app: &Rc<App>, acc: &Account) -> gtk::Box {
    let b = base_row(acc);
    let open = gtk::Button::from_icon_name("go-next-symbolic");
    open.add_css_class("flat");
    open.set_valign(gtk::Align::Center);
    open.set_tooltip_text(Some("View profile"));
    let a = app.clone();
    let acc = acc.clone();
    open.connect_clicked(move |_| pages::open_profile(&a, acc.clone()));
    b.append(&open);
    b
}

pub fn follow_request_row(app: &Rc<App>, acc: &Account) -> gtk::Box {
    let b = row(app, acc);
    for (label, verb, class) in [
        ("Accept", "authorize", "suggested-action"),
        ("Reject", "reject", "destructive-action"),
    ] {
        let btn = gtk::Button::with_label(label);
        btn.add_css_class(class);
        btn.set_valign(gtk::Align::Center);
        let a = app.clone();
        let id = acc.id.clone();
        let row = b.downgrade();
        btn.connect_clicked(move |_| {
            let api = a.api();
            let id = id.clone();
            let row = row.clone();
            let a2 = a.clone();
            a.spawn(async move {
                api.post(&format!("/api/v1/follow_requests/{id}/{verb}"), Value::Null)
                    .await?;
                if let Some(r) = row.upgrade() {
                    r.set_sensitive(false);
                }
                a2.toast(if verb == "authorize" {
                    "Follow request accepted"
                } else {
                    "Follow request rejected"
                });
                Ok(())
            });
        });
        b.append(&btn);
    }
    b
}

pub fn tag_row(tag: &Tag) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    b.add_css_class("account-row");
    let l = gtk::Label::new(None);
    l.set_markup(&format!("<b>#{}</b>", glib::markup_escape_text(&tag.name)));
    l.set_xalign(0.0);
    l.set_hexpand(true);
    b.append(&l);
    if tag.recent_uses > 0 {
        let u = gtk::Label::new(Some(&format!(
            "{} people recently",
            util::count(tag.recent_uses)
        )));
        u.add_css_class("dim-label");
        b.append(&u);
    }
    b
}

pub fn link_row(_app: &Rc<App>, item: &Value) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    b.add_css_class("account-row");
    if let Some(img) = item["image"].nonempty_str() {
        let p = gtk::Picture::new();
        p.set_content_fit(gtk::ContentFit::Cover);
        p.set_can_shrink(true);
        p.set_size_request(72, 72);
        api::load_picture(&p, img);
        let frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
        frame.set_overflow(gtk::Overflow::Hidden);
        frame.add_css_class("card-image");
        frame.append(&p);
        b.append(&frame);
    }
    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    for (s, class) in [
        (item["provider_name"].str(), "dim-label"),
        (item["title"].str(), "heading"),
        (item["description"].str(), "caption"),
    ] {
        if s.is_empty() {
            continue;
        }
        let l = gtk::Label::new(Some(s));
        l.set_xalign(0.0);
        l.set_wrap(true);
        l.set_lines(2);
        l.set_ellipsize(pango::EllipsizeMode::End);
        l.add_css_class(class);
        text.append(&l);
    }
    b.append(&text);
    b
}
