//! The post composer: new posts, replies, edits and re-drafts.

use super::app::App;
use crate::api;
use crate::html;
use crate::json::{Value, obj};
use crate::model::{Media, Status};
use crate::util;
use gtk::{gdk, gio, glib, pango, prelude::*};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub enum Mode {
    New,
    Reply(Status),
    Edit(Status),
    Mention(String),
    /// A just-deleted status and the server's delete response (which carries
    /// the source text).
    Redraft(Box<Status>, Value),
}

const VISIBILITIES: [&str; 4] = ["public", "unlisted", "private", "direct"];
const DURATIONS: [(&str, i64); 8] = [
    ("5 minutes", 300),
    ("30 minutes", 1800),
    ("1 hour", 3600),
    ("6 hours", 21600),
    ("12 hours", 43200),
    ("1 day", 86400),
    ("3 days", 259200),
    ("7 days", 604800),
];

/// An attachment in the composer: being uploaded, or uploaded with an id.
struct Attachment {
    id: RefCell<Option<String>>,
    original_alt: RefCell<String>,
    alt: gtk::Entry,
    widget: gtk::Box,
}

struct Composer {
    app: Rc<App>,
    win: gtk::Window,
    text: gtk::TextView,
    cw_toggle: gtk::ToggleButton,
    cw: gtk::Entry,
    vis: gtk::DropDown,
    lang: gtk::Entry,
    sensitive: gtk::CheckButton,
    counter: gtk::Label,
    post_btn: gtk::Button,
    attach_box: gtk::Box,
    attachments: RefCell<Vec<Rc<Attachment>>>,
    poll_toggle: gtk::ToggleButton,
    poll_box: gtk::Box,
    poll_options: gtk::Box,
    poll_duration: gtk::DropDown,
    poll_multiple: gtk::CheckButton,
    reply_to: Option<String>,
    edit_id: Option<String>,
    uploading: Cell<u32>,
}

