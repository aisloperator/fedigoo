//! The Followed hashtags page: a table of every followed hashtag, sorted by
//! clicking a column header (name, or people posting recently), opened with
//! a single click, and unfollowed (or followed again) with a button on each
//! row.
//!
//! The whole list is loaded at once, following the `Link` pages, since a
//! sort over part of it would be wrong.  Mastodon reports no time a tag was
//! followed, and doesn't document the order it lists them in, so there is no
//! "date followed" column.

use super::app::App;
use super::pages;
use crate::api::{self, Body};
use crate::model::Tag;
use crate::util;
use gtk::{gio, glib, prelude::*};
use std::cell::Cell;
use std::cmp::Ordering;
use std::rc::Rc;

/// The most the server returns per page (Mastodon's maximum for this call).
const PAGE: &str = "200";

/// The table's width.  Its natural width stays under this whatever rows are
/// in view (the name labels ellipsize past `NAME_CHARS`), so sorting or
/// scrolling doesn't change it.
const TABLE_WIDTH: i32 = 400;
const NAME_CHARS: i32 = 20;
/// Widths of the number and button columns; the name column takes the rest.
const PEOPLE_WIDTH: i32 = 130;
const BUTTON_WIDTH: i32 = 56;

/// A row: the tag, and whether it's still followed (after an Unfollow, the
/// row stays, struck through, with a button to follow it again).
struct Row {
    tag: Tag,
    following: bool,
}

fn row_of(obj: &glib::Object) -> Option<std::cell::Ref<'_, Row>> {
    obj.downcast_ref::<glib::BoxedAnyObject>()
        .and_then(|b| b.try_borrow::<Row>().ok())
}

/// Hashtag names in alphabetical order, ignoring case.
fn by_name(a: &Tag, b: &Tag) -> Ordering {
    a.name
        .to_lowercase()
        .cmp(&b.name.to_lowercase())
        .then_with(|| a.name.cmp(&b.name))
}

/// Fewest people posting recently first; the same number by name.
fn by_people(a: &Tag, b: &Tag) -> Ordering {
    a.recent_uses
        .cmp(&b.recent_uses)
        .then_with(|| by_name(a, b))
}

fn sorter(cmp: fn(&Tag, &Tag) -> Ordering) -> gtk::CustomSorter {
    gtk::CustomSorter::new(move |a, b| match (row_of(a), row_of(b)) {
        (Some(a), Some(b)) => cmp(&a.tag, &b.tag).into(),
        _ => gtk::Ordering::Equal,
    })
}

/// A column whose cells are made by `setup` and filled by `bind`.
fn column(
    title: &str,
    setup: impl Fn(&gtk::ColumnViewCell) + 'static,
    bind: impl Fn(&gtk::ColumnViewCell, &Row) + 'static,
) -> gtk::ColumnViewColumn {
    let f = gtk::SignalListItemFactory::new();
    f.connect_setup(move |_, o| {
        if let Some(cell) = o.downcast_ref::<gtk::ColumnViewCell>() {
            setup(cell);
        }
    });
    f.connect_bind(move |_, o| {
        let Some(cell) = o.downcast_ref::<gtk::ColumnViewCell>() else {
            return;
        };
        if let Some(item) = cell.item()
            && let Some(row) = row_of(&item)
        {
            bind(cell, &row);
        }
    });
    gtk::ColumnViewColumn::new(Some(title), Some(f))
}

