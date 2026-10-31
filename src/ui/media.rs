//! Media attachment thumbnails and the full-size image viewer.

use super::app::App;
use super::fit::FitRow;
use crate::api;
use crate::model::Media;
use gtk::{glib, prelude::*};
use std::cell::Cell;
use std::rc::Rc;

pub fn grid(app: &Rc<App>, media: &[Media], sensitive: bool) -> gtk::Widget {
    let grid = gtk::Box::new(gtk::Orientation::Vertical, 4);
    grid.add_css_class("media-grid");
    let cols = if media.len() == 1 { 1 } else { 2 };
    // Rows share the height limit, so a 2×2 grid is no taller than one image.
    let rows = media.len().div_ceil(cols);
    let fraction = 1.0 / rows as f64;
    let mut row = FitRow::new(cols, fraction);
    for (i, m) in media.iter().enumerate() {
        // Whole image, scaled to fit: the FitRow sizes each button to its
        // image's aspect ratio, so nothing is cropped.
        let pic = gtk::Picture::new();
        pic.set_content_fit(gtk::ContentFit::Contain);
        pic.set_can_shrink(true);
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&pic));
        overlay.set_overflow(gtk::Overflow::Hidden);
        overlay.add_css_class("media-thumb");
        if !m.is_image() {
            let badge = gtk::Label::new(Some(match m.kind.as_str() {
                "gifv" => "GIF",
                "video" => "▶ Video",
                "audio" => "♪ Audio",
                _ => "File",
            }));
            badge.add_css_class("media-badge");
            badge.set_halign(gtk::Align::Start);
            badge.set_valign(gtk::Align::End);
            badge.set_margin_start(6);
            badge.set_margin_bottom(6);
            overlay.add_overlay(&badge);
        }
        if m.description.is_some() {
            let alt = gtk::Label::new(Some("ALT"));
            alt.add_css_class("media-badge");
            alt.set_halign(gtk::Align::End);
            alt.set_valign(gtk::Align::End);
            alt.set_margin_end(6);
            alt.set_margin_bottom(6);
            overlay.add_overlay(&alt);
        }
        let btn = gtk::Button::builder().child(&overlay).build();
        btn.add_css_class("flat");
        btn.add_css_class("media-button");
        btn.set_tooltip_text(Some(m.description.as_deref().unwrap_or("No description")));
        let a = app.clone();
        let all = media.to_vec();
        btn.connect_clicked(move |_| open(&a, &all, i));
        if i % cols == 0 {
            row = FitRow::new(cols, fraction);
            grid.append(&row);
        }
        row.append(&btn, m.aspect);
        let (pic_w, row_w, btn_w) = (pic.downgrade(), row.downgrade(), btn.downgrade());
        api::with_texture(&m.preview_url, move |t| {
            if let (Some(r), Some(b)) = (row_w.upgrade(), btn_w.upgrade()) {
                r.set_aspect_if_unknown(&b, t.width() as f64 / t.height().max(1) as f64);
            }
            if let Some(p) = pic_w.upgrade() {
                p.set_paintable(Some(t));
            }
        });
    }

    if !sensitive || app.config.borrow().settings.show_sensitive_media {
        return grid.upcast();
    }
    // Hide sensitive media behind a click-through cover.
    let stack = gtk::Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    let cover = gtk::Button::with_label("Sensitive content — click to show");
    cover.add_css_class("sensitive-cover");
    cover.set_size_request(-1, 120);
    stack.add_child(&cover);
    stack.add_child(&grid);
    let hide = gtk::Button::from_icon_name("view-conceal-symbolic");
    hide.set_tooltip_text(Some("Hide media"));
    hide.add_css_class("osd");
    hide.set_halign(gtk::Align::End);
    hide.set_valign(gtk::Align::Start);
    hide.set_margin_top(6);
    hide.set_margin_end(6);
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&stack));
    overlay.add_overlay(&hide);
    hide.set_visible(false);
    cover.connect_clicked(glib::clone!(
        #[weak]
        stack,
        #[weak]
        grid,
        #[weak]
        hide,
        move |_| {
            stack.set_visible_child(&grid);
            hide.set_visible(true);
        }
    ));
    hide.connect_clicked(glib::clone!(
        #[weak]
        stack,
        #[weak]
        cover,
        move |h| {
            stack.set_visible_child(&cover);
            h.set_visible(false);
        }
    ));
    overlay.upcast()
}