pub fn open(app: &Rc<App>, mode: Mode) {
    if app.config.borrow().active_account().is_none() {
        return;
    }
    let title = match &mode {
        Mode::Reply(_) => "Reply",
        Mode::Edit(_) => "Edit post",
        Mode::Redraft(..) => "Re-draft post",
        _ => "New post",
    };
    let win = gtk::Window::builder()
        .title(title)
        .transient_for(&app.window)
        .default_width(620)
        .default_height(480)
        .build();
    let vb = gtk::Box::new(gtk::Orientation::Vertical, 8);
    vb.set_margin_top(12);
    vb.set_margin_bottom(12);
    vb.set_margin_start(12);
    vb.set_margin_end(12);
    win.set_child(Some(&vb));

    if let Mode::Reply(s) = &mode {
        let l = gtk::Label::new(None);
        l.set_markup(&format!(
            "<small>Replying to <b>@{}</b>: {}</small>",
            glib::markup_escape_text(&s.account.acct),
            glib::markup_escape_text(&html::to_text(&s.content_html).replace('\n', " "))
        ));
        l.set_ellipsize(pango::EllipsizeMode::End);
        l.set_xalign(0.0);
        l.add_css_class("dim-label");
        vb.append(&l);
    }

    let cw = gtk::Entry::new();
    cw.set_placeholder_text(Some("Content warning"));
    cw.set_visible(false);
    vb.append(&cw);

    let text = gtk::TextView::new();
    text.set_wrap_mode(gtk::WrapMode::WordChar);
    text.set_top_margin(8);
    text.set_bottom_margin(8);
    text.set_left_margin(8);
    text.set_right_margin(8);
    text.add_css_class("compose-text");
    let sw = gtk::ScrolledWindow::builder()
        .child(&text)
        .vexpand(true)
        .min_content_height(140)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();
    sw.add_css_class("frame");
    vb.append(&sw);

    let attach_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    attach_box.set_visible(false);
    vb.append(&attach_box);

    let poll_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
    poll_box.add_css_class("poll-editor");
    poll_box.set_visible(false);
    let poll_options = gtk::Box::new(gtk::Orientation::Vertical, 4);
    poll_box.append(&poll_options);
    let prow = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let add_opt = gtk::Button::with_label("Add option");
    let poll_duration = gtk::DropDown::from_strings(&DURATIONS.map(|d| d.0));
    poll_duration.set_selected(5);
    let poll_multiple = gtk::CheckButton::with_label("Multiple choice");
    prow.append(&add_opt);
    prow.append(&poll_duration);
    prow.append(&poll_multiple);
    poll_box.append(&prow);
    vb.append(&poll_box);

    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let attach_btn = gtk::Button::from_icon_name("mail-attachment-symbolic");
    attach_btn.set_tooltip_text(Some("Add images, video or audio"));
    let poll_toggle = gtk::ToggleButton::new();
    poll_toggle.set_icon_name("view-list-bullet-symbolic");
    poll_toggle.set_tooltip_text(Some("Add a poll"));
    let cw_toggle = gtk::ToggleButton::with_label("CW");
    cw_toggle.set_tooltip_text(Some("Add a content warning"));
    let vis = gtk::DropDown::from_strings(&VISIBILITIES.map(util::visibility_label));
    vis.set_tooltip_text(Some("Who can see this post"));
    let lang = gtk::Entry::new();
    lang.set_max_length(3);
    lang.set_width_chars(3);
    lang.set_max_width_chars(4);
    lang.set_placeholder_text(Some("lang"));
    lang.set_tooltip_text(Some("Language code, e.g. en (optional)"));
    let sensitive = gtk::CheckButton::with_label("Sensitive");
    sensitive.set_tooltip_text(Some("Mark media as sensitive"));
    sensitive.set_visible(false);
    let counter = gtk::Label::new(None);
    counter.set_hexpand(true);
    counter.set_xalign(1.0);
    let post_btn = gtk::Button::with_label(if matches!(mode, Mode::Edit(_)) {
        "Save"
    } else {
        "Post"
    });
    post_btn.add_css_class("suggested-action");
    post_btn.set_tooltip_text(Some("Ctrl+Enter"));
    for w in [
        attach_btn.upcast_ref::<gtk::Widget>(),
        poll_toggle.upcast_ref(),
        cw_toggle.upcast_ref(),
        vis.upcast_ref(),
        lang.upcast_ref(),
        sensitive.upcast_ref(),
        counter.upcast_ref(),
        post_btn.upcast_ref(),
    ] {
        bar.append(w);
    }
    vb.append(&bar);

    // Default visibility: the account's preference.
    let default_vis = app
        .me
        .borrow()
        .as_ref()
        .and_then(|m| m.source_privacy.clone())
        .unwrap_or_else(|| "public".into());

    let c = Rc::new(Composer {
        app: app.clone(),
        win: win.clone(),
        text,
        cw_toggle,
        cw,
        vis,
        lang,
        sensitive,
        counter,
        post_btn,
        attach_box,
        attachments: RefCell::new(Vec::new()),
        poll_toggle,
        poll_box,
        poll_options,
        poll_duration,
        poll_multiple,
        reply_to: match &mode {
            Mode::Reply(s) => Some(s.id.clone()),
            Mode::Redraft(s, _) => s.in_reply_to_id.clone(),
            _ => None,
        },
        edit_id: match &mode {
            Mode::Edit(s) => Some(s.id.clone()),
            _ => None,
        },
        uploading: Cell::new(0),
    });
    c.set_visibility(&default_vis);
    c.sensitive
        .set_active(app.me.borrow().as_ref().is_some_and(|m| m.source_sensitive));

    // Prefill from the mode.
    match &mode {
        Mode::New => {}
        Mode::Mention(acct) => c.text.buffer().set_text(&format!("@{acct} ")),
        Mode::Reply(s) => {
            let me = app
                .config
                .borrow()
                .active_account()
                .map(|a| a.acct.clone())
                .unwrap_or_default();
            let my_id = app.my_id();
            let mut handles = Vec::new();
            if s.account.id != my_id {
                handles.push(format!("@{}", s.account.acct));
            }
            for m in &s.mentions {
                let h = format!("@{}", m.acct);
                if m.id != my_id && m.acct != me && !handles.contains(&h) {
                    handles.push(h);
                }
            }
            if !handles.is_empty() {
                c.text.buffer().set_text(&format!("{} ", handles.join(" ")));
            }
            c.set_visibility(&s.visibility);
            if !s.spoiler_text.is_empty() {
                c.cw_toggle.set_active(true);
                c.cw.set_text(&s.spoiler_text);
            }
            if let Some(l) = &s.language {
                c.lang.set_text(l);
            }
        }
        Mode::Edit(s) => {
            c.set_visibility(&s.visibility);
            c.vis.set_sensitive(false);
            c.fill_from(s, None);
            // Fetch the plain-text source for faithful editing.
            let api = app.api();
            let id = s.id.clone();
            let cw = Rc::downgrade(&c);
            app.spawn(async move {
                let v = api
                    .get(&format!("/api/v1/statuses/{id}/source"), &[])
                    .await?;
                if let Some(c) = cw.upgrade() {
                    c.text.buffer().set_text(v["text"].str());
                    let sp = v["spoiler_text"].str();
                    c.cw.set_text(sp);
                    c.cw_toggle.set_active(!sp.is_empty());
                }
                Ok(())
            });
        }
        Mode::Redraft(s, deleted) => {
            c.set_visibility(&s.visibility);
            c.fill_from(s, deleted["text"].opt_str());
        }
    }

    // Wiring.
    let w = Rc::downgrade(&c);
    c.text.buffer().connect_changed(move |_| {
        if let Some(c) = w.upgrade() {
            c.update_counter();
        }
    });
    let w = Rc::downgrade(&c);
    c.cw.connect_changed(move |_| {
        if let Some(c) = w.upgrade() {
            c.update_counter();
        }
    });
    let w = Rc::downgrade(&c);
    c.cw_toggle.connect_toggled(move |t| {
        if let Some(c) = w.upgrade() {
            c.cw.set_visible(t.is_active());
            if t.is_active() {
                c.cw.grab_focus();
            }
            c.update_counter();
        }
    });
    let w = Rc::downgrade(&c);
    c.poll_toggle.connect_toggled(move |t| {
        if let Some(c) = w.upgrade() {
            if t.is_active() && !c.attachments.borrow().is_empty() {
                c.app.toast("A post can't have both a poll and attachments");
                t.set_active(false);
                return;
            }
            c.poll_box.set_visible(t.is_active());
            if t.is_active() && c.poll_options.first_child().is_none() {
                c.add_poll_option("");
                c.add_poll_option("");
            }
        }
    });
    let w = Rc::downgrade(&c);
    add_opt.connect_clicked(move |_| {
        if let Some(c) = w.upgrade() {
            c.add_poll_option("");
        }
    });
    let w = Rc::downgrade(&c);
    attach_btn.connect_clicked(move |_| {
        if let Some(c) = w.upgrade() {
            c.pick_files();
        }
    });
    let w = Rc::downgrade(&c);
    c.post_btn.connect_clicked(move |_| {
        if let Some(c) = w.upgrade() {
            c.submit();
        }
    });
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let w = Rc::downgrade(&c);
    keys.connect_key_pressed(move |_, key, _, mods| {
        let Some(c) = w.upgrade() else {
            return glib::Propagation::Proceed;
        };
        if (key == gdk::Key::Return || key == gdk::Key::KP_Enter)
            && mods.contains(gdk::ModifierType::CONTROL_MASK)
        {
            c.submit();
            return glib::Propagation::Stop;
        }
        if key == gdk::Key::Escape {
            c.close_asking();
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    win.add_controller(keys);
    // Paste images straight from the clipboard.
    let w = Rc::downgrade(&c);
    c.text.connect_paste_clipboard(move |tv| {
        let Some(c) = w.upgrade() else { return };
        let clip = tv.clipboard();
        if clip.formats().contains_type(gdk::Texture::static_type()) {
            tv.stop_signal_emission_by_name("paste-clipboard");
            let cw = Rc::downgrade(&c);
            glib::spawn_future_local(async move {
                if let (Ok(Some(t)), Some(c)) = (clip.read_texture_future().await, cw.upgrade()) {
                    c.add_upload(
                        "pasted.png".into(),
                        "image/png".into(),
                        t.save_to_png_bytes(),
                    );
                }
            });
        }
    });
    // The window's handler owns the composer, so it lives as long as the
    // window does; the handlers are dropped when the window is destroyed.
    let keep = c.clone();
    win.connect_close_request(move |_| {
        let _ = &keep;
        glib::Propagation::Proceed
    });

    c.update_counter();
    win.present();
    c.text.grab_focus();
    let buf = c.text.buffer();
    buf.place_cursor(&buf.end_iter());
}

impl Composer {
    fn set_visibility(&self, v: &str) {
        if let Some(i) = VISIBILITIES.iter().position(|x| *x == v) {
            self.vis.set_selected(i as u32);
        }
    }

    fn fill_from(self: &Rc<Self>, s: &Status, source_text: Option<&str>) {
        let text = source_text
            .map(str::to_string)
            .unwrap_or_else(|| html::to_text(&s.content_html));
        self.text.buffer().set_text(&text);
        if !s.spoiler_text.is_empty() {
            self.cw_toggle.set_active(true);
            self.cw.set_text(&s.spoiler_text);
        }
        self.sensitive.set_active(s.sensitive);
        if let Some(l) = &s.language {
            self.lang.set_text(l);
        }
        for m in &s.media {
            self.add_existing(m);
        }
        if let Some(p) = &s.poll {
            self.poll_toggle.set_active(true);
            while let Some(c) = self.poll_options.first_child() {
                self.poll_options.remove(&c);
            }
            for o in &p.options {
                self.add_poll_option(&o.title);
            }
            self.poll_multiple.set_active(p.multiple);
        }
    }

    fn text(&self) -> String {
        let b = self.text.buffer();
        b.text(&b.start_iter(), &b.end_iter(), false).to_string()
    }

    fn remaining(&self) -> i64 {
        let mut n = util::post_length(&self.text());
        if self.cw_toggle.is_active() {
            n += self.cw.text().chars().count();
        }
        self.app.max_chars.get() as i64 - n as i64
    }

    fn update_counter(&self) {
        let left = self.remaining();
        self.counter.set_text(&left.to_string());
        if left < 0 {
            self.counter.add_css_class("error");
        } else {
            self.counter.remove_css_class("error");
        }
        self.post_btn
            .set_sensitive(left >= 0 && self.uploading.get() == 0);
    }

    fn add_poll_option(self: &Rc<Self>, initial: &str) {
        let count = {
            let mut n = 0;
            let mut c = self.poll_options.first_child();
            while let Some(w) = c {
                n += 1;
                c = w.next_sibling();
            }
            n
        };
        if count >= self.app.max_poll_options.get() {
            self.app.toast("Maximum number of poll options reached");
            return;
        }
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        let e = gtk::Entry::new();
        e.set_placeholder_text(Some(&format!("Option {}", count + 1)));
        e.set_text(initial);
        e.set_hexpand(true);
        let rm = gtk::Button::from_icon_name("list-remove-symbolic");
        rm.add_css_class("flat");
        let opts = self.poll_options.downgrade();
        rm.connect_clicked(glib::clone!(
            #[weak]
            row,
            move |_| {
                if let Some(o) = opts.upgrade() {
                    o.remove(&row);
                }
            }
        ));
        row.append(&e);
        row.append(&rm);
        self.poll_options.append(&row);
    }

    fn poll_option_texts(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut c = self.poll_options.first_child();
        while let Some(row) = c {
            if let Some(e) = row.first_child().and_downcast::<gtk::Entry>() {
                let t = e.text().trim().to_string();
                if !t.is_empty() {
                    out.push(t);
                }
            }
            c = row.next_sibling();
        }
        out
    }

    fn attachment_tile(self: &Rc<Self>, alt_text: &str) -> Rc<Attachment> {
        let widget = gtk::Box::new(gtk::Orientation::Vertical, 4);
        widget.add_css_class("attachment");
        let pic = gtk::Picture::new();
        pic.set_content_fit(gtk::ContentFit::Cover);
        pic.set_can_shrink(true);
        pic.set_size_request(110, 80);
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&pic));
        let spinner = gtk::Spinner::new();
        spinner.set_halign(gtk::Align::Center);
        spinner.set_valign(gtk::Align::Center);
        overlay.add_overlay(&spinner);
        let rm = gtk::Button::from_icon_name("window-close-symbolic");
        rm.add_css_class("osd");
        rm.add_css_class("circular");
        rm.set_halign(gtk::Align::End);
        rm.set_valign(gtk::Align::Start);
        overlay.add_overlay(&rm);
        widget.append(&overlay);
        let alt = gtk::Entry::new();
        alt.set_placeholder_text(Some("Alt text"));
        alt.set_text(alt_text);
        alt.set_width_chars(12);
        alt.set_tooltip_text(Some("Describe this for people with visual impairments"));
        widget.append(&alt);
        let att = Rc::new(Attachment {
            id: RefCell::new(None),
            original_alt: RefCell::new(alt_text.to_string()),
            alt,
            widget: widget.clone(),
        });
        let w = Rc::downgrade(self);
        let aw = Rc::downgrade(&att);
        rm.connect_clicked(move |_| {
            if let (Some(c), Some(a)) = (w.upgrade(), aw.upgrade()) {
                c.attachments.borrow_mut().retain(|x| !Rc::ptr_eq(x, &a));
                c.attach_box.remove(&a.widget);
                c.sync_attach_ui();
            }
        });
        self.attach_box.append(&widget);
        self.attachments.borrow_mut().push(att.clone());
        self.sync_attach_ui();
        att
    }

    fn sync_attach_ui(&self) {
        let any = !self.attachments.borrow().is_empty();
        self.attach_box.set_visible(any);
        self.sensitive.set_visible(any);
    }

    fn add_existing(self: &Rc<Self>, m: &Media) {
        let att = self.attachment_tile(m.description.as_deref().unwrap_or(""));
        *att.id.borrow_mut() = Some(m.id.clone());
        if let Some(pic) = att
            .widget
            .first_child()
            .and_downcast::<gtk::Overlay>()
            .and_then(|o| o.child())
            .and_downcast::<gtk::Picture>()
        {
            api::load_picture(&pic, &m.preview_url);
        }
    }

    fn pick_files(self: &Rc<Self>) {
        if self.poll_toggle.is_active() {
            self.app
                .toast("A post can't have both a poll and attachments");
            return;
        }
        let filter = gtk::FileFilter::new();
        for t in ["image/*", "video/*", "audio/*"] {
            filter.add_mime_type(t);
        }
        filter.set_name(Some("Media"));
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        let dlg = gtk::FileDialog::builder()
            .title("Attach media")
            .filters(&filters)
            .modal(true)
            .build();
        let w = Rc::downgrade(self);
        let win = self.win.clone();
        glib::spawn_future_local(async move {
            let Ok(files) = dlg.open_multiple_future(Some(&win)).await else {
                return;
            };
            for f in files.iter::<gio::File>().flatten() {
                let Some(c) = w.upgrade() else { return };
                if c.attachments.borrow().len() >= c.app.max_media.get() {
                    c.app.toast("Maximum number of attachments reached");
                    break;
                }
                match f.load_bytes_future().await {
                    Ok((bytes, _)) => {
                        let name = f
                            .basename()
                            .map(|p| p.to_string_lossy().to_string())
                            .unwrap_or_else(|| "file".into());
                        let (ctype, _) = gio::content_type_guess(Some(&name), &bytes[..]);
                        let mime = gio::content_type_get_mime_type(&ctype)
                            .map(|m| m.to_string())
                            .unwrap_or_else(|| "application/octet-stream".into());
                        c.add_upload(name, mime, bytes);
                    }
                    Err(e) => c.app.toast(&e.to_string()),
                }
            }
        });
    }

    fn add_upload(self: &Rc<Self>, name: String, mime: String, bytes: glib::Bytes) {
        let att = self.attachment_tile("");
        let overlay = att.widget.first_child().and_downcast::<gtk::Overlay>();
        let pic = overlay
            .as_ref()
            .and_then(|o| o.child())
            .and_downcast::<gtk::Picture>();
        if let (Some(p), Ok(t)) = (&pic, gdk::Texture::from_bytes(&bytes)) {
            p.set_paintable(Some(&t));
        }
        let spinner = overlay
            .as_ref()
            .and_then(|o| o.last_child())
            .and_then(|w| w.prev_sibling())
            .and_downcast::<gtk::Spinner>();
        if let Some(s) = &spinner {
            s.start();
        }
        self.uploading.set(self.uploading.get() + 1);
        self.update_counter();
        let api = self.app.api();
        let w = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let res = api.upload_media(&name, &mime, &bytes, "").await;
            if let Some(s) = &spinner {
                s.stop();
                s.set_visible(false);
            }
            let Some(c) = w.upgrade() else { return };
            c.uploading.set(c.uploading.get() - 1);
            match res {
                Ok(v) => *att.id.borrow_mut() = Some(v["id"].str().to_string()),
                Err(e) => {
                    c.app.toast(&format!("Upload failed: {e}"));
                    c.attachments.borrow_mut().retain(|x| !Rc::ptr_eq(x, &att));
                    c.attach_box.remove(&att.widget);
                    c.sync_attach_ui();
                }
            }
            c.update_counter();
        });
    }

    fn is_dirty(&self) -> bool {
        !self.text().trim().is_empty() || !self.attachments.borrow().is_empty()
    }

    fn close_asking(self: &Rc<Self>) {
        if !self.is_dirty() {
            self.win.close();
            return;
        }
        let dlg = gtk::AlertDialog::builder()
            .message("Discard this post?")
            .buttons(["Keep editing", "Discard"])
            .cancel_button(0)
            .default_button(0)
            .build();
        let win = self.win.clone();
        glib::spawn_future_local(async move {
            if dlg.choose_future(Some(&win)).await == Ok(1) {
                win.close();
            }
        });
    }

    fn submit(self: &Rc<Self>) {
        if !self.post_btn.is_sensitive() {
            return;
        }
        let text = self.text();
        let attachments: Vec<Rc<Attachment>> = self.attachments.borrow().clone();
        let poll_opts = if self.poll_toggle.is_active() {
            self.poll_option_texts()
        } else {
            vec![]
        };
        if text.trim().is_empty() && attachments.is_empty() {
            self.app.toast("Write something first");
            return;
        }
        if self.poll_toggle.is_active() && poll_opts.len() < 2 {
            self.app.toast("A poll needs at least two options");
            return;
        }
        let mut body = obj([("status", text.into())]);
        if self.edit_id.is_none() {
            body.set(
                "visibility",
                VISIBILITIES[self.vis.selected() as usize % 4].into(),
            );
        }
        if let Some(r) = &self.reply_to {
            body.set("in_reply_to_id", r.as_str().into());
        }
        if self.cw_toggle.is_active() && !self.cw.text().trim().is_empty() {
            body.set("spoiler_text", self.cw.text().trim().into());
        } else if self.edit_id.is_some() {
            body.set("spoiler_text", "".into());
        }
        let lang = self.lang.text().trim().to_lowercase();
        if !lang.is_empty() {
            body.set("language", lang.into());
        }
        let ids: Vec<Value> = attachments
            .iter()
            .filter_map(|a| a.id.borrow().clone())
            .map(Value::from)
            .collect();
        if !ids.is_empty() {
            body.set("media_ids", ids.into());
            body.set("sensitive", self.sensitive.is_active().into());
        } else if self.edit_id.is_some() {
            body.set("media_ids", Value::Arr(vec![]));
        }
        if !poll_opts.is_empty() {
            body.set(
                "poll",
                obj([
                    (
                        "options",
                        poll_opts
                            .into_iter()
                            .map(Value::from)
                            .collect::<Vec<_>>()
                            .into(),
                    ),
                    (
                        "expires_in",
                        DURATIONS[self.poll_duration.selected() as usize % DURATIONS.len()]
                            .1
                            .into(),
                    ),
                    ("multiple", self.poll_multiple.is_active().into()),
                ]),
            );
        }
        // Alt text changed after upload is sent separately.
        let alt_updates: Vec<(String, String)> = attachments
            .iter()
            .filter_map(|a| {
                let id = a.id.borrow().clone()?;
                let alt = a.alt.text().to_string();
                (alt != *a.original_alt.borrow()).then_some((id, alt))
            })
            .collect();
        if self.edit_id.is_some() && !alt_updates.is_empty() {
            body.set(
                "media_attributes",
                alt_updates
                    .iter()
                    .map(|(id, alt)| {
                        obj([
                            ("id", id.as_str().into()),
                            ("description", alt.as_str().into()),
                        ])
                    })
                    .collect::<Vec<_>>()
                    .into(),
            );
        }

        self.post_btn.set_sensitive(false);
        self.win.set_sensitive(false);
        let api = self.app.api();
        let edit_id = self.edit_id.clone();
        let w = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let res = async {
                if edit_id.is_none() {
                    for (id, alt) in &alt_updates {
                        api.put(
                            &format!("/api/v1/media/{id}"),
                            obj([("description", alt.as_str().into())]),
                        )
                        .await?;
                    }
                }
                match &edit_id {
                    Some(id) => api.put(&format!("/api/v1/statuses/{id}"), body).await,
                    None => api.post("/api/v1/statuses", body).await,
                }
            }
            .await;
            let Some(c) = w.upgrade() else { return };
            match res {
                Ok(v) => {
                    c.app.status_posted(&v);
                    c.app.toast(if edit_id.is_some() {
                        "Post updated"
                    } else {
                        "Posted"
                    });
                    c.win.close();
                }
                Err(e) => {
                    c.app.toast(&e);
                    c.win.set_sensitive(true);
                    c.update_counter();
                }
            }
        });
    }
}
