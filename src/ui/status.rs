//! Widgets for a single post, a notification and a conversation.

use super::{app::App, compose, media, pages};
use crate::api;
use crate::html;
use crate::json::{Value, obj};
use crate::model::{Account, Notification, Poll, Status};
use crate::util;
use gtk::{glib, pango, prelude::*};
use std::cell::RefCell;
use std::rc::Rc;

pub fn avatar(url: &str, size: i32) -> gtk::Image {
    let img = gtk::Image::from_icon_name("avatar-default-symbolic");
    img.set_pixel_size(size);
    img.add_css_class("avatar");
    api::load_image(&img, url);
    img
}

pub fn avatar_button(app: &Rc<App>, acc: &Account, size: i32) -> gtk::Button {
    let b = gtk::Button::builder()
        .child(&avatar(&acc.avatar, size))
        .build();
    b.add_css_class("flat");
    b.add_css_class("avatar-button");
    b.set_valign(gtk::Align::Start);
    b.set_tooltip_text(Some(&format!("@{}", acc.acct)));
    let a = app.clone();
    let acc = acc.clone();
    b.connect_clicked(move |_| pages::open_profile(&a, acc.clone()));
    b
}

/// A wrapping label for HTML content whose links route through the app.
pub fn rich_label(app: &Rc<App>, html_text: &str, status: Option<&Status>) -> gtk::Label {
    let l = gtk::Label::new(None);
    l.set_markup(&html::to_markup(html_text));
    l.set_wrap(true);
    l.set_wrap_mode(pango::WrapMode::WordChar);
    l.set_xalign(0.0);
    l.set_halign(gtk::Align::Fill);
    l.set_hexpand(true);
    l.set_max_width_chars(70);
    l.set_width_chars(10);
    l.add_css_class("content");
    let a = app.clone();
    let st = status.cloned();
    l.connect_activate_link(move |_, uri| {
        a.activate_link(uri, st.as_ref());
        glib::Propagation::Stop
    });
    l
}

fn name_markup(acc: &Account) -> String {
    format!(
        "<b>{}</b>{}",
        glib::markup_escape_text(acc.name()),
        if acc.bot { " <small>🤖</small>" } else { "" }
    )
}

fn action_button(icon: &str, tooltip: &str) -> (gtk::Button, gtk::Label) {
    let b = gtk::Button::new();
    let bx = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    bx.append(&gtk::Image::from_icon_name(icon));
    let l = gtk::Label::new(None);
    bx.append(&l);
    b.set_child(Some(&bx));
    b.add_css_class("flat");
    b.add_css_class("action");
    b.set_tooltip_text(Some(tooltip));
    (b, l)
}

struct Actions {
    reply: gtk::Label,
    boost: (gtk::Button, gtk::Label),
    fav: (gtk::Button, gtk::Label),
    bookmark: gtk::Button,
}

impl Actions {
    fn update(&self, s: &Status) {
        self.reply.set_text(&count_text(s.replies_count));
        self.boost.1.set_text(&count_text(s.reblogs_count));
        self.fav.1.set_text(&count_text(s.favourites_count));
        set_class(&self.boost.0, "active-boost", s.reblogged);
        set_class(&self.fav.0, "active-fav", s.favourited);
        set_class(&self.bookmark, "active-bookmark", s.bookmarked);
        self.boost
            .0
            .set_tooltip_text(Some(if s.reblogged { "Undo boost" } else { "Boost" }));
        self.fav.0.set_tooltip_text(Some(if s.favourited {
            "Unfavourite"
        } else {
            "Favourite"
        }));
        self.bookmark.set_tooltip_text(Some(if s.bookmarked {
            "Remove bookmark"
        } else {
            "Bookmark"
        }));
        let can_boost = matches!(s.visibility.as_str(), "public" | "unlisted");
        self.boost.0.set_sensitive(can_boost || s.reblogged);
    }
}

