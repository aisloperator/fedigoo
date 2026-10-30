//! OAuth login: register the app, send the user to the browser to authorise
//! it, and exchange the pasted authorisation code for a token.

use super::app::App;
use crate::api::{self, Api};
use crate::config::{self, AppReg};
use crate::model;
use gtk::{glib, prelude::*};
use std::rc::Rc;

pub fn open(app: &Rc<App>) {
    let win = gtk::Window::builder()
        .title("Log in to Mastodon")
        .transient_for(&app.window)
        .modal(true)
        .default_width(480)
        .resizable(false)
        .build();
    let stack = gtk::Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::SlideLeft);

    let page = |spacing| {
        let b = gtk::Box::new(gtk::Orientation::Vertical, spacing);
        b.set_margin_top(20);
        b.set_margin_bottom(20);
        b.set_margin_start(20);
        b.set_margin_end(20);
        b
    };

    // Step 1: instance.
    let p1 = page(12);
    let l = gtk::Label::new(None);
    l.set_markup("<b>Which server is your account on?</b>");
    l.set_xalign(0.0);
    p1.append(&l);
    let instance = gtk::Entry::new();
    instance.set_placeholder_text(Some("e.g. mastodon.social"));
    p1.append(&instance);
    let next = gtk::Button::with_label("Continue");
    next.add_css_class("suggested-action");
    next.set_halign(gtk::Align::End);
    p1.append(&next);
    stack.add_named(&p1, Some("instance"));

    // Step 2: code.
    let p2 = page(12);
    let info = gtk::Label::new(None);
    info.set_wrap(true);
    info.set_xalign(0.0);
    p2.append(&info);
    let url_label = gtk::Label::new(None);
    url_label.set_selectable(true);
    url_label.set_wrap(true);
    url_label.set_wrap_mode(gtk::pango::WrapMode::Char);
    url_label.set_xalign(0.0);
    url_label.add_css_class("caption");
    p2.append(&url_label);
    let reopen = gtk::Button::with_label("Open browser again");
    reopen.set_halign(gtk::Align::Start);
    p2.append(&reopen);
    let code = gtk::Entry::new();
    code.set_placeholder_text(Some("Paste the authorisation code here"));
    p2.append(&code);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.set_halign(gtk::Align::End);
    let back = gtk::Button::with_label("Back");
    let login = gtk::Button::with_label("Log in");
    login.add_css_class("suggested-action");
    row.append(&back);
    row.append(&login);
    p2.append(&row);
    stack.add_named(&p2, Some("code"));

    let status = gtk::Label::new(None);
    status.add_css_class("error");
    status.set_wrap(true);
    status.set_visible(false);
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    outer.append(&stack);
    outer.append(&status);
    status.set_margin_bottom(12);
    win.set_child(Some(&outer));

    let reg: Rc<std::cell::RefCell<Option<AppReg>>> = Rc::default();

    let a = app.clone();
    let go = {
        let reg = reg.clone();
        let (stack, info, url_label, status, win) = (
            stack.clone(),
            info.clone(),
            url_label.clone(),
            status.clone(),
            win.clone(),
        );
        let code = code.clone();
        Rc::new(move |e: &gtk::Entry| {
            let host = api::normalize_instance(&e.text());
            if host.is_empty() || !host.contains('.') {
                status.set_text("Please enter a server name such as mastodon.social.");
                status.set_visible(true);
                return;
            }
            status.set_visible(false);
            win.set_sensitive(false);
            let a = a.clone();
            let (reg, stack, info, url_label, status, win, code) = (
                reg.clone(),
                stack.clone(),
                info.clone(),
                url_label.clone(),
                status.clone(),
                win.clone(),
                code.clone(),
            );
            glib::spawn_future_local(async move {
                let r = register(&a, &host).await;
                win.set_sensitive(true);
                match r {
                    Ok(r) => {
                        let url = authorize_url(&r);
                        info.set_markup(&format!(
                            "A browser window has been opened on <b>{}</b>. Log in there, authorise FediGoo, then paste the code you are shown below.\n\nIf no browser opened, visit:",
                            glib::markup_escape_text(&host)
                        ));
                        url_label.set_text(&url);
                        a.open_url(&url);
                        *reg.borrow_mut() = Some(r);
                        stack.set_visible_child_name("code");
                        code.grab_focus();
                    }
                    Err(e) => {
                        status.set_text(&format!("Could not contact {host}: {e}"));
                        status.set_visible(true);
                    }
                }
            });
        })
    };
    let g = go.clone();
    instance.connect_activate(move |e| g(e));
    next.connect_clicked(glib::clone!(
        #[weak]
        instance,
        move |_| go(&instance)
    ));
    back.connect_clicked(glib::clone!(
        #[weak]
        stack,
        move |_| stack.set_visible_child_name("instance")
    ));
    let a = app.clone();
    let r2 = reg.clone();
    reopen.connect_clicked(move |_| {
        if let Some(r) = r2.borrow().as_ref() {
            a.open_url(&authorize_url(r));
        }
    });

    let a = app.clone();
    let finish = {
        let (status, win) = (status.clone(), win.clone());
        Rc::new(move |e: &gtk::Entry| {
            let code = e.text().trim().to_string();
            let Some(r) = reg.borrow().clone() else {
                return;
            };
            if code.is_empty() {
                return;
            }
            win.set_sensitive(false);
            let a = a.clone();
            let (status, win) = (status.clone(), win.clone());
            glib::spawn_future_local(async move {
                match exchange(&r, &code).await {
                    Ok(acc) => {
                        win.close();
                        a.toast(&format!("Logged in as {}", acc.full_acct()));
                        a.add_account(acc);
                    }
                    Err(e) => {
                        win.set_sensitive(true);
                        status.set_text(&format!("Login failed: {e}"));
                        status.set_visible(true);
                    }
                }
            });
        })
    };
    let f = finish.clone();
    code.connect_activate(move |e| f(e));
    login.connect_clicked(glib::clone!(
        #[weak]
        code,
        move |_| finish(&code)
    ));
    win.present();
}