pub fn page(app: &Rc<App>, title: &str) {
    let store = gio::ListStore::new::<glib::BoxedAnyObject>();
    let view = gtk::ColumnView::new(None::<gtk::NoSelection>);
    view.add_css_class("followed-tags");
    view.set_single_click_activate(true);
    view.set_reorderable(false);
    view.set_cursor_from_name(Some("pointer"));

    let name = column(
        "Hashtag",
        |cell| {
            let l = gtk::Label::new(None);
            l.set_xalign(0.0);
            l.set_ellipsize(gtk::pango::EllipsizeMode::End);
            l.set_width_chars(8);
            l.set_max_width_chars(NAME_CHARS);
            cell.set_child(Some(&l));
        },
        |cell, row| {
            let Some(l) = cell.child().and_downcast::<gtk::Label>() else {
                return;
            };
            let name = glib::markup_escape_text(&row.tag.name);
            if row.following {
                l.set_markup(&format!("<b>#{name}</b>"));
                l.remove_css_class("dim-label");
            } else {
                l.set_markup(&format!("<s>#{name}</s>"));
                l.add_css_class("dim-label");
            }
        },
    );
    name.set_expand(true);
    name.set_sorter(Some(&sorter(by_name)));

    let people = column(
        "People, 2 days",
        |cell| {
            let l = gtk::Label::new(None);
            l.set_xalign(1.0);
            l.add_css_class("numeric");
            cell.set_child(Some(&l));
        },
        |cell, row| {
            if let Some(l) = cell.child().and_downcast::<gtk::Label>() {
                l.set_text(&util::count(row.tag.recent_uses));
                l.set_tooltip_text(Some(&format!(
                    "{} people posted with #{} today and yesterday",
                    row.tag.recent_uses, row.tag.name
                )));
            }
        },
    );
    people.set_sorter(Some(&sorter(by_people)));
    people.set_fixed_width(PEOPLE_WIDTH);

    let a = app.clone();
    let store_w = store.downgrade();
    let unfollow = column(
        "",
        move |cell| {
            let b = gtk::Button::new();
            b.add_css_class("flat");
            b.set_halign(gtk::Align::Center);
            b.set_valign(gtk::Align::Center);
            cell.set_child(Some(&b));
            let a = a.clone();
            let store_w = store_w.clone();
            let cell_w = cell.downgrade();
            b.connect_clicked(move |b| {
                let (Some(cell), Some(store)) = (cell_w.upgrade(), store_w.upgrade()) else {
                    return;
                };
                let Some(item) = cell.item() else { return };
                let Some((name, following)) =
                    row_of(&item).map(|r| (r.tag.name.clone(), r.following))
                else {
                    return;
                };
                b.set_sensitive(false);
                let verb = if following { "unfollow" } else { "follow" };
                let path = format!("/api/v1/tags/{}/{verb}", api::escape(&name));
                let api = a.api();
                let b = b.downgrade();
                let store = store.downgrade();
                a.spawn(async move {
                    let res = api.post(&path, crate::json::Value::Null).await;
                    if let Some(b) = b.upgrade() {
                        b.set_sensitive(true);
                    }
                    let v = res?;
                    // A new item for the row, since GTK doesn't rebind the
                    // cells of an item it already shows.
                    let tag = row_of(&item).map(|r| r.tag.clone());
                    if let (Some(store), Some(tag)) = (store.upgrade(), tag)
                        && let Some(pos) = store.find(&item)
                    {
                        let following = v["following"].bool();
                        store.splice(pos, 1, &[glib::BoxedAnyObject::new(Row { tag, following })]);
                    }
                    Ok(())
                });
            });
        },
        |cell, row| {
            let Some(b) = cell.child().and_downcast::<gtk::Button>() else {
                return;
            };
            if row.following {
                b.set_icon_name("user-trash-symbolic");
                b.set_tooltip_text(Some(&format!("Unfollow #{}", row.tag.name)));
            } else {
                b.set_icon_name("list-add-symbolic");
                b.set_tooltip_text(Some(&format!("Follow #{} again", row.tag.name)));
            }
        },
    );

    unfollow.set_fixed_width(BUTTON_WIDTH);
    view.append_column(&name);
    view.append_column(&people);
    view.append_column(&unfollow);
    let sorted = gtk::SortListModel::new(Some(store.clone()), view.sorter());
    view.set_model(Some(&gtk::NoSelection::new(Some(sorted.clone()))));
    view.sort_by_column(Some(&people), gtk::SortType::Descending);
    // A new order starts from the top, not from the row that was first.
    if let Some(sorter) = view.sorter() {
        sorter.connect_changed(glib::clone!(
            #[weak]
            view,
            move |_, _| {
                if view.model().is_some_and(|m| m.n_items() > 0) {
                    view.scroll_to(0, None, gtk::ListScrollFlags::NONE, None);
                }
            }
        ));
    }

    let a = app.clone();
    view.connect_activate(move |_, pos| {
        if let Some(item) = sorted.item(pos)
            && let Some(name) = row_of(&item).map(|r| r.tag.name.clone())
        {
            pages::open_hashtag(&a, &name);
        }
    });

    let hint = gtk::Label::new(Some(
        "Click a column title to sort; click again to reverse.",
    ));
    hint.add_css_class("dim-label");
    hint.add_css_class("page-header");
    hint.set_wrap(true);
    hint.set_xalign(0.0);
    let status = gtk::Label::new(None);
    status.add_css_class("dim-label");
    status.set_wrap(true);
    status.set_margin_top(24);
    status.set_visible(false);
    let spinner = gtk::Spinner::new();
    spinner.set_margin_top(24);

    let scrolled = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .propagate_natural_width(true)
        .child(&view)
        .build();
    scrolled.set_size_request(TABLE_WIDTH, -1);
    let col = gtk::Box::new(gtk::Orientation::Vertical, 0);
    col.add_css_class("timeline-column");
    col.set_halign(gtk::Align::Center);
    col.append(&hint);
    col.append(&spinner);
    col.append(&status);
    col.append(&scrolled);

    let load: Rc<dyn Fn()> = {
        let a = app.clone();
        let store = store.downgrade();
        let spinner = spinner.downgrade();
        let status = status.downgrade();
        let epoch = Rc::new(Cell::new(0u32));
        Rc::new(move || {
            let (Some(store), Some(spinner), Some(status)) =
                (store.upgrade(), spinner.upgrade(), status.upgrade())
            else {
                return;
            };
            epoch.set(epoch.get() + 1);
            let mine = epoch.get();
            store.remove_all();
            status.set_visible(false);
            spinner.set_visible(true);
            spinner.start();
            let api = a.api();
            let (store, spinner, status) =
                (store.downgrade(), spinner.downgrade(), status.downgrade());
            let epoch = epoch.clone();
            a.spawn(async move {
                let mut tags = Vec::new();
                let mut next: Option<String> = None;
                let res = loop {
                    let r = match &next {
                        Some(url) => api.request("GET", url, &[], Body::Empty).await,
                        None => {
                            api.request("GET", "/api/v1/followed_tags", &[("limit", PAGE)], Body::Empty)
                                .await
                        }
                    };
                    match r {
                        Ok(r) => {
                            tags.extend(r.json.arr().iter().map(Tag::from_json));
                            match r.next {
                                // An empty page ends it too, in case a
                                // server keeps sending a next link.
                                Some(n) if !r.json.arr().is_empty() => next = Some(n),
                                _ => break Ok(()),
                            }
                        }
                        Err(e) => break Err(e),
                    }
                };
                if epoch.get() != mine {
                    return Ok(());
                }
                let (Some(store), Some(spinner), Some(status)) =
                    (store.upgrade(), spinner.upgrade(), status.upgrade())
                else {
                    return Ok(());
                };
                spinner.stop();
                spinner.set_visible(false);
                let items: Vec<glib::BoxedAnyObject> = tags
                    .into_iter()
                    .map(|tag| {
                        glib::BoxedAnyObject::new(Row {
                            tag,
                            following: true,
                        })
                    })
                    .collect();
                store.extend_from_slice(&items);
                if res.is_ok() && items.is_empty() {
                    status.set_text(
                        "You don't follow any hashtags. Open a hashtag and choose Follow to see its posts in your Home feed.",
                    );
                    status.set_visible(true);
                }
                res
            });
        })
    };
    load();
    app.push(title, col, None, Some(load));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(name: &str, recent_uses: i64) -> Tag {
        Tag {
            name: name.into(),
            recent_uses,
        }
    }

    #[test]
    fn sorts_by_name_ignoring_case() {
        let mut v = [tag("rust", 1), tag("Linux", 9), tag("accessibility", 3)];
        v.sort_by(by_name);
        let names: Vec<_> = v.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["accessibility", "Linux", "rust"]);
    }

    #[test]
    fn sorts_by_people_then_name() {
        let mut v = [tag("b", 5), tag("c", 2), tag("a", 5), tag("d", 0)];
        v.sort_by(by_people);
        let names: Vec<_> = v.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["d", "c", "a", "b"]);
    }
}