fn count_text(n: i64) -> String {
    if n > 0 { util::count(n) } else { String::new() }
}

fn set_class(w: &impl IsA<gtk::Widget>, class: &str, on: bool) {
    if on {
        w.add_css_class(class);
    } else {
        w.remove_css_class(class);
    }
}

/// Builds the widget for a post.  `focused` is used for the main post of a
/// thread view, which shows extra detail.
pub fn build(app: &Rc<App>, status: Status, focused: bool) -> gtk::Widget {
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 4);
    outer.add_css_class("status");
    if focused {
        outer.add_css_class("focused");
    }

    if let Some(title) = &status.filtered {
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let l = gtk::Label::new(Some(&format!("Filtered: {title}")));
        l.add_css_class("dim-label");
        l.set_hexpand(true);
        l.set_xalign(0.0);
        let show = gtk::Button::with_label("Show anyway");
        show.add_css_class("flat");
        bar.append(&l);
        bar.append(&show);
        outer.append(&bar);
        let mut unfiltered = status.clone();
        unfiltered.filtered = None;
        let a = app.clone();
        show.connect_clicked(glib::clone!(
            #[weak]
            outer,
            #[weak]
            bar,
            move |_| {
                outer.remove(&bar);
                outer.append(&build(&a, unfiltered.clone(), focused));
            }
        ));
        return outer.upcast();
    }

    if status.reblog.is_some() {
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        line.add_css_class("boost-line");
        line.append(&gtk::Image::from_icon_name(
            "media-playlist-repeat-symbolic",
        ));
        let who = gtk::Button::new();
        let l = gtk::Label::new(None);
        l.set_markup(&format!("{} boosted", name_markup(&status.account)));
        l.set_ellipsize(pango::EllipsizeMode::End);
        who.set_child(Some(&l));
        who.add_css_class("flat");
        who.add_css_class("link-button");
        let a = app.clone();
        let acc = status.account.clone();
        who.connect_clicked(move |_| pages::open_profile(&a, acc.clone()));
        line.append(&who);
        let t = gtk::Label::new(Some(&util::relative_time(&status.created_at)));
        t.add_css_class("dim-label");
        t.set_hexpand(true);
        t.set_xalign(1.0);
        line.append(&t);
        outer.append(&line);
    }

    let s = status.shown().clone();
    let state = Rc::new(RefCell::new(s.clone()));

    let hbox = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    hbox.append(&avatar_button(
        app,
        &s.account,
        if focused { 52 } else { 44 },
    ));
    let main = gtk::Box::new(gtk::Orientation::Vertical, 4);
    main.set_hexpand(true);
    hbox.append(&main);
    outer.append(&hbox);

    // Header: name, handle, visibility, time.
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let name = gtk::Label::new(None);
    name.set_markup(&name_markup(&s.account));
    name.set_ellipsize(pango::EllipsizeMode::End);
    name.set_xalign(0.0);
    let acct = gtk::Label::new(Some(&format!("@{}", s.account.acct)));
    acct.add_css_class("dim-label");
    acct.set_ellipsize(pango::EllipsizeMode::End);
    acct.set_xalign(0.0);
    acct.set_hexpand(true);
    let names = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    names.append(&name);
    names.append(&acct);
    names.set_hexpand(true);
    head.append(&names);
    let vis = gtk::Image::from_icon_name(util::visibility_icon(&s.visibility));
    vis.set_tooltip_text(Some(util::visibility_label(&s.visibility)));
    vis.add_css_class("dim-label");
    head.append(&vis);
    if s.edited_at.is_some() {
        let e = gtk::Label::new(Some("✎"));
        e.add_css_class("dim-label");
        e.set_tooltip_text(Some(&format!(
            "Edited {}",
            util::absolute_time(s.edited_at.as_deref().unwrap_or(""))
        )));
        head.append(&e);
    }
    let time = gtk::Button::with_label(&util::relative_time(&s.created_at));
    time.add_css_class("flat");
    time.add_css_class("time");
    time.set_tooltip_text(Some(&util::absolute_time(&s.created_at)));
    let a = app.clone();
    let s2 = s.clone();
    time.connect_clicked(move |_| pages::open_thread(&a, &s2));
    head.append(&time);
    main.append(&head);

    if let Some(rid) = &s.in_reply_to_account_id {
        let who = if *rid == s.account.id {
            "Continued thread".to_string()
        } else {
            s.mentions
                .iter()
                .find(|m| &m.id == rid)
                .map(|m| format!("Replying to @{}", m.acct))
                .unwrap_or_else(|| "Reply".to_string())
        };
        let l = gtk::Label::new(Some(&format!("↩ {who}")));
        l.add_css_class("dim-label");
        l.add_css_class("caption");
        l.set_xalign(0.0);
        main.append(&l);
    }

    // Body, possibly behind a content warning.
    let body = gtk::Box::new(gtk::Orientation::Vertical, 6);
    if !s.spoiler_text.is_empty() {
        let cw = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        cw.add_css_class("cw");
        let l = gtk::Label::new(Some(&s.spoiler_text));
        l.set_wrap(true);
        l.set_wrap_mode(pango::WrapMode::WordChar);
        l.set_xalign(0.0);
        l.set_hexpand(true);
        let toggle = gtk::ToggleButton::with_label("Show more");
        toggle.set_valign(gtk::Align::Center);
        cw.append(&l);
        cw.append(&toggle);
        main.append(&cw);
        let expand = app.config.borrow().settings.expand_content_warnings || focused;
        body.set_visible(expand);
        toggle.set_active(expand);
        if expand {
            toggle.set_label("Show less");
        }
        toggle.connect_toggled(glib::clone!(
            #[weak]
            body,
            move |t| {
                body.set_visible(t.is_active());
                t.set_label(if t.is_active() {
                    "Show less"
                } else {
                    "Show more"
                });
            }
        ));
    }
    main.append(&body);

    let content = rich_label(app, &s.content_html, Some(&s));
    content.set_selectable(focused);
    if s.content_html.trim().is_empty() {
        content.set_visible(false);
    }
    body.append(&content);

    if !s.media.is_empty() {
        body.append(&media::grid(app, &s.media, s.sensitive));
    }
    if let Some(poll) = &s.poll {
        body.append(&poll_widget(app, poll.clone()));
    }
    if let (Some(card), true) = (&s.card, s.media.is_empty()) {
        body.append(&card_widget(app, card));
    }

    if focused {
        let mut details = util::absolute_time(&s.created_at);
        if let Some(app_name) = &s.application {
            details.push_str(&format!(" · via {app_name}"));
        }
        if let Some(e) = &s.edited_at {
            details.push_str(&format!(" · edited {}", util::absolute_time(e)));
        }
        let d = gtk::Label::new(Some(&details));
        d.add_css_class("dim-label");
        d.set_xalign(0.0);
        d.set_wrap(true);
        main.append(&d);
        let stats = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        for (label, path) in [
            (format!("{} boosts", s.reblogs_count), "reblogged_by"),
            (
                format!("{} favourites", s.favourites_count),
                "favourited_by",
            ),
        ] {
            let b = gtk::Button::with_label(&label);
            b.add_css_class("flat");
            let a = app.clone();
            let id = s.id.clone();
            let title = if path == "reblogged_by" {
                "Boosted by"
            } else {
                "Favourited by"
            };
            b.connect_clicked(move |_| {
                pages::open_accounts(&a, title, &format!("/api/v1/statuses/{id}/{path}"));
            });
            stats.append(&b);
        }
        main.append(&stats);
    }

    // Action bar.
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    bar.add_css_class("actions");
    let (reply_btn, reply_l) = action_button("mail-reply-sender-symbolic", "Reply");
    let boost = action_button("media-playlist-repeat-symbolic", "Boost");
    let fav = action_button("starred-symbolic", "Favourite");
    let bookmark = gtk::Button::from_icon_name("user-bookmarks-symbolic");
    bookmark.add_css_class("flat");
    bookmark.add_css_class("action");
    let more = gtk::MenuButton::new();
    more.set_icon_name("view-more-symbolic");
    more.add_css_class("flat");
    more.set_tooltip_text(Some("More"));
    bar.append(&reply_btn);
    bar.append(&boost.0);
    bar.append(&fav.0);
    bar.append(&bookmark);
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    bar.append(&spacer);
    bar.append(&more);
    main.append(&bar);

    let actions = Rc::new(Actions {
        reply: reply_l,
        boost,
        fav,
        bookmark,
    });
    actions.update(&s);

    let a = app.clone();
    let st = state.clone();
    reply_btn
        .connect_clicked(move |_| compose::open(&a, compose::Mode::Reply(st.borrow().clone())));

    let toggle =
        |btn: &gtk::Button, on: fn(&Status) -> bool, yes: &'static str, no: &'static str| {
            let a = app.clone();
            let st = state.clone();
            let acts = actions.clone();
            btn.connect_clicked(move |b| {
                let (id, is_on) = {
                    let s = st.borrow();
                    (s.id.clone(), on(&s))
                };
                let verb = if is_on { no } else { yes };
                let api = a.api();
                let st = st.clone();
                let acts = acts.clone();
                b.set_sensitive(false);
                let b = b.downgrade();
                a.spawn(async move {
                    let res = api
                        .post(&format!("/api/v1/statuses/{id}/{verb}"), Value::Null)
                        .await;
                    if let Some(b) = b.upgrade() {
                        b.set_sensitive(true);
                    }
                    let v = res?;
                    // Boosting returns the wrapper status; we want the original.
                    let v = if v["reblog"].is_null() {
                        v
                    } else {
                        v["reblog"].clone()
                    };
                    let mut updated = Status::from_json(&v);
                    // Some servers return stale counters right after toggling.
                    if verb == "favourite" && !updated.favourited {
                        updated.favourited = true;
                    }
                    *st.borrow_mut() = updated.clone();
                    acts.update(&updated);
                    Ok(())
                });
            });
        };
    toggle(&actions.boost.0, |s| s.reblogged, "reblog", "unreblog");
    toggle(&actions.fav.0, |s| s.favourited, "favourite", "unfavourite");
    toggle(
        &actions.bookmark,
        |s| s.bookmarked,
        "bookmark",
        "unbookmark",
    );

    more.set_popover(Some(&more_menu(app, &state, &content, &outer)));
    outer.upcast()
}

