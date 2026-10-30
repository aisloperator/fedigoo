//! A paged, scrollable list of API items (statuses, notifications, accounts…).

use super::{account, app::App, pages, status};
use crate::api::Body;
use crate::json::Value;
use crate::model::{Account, Notification, Status, Tag};
use gtk::{glib, prelude::*};
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

#[derive(Clone, Debug)]
pub enum Source {
    Statuses(String, Vec<(String, String)>),
    Notifications(Vec<(String, String)>),
    Conversations,
    Accounts(String),
    FollowRequests,
    Tags(String),
    Links,
    Lists,
}

impl Source {
    pub fn statuses(path: &str, query: &[(&str, &str)]) -> Source {
        Source::Statuses(
            path.to_string(),
            query
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        )
    }

    fn request(&self) -> (String, Vec<(String, String)>) {
        let limit = || ("limit".to_string(), "30".to_string());
        match self {
            Source::Statuses(p, q) => {
                let mut q = q.clone();
                q.push(limit());
                (p.clone(), q)
            }
            Source::Notifications(q) => {
                let mut q = q.clone();
                q.push(limit());
                ("/api/v1/notifications".into(), q)
            }
            Source::Conversations => ("/api/v1/conversations".into(), vec![limit()]),
            Source::Accounts(p) => (p.clone(), vec![("limit".into(), "40".into())]),
            Source::FollowRequests => (
                "/api/v1/follow_requests".into(),
                vec![("limit".into(), "40".into())],
            ),
            Source::Tags(p) => (p.clone(), vec![("limit".into(), "20".into())]),
            Source::Links => (
                "/api/v1/trends/links".into(),
                vec![("limit".into(), "20".into())],
            ),
            Source::Lists => ("/api/v1/lists".into(), vec![]),
        }
    }

    /// Whether a streamed item of `event` belongs in this timeline.
    fn accepts(&self, event: &str, item: &Value) -> bool {
        match (self, event) {
            (Source::Statuses(p, _), "update") => p == "/api/v1/timelines/home",
            (Source::Notifications(q), "notification") => {
                let kind = item["type"].str();
                let types: Vec<_> = q
                    .iter()
                    .filter(|(k, _)| k == "types[]")
                    .map(|(_, v)| v)
                    .collect();
                types.is_empty() || types.iter().any(|t| *t == kind)
            }
            _ => false,
        }
    }
}

pub struct Timeline {
    app: Rc<App>,
    me: Weak<Timeline>,
    root: gtk::Box,
    scroller: gtk::ScrolledWindow,
    list: gtk::ListBox,
    more_btn: gtk::Button,
    spinner: gtk::Spinner,
    end_label: gtk::Label,
    new_btn: gtk::Button,
    source: RefCell<Source>,
    next: RefCell<Option<String>>,
    offset: Cell<usize>,
    loading: Cell<bool>,
    epoch: Cell<u32>,
    pending: RefCell<Vec<Value>>,
    /// Status ids shown by each row, for live deletes and edits.
    rows: RefCell<Vec<(Vec<String>, gtk::ListBoxRow)>>,
    activations: RefCell<Vec<(gtk::ListBoxRow, Rc<dyn Fn()>)>>,
}

impl Timeline {
    pub fn new(app: &Rc<App>, source: Source, header: Option<gtk::Widget>) -> Rc<Timeline> {
        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::None);
        list.set_activate_on_single_click(false);
        list.add_css_class("timeline");

