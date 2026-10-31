//! The main window, navigation, account handling and streaming.

use super::{compose, fit, login, pages, prefs, timeline::Timeline};
use crate::api::{self, Api};
use crate::config::Config;
use crate::json::Value;
use crate::model::{self, Status};
use gtk::{gdk, gio, glib, prelude::*};
use soup::prelude::*;
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::future::Future;
use std::rc::Rc;

const CSS: &str = include_str!("style.css");

/// One entry on the navigation stack.
struct Page {
    widget: gtk::Widget,
    title: String,
    /// Keeps the page's controller alive while the page is shown.
    _keep: Option<Rc<dyn Any>>,
    refresh: Option<Rc<dyn Fn()>>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Root {
    Home,
    Notifications,
    Mentions,
    Local,
    Federated,
    Explore,
    Search,
    Conversations,
    Bookmarks,
    Favourites,
    Lists,
    FollowedTags,
    FollowRequests,
    Profile,
    Mutes,
    Blocks,
}

const ROOTS: &[(Root, &str, &str)] = &[
    (Root::Home, "Home", "user-home-symbolic"),
    (
        Root::Notifications,
        "Notifications",
        "preferences-system-notifications-symbolic",
    ),
    (Root::Mentions, "Mentions", "mail-replied-symbolic"),
    (
        Root::Conversations,
        "Private mentions",
        "mail-unread-symbolic",
    ),
    (Root::Local, "Local", "network-server-symbolic"),
    (Root::Federated, "Federated", "network-workgroup-symbolic"),
    (Root::Explore, "Explore", "web-browser-symbolic"),
    (Root::Search, "Search", "system-search-symbolic"),
    (Root::Bookmarks, "Bookmarks", "user-bookmarks-symbolic"),
    (Root::Favourites, "Favourites", "starred-symbolic"),
    (Root::Lists, "Lists", "view-list-symbolic"),
    (
        Root::FollowedTags,
        "Followed hashtags",
        "bookmark-new-symbolic",
    ),
    (Root::FollowRequests, "Follow requests", "list-add-symbolic"),
    (Root::Profile, "My profile", "user-info-symbolic"),
    (
        Root::Mutes,
        "Muted accounts",
        "notifications-disabled-symbolic",
    ),
    (
        Root::Blocks,
        "Blocked accounts",
        "action-unavailable-symbolic",
    ),
];

pub struct App {
    pub window: gtk::ApplicationWindow,
    pub config: RefCell<Config>,
    api: RefCell<Api>,
    pub me: RefCell<Option<model::Account>>,
    pub max_chars: Cell<usize>,
    pub max_poll_options: Cell<usize>,
    pub max_media: Cell<usize>,
    nav: RefCell<Vec<Page>>,
    stack: gtk::Stack,
    back_btn: gtk::Button,
    compose_btn: gtk::Button,
    refresh_btn: gtk::Button,
    title: gtk::Label,
    subtitle: gtk::Label,
    sidebar: gtk::ListBox,
    notif_badge: gtk::Label,
    unread: Cell<u32>,
    account_btn: gtk::MenuButton,
    toast: gtk::Revealer,
    toast_label: gtk::Label,
    live: RefCell<Vec<std::rc::Weak<Timeline>>>,
    threads: RefCell<Vec<std::rc::Weak<pages::ThreadView>>>,
    stream: RefCell<Option<soup::WebsocketConnection>>,
    /// Bumped on every account switch so stale async work can bail out.
    generation: Cell<u32>,
}

impl App {
    pub fn new(gtk_app: &gtk::Application) -> Rc<App> {
        if let Some(display) = gdk::Display::default() {
            let css = gtk::CssProvider::new();
            css.load_from_string(CSS);
            gtk::style_context_add_provider_for_display(
                &display,
                &css,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }

        let window = gtk::ApplicationWindow::builder()
            .application(gtk_app)
            .title("FediGoo")
            .default_width(1000)
            .default_height(820)
            .icon_name(crate::APP_ID)
            .build();

        let header = gtk::HeaderBar::new();
        let back_btn = gtk::Button::from_icon_name("go-previous-symbolic");
        back_btn.set_tooltip_text(Some("Back (Alt+Left)"));
        back_btn.set_visible(false);
        let compose_btn = gtk::Button::from_icon_name("document-edit-symbolic");
        compose_btn.set_tooltip_text(Some("New post (Ctrl+N)"));
        let refresh_btn = gtk::Button::from_icon_name("view-refresh-symbolic");
        refresh_btn.set_tooltip_text(Some("Refresh (F5)"));
        header.pack_start(&back_btn);
        header.pack_start(&compose_btn);
        header.pack_start(&refresh_btn);

        let title = gtk::Label::new(Some("FediGoo"));
        title.add_css_class("title");
        let subtitle = gtk::Label::new(None);
        subtitle.add_css_class("subtitle");
        subtitle.set_visible(false);
        let tbox = gtk::Box::new(gtk::Orientation::Vertical, 0);
        tbox.set_valign(gtk::Align::Center);
        tbox.append(&title);
        tbox.append(&subtitle);
        header.set_title_widget(Some(&tbox));

        let account_btn = gtk::MenuButton::new();
        account_btn.set_icon_name("open-menu-symbolic");
        account_btn.set_tooltip_text(Some("Accounts and preferences"));
        header.pack_end(&account_btn);
        window.set_titlebar(Some(&header));

        let sidebar = gtk::ListBox::new();
        sidebar.add_css_class("navigation-sidebar");
        sidebar.set_selection_mode(gtk::SelectionMode::Single);
        let notif_badge = gtk::Label::new(None);
        notif_badge.add_css_class("badge");
        notif_badge.set_visible(false);
        for (root, label, icon) in ROOTS {
            let b = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            b.set_margin_start(6);
            b.set_margin_end(6);
            b.set_margin_top(4);
            b.set_margin_bottom(4);
            b.append(&gtk::Image::from_icon_name(icon));
            let l = gtk::Label::new(Some(label));
            l.set_xalign(0.0);
            l.set_hexpand(true);
            b.append(&l);
            if *root == Root::Notifications {
                b.append(&notif_badge);
            }
            sidebar.append(&b);
        }
        let side_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&sidebar)
            .width_request(200)
            .build();

        let stack = gtk::Stack::new();
        stack.set_transition_type(gtk::StackTransitionType::Crossfade);
        stack.set_hexpand(true);
        stack.set_vexpand(true);

        let toast_label = gtk::Label::new(None);
        toast_label.set_wrap(true);
        toast_label.set_max_width_chars(70);
        let toast_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        toast_box.add_css_class("toast");
        toast_box.append(&toast_label);
        let toast = gtk::Revealer::builder()
            .child(&toast_box)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::End)
            .margin_bottom(18)
            .transition_type(gtk::RevealerTransitionType::SlideUp)
            .can_target(false)
            .build();
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&stack));
        overlay.add_overlay(&toast);

        let paned = gtk::Paned::new(gtk::Orientation::Horizontal);
        paned.set_start_child(Some(&side_scroll));
        paned.set_end_child(Some(&overlay));
        paned.set_resize_start_child(false);
        paned.set_shrink_start_child(false);
        paned.set_position(210);
        window.set_child(Some(&paned));

        let app = Rc::new(App {
            window,
            config: RefCell::new(Config::load()),
            api: RefCell::new(Api::new("", None)),
            me: RefCell::new(None),
            max_chars: Cell::new(500),
            max_poll_options: Cell::new(4),
            max_media: Cell::new(4),
            nav: RefCell::new(Vec::new()),
            stack,
            back_btn,
            compose_btn,
            refresh_btn,
            title,
            subtitle,
            sidebar,
            notif_badge,
            unread: Cell::new(0),
            account_btn,
            toast,
            toast_label,
            live: RefCell::new(Vec::new()),
            threads: RefCell::new(Vec::new()),
            stream: RefCell::new(None),
            generation: Cell::new(0),
        });
        app.connect_signals(gtk_app);
        app.rebuild_account_menu();
        app.start();
        app
    }

    fn connect_signals(self: &Rc<Self>, gtk_app: &gtk::Application) {
        // Media thumbnails are limited to about half the window height.
        self.window.connect_realize(|w| {
            if let Some(surface) = w.surface() {
                fit::set_window_height(surface.height());
                surface.connect_height_notify(|s| fit::set_window_height(s.height()));
            }
        });
        let a = self.clone();
        self.back_btn.connect_clicked(move |_| a.pop());
        let a = self.clone();
        self.compose_btn
            .connect_clicked(move |_| compose::open(&a, compose::Mode::New));
        let a = self.clone();
        self.refresh_btn.connect_clicked(move |_| a.refresh());
        let a = self.clone();
        self.sidebar.connect_row_activated(move |_, row| {
            if let Some((root, ..)) = ROOTS.get(row.index() as usize) {
                a.show_root(*root);
            }
        });

        let actions: [(&str, &[&str], Box<dyn Fn(&Rc<App>)>); 5] = [
            (
                "compose",
                &["<Control>n"],
                Box::new(|a| compose::open(a, compose::Mode::New)),
            ),
            ("refresh", &["F5", "<Control>r"], Box::new(|a| a.refresh())),
            ("back", &["<Alt>Left", "Escape"], Box::new(|a| a.pop())),
            (
                "search",
                &["<Control>f"],
                Box::new(|a| a.show_root(Root::Search)),
            ),
            ("quit", &["<Control>q"], Box::new(|a| a.window.close())),
        ];
        for (name, accels, f) in actions {
            let act = gio::SimpleAction::new(name, None);
            let a = self.clone();
            act.connect_activate(move |_, _| f(&a));
            self.window.add_action(&act);
            gtk_app.set_accels_for_action(&format!("win.{name}"), accels);
        }
        let a = self.clone();
        self.window.connect_close_request(move |_| {
            if let Some(ws) = a.stream.borrow_mut().take() {
                ws.close(1000, None);
            }
            glib::Propagation::Proceed
        });
    }

    pub fn api(&self) -> Api {
        self.api.borrow().clone()
    }

    pub fn my_id(&self) -> String {
        self.me
            .borrow()
            .as_ref()
            .map(|m| m.id.clone())
            .unwrap_or_default()
    }

    pub fn generation(&self) -> u32 {
        self.generation.get()
    }

    /// Runs `fut` on the main loop, reporting any error as a toast.
    pub fn spawn(self: &Rc<Self>, fut: impl Future<Output = Result<(), String>> + 'static) {
        let a = self.clone();
        glib::spawn_future_local(async move {
            if let Err(e) = fut.await {
                a.toast(&e);
            }
        });
    }

    pub fn toast(&self, msg: &str) {
        self.toast_label.set_text(msg);
        self.toast.set_reveal_child(true);
        let toast = self.toast.downgrade();
        let label = self.toast_label.downgrade();
        let text = msg.to_string();
        glib::timeout_add_seconds_local_once(4, move || {
            // Only hide if no newer toast replaced this one.
            if let (Some(t), Some(l)) = (toast.upgrade(), label.upgrade())
                && l.text() == text
            {
                t.set_reveal_child(false);
            }
        });
    }

    pub fn open_url(&self, url: &str) {
        gtk::UriLauncher::new(url).launch(Some(&self.window), gio::Cancellable::NONE, |_| {});
    }

    pub fn copy_text(&self, text: &str) {
        self.window.clipboard().set_text(text);
        self.toast("Copied to clipboard");
    }

    // ----- Accounts -----

    fn start(self: &Rc<Self>) {
        let has = self.config.borrow().active_account().is_some();
        if has {
            let idx = self.config.borrow().active;
            self.activate_account(idx);
        } else {
            self.show_welcome();
        }
    }

    fn show_welcome(self: &Rc<Self>) {
        *self.me.borrow_mut() = None;
        self.sidebar.set_sensitive(false);
        self.compose_btn.set_sensitive(false);
        self.refresh_btn.set_sensitive(false);
        let b = gtk::Box::new(gtk::Orientation::Vertical, 18);
        b.set_valign(gtk::Align::Center);
        b.set_halign(gtk::Align::Center);
        let t = gtk::Label::new(None);
        t.set_markup("<span size='xx-large' weight='bold'>Welcome to FediGoo</span>");
        b.append(&t);
        let l = gtk::Label::new(Some(
            "A Mastodon client for the Linux desktop.\nLog in to your instance to get started.",
        ));
        l.set_justify(gtk::Justification::Center);
        l.add_css_class("dim-label");
        b.append(&l);
        let btn = gtk::Button::with_label("Log in…");
        btn.add_css_class("suggested-action");
        btn.add_css_class("pill");
        btn.set_halign(gtk::Align::Center);
        let a = self.clone();
        btn.connect_clicked(move |_| login::open(&a));
        b.append(&btn);
        self.reset_nav();
        self.push("FediGoo", b, None, None);
    }

    pub fn add_account(self: &Rc<Self>, account: crate::config::Account) {
        let idx = {
            let mut c = self.config.borrow_mut();
            c.accounts
                .retain(|x| !(x.instance == account.instance && x.id == account.id));
            c.accounts.push(account);
            c.active = c.accounts.len() - 1;
            c.save();
            c.active
        };
        self.activate_account(idx);
    }

    pub fn activate_account(self: &Rc<Self>, idx: usize) {
        let Some(acc) = self.config.borrow().accounts.get(idx).cloned() else {
            return;
        };
        {
            let mut c = self.config.borrow_mut();
            c.active = idx;
            c.save();
        }
        self.generation.set(self.generation.get().wrapping_add(1));
        if let Some(ws) = self.stream.borrow_mut().take() {
            ws.close(1000, None);
        }
        *self.api.borrow_mut() = Api::new(&acc.instance, Some(&acc.token));
        *self.me.borrow_mut() = None;
        self.unread.set(0);
        self.notif_badge.set_visible(false);
        self.sidebar.set_sensitive(true);
        self.compose_btn.set_sensitive(true);
        self.refresh_btn.set_sensitive(true);
        self.window
            .set_title(Some(&format!("FediGoo — {}", acc.full_acct())));
        self.rebuild_account_menu();
        self.show_root(Root::Home);

        let a = self.clone();
        let api = self.api();
        let generation = self.generation();
        self.spawn(async move {
            let v = api.get("/api/v1/accounts/verify_credentials", &[]).await?;
            if a.generation() != generation {
                return Ok(());
            }
            let me = model::Account::from_json(&v);
            {
                // Keep the stored name and avatar fresh for the account menu.
                let mut c = a.config.borrow_mut();
                if let Some(x) = c.accounts.get_mut(idx) {
                    x.display_name = me.name().to_string();
                    x.avatar = me.avatar.clone();
                    x.acct = me.acct.clone();
                }
                c.save();
            }
            *a.me.borrow_mut() = Some(me);
            a.rebuild_account_menu();
            let streaming = a.load_instance_info(&api).await;
            if a.config.borrow().settings.stream_updates {
                a.connect_stream(streaming, generation);
            }
            Ok(())
        });
    }

    /// Reads instance limits; returns the streaming base URL.
    async fn load_instance_info(&self, api: &Api) -> String {
        let fallback = format!("wss://{}", api.host);
        let Ok(v) = api.get("/api/v2/instance", &[]).await else {
            return fallback;
        };
        let conf = &v["configuration"];
        let n = conf["statuses"]["max_characters"].i64();
        if n > 0 {
            self.max_chars.set(n as usize);
        }
        let n = conf["polls"]["max_options"].i64();
        if n > 0 {
            self.max_poll_options.set(n as usize);
        }
        let n = conf["statuses"]["max_media_attachments"].i64();
        if n > 0 {
            self.max_media.set(n as usize);
        }
        conf["urls"]["streaming"]
            .nonempty_str()
            .map(str::to_string)
            .unwrap_or(fallback)
    }

    pub fn log_out(self: &Rc<Self>) {
        let (acc, app) = {
            let c = self.config.borrow();
            let Some(acc) = c.active_account().cloned() else {
                return;
            };
            let app = c.app_for(&acc.instance).cloned();
            (acc, app)
        };
        // Revoke the token on the server; best effort.
        if let Some(reg) = app {
            let api = Api::new(&acc.instance, None);
            let token = acc.token.clone();
            glib::spawn_future_local(async move {
                let _ = api
                    .post_form(
                        "/oauth/revoke",
                        &[
                            ("client_id", &reg.client_id),
                            ("client_secret", &reg.client_secret),
                            ("token", &token),
                        ],
                    )
                    .await;
            });
        }
        let next = {
            let mut c = self.config.borrow_mut();
            c.accounts
                .retain(|x| !(x.instance == acc.instance && x.id == acc.id));
            c.active = 0;
            c.save();
            (!c.accounts.is_empty()).then_some(0)
        };
        self.generation.set(self.generation.get().wrapping_add(1));
        if let Some(ws) = self.stream.borrow_mut().take() {
            ws.close(1000, None);
        }
        self.window.set_title(Some("FediGoo"));
        match next {
            Some(i) => self.activate_account(i),
            None => {
                self.rebuild_account_menu();
                self.show_welcome();
            }
        }
        self.toast(&format!("Logged out of {}", acc.full_acct()));
    }

    pub fn rebuild_account_menu(self: &Rc<Self>) {
        let pop = gtk::Popover::new();
        let b = gtk::Box::new(gtk::Orientation::Vertical, 2);
        b.set_margin_top(6);
        b.set_margin_bottom(6);
        let (accounts, active) = {
            let c = self.config.borrow();
            (c.accounts.clone(), c.active)
        };
        for (i, acc) in accounts.iter().enumerate() {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let img = gtk::Image::new();
            img.set_pixel_size(28);
            img.add_css_class("avatar");
            api::load_image(&img, &acc.avatar);
            row.append(&img);
            let l = gtk::Label::new(None);
            let name = if acc.display_name.is_empty() {
                &acc.acct
            } else {
                &acc.display_name
            };
            l.set_markup(&format!(
                "{}\n<small>{}</small>",
                glib::markup_escape_text(name),
                glib::markup_escape_text(&acc.full_acct())
            ));
            l.set_xalign(0.0);
            l.set_hexpand(true);
            row.append(&l);
            if i == active {
                row.append(&gtk::Image::from_icon_name("object-select-symbolic"));
            }
            let btn = gtk::Button::builder().child(&row).build();
            btn.add_css_class("flat");
            let a = self.clone();
            btn.connect_clicked(glib::clone!(
                #[weak]
                pop,
                move |_| {
                    pop.popdown();
                    a.activate_account(i);
                }
            ));
            b.append(&btn);
        }
        if !accounts.is_empty() {
            b.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        }
        let item = |label: &str, f: Box<dyn Fn(&Rc<App>)>| {
            let btn = gtk::Button::with_label(label);
            btn.add_css_class("flat");
            if let Some(l) = btn.child().and_downcast::<gtk::Label>() {
                l.set_xalign(0.0);
            }
            let a = self.clone();
            btn.connect_clicked(glib::clone!(
                #[weak]
                pop,
                move |_| {
                    pop.popdown();
                    f(&a);
                }
            ));
            b.append(&btn);
            btn
        };
        item("Add account…", Box::new(login::open));
        if !accounts.is_empty() {
            item("Edit profile…", Box::new(pages::edit_profile));
            item(
                "Log out of this account",
                Box::new(|a| {
                    let a = a.clone();
                    let dlg = gtk::AlertDialog::builder()
                        .message("Log out?")
                        .detail("The access token for this account will be revoked.")
                        .buttons(["Cancel", "Log out"])
                        .cancel_button(0)
                        .default_button(1)
                        .build();
                    let win = a.window.clone();
                    glib::spawn_future_local(async move {
                        if dlg.choose_future(Some(&win)).await == Ok(1) {
                            a.log_out();
                        }
                    });
                }),
            );
        }
        b.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        item("Preferences…", Box::new(prefs::open));
        item(
            "Keyboard shortcuts",
            Box::new(|a| {
                a.toast("Ctrl+N new post · F5 refresh · Alt+Left/Esc back · Ctrl+F search · Ctrl+Enter send · Ctrl+Q quit")
            }),
        );
        item(
            "About FediGoo",
            Box::new(|a| {
                gtk::AboutDialog::builder()
                    .program_name("FediGoo")
                    .version(env!("CARGO_PKG_VERSION"))
                    // Credits and license are stated on the main page and also
                    // in the standard fields (which add Credits/License tabs).
                    .comments(
                        "A Mastodon client for the Linux desktop.\n\n\
                         Built with Rust and GTK 4.\n\n\
                         Created by AI Sloperator with Claude Code.\n\n\
                         No license is given.",
                    )
                    .logo_icon_name(crate::APP_ID)
                    .authors(["AI Sloperator"])
                    .license_type(gtk::License::Custom)
                    .license("No license is given.")
                    .website("https://www.aisloperator.com/")
                    .website_label("www.aisloperator.com")
                    .transient_for(&a.window)
                    .modal(true)
                    .build()
                    .present();
            }),
        );
        pop.set_child(Some(&b));
        self.account_btn.set_popover(Some(&pop));
    }

    // ----- Navigation -----

    fn reset_nav(&self) {
        for p in self.nav.borrow_mut().drain(..) {
            self.stack.remove(&p.widget);
        }
    }

    /// Pushes a page.  `keep` is held for as long as the page exists.
    pub fn push(
        &self,
        title: &str,
        widget: impl IsA<gtk::Widget>,
        keep: Option<Rc<dyn Any>>,
        refresh: Option<Rc<dyn Fn()>>,
    ) {
        let widget = widget.upcast::<gtk::Widget>();
        self.stack.add_child(&widget);
        self.stack.set_visible_child(&widget);
        self.nav.borrow_mut().push(Page {
            widget,
            title: title.to_string(),
            _keep: keep,
            refresh,
        });
        self.update_header();
    }

    pub fn push_timeline(&self, title: &str, tl: Rc<Timeline>) {
        self.live.borrow_mut().retain(|w| w.strong_count() > 0);
        self.live.borrow_mut().push(Rc::downgrade(&tl));
        let w = tl.widget();
        let t = tl.clone();
        self.push(title, w, Some(tl), Some(Rc::new(move || t.reload())));
    }

    pub fn pop(&self) {
        let removed = {
            let mut nav = self.nav.borrow_mut();
            if nav.len() <= 1 {
                return;
            }
            nav.pop()
        };
        if let Some(p) = removed {
            if let Some(top) = self.nav.borrow().last() {
                self.stack.set_visible_child(&top.widget);
            }
            self.stack.remove(&p.widget);
        }
        self.update_header();
    }

    fn update_header(&self) {
        let nav = self.nav.borrow();
        self.back_btn.set_visible(nav.len() > 1);
        if let Some(p) = nav.last() {
            self.title.set_text(&p.title);
            self.refresh_btn.set_sensitive(p.refresh.is_some());
        }
        let acc = self.config.borrow().active_account().map(|a| a.full_acct());
        self.subtitle.set_visible(acc.is_some());
        self.subtitle.set_text(&acc.unwrap_or_default());
    }

    pub fn refresh(&self) {
        let r = self.nav.borrow().last().and_then(|p| p.refresh.clone());
        if let Some(r) = r {
            r();
        }
    }

    pub fn show_root(self: &Rc<Self>, root: Root) {
        if let Some(i) = ROOTS.iter().position(|(r, ..)| *r == root)
            && let Some(row) = self.sidebar.row_at_index(i as i32)
            && self.sidebar.selected_row().as_ref() != Some(&row)
        {
            self.sidebar.select_row(Some(&row));
        }
        self.reset_nav();
        if root == Root::Notifications {
            self.unread.set(0);
            self.notif_badge.set_visible(false);
        }
        let title = ROOTS
            .iter()
            .find(|(r, ..)| *r == root)
            .map(|r| r.1)
            .unwrap_or("");
        pages::root_page(self, root, title);
    }

    // ----- Link routing -----

    /// Handles a click on a link inside post or profile text.  Mentions and
    /// hashtags open inside the app; other links open in the browser, except
    /// links to posts/profiles on other instances, which are resolved first.
    pub fn activate_link(self: &Rc<Self>, uri: &str, status: Option<&Status>) {
        if let Some(st) = status {
            if let Some(m) = st.mentions.iter().find(|m| m.url == uri) {
                pages::open_profile_id(self, &m.id);
                return;
            }
            if let Some(tag) = hashtag_from_url(uri)
                && st.tags.iter().any(|t| t.eq_ignore_ascii_case(&tag))
            {
                pages::open_hashtag(self, &tag);
                return;
            }
        }
        if let Some(tag) = hashtag_from_url(uri) {
            pages::open_hashtag(self, &tag);
            return;
        }
        let Ok(parsed) = glib::Uri::parse(uri, glib::UriFlags::NONE) else {
            self.open_url(uri);
            return;
        };
        let path = parsed.path().to_string();
        let looks_fedi =
            path.starts_with("/@") || path.starts_with("/users/") || path.contains("/statuses/");
        if !looks_fedi {
            self.open_url(uri);
            return;
        }
        let a = self.clone();
        let api = self.api();
        let uri = uri.to_string();
        glib::spawn_future_local(async move {
            let res = api
                .get(
                    "/api/v2/search",
                    &[("q", &uri), ("resolve", "true"), ("limit", "1")],
                )
                .await
                .unwrap_or(Value::Null);
            if let Some(s) = res["statuses"].arr().first() {
                pages::open_thread(&a, &Status::from_json(s));
            } else if let Some(acc) = res["accounts"].arr().first() {
                pages::open_profile(&a, model::Account::from_json(acc));
            } else {
                a.open_url(&uri);
            }
        });
    }

    // ----- Live updates -----

    fn connect_stream(self: &Rc<Self>, base: String, generation: u32) {
        let api = self.api();
        let Some(token) = api.token.clone() else {
            return;
        };
        let base = base.trim_end_matches('/').to_string();
        let url = format!("{base}/api/v1/streaming?stream=user");
        let Ok(msg) = soup::Message::new("GET", &url) else {
            return;
        };
        if let Some(h) = msg.request_headers() {
            h.append("Authorization", &format!("Bearer {token}"));
        }
        let a = self.clone();
        glib::spawn_future_local(async move {
            let res = api::session()
                .websocket_connect_async_future(&msg, None, &[], glib::Priority::DEFAULT)
                .await;
            if a.generation() != generation {
                if let Ok(ws) = res {
                    ws.close(1000, None);
                }
                return;
            }
            let ws = match res {
                Ok(ws) => ws,
                Err(e) => {
                    eprintln!("fedigoo: streaming unavailable: {e}");
                    a.reconnect_later(base, generation);
                    return;
                }
            };
            ws.set_keepalive_interval(30);
            let a2 = a.clone();
            ws.connect_message(move |_, _kind, bytes| {
                if let Ok(v) = Value::parse(&String::from_utf8_lossy(bytes)) {
                    a2.on_stream_event(&v);
                }
            });
            let a3 = a.clone();
            let base2 = base.clone();
            ws.connect_closed(move |_| {
                if a3.generation() == generation {
                    a3.stream.borrow_mut().take();
                    a3.reconnect_later(base2.clone(), generation);
                }
            });
            *a.stream.borrow_mut() = Some(ws);
        });
    }

    fn reconnect_later(self: &Rc<Self>, base: String, generation: u32) {
        let a = self.clone();
        glib::timeout_add_seconds_local_once(20, move || {
            if a.generation() == generation && a.config.borrow().settings.stream_updates {
                a.connect_stream(base, generation);
            }
        });
    }

    pub fn stop_stream(&self) {
        if let Some(ws) = self.stream.borrow_mut().take() {
            ws.close(1000, None);
        }
    }

    pub fn restart_stream(self: &Rc<Self>) {
        self.generation.set(self.generation.get().wrapping_add(1));
        self.stop_stream();
        if self.config.borrow().settings.stream_updates {
            let a = self.clone();
            let api = self.api();
            let generation = self.generation();
            glib::spawn_future_local(async move {
                let base = a.load_instance_info(&api).await;
                a.connect_stream(base, generation);
            });
        }
    }

    pub fn register_thread(&self, view: &Rc<pages::ThreadView>) {
        let mut threads = self.threads.borrow_mut();
        threads.retain(|w| w.strong_count() > 0);
        threads.push(Rc::downgrade(view));
    }

    /// Called with the server's copy of a post the user just published or
    /// edited, so open views show it without a manual refresh.
    pub fn status_posted(&self, v: &Value) {
        let st = Status::from_json(v);
        // Edits: update the post wherever a timeline shows it.
        for tl in self.live_timelines() {
            tl.on_stream("status.update", v);
        }
        // Replies and edits: reload any open thread they belong to.
        let threads: Vec<_> = self
            .threads
            .borrow()
            .iter()
            .filter_map(|w| w.upgrade())
            .collect();
        for t in threads {
            let parent_shown = st.in_reply_to_id.as_deref().is_some_and(|p| t.shows(p));
            if parent_shown || t.shows(&st.id) {
                t.reload_and_show(&st.id);
            }
        }
    }

    fn live_timelines(&self) -> Vec<Rc<Timeline>> {
        self.live
            .borrow()
            .iter()
            .filter_map(|w| w.upgrade())
            .collect()
    }

    fn on_stream_event(&self, v: &Value) {
        let event = v["event"].str();
        let payload = v["payload"].str();
        match event {
            "update" | "notification" | "status.update" => {
                let Ok(item) = Value::parse(payload) else {
                    return;
                };
                for tl in self.live_timelines() {
                    tl.on_stream(event, &item);
                }
                if event == "notification" {
                    let on_page =
                        self.title.text() == "Notifications" && self.nav.borrow().len() == 1;
                    if !on_page {
                        self.unread.set(self.unread.get() + 1);
                        self.notif_badge.set_text(&self.unread.get().to_string());
                        self.notif_badge.set_visible(true);
                    }
                }
            }
            "delete" => {
                for tl in self.live_timelines() {
                    tl.remove_status(payload);
                }
            }
            _ => {}
        }
    }
}

fn hashtag_from_url(uri: &str) -> Option<String> {
    let (_, rest) = uri.split_once("/tags/")?;
    let tag = rest.split(['/', '?', '#']).next()?;
    let tag = glib::Uri::unescape_string(tag, None)
        .map(|s| s.to_string())
        .unwrap_or_else(|| tag.to_string());
    (!tag.is_empty()).then_some(tag)
}