fn more_menu(
    app: &Rc<App>,
    state: &Rc<RefCell<Status>>,
    content: &gtk::Label,
    outer: &gtk::Box,
) -> gtk::Popover {
    let pop = gtk::Popover::new();
    let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
    pop.set_child(Some(&b));
    let s = state.borrow().clone();
    let mine = s.account.id == app.my_id();

    let item = |label: &str, f: Box<dyn Fn()>| {
        let btn = gtk::Button::with_label(label);
        btn.add_css_class("flat");
        if let Some(l) = btn.child().and_downcast::<gtk::Label>() {
            l.set_xalign(0.0);
        }
        btn.connect_clicked(glib::clone!(
            #[weak]
            pop,
            move |_| {
                pop.popdown();
                f();
            }
        ));
        b.append(&btn);
    };
    let a = app.clone();
    let st = state.clone();
    item(
        "Open thread",
        Box::new(move || pages::open_thread(&a, &st.borrow())),
    );
    let a = app.clone();
    let link = s.link();
    item("Open in browser", Box::new(move || a.open_url(&link)));
    let a = app.clone();
    let link = s.link();
    item("Copy link", Box::new(move || a.copy_text(&link)));
    let a = app.clone();
    let text = html::to_text(&s.content_html);
    item("Copy text", Box::new(move || a.copy_text(&text)));

    if s.language.is_some() && !mine {
        let a = app.clone();
        let id = s.id.clone();
        let content = content.downgrade();
        item(
            "Translate",
            Box::new(move || {
                let api = a.api();
                let id = id.clone();
                let content = content.clone();
                a.spawn(async move {
                    let v = api
                        .post(&format!("/api/v1/statuses/{id}/translate"), Value::Null)
                        .await?;
                    if let Some(l) = content.upgrade() {
                        let provider = v["provider"].str();
                        l.set_markup(&format!(
                            "{}\n\n<small><i>Translated from {} by {}</i></small>",
                            html::to_markup(v["content"].str()),
                            glib::markup_escape_text(v["detected_source_language"].str()),
                            glib::markup_escape_text(provider)
                        ));
                    }
                    Ok(())
                });
            }),
        );
    }

    b.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    let simple_post = |path: String, msg: &'static str| {
        let a = app.clone();
        Box::new(move || {
            let api = a.api();
            let path = path.clone();
            let a2 = a.clone();
            a.spawn(async move {
                api.post(&path, Value::Null).await?;
                a2.toast(msg);
                Ok(())
            });
        }) as Box<dyn Fn()>
    };
    item(
        if s.muted {
            "Unmute conversation"
        } else {
            "Mute conversation"
        },
        simple_post(
            format!(
                "/api/v1/statuses/{}/{}",
                s.id,
                if s.muted { "unmute" } else { "mute" }
            ),
            if s.muted {
                "Conversation unmuted"
            } else {
                "Conversation muted"
            },
        ),
    );

    if mine {
        let a = app.clone();
        let st = state.clone();
        item(
            "Edit",
            Box::new(move || compose::open(&a, compose::Mode::Edit(st.borrow().clone()))),
        );
        if matches!(s.visibility.as_str(), "public" | "unlisted") {
            item(
                if s.pinned {
                    "Unpin from profile"
                } else {
                    "Pin on profile"
                },
                simple_post(
                    format!(
                        "/api/v1/statuses/{}/{}",
                        s.id,
                        if s.pinned { "unpin" } else { "pin" }
                    ),
                    if s.pinned {
                        "Unpinned"
                    } else {
                        "Pinned on profile"
                    },
                ),
            );
        }
        for redraft in [false, true] {
            let a = app.clone();
            let st = state.clone();
            let outer = outer.downgrade();
            item(
                if redraft {
                    "Delete & re-draft"
                } else {
                    "Delete"
                },
                Box::new(move || {
                    let a = a.clone();
                    let s = st.borrow().clone();
                    let outer = outer.clone();
                    let dlg = gtk::AlertDialog::builder()
                        .message(if redraft {
                            "Delete and re-draft this post?"
                        } else {
                            "Delete this post?"
                        })
                        .detail("Boosts and favourites will be lost.")
                        .buttons(["Cancel", "Delete"])
                        .cancel_button(0)
                        .default_button(1)
                        .build();
                    let win = a.window.clone();
                    glib::spawn_future_local(async move {
                        if dlg.choose_future(Some(&win)).await != Ok(1) {
                            return;
                        }
                        let api = a.api();
                        match api
                            .request(
                                "DELETE",
                                &format!("/api/v1/statuses/{}", s.id),
                                &[],
                                api::Body::Empty,
                            )
                            .await
                        {
                            Ok(resp) => {
                                if let Some(o) = outer.upgrade() {
                                    o.set_sensitive(false);
                                    o.set_opacity(0.4);
                                }
                                if redraft {
                                    compose::open(
                                        &a,
                                        compose::Mode::Redraft(Box::new(s), resp.json),
                                    );
                                } else {
                                    a.toast("Post deleted");
                                }
                            }
                            Err(e) => a.toast(&e),
                        }
                    });
                }),
            );
        }
    } else {
        let acct = s.account.acct.clone();
        let a = app.clone();
        let acct2 = acct.clone();
        item(
            &format!("Mention @{acct}"),
            Box::new(move || compose::open(&a, compose::Mode::Mention(acct2.clone()))),
        );
        let a = app.clone();
        let acc = s.account.clone();
        item(
            &format!("Mute @{acct}"),
            Box::new(move || pages::confirm_account_action(&a, &acc, "mute")),
        );
        let a = app.clone();
        let acc = s.account.clone();
        item(
            &format!("Block @{acct}"),
            Box::new(move || pages::confirm_account_action(&a, &acc, "block")),
        );
        let a = app.clone();
        let acc = s.account.clone();
        let sid = s.id.clone();
        item(
            &format!("Report @{acct}…"),
            Box::new(move || pages::report(&a, &acc, Some(&sid))),
        );
    }
    pop
}