fn authorize_url(r: &AppReg) -> String {
    format!(
        "https://{}/oauth/authorize?{}",
        r.instance,
        api::form_encode(&[
            ("client_id", &r.client_id),
            ("scope", api::SCOPES),
            ("redirect_uri", api::REDIRECT_URI),
            ("response_type", "code"),
        ])
    )
}

/// Returns this instance's client registration, creating one if needed.
async fn register(app: &Rc<App>, host: &str) -> Result<AppReg, String> {
    if let Some(r) = app.config.borrow().app_for(host).cloned() {
        return Ok(r);
    }
    let api = Api::new(host, None);
    let v = api
        .post_form(
            "/api/v1/apps",
            &[
                ("client_name", api::CLIENT_NAME),
                ("redirect_uris", api::REDIRECT_URI),
                ("scopes", api::SCOPES),
            ],
        )
        .await?;
    let reg = AppReg {
        instance: host.to_string(),
        client_id: v["client_id"].str().to_string(),
        client_secret: v["client_secret"].str().to_string(),
    };
    if reg.client_id.is_empty() {
        return Err("the server did not return client credentials".into());
    }
    let mut c = app.config.borrow_mut();
    c.apps.retain(|a| a.instance != host);
    c.apps.push(reg.clone());
    c.save();
    Ok(reg)
}

async fn exchange(r: &AppReg, code: &str) -> Result<config::Account, String> {
    let api = Api::new(&r.instance, None);
    let v = api
        .post_form(
            "/oauth/token",
            &[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("client_id", &r.client_id),
                ("client_secret", &r.client_secret),
                ("redirect_uri", api::REDIRECT_URI),
                ("scope", api::SCOPES),
            ],
        )
        .await?;
    let token = v["access_token"].str().to_string();
    if token.is_empty() {
        return Err("no access token in response".into());
    }
    let authed = Api::new(&r.instance, Some(&token));
    let me = model::Account::from_json(
        &authed
            .get("/api/v1/accounts/verify_credentials", &[])
            .await?,
    );
    Ok(config::Account {
        instance: r.instance.clone(),
        token,
        id: me.id.clone(),
        acct: me.acct.clone(),
        display_name: me.name().to_string(),
        avatar: me.avatar.clone(),
    })
}
