//! The user's keyword and hashtag filters (`/api/v2/filters`): a page that
//! lists them and an editor to add, change or delete them.

use super::app::App;
use super::pages;
use crate::json::Value;
use crate::model::{self, Expiry, FILTER_CONTEXTS, Filter, FilterKeyword};
use crate::util;
use gtk::{glib, pango, prelude::*};
use std::cell::RefCell;
use std::rc::Rc;

/// Expiry choices offered by the editor, as Mastodon's web interface does.
const EXPIRY_CHOICES: [(&str, i64); 6] = [
    ("30 minutes", 30 * 60),
    ("1 hour", 3600),
    ("6 hours", 6 * 3600),
    ("12 hours", 12 * 3600),
    ("1 day", 86400),
    ("1 week", 7 * 86400),
];

/// The Filters page.
pub fn page(app: &Rc<App>, title: &str) {
    let vb = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    header.add_css_class("page-header");
    let intro = gtk::Label::new(Some(
        "Posts matching a filter are collapsed behind a warning or hidden. \
         Hashtags are keywords too, such as #example.",
    ));
    intro.set_wrap(true);
    intro.set_xalign(0.0);
    intro.set_hexpand(true);
    intro.add_css_class("dim-label");
    let add = gtk::Button::with_label("New filter…");
    add.set_valign(gtk::Align::Center);
    header.append(&intro);
    header.append(&add);
    vb.append(&header);

    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::None);
    list.add_css_class("timeline");
    vb.append(&list);
    let note = gtk::Label::new(Some("Loading…"));
    note.add_css_class("dim-label");
    note.set_margin_top(24);
    vb.append(&note);

    let filters: Rc<RefCell<Vec<Filter>>> = Rc::default();
    let reload: Rc<dyn Fn()> = {
        let a = app.clone();
        let (list, note) = (list.downgrade(), note.downgrade());
        let filters = filters.clone();
        Rc::new(move || {
            let (list, note, filters) = (list.clone(), note.clone(), filters.clone());
            let api = a.api();
            glib::spawn_future_local(async move {
                let result = api.get("/api/v2/filters", &[]).await;
                let (Some(list), Some(note)) = (list.upgrade(), note.upgrade()) else {
                    return;
                };
                list.remove_all();
                match result {
                    Ok(v) => {
                        let fs: Vec<Filter> = v.arr().iter().map(Filter::from_json).collect();
                        for f in &fs {
                            list.append(&row(f));
                        }
                        note.set_text("No filters yet.");
                        note.set_visible(fs.is_empty());
                        *filters.borrow_mut() = fs;
                    }
                    Err(e) => {
                        note.set_text(&format!("Could not load filters: {e}"));
                        note.set_visible(true);
                        filters.borrow_mut().clear();
                    }
                }
            });
        })
    };

    let (a, r) = (app.clone(), reload.clone());
    add.connect_clicked(move |_| edit(&a, None, r.clone()));
    let (a, r, fs) = (app.clone(), reload.clone(), filters.clone());
    list.connect_row_activated(move |_, row| {
        let f = fs.borrow().get(row.index() as usize).cloned();
        if let Some(f) = f {
            edit(&a, Some(f), r.clone());
        }
    });

    reload();
    let keep: Rc<dyn std::any::Any> = filters;
    app.push(title, pages::column(&vb), Some(keep), Some(reload));
}

/// One filter in the list: its title and a summary; a click opens the editor.
fn row(f: &Filter) -> gtk::ListBoxRow {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    b.add_css_class("account-row");
    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_hexpand(true);
    let title = gtk::Label::new(None);
    title.set_markup(&format!(
        "<b>{}</b>",
        glib::markup_escape_text(if f.title.is_empty() {
            "Untitled"
        } else {
            &f.title
        })
    ));
    title.set_xalign(0.0);
    title.set_ellipsize(pango::EllipsizeMode::End);
    text.append(&title);
    for line in [keywords_summary(f), details_summary(f)] {
        let l = gtk::Label::new(Some(&line));
        l.set_xalign(0.0);
        l.set_wrap(true);
        l.set_wrap_mode(pango::WrapMode::WordChar);
        l.add_css_class("dim-label");
        text.append(&l);
    }
    b.append(&text);
    let go = gtk::Image::from_icon_name("go-next-symbolic");
    go.add_css_class("dim-label");
    b.append(&go);
    let row = gtk::ListBoxRow::new();
    row.set_child(Some(&b));
    row.set_tooltip_text(Some("Edit filter"));
    row.set_cursor_from_name(Some("pointer"));
    row
}

fn keywords_summary(f: &Filter) -> String {
    if f.keywords.is_empty() {
        return "No keywords".into();
    }
    let words: Vec<String> = f
        .keywords
        .iter()
        .map(|k| {
            if k.whole_word {
                k.keyword.clone()
            } else {
                format!("{} (partial words too)", k.keyword)
            }
        })
        .collect();
    words.join(", ")
}

