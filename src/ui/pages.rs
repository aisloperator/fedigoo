//! The individual pages: timelines, profiles, threads, search, lists, and the
//! smaller dialogs that go with them.

use super::app::{App, Root};
use super::timeline::{Source, Timeline};
use super::{account, compose, status};
use crate::api::{self, Body};
use crate::html;
use crate::json::{Value, obj};
use crate::model::{Account, Relationship, Status, Tag};
use crate::util;
use gtk::{gio, glib, prelude::*};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};

/// Wraps `child` in a centred, width-limited scrolling column.
fn column(child: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.add_css_class("timeline-column");
    content.set_size_request(380, -1);
    content.set_hexpand(true);
    content.append(child);
    let clamp = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    clamp.set_halign(gtk::Align::Center);
    clamp.set_hexpand(true);
    clamp.append(&content);
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&clamp)
        .build()
}

fn header_box() -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 8);
    b.add_css_class("page-header");
    b
}

/// A row of linked toggle buttons; calls `f` with the chosen index.
fn tabs(labels: &[&str], f: impl Fn(usize) + 'static) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    b.add_css_class("linked");
    b.set_halign(gtk::Align::Center);
    let f = Rc::new(f);
    let mut first: Option<gtk::ToggleButton> = None;
    for (i, label) in labels.iter().enumerate() {
        let t = gtk::ToggleButton::with_label(label);
        if let Some(g) = &first {
            t.set_group(Some(g));
        } else {
            t.set_active(true);
            first = Some(t.clone());
        }
        let f = f.clone();
        t.connect_toggled(move |t| {
            if t.is_active() {
                f(i);
            }
        });
        b.append(&t);
    }
    b
}

pub fn root_page(app: &Rc<App>, root: Root, title: &str) {
    let st = |path: &str, q: &[(&str, &str)]| Source::statuses(path, q);
    let (source, header): (Source, Option<gtk::Widget>) = match root {
        Root::Home => (st("/api/v1/timelines/home", &[]), None),
        Root::Notifications => (
            Source::Notifications(vec![]),
            Some(notifications_header(app)),
        ),
        Root::Mentions => (
            Source::Notifications(vec![("types[]".into(), "mention".into())]),
            None,
        ),
        Root::Local => (st("/api/v1/timelines/public", &[("local", "true")]), None),
        Root::Federated => (st("/api/v1/timelines/public", &[]), None),
        Root::Conversations => (Source::Conversations, None),
        Root::Bookmarks => (st("/api/v1/bookmarks", &[]), None),
        Root::Favourites => (st("/api/v1/favourites", &[]), None),
        Root::FollowedTags => (Source::Tags("/api/v1/followed_tags".into()), None),
        Root::FollowRequests => (Source::FollowRequests, None),
        Root::Mutes => (Source::Accounts("/api/v1/mutes".into()), None),
        Root::Blocks => (Source::Accounts("/api/v1/blocks".into()), None),
        Root::Explore => return explore(app, title),
        Root::Search => return search(app, title),
        Root::Lists => return lists(app, title),
        Root::Profile => {
            let me = app.me.borrow().clone();
            match me {
                Some(me) => open_profile(app, me),
                None => {
                    let id = app
                        .config
                        .borrow()
                        .active_account()
                        .map(|a| a.id.clone())
                        .unwrap_or_default();
                    open_profile_id(app, &id);
                }
            }
            return;
        }
    };
    let tl = Timeline::new(app, source, header);
    app.push_timeline(title, tl);
}

fn notifications_header(app: &Rc<App>) -> gtk::Widget {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    b.add_css_class("page-header");
    b.set_halign(gtk::Align::End);
    let clear = gtk::Button::with_label("Clear all");
    clear.add_css_class("flat");
    let a = app.clone();
    clear.connect_clicked(move |_| {
        let a = a.clone();
        let dlg = gtk::AlertDialog::builder()
            .message("Clear all notifications?")
            .detail("This permanently removes them from your account.")
            .buttons(["Cancel", "Clear"])
            .cancel_button(0)
            .default_button(1)
            .build();
        let win = a.window.clone();
        glib::spawn_future_local(async move {
            if dlg.choose_future(Some(&win)).await == Ok(1) {
                match a
                    .api()
                    .post("/api/v1/notifications/clear", Value::Null)
                    .await
                {
                    Ok(_) => a.refresh(),
                    Err(e) => a.toast(&e),
                }
            }
        });
    });
    b.append(&clear);
    b.upcast()
}

fn explore(app: &Rc<App>, title: &str) {
    let header = header_box();
    let tl_slot: Rc<std::cell::RefCell<Weak<Timeline>>> = Rc::default();
    let slot = tl_slot.clone();
    header.append(&tabs(&["Posts", "Hashtags", "News"], move |i| {
        if let Some(tl) = slot.borrow().upgrade() {
            tl.set_source(match i {
                0 => Source::statuses("/api/v1/trends/statuses", &[]),
                1 => Source::Tags("/api/v1/trends/tags".into()),
                _ => Source::Links,
            });
        }
    }));
    let tl = Timeline::new(
        app,
        Source::statuses("/api/v1/trends/statuses", &[]),
        Some(header.upcast()),
    );
    *tl_slot.borrow_mut() = Rc::downgrade(&tl);
    app.push_timeline(title, tl);
}

// ----- Search -----