fn card_widget(app: &Rc<App>, card: &crate::model::Card) -> gtk::Widget {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    if let Some(img) = &card.image {
        let p = gtk::Picture::new();
        p.set_content_fit(gtk::ContentFit::Cover);
        p.set_size_request(80, 80);
        p.set_can_shrink(true);
        api::load_picture(&p, img);
        let frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
        frame.set_size_request(80, 80);
        frame.set_overflow(gtk::Overflow::Hidden);
        frame.add_css_class("card-image");
        frame.append(&p);
        b.append(&frame);
    }
    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_valign(gtk::Align::Center);
    text.set_hexpand(true);
    let add = |s: &str, class: &str, lines: i32| {
        if s.is_empty() {
            return;
        }
        let l = gtk::Label::new(Some(s));
        l.set_xalign(0.0);
        l.set_wrap(true);
        l.set_wrap_mode(pango::WrapMode::WordChar);
        l.set_lines(lines);
        l.set_ellipsize(pango::EllipsizeMode::End);
        l.set_max_width_chars(60);
        l.add_css_class(class);
        text.append(&l);
    };
    let host = glib::Uri::parse(&card.url, glib::UriFlags::NONE)
        .ok()
        .and_then(|u| u.host())
        .map(|h| h.to_string())
        .unwrap_or_else(|| card.provider.clone());
    add(&host, "dim-label", 1);
    add(&card.title, "heading", 2);
    add(&card.description, "dim-label", 2);
    b.append(&text);
    let btn = gtk::Button::builder().child(&b).build();
    btn.add_css_class("card");
    btn.set_tooltip_text(Some(&card.url));
    let a = app.clone();
    let url = card.url.clone();
    btn.connect_clicked(move |_| a.open_url(&url));
    btn.upcast()
}