fn details_summary(f: &Filter) -> String {
    let places: Vec<&str> = FILTER_CONTEXTS
        .iter()
        .filter(|(c, _)| f.context.iter().any(|x| x == c))
        .map(|(_, label)| *label)
        .collect();
    let mut parts = vec![if places.is_empty() {
        "Nowhere".to_string()
    } else {
        places.join(", ")
    }];
    parts.push(action_label(&f.action).into());
    if let Some(exp) = &f.expires_at {
        parts.push(expiry_label(exp));
    }
    parts.join(" · ")
}

fn action_label(action: &str) -> &'static str {
    match action {
        "hide" => "Hide",
        "blur" => "Blur media",
        _ => "Warn",
    }
}

fn expiry_label(expires_at: &str) -> String {
    match util::time_left(expires_at).as_str() {
        "closed" => "Expired".into(),
        "" => String::new(),
        left => format!("Expires: {left}"),
    }
}

/// One keyword row in the editor.
struct KeywordRow {
    id: String,
    widget: gtk::Box,
    entry: gtk::Entry,
    whole: gtk::CheckButton,
}

/// The editor for a new filter (`old` is None) or an existing one.
fn edit(app: &Rc<App>, old: Option<Filter>, reload: Rc<dyn Fn()>) {
    let (win, vb) = pages::dialog(
        app,
        if old.is_some() {
            "Edit filter"
        } else {
            "New filter"
        },
        520,
        780,
    );
    let form = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&form)
        .build();
    vb.append(&scroll);
    let heading = |text: &str| {
        let l = gtk::Label::new(Some(text));
        l.set_xalign(0.0);
        l.add_css_class("heading");
        l.set_margin_top(6);
        l
    };

    form.append(&heading("Title"));
    let title = gtk::Entry::new();
    title.set_placeholder_text(Some("For example: Spoilers"));
    if let Some(o) = &old {
        title.set_text(&o.title);
    }
    form.append(&title);

    form.append(&heading("Keywords and hashtags"));
    let kw_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
    form.append(&kw_box);
    let rows: Rc<RefCell<Vec<KeywordRow>>> = Rc::default();
    let add_kw = {
        let (kw_box, rows) = (kw_box.downgrade(), rows.clone());
        Rc::new(move |k: &FilterKeyword| {
            let Some(kw_box) = kw_box.upgrade() else {
                return;
            };
            let b = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let entry = gtk::Entry::new();
            entry.set_hexpand(true);
            entry.set_placeholder_text(Some("Word, phrase or #hashtag"));
            entry.set_text(&k.keyword);
            let whole = gtk::CheckButton::with_label("Whole word");
            whole.set_active(k.whole_word);
            whole.set_tooltip_text(Some("Match only the whole word, not part of a longer one"));
            let remove = gtk::Button::from_icon_name("list-remove-symbolic");
            remove.add_css_class("flat");
            remove.set_tooltip_text(Some("Remove keyword"));
            b.append(&entry);
            b.append(&whole);
            b.append(&remove);
            kw_box.append(&b);
            let rows_w = Rc::downgrade(&rows);
            remove.connect_clicked(glib::clone!(
                #[weak]
                b,
                #[weak]
                kw_box,
                move |_| {
                    kw_box.remove(&b);
                    if let Some(rows) = rows_w.upgrade() {
                        rows.borrow_mut().retain(|r| r.widget != b);
                    }
                }
            ));
            rows.borrow_mut().push(KeywordRow {
                id: k.id.clone(),
                widget: b,
                entry,
                whole,
            });
        })
    };
    match &old {
        Some(o) if !o.keywords.is_empty() => o.keywords.iter().for_each(|k| add_kw(k)),
        _ => add_kw(&FilterKeyword {
            whole_word: true,
            ..Default::default()
        }),
    }
    let more = gtk::Button::with_label("Add keyword");
    more.set_halign(gtk::Align::Start);
    let a = add_kw.clone();
    more.connect_clicked(move |_| {
        a(&FilterKeyword {
            whole_word: true,
            ..Default::default()
        })
    });
    form.append(&more);

    form.append(&heading("Filter in"));
    let places: Vec<(&str, gtk::CheckButton)> = FILTER_CONTEXTS
        .iter()
        .map(|(ctx, label)| {
            let c = gtk::CheckButton::with_label(label);
            c.set_active(
                old.as_ref()
                    .is_none_or(|o| o.context.iter().any(|x| x == ctx)),
            );
            form.append(&c);
            (*ctx, c)
        })
        .collect();

    form.append(&heading("Action"));
    let mut actions: Vec<(&str, gtk::CheckButton)> = vec![
        ("warn", "Warn: collapse the post behind the filter’s title"),
        ("hide", "Hide: don’t show the post at all"),
    ]
    .into_iter()
    .map(|(a, l)| (a, gtk::CheckButton::with_label(l)))
    .collect();
    // "blur" (Mastodon 4.4) is kept if the filter already uses it.
    if old.as_ref().is_some_and(|o| o.action == "blur") {
        actions.push((
            "blur",
            gtk::CheckButton::with_label("Blur media (FediGoo shows this as a warning)"),
        ));
    }
    let current = old.as_ref().map_or("warn", |o| o.action.as_str());
    for (i, (a, c)) in actions.iter().enumerate() {
        if i > 0 {
            c.set_group(Some(&actions[0].1));
        }
        c.set_active(*a == current || (i == 0 && !actions.iter().any(|(x, _)| *x == current)));
        form.append(c);
    }

    form.append(&heading("Expires"));
    let mut expiry_opts: Vec<(String, Expiry)> = Vec::new();
    if let Some(exp) = old.as_ref().and_then(|o| o.expires_at.as_deref()) {
        expiry_opts.push((
            format!("Unchanged ({})", expiry_label(exp)),
            Expiry::Unchanged,
        ));
    }
    expiry_opts.push(("Never".into(), Expiry::Never));
    for (label, secs) in EXPIRY_CHOICES {
        expiry_opts.push((format!("In {label}"), Expiry::In(secs)));
    }
    let labels: Vec<&str> = expiry_opts.iter().map(|(l, _)| l.as_str()).collect();
    let expiry = gtk::DropDown::from_strings(&labels);
    expiry.set_halign(gtk::Align::Start);
    form.append(&expiry);

    let error = gtk::Label::new(None);
    error.add_css_class("error");
    error.set_wrap(true);
    error.set_xalign(0.0);
    error.set_visible(false);
    vb.append(&error);

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    if let Some(o) = &old {
        let del = gtk::Button::with_label("Delete filter");
        del.add_css_class("destructive-action");
        let (a, id, t, r) = (app.clone(), o.id.clone(), o.title.clone(), reload.clone());
        del.connect_clicked(glib::clone!(
            #[weak]
            win,
            move |_| delete(&a, &win, &id, &t, r.clone())
        ));
        buttons.append(&del);
    }
    let (row, ok) = pages::button_row(&win, "Save");
    row.set_hexpand(true);
    buttons.append(&row);
    vb.append(&buttons);

    let a = app.clone();
    ok.connect_clicked(glib::clone!(
        #[weak]
        win,
        #[weak]
        title,
        #[weak]
        error,
        #[weak]
        expiry,
        move |ok| {
            let context: Vec<String> = places
                .iter()
                .filter(|(_, c)| c.is_active())
                .map(|(ctx, _)| ctx.to_string())
                .collect();
            let fail = |msg: &str| {
                error.set_text(msg);
                error.set_visible(true);
            };
            if title.text().trim().is_empty() {
                return fail("Give the filter a title.");
            }
            if context.is_empty() {
                return fail("Choose at least one place to filter in.");
            }
            let keywords: Vec<FilterKeyword> = rows
                .borrow()
                .iter()
                .map(|r| FilterKeyword {
                    id: r.id.clone(),
                    keyword: r.entry.text().to_string(),
                    whole_word: r.whole.is_active(),
                })
                .collect();
            let action = actions
                .iter()
                .find(|(_, c)| c.is_active())
                .map_or("warn", |(a, _)| *a);
            let exp = expiry_opts
                .get(expiry.selected() as usize)
                .map_or(Expiry::Never, |(_, e)| *e);
            let body = model::filter_body(
                &title.text(),
                &context,
                action,
                exp,
                &keywords,
                old.as_ref(),
            );
            error.set_visible(false);
            ok.set_sensitive(false);
            let (a, api, reload, ok) = (a.clone(), a.api(), reload.clone(), ok.clone());
            let id = old.as_ref().map(|o| o.id.clone());
            glib::spawn_future_local(async move {
                let res: Result<Value, String> = match &id {
                    Some(id) => api.put(&format!("/api/v2/filters/{id}"), body).await,
                    None => api.post("/api/v2/filters", body).await,
                };
                match res {
                    Ok(_) => {
                        win.close();
                        reload();
                        a.toast("Filter saved. It applies to posts loaded from now on.");
                    }
                    Err(e) => {
                        error.set_text(&format!("Could not save the filter: {e}"));
                        error.set_visible(true);
                        ok.set_sensitive(true);
                    }
                }
            });
        }
    ));
    win.present();
    title.grab_focus();
}

fn delete(app: &Rc<App>, editor: &gtk::Window, id: &str, title: &str, reload: Rc<dyn Fn()>) {
    let dlg = gtk::AlertDialog::builder()
        .message(format!("Delete the filter “{title}”?"))
        .detail("Posts it hid or collapsed will show normally once they are loaded again.")
        .buttons(["Cancel", "Delete"])
        .cancel_button(0)
        .default_button(1)
        .build();
    let (a, editor, id) = (app.clone(), editor.clone(), id.to_string());
    glib::spawn_future_local(async move {
        if dlg.choose_future(Some(&editor)).await != Ok(1) {
            return;
        }
        match a.api().delete(&format!("/api/v2/filters/{id}")).await {
            Ok(_) => {
                editor.close();
                reload();
            }
            Err(e) => a.toast(&format!("Could not delete the filter: {e}")),
        }
    });
}