fn search(app: &Rc<App>, title: &str) {
    let vb = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let entry = gtk::SearchEntry::new();
    entry.set_placeholder_text(Some("Search accounts, #hashtags, posts or paste a URL"));
    entry.set_margin_top(12);
    entry.set_margin_start(12);
    entry.set_margin_end(12);
    let results = gtk::Box::new(gtk::Orientation::Vertical, 0);
    vb.append(&entry);
    vb.append(&results);
    let a = app.clone();
    entry.connect_activate(glib::clone!(
        #[weak]
        results,
        move |e| {
            let q = e.text().trim().to_string();
            if q.is_empty() {
                return;
            }
            while let Some(c) = results.first_child() {
                results.remove(&c);
            }
            let spinner = gtk::Spinner::new();
            spinner.start();
            spinner.set_margin_top(20);
            results.append(&spinner);
            let api = a.api();
            let a2 = a.clone();
            let results = results.downgrade();
            a.spawn(async move {
                let res = api
                    .get(
                        "/api/v2/search",
                        &[("q", &q), ("resolve", "true"), ("limit", "20")],
                    )
                    .await;
                let Some(results) = results.upgrade() else {
                    return Ok(());
                };
                results.remove(&spinner);
                let v = res?;
                show_search_results(&a2, &results, &v);
                Ok(())
            });
        }
    ));
    let page = column(&vb);
    let e = entry.clone();
    app.push(title, page, None, None);
    e.grab_focus();
}

fn section_label(text: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.add_css_class("heading");
    l.add_css_class("section");
    l.set_xalign(0.0);
    l
}

fn show_search_results(app: &Rc<App>, results: &gtk::Box, v: &Value) {
    let mut any = false;
    let list = |items: &[Value], title: &str, f: &dyn Fn(&Value) -> (gtk::Widget, Rc<dyn Fn()>)| {
        if items.is_empty() {
            return false;
        }
        results.append(&section_label(title));
        let lb = gtk::ListBox::new();
        lb.set_selection_mode(gtk::SelectionMode::None);
        lb.add_css_class("timeline");
        let mut acts = Vec::new();
        for item in items {
            let (w, act) = f(item);
            lb.append(&w);
            acts.push(act);
        }
        lb.connect_row_activated(move |_, row| {
            if let Some(f) = acts.get(row.index() as usize) {
                f();
            }
        });
        results.append(&lb);
        true
    };
    any |= list(v["accounts"].arr(), "Accounts", &|item| {
        let acc = Account::from_json(item);
        let a = app.clone();
        let acc2 = acc.clone();
        (
            account::row(app, &acc).upcast(),
            Rc::new(move || open_profile(&a, acc2.clone())),
        )
    });
    any |= list(v["hashtags"].arr(), "Hashtags", &|item| {
        let tag = Tag::from_json(item);
        let a = app.clone();
        let name = tag.name.clone();
        (
            account::tag_row(&tag).upcast(),
            Rc::new(move || open_hashtag(&a, &name)),
        )
    });
    any |= list(v["statuses"].arr(), "Posts", &|item| {
        let s = Status::from_json(item);
        let a = app.clone();
        let s2 = s.clone();
        (
            status::build(app, s, false),
            Rc::new(move || open_thread(&a, &s2)),
        )
    });
    if !any {
        let l = gtk::Label::new(Some("No results."));
        l.add_css_class("dim-label");
        l.set_margin_top(20);
        results.append(&l);
    }
}

// ----- Threads -----