fn poll_widget(app: &Rc<App>, poll: Poll) -> gtk::Widget {
    let holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
    fill_poll(app, &holder, poll);
    holder.upcast()
}

fn fill_poll(app: &Rc<App>, holder: &gtk::Box, poll: Poll) {
    while let Some(c) = holder.first_child() {
        holder.remove(&c);
    }
    let b = gtk::Box::new(gtk::Orientation::Vertical, 4);
    b.add_css_class("poll");
    holder.append(&b);
    let show_results = poll.voted || poll.expired;
    let total = if poll.multiple {
        poll.voters_count.unwrap_or(poll.votes_count)
    } else {
        poll.votes_count
    }
    .max(1);
    let mut checks: Vec<gtk::CheckButton> = Vec::new();
    for (i, opt) in poll.options.iter().enumerate() {
        if show_results {
            let frac = opt.votes as f64 / total as f64;
            let row = gtk::Box::new(gtk::Orientation::Vertical, 2);
            let l = gtk::Label::new(None);
            let mine = if poll.own_votes.contains(&(i as i64)) {
                " ✓"
            } else {
                ""
            };
            l.set_markup(&format!(
                "<b>{:.0}%</b>  {}{}",
                frac * 100.0,
                glib::markup_escape_text(&opt.title),
                mine
            ));
            l.set_xalign(0.0);
            l.set_wrap(true);
            let bar = gtk::LevelBar::for_interval(0.0, 1.0);
            bar.set_value(frac.min(1.0));
            row.append(&l);
            row.append(&bar);
            b.append(&row);
        } else {
            let c = gtk::CheckButton::with_label(&opt.title);
            if !poll.multiple
                && let Some(first) = checks.first()
            {
                c.set_group(Some(first));
            }
            checks.push(c.clone());
            b.append(&c);
        }
    }
    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    if !show_results {
        let vote = gtk::Button::with_label("Vote");
        vote.add_css_class("suggested-action");
        footer.append(&vote);
        let a = app.clone();
        let id = poll.id.clone();
        let holder_w = holder.downgrade();
        vote.connect_clicked(move |_| {
            let choices: Vec<Value> = checks
                .iter()
                .enumerate()
                .filter(|(_, c)| c.is_active())
                .map(|(i, _)| (i as i64).into())
                .collect();
            if choices.is_empty() {
                a.toast("Pick an option first");
                return;
            }
            let api = a.api();
            let a2 = a.clone();
            let id = id.clone();
            let holder_w = holder_w.clone();
            a.spawn(async move {
                let v = api
                    .post(
                        &format!("/api/v1/polls/{id}/votes"),
                        obj([("choices", choices.into())]),
                    )
                    .await?;
                if let Some(h) = holder_w.upgrade() {
                    fill_poll(&a2, &h, Poll::from_json(&v));
                }
                Ok(())
            });
        });
    }
    let people = poll.voters_count.unwrap_or(poll.votes_count);
    let mut info = format!(
        "{} {}",
        people,
        if people == 1 { "person" } else { "people" }
    );
    if poll.expired {
        info.push_str(" · closed");
    } else if let Some(e) = &poll.expires_at {
        info.push_str(&format!(" · {}", util::time_left(e)));
    }
    let l = gtk::Label::new(Some(&info));
    l.add_css_class("dim-label");
    footer.append(&l);
    if show_results && !poll.expired {
        let refresh = gtk::Button::with_label("Refresh");
        refresh.add_css_class("flat");
        let a = app.clone();
        let id = poll.id.clone();
        let holder_w = holder.downgrade();
        refresh.connect_clicked(move |_| {
            let api = a.api();
            let a2 = a.clone();
            let id = id.clone();
            let holder_w = holder_w.clone();
            a.spawn(async move {
                let v = api.get(&format!("/api/v1/polls/{id}"), &[]).await?;
                if let Some(h) = holder_w.upgrade() {
                    fill_poll(&a2, &h, Poll::from_json(&v));
                }
                Ok(())
            });
        });
        footer.append(&refresh);
    }
    b.append(&footer);
}