/// Opens attachment `index` of `media`: images in the built-in viewer,
/// everything else in the default external application.
pub fn open(app: &Rc<App>, media: &[Media], index: usize) {
    let Some(m) = media.get(index) else { return };
    if !m.is_image() {
        app.open_url(&m.url);
        return;
    }
    let images: Vec<Media> = media.iter().filter(|m| m.is_image()).cloned().collect();
    let start = images.iter().position(|x| x.id == m.id).unwrap_or(0);

    let win = gtk::Window::builder()
        .title("Image")
        .transient_for(&app.window)
        .default_width(1000)
        .default_height(760)
        .build();
    let pic = gtk::Picture::new();
    pic.set_content_fit(gtk::ContentFit::Contain);
    pic.set_can_shrink(true);
    pic.set_vexpand(true);
    pic.add_css_class("viewer");
    let alt = gtk::Label::new(None);
    alt.set_wrap(true);
    alt.set_selectable(true);
    alt.set_margin_top(6);
    alt.set_margin_bottom(6);
    alt.set_margin_start(12);
    alt.set_margin_end(12);
    let prev = gtk::Button::from_icon_name("go-previous-symbolic");
    let next = gtk::Button::from_icon_name("go-next-symbolic");
    let counter = gtk::Label::new(None);
    let browser = gtk::Button::with_label("Open in browser");
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    bar.set_margin_start(8);
    bar.set_margin_end(8);
    bar.set_margin_bottom(8);
    bar.append(&prev);
    bar.append(&counter);
    bar.append(&next);
    let sp = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    sp.set_hexpand(true);
    bar.append(&sp);
    bar.append(&browser);
    let vb = gtk::Box::new(gtk::Orientation::Vertical, 0);
    vb.append(&pic);
    vb.append(&alt);
    vb.append(&bar);
    win.set_child(Some(&vb));

    let images = Rc::new(images);
    let idx = Rc::new(Cell::new(start));
    let show = {
        let images = images.clone();
        let idx = idx.clone();
        let (pic, alt, counter, prev, next) = (
            pic.clone(),
            alt.clone(),
            counter.clone(),
            prev.clone(),
            next.clone(),
        );
        Rc::new(move || {
            let i = idx.get();
            let m = &images[i];
            pic.set_paintable(None::<&gtk::gdk::Paintable>);
            // Show the cached preview first, then swap in the full image.
            api::load_picture(&pic, &m.preview_url);
            api::load_picture(&pic, &m.url);
            alt.set_text(m.description.as_deref().unwrap_or(""));
            alt.set_visible(m.description.is_some());
            counter.set_text(&format!("{} / {}", i + 1, images.len()));
            prev.set_sensitive(i > 0);
            next.set_sensitive(i + 1 < images.len());
        })
    };
    show();
    let (s, i, n) = (show.clone(), idx.clone(), images.len());
    prev.connect_clicked(move |_| {
        if i.get() > 0 {
            i.set(i.get() - 1);
            s();
        }
    });
    let (s, i) = (show.clone(), idx.clone());
    next.connect_clicked(move |_| {
        if i.get() + 1 < n {
            i.set(i.get() + 1);
            s();
        }
    });
    let a = app.clone();
    let (imgs, i) = (images.clone(), idx.clone());
    browser.connect_clicked(move |_| {
        let m = &imgs[i.get()];
        a.open_url(m.remote_url.as_deref().unwrap_or(&m.url));
    });
    let keys = gtk::EventControllerKey::new();
    let (s, i) = (show.clone(), idx.clone());
    keys.connect_key_pressed(glib::clone!(
        #[weak]
        win,
        #[upgrade_or]
        glib::Propagation::Proceed,
        move |_, key, _, _| {
            use gtk::gdk::Key;
            match key {
                Key::Escape => win.close(),
                Key::Left if i.get() > 0 => {
                    i.set(i.get() - 1);
                    s();
                }
                Key::Right if i.get() + 1 < n => {
                    i.set(i.get() + 1);
                    s();
                }
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        }
    ));
    win.add_controller(keys);
    win.present();
}