/// Scrolls the enclosing scrolled window so `row` is at the top.  Rows
/// around it may still be changing height (images loading, image rows doing
/// their extra layout pass), so the target is tracked each frame until it has
/// been stable for a moment, for at most ~3 s, or until the user scrolls.
fn scroll_to_row(row: &gtk::ListBoxRow) {
    const STABLE_FRAMES: u32 = 15;
    const MAX_FRAMES: u32 = 180;
    let frames = std::cell::Cell::new(0u32);
    let stable = std::cell::Cell::new(0u32);
    let last_set = std::cell::Cell::new(None::<f64>);
    row.add_tick_callback(move |row, _| {
        frames.set(frames.get() + 1);
        let sw = row
            .ancestor(gtk::ScrolledWindow::static_type())
            .and_downcast::<gtk::ScrolledWindow>();
        let target = sw.as_ref().and_then(|sw| {
            let bounds = sw.child().and_then(|c| row.compute_bounds(&c))?;
            (row.height() > 0).then(|| (bounds.y() as f64 - 8.0).max(0.0))
        });
        if let (Some(sw), Some(target)) = (sw, target) {
            let adj = sw.vadjustment();
            // Someone else moved it (the user scrolled): stop following.
            if last_set
                .get()
                .is_some_and(|v| (adj.value() - v).abs() > 1.0)
            {
                return glib::ControlFlow::Break;
            }
            let reachable = target.min(adj.upper() - adj.page_size()).max(0.0);
            if last_set.get().is_some_and(|v| (v - reachable).abs() < 0.5) {
                stable.set(stable.get() + 1);
            } else {
                stable.set(0);
            }
            adj.set_value(reachable);
            last_set.set(Some(adj.value()));
        }
        if stable.get() >= STABLE_FRAMES || frames.get() >= MAX_FRAMES {
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
}

/// An open thread page, so it can be refreshed when the user replies in it.
pub struct ThreadView {
    /// Ids of every post currently shown.
    ids: Rc<RefCell<HashSet<String>>>,
    /// Reloads the thread, then scrolls to the given post if it's shown.
    load: Rc<dyn Fn(Option<String>)>,
}

impl ThreadView {
    pub fn shows(&self, id: &str) -> bool {
        self.ids.borrow().contains(id)
    }

    pub fn reload_and_show(&self, id: &str) {
        (self.load)(Some(id.to_string()));
    }
}

pub fn open_thread(app: &Rc<App>, st: &Status) {
    let vb = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let page = column(&vb);
    let a = app.clone();
    let id = st.id.clone();
    let initial = st.clone();
    let vb_w = vb.downgrade();
    let ids: Rc<RefCell<HashSet<String>>> = Rc::default();
    let ids2 = ids.clone();
    let load: Rc<dyn Fn(Option<String>)> = Rc::new(move |scroll_to: Option<String>| {
        let Some(vb) = vb_w.upgrade() else { return };
        while let Some(c) = vb.first_child() {
            vb.remove(&c);
        }
        let lb = gtk::ListBox::new();
        lb.set_selection_mode(gtk::SelectionMode::None);
        lb.add_css_class("timeline");
        lb.append(&status::build(&a, initial.clone(), true));
        vb.append(&lb);
        let api = a.api();
        let id = id.clone();
        let a2 = a.clone();
        let lb_w = lb.downgrade();
        let ids = ids2.clone();
        a.spawn(async move {
            let (main, ctx) = (
                api.get(&format!("/api/v1/statuses/{id}"), &[]).await?,
                api.get(&format!("/api/v1/statuses/{id}/context"), &[])
                    .await?,
            );
            let Some(lb) = lb_w.upgrade() else {
                return Ok(());
            };
            lb.remove_all();
            let mut items: Vec<(Status, i32, bool)> = Vec::new();
            for s in ctx["ancestors"].arr() {
                items.push((Status::from_json(s), 0, false));
            }
            items.push((Status::from_json(&main), 0, true));
            // Indent replies by their depth below the focused post.
            let mut depth: HashMap<String, i32> = HashMap::new();
            depth.insert(id.clone(), 0);
            for s in ctx["descendants"].arr() {
                let s = Status::from_json(s);
                let d = s
                    .in_reply_to_id
                    .as_ref()
                    .and_then(|p| depth.get(p))
                    .map_or(1, |d| d + 1);
                depth.insert(s.id.clone(), d);
                items.push((s, d, false));
            }
            *ids.borrow_mut() = items.iter().map(|(s, ..)| s.id.clone()).collect();
            let mut acts: Vec<Rc<dyn Fn()>> = Vec::new();
            let mut focus_row = None;
            for (s, d, focused) in items {
                let w = status::build(&a2, s.clone(), focused);
                // Indent via CSS padding: margins upset GTK's size checks here.
                let depth = d.saturating_sub(1).min(5);
                if depth > 0 {
                    w.add_css_class("nested");
                    w.add_css_class(&format!("indent{depth}"));
                }
                let row = gtk::ListBoxRow::new();
                row.set_child(Some(&w));
                lb.append(&row);
                // Focus the requested post (e.g. a new reply), else the main one.
                let wanted = match &scroll_to {
                    Some(target) => *target == s.id,
                    None => focused,
                };
                if wanted || (focused && focus_row.is_none()) {
                    focus_row = Some(row);
                }
                let a3 = a2.clone();
                acts.push(Rc::new(move || open_thread(&a3, &s)));
            }
            lb.connect_row_activated(move |_, row| {
                if let Some(f) = acts.get(row.index() as usize) {
                    f();
                }
            });
            if let Some(row) = focus_row {
                scroll_to_row(&row);
            }
            Ok(())
        });
    });
    load(None);
    let view = Rc::new(ThreadView {
        ids,
        load: load.clone(),
    });
    app.register_thread(&view);
    app.push(
        "Thread",
        page,
        Some(view),
        Some(Rc::new(move || load(None))),
    );
}

// ----- Hashtags & lists -----

pub fn open_hashtag(app: &Rc<App>, tag: &str) {
    let tag = tag.trim_start_matches('#').to_string();
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    header.add_css_class("page-header");
    let l = gtk::Label::new(None);
    l.set_markup(&format!(
        "<span size='x-large' weight='bold'>#{}</span>",
        glib::markup_escape_text(&tag)
    ));
    l.set_hexpand(true);
    l.set_xalign(0.0);
    header.append(&l);
    let follow = gtk::Button::with_label("Follow");
    follow.set_sensitive(false);
    header.append(&follow);

    let api = app.api();
    let path = format!("/api/v1/tags/{}", api::escape(&tag));
    let following = Rc::new(std::cell::Cell::new(false));
    let set = {
        let following = following.clone();
        move |b: &gtk::Button, on: bool| {
            following.set(on);
            b.set_label(if on { "Unfollow" } else { "Follow" });
            if on {
                b.remove_css_class("suggested-action");
            } else {
                b.add_css_class("suggested-action");
            }
            b.set_sensitive(true);
        }
    };
    let set = Rc::new(set);
    {
        let follow = follow.downgrade();
        let set = set.clone();
        let path = path.clone();
        let api = api.clone();
        glib::spawn_future_local(async move {
            if let (Ok(v), Some(b)) = (api.get(&path, &[]).await, follow.upgrade()) {
                set(&b, v["following"].bool());
            }
        });
    }
    let a = app.clone();
    follow.connect_clicked(move |b| {
        let verb = if following.get() {
            "unfollow"
        } else {
            "follow"
        };
        let api = a.api();
        let path = format!("{path}/{verb}");
        let b = b.downgrade();
        let set = set.clone();
        if let Some(b) = b.upgrade() {
            b.set_sensitive(false);
        }
        a.spawn(async move {
            let v = api.post(&path, Value::Null).await;
            if let Some(b) = b.upgrade() {
                b.set_sensitive(true);
                set(&b, v?["following"].bool());
            }
            Ok(())
        });
    });
    let tl = Timeline::new(
        app,
        Source::statuses(&format!("/api/v1/timelines/tag/{}", api::escape(&tag)), &[]),
        Some(header.upcast()),
    );
    app.push_timeline(&format!("#{tag}"), tl);
}

fn lists(app: &Rc<App>, title: &str) {
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    header.add_css_class("page-header");
    let entry = gtk::Entry::new();
    entry.set_placeholder_text(Some("New list name"));
    entry.set_hexpand(true);
    let create = gtk::Button::with_label("Create list");
    header.append(&entry);
    header.append(&create);
    let tl = Timeline::new(app, Source::Lists, Some(header.upcast()));
    let a = app.clone();
    let w = Rc::downgrade(&tl);
    let do_create = Rc::new(move |e: &gtk::Entry| {
        let name = e.text().trim().to_string();
        if name.is_empty() {
            return;
        }
        e.set_text("");
        let api = a.api();
        let w = w.clone();
        a.spawn(async move {
            api.post("/api/v1/lists", obj([("title", name.into())]))
                .await?;
            if let Some(t) = w.upgrade() {
                t.reload();
            }
            Ok(())
        });
    });
    let f = do_create.clone();
    entry.connect_activate(move |e| f(e));
    create.connect_clicked(glib::clone!(
        #[weak]
        entry,
        move |_| do_create(&entry)
    ));
    app.push_timeline(title, tl);
}

pub fn list_row(app: &Rc<App>, item: &Value, tl: Weak<Timeline>) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    b.add_css_class("account-row");
    b.append(&gtk::Image::from_icon_name("view-list-symbolic"));
    let l = gtk::Label::new(Some(item["title"].str()));
    l.set_hexpand(true);
    l.set_xalign(0.0);
    b.append(&l);
    let id = item["id"].str().to_string();
    let members = gtk::Button::with_label("Members");
    members.add_css_class("flat");
    let a = app.clone();
    let id2 = id.clone();
    members.connect_clicked(move |_| {
        open_accounts(&a, "List members", &format!("/api/v1/lists/{id2}/accounts"))
    });
    b.append(&members);
    let del = gtk::Button::from_icon_name("user-trash-symbolic");
    del.add_css_class("flat");
    del.set_tooltip_text(Some("Delete list"));
    let a = app.clone();
    let title = item["title"].str().to_string();
    del.connect_clicked(move |_| {
        let a = a.clone();
        let id = id.clone();
        let tl = tl.clone();
        let dlg = gtk::AlertDialog::builder()
            .message(format!("Delete the list “{title}”?"))
            .buttons(["Cancel", "Delete"])
            .cancel_button(0)
            .default_button(1)
            .build();
        let win = a.window.clone();
        glib::spawn_future_local(async move {
            if dlg.choose_future(Some(&win)).await != Ok(1) {
                return;
            }
            match a.api().delete(&format!("/api/v1/lists/{id}")).await {
                Ok(_) => {
                    if let Some(t) = tl.upgrade() {
                        t.reload();
                    }
                }
                Err(e) => a.toast(&e),
            }
        });
    });
    b.append(&del);
    b
}

pub fn open_list(app: &Rc<App>, id: &str, title: &str) {
    let tl = Timeline::new(
        app,
        Source::statuses(&format!("/api/v1/timelines/list/{id}"), &[]),
        None,
    );
    app.push_timeline(title, tl);
}

pub fn open_accounts(app: &Rc<App>, title: &str, path: &str) {
    let tl = Timeline::new(app, Source::Accounts(path.to_string()), None);
    app.push_timeline(title, tl);
}

// ----- Profiles -----

pub fn open_profile_id(app: &Rc<App>, id: &str) {
    let api = app.api();
    let a = app.clone();
    let id = id.to_string();
    app.spawn(async move {
        let v = api.get(&format!("/api/v1/accounts/{id}"), &[]).await?;
        open_profile(&a, Account::from_json(&v));
        Ok(())
    });
}

pub fn open_profile(app: &Rc<App>, acc: Account) {
    let header = header_box();
    header.add_css_class("profile-header");

    if !acc.header.is_empty() && !acc.header.ends_with("missing.png") {
        let banner = gtk::Picture::new();
        banner.set_content_fit(gtk::ContentFit::Cover);
        banner.set_can_shrink(true);
        banner.set_size_request(-1, 150);
        banner.add_css_class("banner");
        api::load_picture(&banner, &acc.header);
        header.append(&banner);
    }

    let top = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let av = gtk::Button::builder()
        .child(&status::avatar(&acc.avatar, 80))
        .build();
    av.add_css_class("flat");
    av.add_css_class("avatar-button");
    // One click opens the viewer: show the same hand as over a link.
    av.set_cursor_from_name(Some("pointer"));
    let a = app.clone();
    let av_url = acc.avatar.clone();
    av.connect_clicked(move |_| {
        super::media::open(
            &a,
            &[crate::model::Media {
                kind: "image".into(),
                url: av_url.clone(),
                preview_url: av_url.clone(),
                ..Default::default()
            }],
            0,
        )
    });
    top.append(&av);
    let names = gtk::Box::new(gtk::Orientation::Vertical, 2);
    names.set_hexpand(true);
    names.set_valign(gtk::Align::Center);
    let n = gtk::Label::new(None);
    n.set_markup(&format!(
        "<span size='x-large' weight='bold'>{}</span>{}{}",
        glib::markup_escape_text(acc.name()),
        if acc.locked { " 🔒" } else { "" },
        if acc.bot {
            " <small>🤖 bot</small>"
        } else {
            ""
        }
    ));
    n.set_xalign(0.0);
    n.set_wrap(true);
    let handle = gtk::Label::new(Some(&format!("@{}", acc.acct)));
    handle.set_xalign(0.0);
    handle.set_selectable(true);
    handle.add_css_class("dim-label");
    names.append(&n);
    names.append(&handle);
    let follows_you = gtk::Label::new(Some("Follows you"));
    follows_you.add_css_class("tag-badge");
    follows_you.set_halign(gtk::Align::Start);
    follows_you.set_visible(false);
    names.append(&follows_you);
    top.append(&names);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    actions.set_valign(gtk::Align::Center);
    top.append(&actions);
    header.append(&top);

    if let Some(moved) = &acc.moved_to {
        let mv = gtk::Button::with_label(&format!("This account has moved to @{}", moved.acct));
        let a = app.clone();
        let m = (**moved).clone();
        mv.connect_clicked(move |_| open_profile(&a, m.clone()));
        header.append(&mv);
    }

    if !acc.note_html.trim().is_empty() {
        let note = status::rich_label(app, &acc.note_html, None);
        note.set_selectable(true);
        header.append(&note);
    }
    if !acc.fields.is_empty() {
        let grid = gtk::Grid::new();
        grid.set_column_spacing(12);
        grid.set_row_spacing(4);
        grid.add_css_class("fields");
        for (i, f) in acc.fields.iter().enumerate() {
            let k = gtk::Label::new(Some(&f.name));
            k.add_css_class("dim-label");
            k.set_xalign(0.0);
            k.set_valign(gtk::Align::Start);
            k.set_wrap(true);
            let v = status::rich_label(app, &f.value_html, None);
            if f.verified {
                v.add_css_class("verified");
                v.set_tooltip_text(Some("Ownership of this link was verified"));
                v.set_markup(&format!("✓ {}", html::to_markup(&f.value_html)));
            }
            grid.attach(&k, 0, i as i32, 1, 1);
            grid.attach(&v, 1, i as i32, 1, 1);
        }
        header.append(&grid);
    }

    let stats = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    let posts = gtk::Label::new(None);
    posts.set_markup(&format!("<b>{}</b> posts", util::count(acc.statuses_count)));
    posts.set_margin_end(8);
    stats.append(&posts);
    for (count, label, path) in [
        (acc.following_count, "following", "following"),
        (acc.followers_count, "followers", "followers"),
    ] {
        let b = gtk::Button::new();
        let l = gtk::Label::new(None);
        l.set_markup(&format!("<b>{}</b> {label}", util::count(count)));
        b.set_child(Some(&l));
        b.add_css_class("flat");
        b.set_cursor_from_name(Some("pointer"));
        let a = app.clone();
        let id = acc.id.clone();
        let title = format!("{} — {label}", acc.name());
        b.connect_clicked(move |_| {
            open_accounts(&a, &title, &format!("/api/v1/accounts/{id}/{path}"))
        });
        stats.append(&b);
    }
    if !acc.created_at.is_empty() {
        let j = gtk::Label::new(Some(&format!(
            "Joined {}",
            util::absolute_time(&acc.created_at)
                .split(' ')
                .next()
                .unwrap_or("")
        )));
        j.add_css_class("dim-label");
        j.set_hexpand(true);
        j.set_xalign(1.0);
        stats.append(&j);
    }
    header.append(&stats);

    let id = acc.id.clone();
    let source_for = move |i: usize| {
        let p = format!("/api/v1/accounts/{id}/statuses");
        match i {
            0 => Source::statuses(&p, &[("exclude_replies", "true")]),
            1 => Source::statuses(&p, &[]),
            2 => Source::statuses(&p, &[("only_media", "true")]),
            _ => Source::statuses(&p, &[("pinned", "true")]),
        }
    };
    let tl_slot: Rc<std::cell::RefCell<Weak<Timeline>>> = Rc::default();
    let slot = tl_slot.clone();
    let sf = source_for.clone();
    header.append(&tabs(
        &["Posts", "Posts & replies", "Media", "Pinned"],
        move |i| {
            if let Some(tl) = slot.borrow().upgrade() {
                tl.set_source(sf(i));
            }
        },
    ));

    let mine = acc.id == app.my_id()
        || app
            .config
            .borrow()
            .active_account()
            .is_some_and(|a| a.id == acc.id);
    if mine {
        let edit = gtk::Button::with_label("Edit profile");
        let a = app.clone();
        edit.connect_clicked(move |_| edit_profile(&a));
        actions.append(&edit);
    } else {
        let api = app.api();
        let a = app.clone();
        let acc2 = acc.clone();
        let actions_w = actions.downgrade();
        let fy = follows_you.downgrade();
        glib::spawn_future_local(async move {
            let v = api
                .get("/api/v1/accounts/relationships", &[("id[]", &acc2.id)])
                .await
                .unwrap_or(Value::Null);
            let rel = Relationship::from_json(v.arr().first().unwrap_or(&Value::Null));
            if let Some(f) = fy.upgrade() {
                f.set_visible(rel.followed_by);
            }
            if let Some(b) = actions_w.upgrade() {
                fill_profile_actions(&a, &b, &acc2, &rel);
            }
        });
    }

    let tl = Timeline::new(app, source_for(0), Some(header.upcast()));
    *tl_slot.borrow_mut() = Rc::downgrade(&tl);
    let title = acc.name().to_string();
    app.push_timeline(&title, tl);
}

fn fill_profile_actions(app: &Rc<App>, b: &gtk::Box, acc: &Account, rel: &Relationship) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
    // Every relationship-changing call returns the new relationship.
    let run = {
        let app = app.clone();
        let acc = acc.clone();
        let b = b.downgrade();
        Rc::new(move |path: String, body: Value| {
            let api = app.api();
            let a = app.clone();
            let acc = acc.clone();
            let b = b.clone();
            app.spawn(async move {
                let v = api
                    .post(&format!("/api/v1/accounts/{}/{path}", acc.id), body)
                    .await?;
                let rel = Relationship::from_json(&v);
                a.change_hidden(|h| {
                    if rel.blocking || rel.muting {
                        h.add_account(&acc.id);
                    } else {
                        h.remove_account(&acc.id);
                    }
                });
                if let Some(b) = b.upgrade() {
                    fill_profile_actions(&a, &b, &acc, &rel);
                }
                Ok(())
            });
        })
    };

    let (label, path) = if rel.blocking {
        ("Unblock", "unblock")
    } else if rel.following {
        ("Unfollow", "unfollow")
    } else if rel.requested {
        ("Cancel request", "unfollow")
    } else {
        (
            if acc.locked {
                "Request follow"
            } else {
                "Follow"
            },
            "follow",
        )
    };
    let main = gtk::Button::with_label(label);
    if path == "follow" {
        main.add_css_class("suggested-action");
    }
    let r = run.clone();
    main.connect_clicked(move |_| r(path.to_string(), Value::Null));
    b.append(&main);

    if rel.following {
        let bell = gtk::ToggleButton::new();
        bell.set_icon_name(if rel.notifying {
            "preferences-system-notifications-symbolic"
        } else {
            "notifications-disabled-symbolic"
        });
        bell.set_active(rel.notifying);
        bell.set_tooltip_text(Some("Notify me when they post"));
        let r = run.clone();
        let on = !rel.notifying;
        bell.connect_clicked(move |_| r("follow".into(), obj([("notify", on.into())])));
        b.append(&bell);
    }

    let mention = gtk::Button::from_icon_name("mail-message-new-symbolic");
    mention.set_tooltip_text(Some("Mention"));
    let a = app.clone();
    let acct = acc.acct.clone();
    mention.connect_clicked(move |_| compose::open(&a, compose::Mode::Mention(acct.clone())));
    b.append(&mention);

    let menu = gtk::MenuButton::new();
    menu.set_icon_name("view-more-symbolic");
    let pop = gtk::Popover::new();
    let mb = gtk::Box::new(gtk::Orientation::Vertical, 0);
    pop.set_child(Some(&mb));
    menu.set_popover(Some(&pop));
    let item = |label: String, f: Rc<dyn Fn()>| {
        let btn = gtk::Button::with_label(&label);
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
        mb.append(&btn);
    };
    let acct = &acc.acct;
    if rel.following {
        let r = run.clone();
        let show = !rel.showing_reblogs;
        item(
            if rel.showing_reblogs {
                format!("Hide boosts from @{acct}")
            } else {
                format!("Show boosts from @{acct}")
            },
            Rc::new(move || r("follow".into(), obj([("reblogs", show.into())]))),
        );
    }
    let r = run.clone();
    if rel.muting {
        item(
            format!("Unmute @{acct}"),
            Rc::new(move || r("unmute".into(), Value::Null)),
        );
    } else {
        let a = app.clone();
        let acc2 = acc.clone();
        item(
            format!("Mute @{acct}"),
            Rc::new(move || confirm_account_action(&a, &acc2, "mute")),
        );
    }
    let r = run.clone();
    if rel.blocking {
        item(
            format!("Unblock @{acct}"),
            Rc::new(move || r("unblock".into(), Value::Null)),
        );
    } else {
        let a = app.clone();
        let acc2 = acc.clone();
        item(
            format!("Block @{acct}"),
            Rc::new(move || confirm_account_action(&a, &acc2, "block")),
        );
    }
    if let Some((_, domain)) = acc.acct.split_once('@') {
        let a = app.clone();
        let domain = domain.to_string();
        item(
            if rel.domain_blocking {
                format!("Unblock domain {domain}")
            } else {
                format!("Block domain {domain}")
            },
            Rc::new({
                let blocking = rel.domain_blocking;
                move || {
                    if blocking {
                        unblock_domain(&a, &domain);
                    } else {
                        confirm_domain_block(&a, &domain);
                    }
                }
            }),
        );
    }
    if rel.following {
        let a = app.clone();
        let acc2 = acc.clone();
        item(
            "Add to or remove from lists…".into(),
            Rc::new(move || manage_lists(&a, &acc2)),
        );
    }
    let a = app.clone();
    let acc2 = acc.clone();
    item(
        format!("Report @{acct}…"),
        Rc::new(move || report(&a, &acc2, None)),
    );
    let a = app.clone();
    let url = acc.url.clone();
    item("Open in browser".into(), Rc::new(move || a.open_url(&url)));
    let a = app.clone();
    let url = acc.url.clone();
    item(
        "Copy link to profile".into(),
        Rc::new(move || a.copy_text(&url)),
    );
    b.append(&menu);
}

pub fn confirm_account_action(app: &Rc<App>, acc: &Account, verb: &'static str) {
    let a = app.clone();
    let acc = acc.clone();
    let (msg, detail) = match verb {
        "mute" => (
            format!("Mute @{}?", acc.acct),
            "You won't see their posts or notifications. They won't know they've been muted.",
        ),
        _ => (
            format!("Block @{}?", acc.acct),
            "They won't be able to follow you or see your posts, and you won't see theirs.",
        ),
    };
    let dlg = gtk::AlertDialog::builder()
        .message(msg)
        .detail(detail)
        .buttons(["Cancel", if verb == "mute" { "Mute" } else { "Block" }])
        .cancel_button(0)
        .default_button(1)
        .build();
    let win = app.window.clone();
    glib::spawn_future_local(async move {
        if dlg.choose_future(Some(&win)).await != Ok(1) {
            return;
        }
        match a
            .api()
            .post(&format!("/api/v1/accounts/{}/{verb}", acc.id), Value::Null)
            .await
        {
            Ok(_) => {
                a.change_hidden(|h| h.add_account(&acc.id));
                a.toast(&format!(
                    "@{} {}",
                    acc.acct,
                    if verb == "mute" { "muted" } else { "blocked" }
                ))
            }
            Err(e) => a.toast(&e),
        }
    });
}

pub fn confirm_domain_block(app: &Rc<App>, domain: &str) {
    let a = app.clone();
    let domain = domain.to_string();
    let dlg = gtk::AlertDialog::builder()
        .message(format!("Block the entire domain {domain}?"))
        .detail(
            "You won't see posts or notifications from anyone on it, and your \
             followers there will be removed.",
        )
        .buttons(["Cancel", "Block domain"])
        .cancel_button(0)
        .default_button(1)
        .build();
    let win = app.window.clone();
    glib::spawn_future_local(async move {
        if dlg.choose_future(Some(&win)).await != Ok(1) {
            return;
        }
        let body = Body::Form(vec![("domain".into(), domain.clone())]);
        match a
            .api()
            .request("POST", "/api/v1/domain_blocks", &[], body)
            .await
        {
            Ok(_) => {
                a.change_hidden(|h| h.add_domain(&domain));
                a.toast(&format!("Blocked {domain}"));
            }
            Err(e) => a.toast(&e),
        }
    });
}

fn unblock_domain(app: &Rc<App>, domain: &str) {
    let api = app.api();
    let a = app.clone();
    let domain = domain.to_string();
    app.spawn(async move {
        let body = Body::Form(vec![("domain".into(), domain.clone())]);
        api.request("DELETE", "/api/v1/domain_blocks", &[], body)
            .await?;
        a.change_hidden(|h| h.remove_domain(&domain));
        a.toast(&format!("Unblocked {domain}"));
        Ok(())
    });
}

fn dialog(app: &Rc<App>, title: &str, w: i32, h: i32) -> (gtk::Window, gtk::Box) {
    let win = gtk::Window::builder()
        .title(title)
        .transient_for(&app.window)
        .modal(true)
        .default_width(w)
        .default_height(h)
        .build();
    let vb = gtk::Box::new(gtk::Orientation::Vertical, 10);
    vb.set_margin_top(14);
    vb.set_margin_bottom(14);
    vb.set_margin_start(14);
    vb.set_margin_end(14);
    win.set_child(Some(&vb));
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(glib::clone!(
        #[weak]
        win,
        #[upgrade_or]
        glib::Propagation::Proceed,
        move |_, k, _, _| {
            if k == gtk::gdk::Key::Escape {
                win.close();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        }
    ));
    win.add_controller(keys);
    (win, vb)
}

fn button_row(cancel_win: &gtk::Window, ok_label: &str) -> (gtk::Box, gtk::Button) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label("Cancel");
    cancel.connect_clicked(glib::clone!(
        #[weak]
        cancel_win,
        move |_| cancel_win.close()
    ));
    let ok = gtk::Button::with_label(ok_label);
    ok.add_css_class("suggested-action");
    row.append(&cancel);
    row.append(&ok);
    (row, ok)
}

fn manage_lists(app: &Rc<App>, acc: &Account) {
    let (win, vb) = dialog(app, &format!("Lists for @{}", acc.acct), 360, 320);
    let info = gtk::Label::new(Some("Loading lists…"));
    vb.append(&info);
    let checks = gtk::Box::new(gtk::Orientation::Vertical, 4);
    vb.append(&checks);
    win.present();
    let api = app.api();
    let a = app.clone();
    let id = acc.id.clone();
    let checks_w = checks.downgrade();
    let info_w = info.downgrade();
    app.spawn(async move {
        let all = api.get("/api/v1/lists", &[]).await?;
        let member = api
            .get(&format!("/api/v1/accounts/{id}/lists"), &[])
            .await?;
        let (Some(checks), Some(info)) = (checks_w.upgrade(), info_w.upgrade()) else {
            return Ok(());
        };
        info.set_text(if all.arr().is_empty() {
            "You have no lists yet. Create one under Lists."
        } else {
            "Toggle membership:"
        });
        for l in all.arr() {
            let lid = l["id"].str().to_string();
            let c = gtk::CheckButton::with_label(l["title"].str());
            c.set_active(member.arr().iter().any(|m| m["id"].str() == lid));
            let a2 = a.clone();
            let id = id.clone();
            c.connect_toggled(move |c| {
                let api = a2.api();
                let on = c.is_active();
                let path = format!("/api/v1/lists/{lid}/accounts");
                let id = id.clone();
                a2.spawn(async move {
                    let body = Body::Form(vec![("account_ids[]".into(), id)]);
                    api.request(if on { "POST" } else { "DELETE" }, &path, &[], body)
                        .await?;
                    Ok(())
                });
            });
            checks.append(&c);
        }
        Ok(())
    });
}

pub fn report(app: &Rc<App>, acc: &Account, status_id: Option<&str>) {
    let (win, vb) = dialog(app, &format!("Report @{}", acc.acct), 440, 360);
    let reason = gtk::DropDown::from_strings(&[
        "Spam",
        "It violates server rules",
        "It's illegal",
        "Something else",
    ]);
    reason.set_selected(3);
    vb.append(&gtk::Label::builder().label("Reason").xalign(0.0).build());
    vb.append(&reason);
    vb.append(
        &gtk::Label::builder()
            .label("Additional comments for the moderators")
            .xalign(0.0)
            .build(),
    );
    let text = gtk::TextView::new();
    text.set_wrap_mode(gtk::WrapMode::WordChar);
    let sw = gtk::ScrolledWindow::builder()
        .child(&text)
        .vexpand(true)
        .min_content_height(100)
        .build();
    sw.add_css_class("frame");
    vb.append(&sw);
    let forward = gtk::CheckButton::with_label("Forward to the remote server too");
    forward.set_visible(acc.acct.contains('@'));
    vb.append(&forward);
    let (row, ok) = button_row(&win, "Send report");
    vb.append(&row);
    let a = app.clone();
    let acc_id = acc.id.clone();
    let sid = status_id.map(str::to_string);
    ok.connect_clicked(glib::clone!(
        #[weak]
        win,
        move |_| {
            let buf = text.buffer();
            let comment = buf
                .text(&buf.start_iter(), &buf.end_iter(), false)
                .to_string();
            let category = match reason.selected() {
                0 => "spam",
                1 => "violation",
                2 => "legal",
                _ => "other",
            };
            let mut body = obj([
                ("account_id", acc_id.as_str().into()),
                ("comment", comment.into()),
                ("category", category.into()),
                ("forward", forward.is_active().into()),
            ]);
            if let Some(s) = &sid {
                body.set("status_ids", vec![Value::from(s.as_str())].into());
            }
            let api = a.api();
            let a2 = a.clone();
            a.spawn(async move {
                api.post("/api/v1/reports", body).await?;
                a2.toast("Report sent. Thank you.");
                Ok(())
            });
            win.close();
        }
    ));
    win.present();
}

pub fn edit_profile(app: &Rc<App>) {
    let (win, vb) = dialog(app, "Edit profile", 520, 640);
    let api = app.api();
    let a = app.clone();
    let win_w = win.downgrade();
    let vb_w = vb.downgrade();
    let loading = gtk::Spinner::new();
    loading.start();
    vb.append(&loading);
    win.present();
    app.spawn(async move {
        let v = api.get("/api/v1/accounts/verify_credentials", &[]).await?;
        let (Some(win), Some(vb)) = (win_w.upgrade(), vb_w.upgrade()) else {
            return Ok(());
        };
        vb.remove(&loading);
        let me = Account::from_json(&v);

        let images = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let av = status::avatar(&me.avatar, 64);
        images.append(&av);
        for (label, field) in [("Change avatar…", "avatar"), ("Change header…", "header")] {
            let b = gtk::Button::with_label(label);
            b.set_valign(gtk::Align::Center);
            let a2 = a.clone();
            let win2 = win.downgrade();
            b.connect_clicked(move |_| upload_profile_image(&a2, field, win2.upgrade()));
            images.append(&b);
        }
        vb.append(&images);

        let grid = gtk::Grid::new();
        grid.set_row_spacing(8);
        grid.set_column_spacing(8);
        let name = gtk::Entry::new();
        name.set_text(&me.display_name);
        name.set_hexpand(true);
        grid.attach(
            &gtk::Label::builder()
                .label("Display name")
                .xalign(0.0)
                .build(),
            0,
            0,
            1,
            1,
        );
        grid.attach(&name, 1, 0, 1, 1);
        vb.append(&grid);
        vb.append(&gtk::Label::builder().label("Bio").xalign(0.0).build());
        let note = gtk::TextView::new();
        note.set_wrap_mode(gtk::WrapMode::WordChar);
        note.buffer().set_text(
            me.source_note
                .as_deref()
                .unwrap_or(&html::to_text(&me.note_html)),
        );
        let sw = gtk::ScrolledWindow::builder()
            .child(&note)
            .min_content_height(90)
            .build();
        sw.add_css_class("frame");
        vb.append(&sw);

        vb.append(
            &gtk::Label::builder()
                .label("Profile metadata")
                .xalign(0.0)
                .build(),
        );
        let fgrid = gtk::Grid::new();
        fgrid.set_row_spacing(4);
        fgrid.set_column_spacing(6);
        let src_fields = v["source"]["fields"].arr();
        let mut field_entries = Vec::new();
        for i in 0..4 {
            let k = gtk::Entry::new();
            k.set_placeholder_text(Some("Label"));
            let val = gtk::Entry::new();
            val.set_placeholder_text(Some("Content"));
            val.set_hexpand(true);
            if let Some(f) = src_fields.get(i) {
                k.set_text(f["name"].str());
                val.set_text(f["value"].str());
            }
            fgrid.attach(&k, 0, i as i32, 1, 1);
            fgrid.attach(&val, 1, i as i32, 1, 1);
            field_entries.push((k, val));
        }
        vb.append(&fgrid);
        let locked = gtk::CheckButton::with_label("Require follow requests (locked account)");
        locked.set_active(me.locked);
        let bot = gtk::CheckButton::with_label("This is an automated (bot) account");
        bot.set_active(me.bot);
        let discoverable = gtk::CheckButton::with_label("Feature profile and posts in discovery");
        discoverable.set_active(v["discoverable"].bool());
        vb.append(&locked);
        vb.append(&bot);
        vb.append(&discoverable);
        let (row, ok) = button_row(&win, "Save");
        vb.append(&row);
        let a2 = a.clone();
        ok.connect_clicked(glib::clone!(
            #[weak]
            win,
            move |_| {
                let buf = note.buffer();
                let bool_s = |b: bool| {
                    if b {
                        "true".to_string()
                    } else {
                        "false".to_string()
                    }
                };
                let mut form: Vec<(String, String)> = vec![
                    ("display_name".into(), name.text().to_string()),
                    (
                        "note".into(),
                        buf.text(&buf.start_iter(), &buf.end_iter(), false)
                            .to_string(),
                    ),
                    ("locked".into(), bool_s(locked.is_active())),
                    ("bot".into(), bool_s(bot.is_active())),
                    ("discoverable".into(), bool_s(discoverable.is_active())),
                ];
                for (i, (k, v)) in field_entries.iter().enumerate() {
                    form.push((
                        format!("fields_attributes[{i}][name]"),
                        k.text().to_string(),
                    ));
                    form.push((
                        format!("fields_attributes[{i}][value]"),
                        v.text().to_string(),
                    ));
                }
                let api = a2.api();
                let a3 = a2.clone();
                a2.spawn(async move {
                    let v = api
                        .request(
                            "PATCH",
                            "/api/v1/accounts/update_credentials",
                            &[],
                            Body::Form(form),
                        )
                        .await?;
                    *a3.me.borrow_mut() = Some(Account::from_json(&v.json));
                    a3.toast("Profile saved");
                    Ok(())
                });
                win.close();
            }
        ));
        Ok(())
    });
}

fn upload_profile_image(app: &Rc<App>, field: &'static str, parent: Option<gtk::Window>) {
    let filter = gtk::FileFilter::new();
    filter.add_mime_type("image/*");
    filter.set_name(Some("Images"));
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    let dlg = gtk::FileDialog::builder()
        .title("Choose an image")
        .filters(&filters)
        .modal(true)
        .build();
    let a = app.clone();
    let parent = parent.unwrap_or_else(|| app.window.clone().upcast());
    app.spawn(async move {
        let Ok(file) = dlg.open_future(Some(&parent)).await else {
            return Ok(());
        };
        let (bytes, _) = file.load_bytes_future().await.map_err(|e| e.to_string())?;
        let fname = file
            .basename()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "image".into());
        let (ctype, _) = gio::content_type_guess(Some(&fname), &bytes[..]);
        let mime = gio::content_type_get_mime_type(&ctype)
            .map(|m| m.to_string())
            .unwrap_or_else(|| "image/png".into());
        let mut mp = soup::Multipart::new("multipart/form-data");
        mp.append_form_file(field, Some(&fname), Some(&mime), &bytes);
        a.toast("Uploading…");
        let v = a
            .api()
            .request(
                "PATCH",
                "/api/v1/accounts/update_credentials",
                &[],
                Body::Multipart(mp),
            )
            .await?;
        *a.me.borrow_mut() = Some(Account::from_json(&v.json));
        a.toast(if field == "avatar" {
            "Avatar updated"
        } else {
            "Header updated"
        });
        Ok(())
    });
}