pub fn notification(app: &Rc<App>, n: Notification) -> gtk::Widget {
    let (icon, verb) = match n.kind.as_str() {
        "mention" => ("mail-replied-symbolic", "mentioned you"),
        "reblog" => ("media-playlist-repeat-symbolic", "boosted your post"),
        "favourite" => ("starred-symbolic", "favourited your post"),
        "follow" => ("list-add-symbolic", "followed you"),
        "follow_request" => ("list-add-symbolic", "requested to follow you"),
        "poll" => ("view-list-bullet-symbolic", "— a poll has ended"),
        "status" => ("preferences-system-notifications-symbolic", "posted"),
        "update" => (
            "document-edit-symbolic",
            "edited a post you interacted with",
        ),
        "admin.sign_up" => ("avatar-default-symbolic", "signed up"),
        "admin.report" => ("dialog-warning-symbolic", "filed a report"),
        "severed_relationships" => (
            "dialog-warning-symbolic",
            "— some relationships were severed",
        ),
        "moderation_warning" => (
            "dialog-warning-symbolic",
            "— you received a moderation warning",
        ),
        "quote" => ("mail-replied-symbolic", "quoted your post"),
        _ => (
            "preferences-system-notifications-symbolic",
            "sent a notification",
        ),
    };
    // Mentions and new posts read best as full posts.
    if let (Some(st), true) = (&n.status, matches!(n.kind.as_str(), "mention" | "status")) {
        let w = build(app, st.clone(), false);
        if n.kind == "status" {
            let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
            b.append(&header_line(app, icon, &n.account, verb, &n.created_at));
            b.append(&w);
            return b.upcast();
        }
        return w;
    }
    let b = gtk::Box::new(gtk::Orientation::Vertical, 4);
    b.add_css_class("status");
    b.add_css_class("notification");
    b.append(&header_line(app, icon, &n.account, verb, &n.created_at));
    match (&n.status, n.kind.as_str()) {
        (Some(st), _) => {
            let body = gtk::Box::new(gtk::Orientation::Vertical, 4);
            body.set_margin_start(30);
            let text = if st.spoiler_text.is_empty() {
                html::to_text(&st.content_html)
            } else {
                format!("CW: {}", st.spoiler_text)
            };
            let l = gtk::Label::new(Some(&text));
            l.set_wrap(true);
            l.set_wrap_mode(pango::WrapMode::WordChar);
            l.set_lines(4);
            l.set_ellipsize(pango::EllipsizeMode::End);
            l.set_xalign(0.0);
            l.add_css_class("dim-label");
            body.append(&l);
            if let Some(p) = &st.poll
                && n.kind == "poll"
            {
                body.append(&poll_widget(app, p.clone()));
            }
            b.append(&body);
        }
        (None, "follow_request") => {
            b.append(&super::account::follow_request_row(app, &n.account));
        }
        (None, "follow") => {
            b.append(&super::account::row(app, &n.account));
        }
        _ => {}
    }
    b.upcast()
}