        let more_btn = gtk::Button::with_label("Load more");
        more_btn.set_halign(gtk::Align::Center);
        more_btn.set_visible(false);
        let spinner = gtk::Spinner::new();
        spinner.set_size_request(24, 24);
        let end_label = gtk::Label::new(Some("Nothing to show here."));
        end_label.add_css_class("dim-label");
        end_label.set_visible(false);
        let footer = gtk::Box::new(gtk::Orientation::Vertical, 6);
        footer.set_margin_top(12);
        footer.set_margin_bottom(24);
        footer.append(&spinner);
        footer.append(&more_btn);
        footer.append(&end_label);

        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.add_css_class("timeline-column");
        if let Some(h) = &header {
            content.append(h);
        }
        content.append(&list);
        content.append(&footer);
        let clamp = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        clamp.set_halign(gtk::Align::Center);
        clamp.append(&content);
        content.set_size_request(380, -1);
        content.set_hexpand(true);
        clamp.set_hexpand(true);

        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&clamp)
            .build();

        let new_btn = gtk::Button::with_label("New posts");
        new_btn.add_css_class("suggested-action");
        new_btn.add_css_class("pill");
        new_btn.set_halign(gtk::Align::Center);
        new_btn.set_valign(gtk::Align::Start);
        new_btn.set_margin_top(8);
        new_btn.set_visible(false);
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&scroller));
        overlay.add_overlay(&new_btn);
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.append(&overlay);

        let tl = Rc::new_cyclic(|me| Timeline {
            app: app.clone(),
            me: me.clone(),
            root,
            scroller,
            list,
            more_btn,
            spinner,
            end_label,
            new_btn,
            source: RefCell::new(source),
            next: RefCell::new(None),
            offset: Cell::new(0),
            loading: Cell::new(false),
            epoch: Cell::new(0),
            pending: RefCell::new(Vec::new()),
            rows: RefCell::new(Vec::new()),
            activations: RefCell::new(Vec::new()),
        });

        let w = Rc::downgrade(&tl);
        tl.more_btn.connect_clicked(move |_| {
            if let Some(t) = w.upgrade() {
                t.load_more();
            }
        });
        let w = Rc::downgrade(&tl);
        tl.scroller.connect_edge_reached(move |_, pos| {
            if pos == gtk::PositionType::Bottom
                && let Some(t) = w.upgrade()
            {
                t.load_more();
            }
        });
        let w = Rc::downgrade(&tl);
        tl.new_btn.connect_clicked(move |_| {
            if let Some(t) = w.upgrade() {
                t.show_pending();
            }
        });
        let w = Rc::downgrade(&tl);
        tl.list.connect_row_activated(move |_, row| {
            let Some(t) = w.upgrade() else { return };
            let f = t
                .activations
                .borrow()
                .iter()
                .find(|(r, _)| r == row)
                .map(|(_, f)| f.clone());
            if let Some(f) = f {
                f();
            }
        });
        tl.reload();
        tl
    }

    pub fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    pub fn set_source(&self, source: Source) {
        *self.source.borrow_mut() = source;
        self.reload();
    }

    pub fn reload(&self) {
        self.epoch.set(self.epoch.get().wrapping_add(1));
        self.list.remove_all();
        self.rows.borrow_mut().clear();
        self.activations.borrow_mut().clear();
        self.pending.borrow_mut().clear();
        self.new_btn.set_visible(false);
        *self.next.borrow_mut() = None;
        self.offset.set(0);
        self.loading.set(false);
        self.fetch(None);
    }

    fn load_more(&self) {
        let next = self.next.borrow().clone();
        if let Some(url) = next
            && !self.loading.get()
        {
            self.fetch(Some(url));
        }
    }

    fn fetch(&self, url: Option<String>) {
        self.loading.set(true);
        self.spinner.set_visible(true);
        self.spinner.start();
        self.more_btn.set_visible(false);
        self.end_label.set_visible(false);
        let (path, query) = match url {
            Some(u) => (u, vec![]),
            None => self.source.borrow().request(),
        };
        let epoch = self.epoch.get();
        let api = self.app.api();
        let me = self.me.clone();
        glib::spawn_future_local(async move {
            let q: Vec<(&str, &str)> = query
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();
            let res = api.request("GET", &path, &q, Body::Empty).await;
            let Some(t) = me.upgrade() else { return };
            if t.epoch.get() != epoch {
                return;
            }
            t.loading.set(false);
            t.spinner.stop();
            t.spinner.set_visible(false);
            match res {
                Ok(resp) => {
                    let items = resp.json.arr();
                    for item in items {
                        t.append_item(item, false);
                    }
                    let offset = t.offset.get() + items.len();
                    t.offset.set(offset);
                    let next = resp.next.or_else(|| {
                        // Trends paginate by offset without a Link header.
                        (path.contains("/trends/") && !items.is_empty()).then(|| {
                            let base = path.split('?').next().unwrap_or(&path);
                            format!("{base}?limit=20&offset={offset}")
                        })
                    });
                    t.more_btn.set_visible(next.is_some());
                    *t.next.borrow_mut() = next;
                    t.end_label.set_visible(t.list.first_child().is_none());
                }
                Err(e) => {
                    t.app.toast(&e);
                    t.end_label.set_text(&format!("Could not load: {e}"));
                    t.end_label.set_visible(true);
                }
            }
        });
    }

    fn make_row(&self, item: &Value) -> Option<(gtk::Widget, Vec<String>, Option<Rc<dyn Fn()>>)> {
        let app = &self.app;
        let source = self.source.borrow().clone();
        Some(match source {
            Source::Statuses(..) => {
                let st = Status::from_json(item);
                let ids = vec![st.id.clone(), st.shown().id.clone()];
                let a = app.clone();
                let s2 = st.clone();
                let act: Rc<dyn Fn()> = Rc::new(move || pages::open_thread(&a, s2.shown()));
                (status::build(app, st, false), ids, Some(act))
            }
            Source::Notifications(_) => {
                let n = Notification::from_json(item);
                let ids = n.status.iter().map(|s| s.id.clone()).collect();
                let a = app.clone();
                let act: Option<Rc<dyn Fn()>> = match n.status.clone() {
                    Some(s) => Some(Rc::new(move || pages::open_thread(&a, &s))),
                    None => {
                        let acc = n.account.clone();
                        Some(Rc::new(move || pages::open_profile(&a, acc.clone())))
                    }
                };
                (status::notification(app, n), ids, act)
            }
            Source::Conversations => {
                let last = &item["last_status"];
                if last.is_null() {
                    return None;
                }
                let st = Status::from_json(last);
                let ids = vec![st.id.clone()];
                let a = app.clone();
                let s2 = st.clone();
                let act: Rc<dyn Fn()> = Rc::new(move || pages::open_thread(&a, &s2));
                (status::conversation(app, item, st), ids, Some(act))
            }
            Source::Accounts(_) => {
                let acc = Account::from_json(item);
                let a = app.clone();
                let acc2 = acc.clone();
                let act: Rc<dyn Fn()> = Rc::new(move || pages::open_profile(&a, acc2.clone()));
                (account::row(app, &acc).upcast(), vec![], Some(act))
            }
            Source::FollowRequests => {
                let acc = Account::from_json(item);
                (
                    account::follow_request_row(app, &acc).upcast(),
                    vec![],
                    None,
                )
            }
            Source::Tags(_) => {
                let tag = Tag::from_json(item);
                let a = app.clone();
                let name = tag.name.clone();
                let act: Rc<dyn Fn()> = Rc::new(move || pages::open_hashtag(&a, &name));
                (account::tag_row(&tag).upcast(), vec![], Some(act))
            }
            Source::Links => {
                let a = app.clone();
                let url = item["url"].str().to_string();
                let act: Rc<dyn Fn()> = Rc::new(move || a.open_url(&url));
                (account::link_row(app, item).upcast(), vec![], Some(act))
            }
            Source::Lists => {
                let id = item["id"].str().to_string();
                let title = item["title"].str().to_string();
                let a = app.clone();
                let act: Rc<dyn Fn()> = Rc::new(move || pages::open_list(&a, &id, &title));
                (
                    pages::list_row(app, item, self.me.clone()).upcast(),
                    vec![],
                    Some(act),
                )
            }
        })
    }

    fn append_item(&self, item: &Value, at_top: bool) {
        let Some((w, ids, act)) = self.make_row(item) else {
            return;
        };
        let row = gtk::ListBoxRow::new();
        row.set_child(Some(&w));
        row.set_activatable(act.is_some());
        if at_top {
            self.list.prepend(&row);
        } else {
            self.list.append(&row);
        }
        if let Some(f) = act {
            self.activations.borrow_mut().push((row.clone(), f));
        }
        self.rows.borrow_mut().push((ids, row));
    }

    fn contains(&self, id: &str) -> bool {
        self.rows
            .borrow()
            .iter()
            .any(|(ids, _)| ids.first().is_some_and(|i| i == id))
    }

    pub fn on_stream(&self, event: &str, item: &Value) {
        if event == "status.update" {
            // Edits: rebuild rows that show this status directly (not boosts).
            if !matches!(*self.source.borrow(), Source::Statuses(..)) {
                return;
            }
            let id = item["id"].str();
            let st = Status::from_json(item);
            for (ids, row) in self.rows.borrow().iter() {
                if ids.first().is_some_and(|i| i == id) {
                    row.set_child(Some(&status::build(&self.app, st.clone(), false)));
                }
            }
            return;
        }
        if !self.source.borrow().accepts(event, item) || self.loading.get() {
            return;
        }
        let id = item["id"].str();
        if self.contains(id) || self.pending.borrow().iter().any(|p| p["id"].str() == id) {
            return;
        }
        let at_top = self.scroller.vadjustment().value() < 1.0;
        if at_top && self.pending.borrow().is_empty() {
            self.append_item(item, true);
            self.end_label.set_visible(false);
        } else {
            self.pending.borrow_mut().push(item.clone());
            let n = self.pending.borrow().len();
            self.new_btn
                .set_label(&format!("{n} new item{}", if n == 1 { "" } else { "s" }));
            self.new_btn.set_visible(true);
        }
    }

    fn show_pending(&self) {
        let items: Vec<Value> = self.pending.borrow_mut().drain(..).collect();
        for item in &items {
            self.append_item(item, true);
        }
        self.new_btn.set_visible(false);
        self.scroller.vadjustment().set_value(0.0);
    }

    pub fn remove_status(&self, id: &str) {
        self.pending.borrow_mut().retain(|p| p["id"].str() != id);
        self.rows.borrow_mut().retain(|(ids, row)| {
            if ids.iter().any(|i| i == id) {
                self.list.remove(row);
                false
            } else {
                true
            }
        });
    }
}