fn header_line(app: &Rc<App>, icon: &str, acc: &Account, verb: &str, time: &str) -> gtk::Box {
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    line.add_css_class("notification-line");
    let i = gtk::Image::from_icon_name(icon);
    i.add_css_class("accent");
    line.append(&i);
    line.append(&avatar_button(app, acc, 24));
    let l = gtk::Label::new(None);
    l.set_markup(&format!(
        "{} {}",
        name_markup(acc),
        glib::markup_escape_text(verb)
    ));
    l.set_ellipsize(pango::EllipsizeMode::End);
    l.set_xalign(0.0);
    l.set_hexpand(true);
    line.append(&l);
    let t = gtk::Label::new(Some(&util::relative_time(time)));
    t.set_tooltip_text(Some(&util::absolute_time(time)));
    t.add_css_class("dim-label");
    line.append(&t);
    line
}

pub fn conversation(app: &Rc<App>, conv: &Value, last: Status) -> gtk::Widget {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let names: Vec<String> = conv["accounts"]
        .arr()
        .iter()
        .map(|a| format!("@{}", a["acct"].str()))
        .collect();
    let l = gtk::Label::new(None);
    let unread = if conv["unread"].bool() {
        "<b>●</b> "
    } else {
        ""
    };
    l.set_markup(&format!(
        "{unread}<small>With {}</small>",
        glib::markup_escape_text(&names.join(", "))
    ));
    l.set_xalign(0.0);
    l.set_margin_start(12);
    l.set_margin_top(6);
    l.add_css_class("dim-label");
    b.append(&l);
    b.append(&build(app, last, false));
    if conv["unread"].bool() {
        let api = app.api();
        let id = conv["id"].str().to_string();
        glib::spawn_future_local(async move {
            let _ = api
                .post(&format!("/api/v1/conversations/{id}/read"), Value::Null)
                .await;
        });
    }
    b.upcast()
}
