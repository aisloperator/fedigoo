//! Keyboard paging through the posts of the current view: PgDn/Space down,
//! PgUp/Shift+Space up.  Pages land on post boundaries where that doesn't
//! skip anything, scroll with a short animation, and paging twice past the
//! top of a feed (or either end of a thread) checks for new posts or
//! replies.  Backspace goes back, like the Back button.

use super::app::App;
use super::pages::{ThreadFocus, ThreadView};
use super::timeline::Timeline;
use gtk::gdk::{Key, ModifierType};
use gtk::{glib, prelude::*};
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

/// How much of the old page stays in view when paging within a tall post.
const OVERLAP: f64 = 40.0;
/// Moves smaller than this (px) to reach a post boundary look like nothing
/// happened, so the page moves on by a plain step instead.
const MIN_MOVE: f64 = 48.0;
const ANIMATION: Duration = Duration::from_millis(160);
/// A second press at an end within this time checks for new posts.
const AGAIN_WITHIN: Duration = Duration::from_secs(4);

/// The new scroll position for paging `dir` (+1 down, -1 up) from `value`,
/// or None if already at that end.  `rows` are the posts' (top, bottom) in
/// content coordinates, in order; `page` is the visible height and `upper`
/// the content height.
pub fn page_target(
    rows: &[(f64, f64)],
    value: f64,
    page: f64,
    upper: f64,
    dir: i32,
) -> Option<f64> {
    let max = (upper - page).max(0.0);
    let step = (page - OVERLAP).max(page / 2.0);
    let target = if dir > 0 {
        let bottom = value + page;
        // The first post that isn't wholly in view: usually the one the
        // bottom edge cuts through.
        match rows.iter().find(|(_, b)| *b > bottom + 0.5) {
            None => max,
            // It starts lower down: bring its start to the top, including
            // the part already visible, for an unbroken read.
            Some((top, _)) if *top >= value + MIN_MOVE => top.min(bottom),
            // It starts at the top (a post taller than the window): page
            // through it.
            Some(_) => value + step,
        }
    } else {
        let t = value - step;
        // The first post starting within the previous screenful, if any,
        // so that the page above begins at a post.
        rows.iter()
            .map(|(top, _)| *top)
            .find(|top| *top >= t - 0.5 && *top <= value - MIN_MOVE)
            .unwrap_or(t)
    };
    let target = target.clamp(0.0, max);
    if (dir > 0 && target <= value + 0.5) || (dir < 0 && target >= value - 0.5) {
        None
    } else {
        Some(target)
    }
}

thread_local! {
    /// Whether focus in the main window was last moved with the keyboard
    /// (navigation keys) rather than by a pointer press or touch.
    static FOCUS_BY_KEYBOARD: Cell<bool> = const { Cell::new(false) };
}

/// Notes a pointer press or touch in the main window (see `handle_key`).
pub fn pointer_pressed() {
    FOCUS_BY_KEYBOARD.with(|k| k.set(false));
}

/// Handles a key press in the main window; true if it paged.
pub fn handle_key(app: &Rc<App>, key: Key, mods: ModifierType) -> bool {
    if matches!(
        key,
        Key::Tab
            | Key::ISO_Left_Tab
            | Key::KP_Tab
            | Key::Up
            | Key::Down
            | Key::Left
            | Key::Right
            | Key::Home
            | Key::End
    ) {
        FOCUS_BY_KEYBOARD.with(|k| k.set(true));
    }
    if mods.intersects(
        ModifierType::CONTROL_MASK
            | ModifierType::ALT_MASK
            | ModifierType::SUPER_MASK
            | ModifierType::META_MASK,
    ) {
        return false;
    }
    let shift = mods.contains(ModifierType::SHIFT_MASK);
    let typing_key = matches!(key, Key::space | Key::BackSpace);
    enum Action {
        Page(i32),
        /// Like the Back button (Alt+Left, Esc).
        Back,
    }
    let action = match key {
        Key::Page_Down | Key::KP_Page_Down => Action::Page(1),
        Key::Page_Up | Key::KP_Page_Up => Action::Page(-1),
        Key::space if shift => Action::Page(-1),
        Key::space => Action::Page(1),
        Key::BackSpace => Action::Back,
        _ => return false,
    };
    let focus = gtk::prelude::GtkWindowExt::focus(&app.window);
    if let Some(f) = &focus {
        // Text fields keep Space and Backspace.  A button, check box or
        // switch keeps Space only if focus was last moved with the keyboard
        // (Tab, arrows): after a click, focus sits on the clicked button (a
        // media thumbnail, a favourite) or wherever GTK moved it when the
        // clicked button hid (the "new posts" pill), and Space would press
        // that again.  GTK's own `focus-visible` can't tell these apart: any
        // key press, Space included, turns it on.
        let editing = f.is::<gtk::Text>()
            || f.is::<gtk::TextView>()
            || f.dynamic_cast_ref::<gtk::Editable>().is_some();
        let pressable =
            f.is::<gtk::Button>() || f.is::<gtk::CheckButton>() || f.is::<gtk::Switch>();
        let keyboard_focus = FOCUS_BY_KEYBOARD.with(Cell::get);
        if (typing_key && editing) || (key == Key::space && pressable && keyboard_focus) {
            return false;
        }
    }
    match action {
        Action::Page(dir) => page(app, dir),
        // Nothing to go back to on a top-level page: leave the key alone.
        Action::Back if !app.can_go_back() => false,
        Action::Back => {
            app.pop();
            true
        }
    }
}

/// Pages the current view; false if it has nothing to scroll.
fn page(app: &Rc<App>, dir: i32) -> bool {
    let Some((widget, keep)) = app.current_page() else {
        return false;
    };
    let Some(sw) = main_scroller(&widget) else {
        return false;
    };
    let adj = sw.vadjustment();
    // Repeated presses during an animation continue from where it's going.
    let value = ANIM.with(|a| {
        a.borrow()
            .as_ref()
            .filter(|anim| anim.scroller.upgrade().as_ref() == Some(&sw))
            .map(|anim| anim.to.get())
    });
    let value = value.unwrap_or(adj.value());
    match page_target(&post_rows(&sw), value, adj.page_size(), adj.upper(), dir) {
        Some(target) => {
            LAST_EDGE.with(|e| *e.borrow_mut() = None);
            animate(&sw, target);
        }
        None => at_end(app, &sw, keep, dir),
    }
    true
}

/// The view's main scrolling area: the tallest one that's showing.
fn main_scroller(root: &gtk::Widget) -> Option<gtk::ScrolledWindow> {
    let mut best: Option<gtk::ScrolledWindow> = None;
    visit(root, &mut |w| {
        if let Some(sw) = w.downcast_ref::<gtk::ScrolledWindow>()
            && sw.is_mapped()
            && sw.vscrollbar_policy() != gtk::PolicyType::Never
            && best.as_ref().is_none_or(|b| sw.height() > b.height())
        {
            best = Some(sw.clone());
        }
    });
    best
}

/// The (top, bottom) of each visible post row in the scroller's content.
fn post_rows(sw: &gtk::ScrolledWindow) -> Vec<(f64, f64)> {
    // Measure against the scrolled content, not the viewport around it,
    // which shifts by the scroll value (see `pages::scroll_to_row`).
    let Some(child) = sw.child() else {
        return Vec::new();
    };
    let content = match child.downcast::<gtk::Viewport>() {
        Ok(viewport) => match viewport.child() {
            Some(c) => c,
            None => return Vec::new(),
        },
        Err(child) => child,
    };
    // The posts are the rows of the tallest list in the view.
    let mut list: Option<gtk::ListBox> = None;
    visit(&content, &mut |w| {
        if let Some(l) = w.downcast_ref::<gtk::ListBox>()
            && l.is_mapped()
            && list.as_ref().is_none_or(|b| l.height() > b.height())
        {
            list = Some(l.clone());
        }
    });
    let Some(list) = list else { return Vec::new() };
    let mut rows = Vec::new();
    let mut child = list.first_child();
    while let Some(row) = child {
        if row.is::<gtk::ListBoxRow>()
            && row.is_visible()
            && row.height() > 0
            && let Some(b) = row.compute_bounds(&content)
        {
            rows.push((b.y() as f64, (b.y() + b.height()) as f64));
        }
        child = row.next_sibling();
    }
    rows
}

fn visit(w: &gtk::Widget, f: &mut dyn FnMut(&gtk::Widget)) {
    f(w);
    let mut c = w.first_child();
    while let Some(child) = c {
        visit(&child, f);
        c = child.next_sibling();
    }
}

// ----- Ends of the view -----

thread_local! {
    /// The last press that hit an end: (scroller, direction, when).
    static LAST_EDGE: RefCell<Option<(glib::WeakRef<gtk::ScrolledWindow>, i32, Instant)>> =
        const { RefCell::new(None) };
}

/// Paging past an end: the first time says so, a second press soon after
/// checks for new posts (the top of a feed) or replies (either end of a
/// thread).
fn at_end(app: &Rc<App>, sw: &gtk::ScrolledWindow, keep: Option<Rc<dyn Any>>, dir: i32) {
    let timeline = keep.clone().and_then(|k| k.downcast::<Timeline>().ok());
    let thread = keep.and_then(|k| k.downcast::<ThreadView>().ok());
    let what = match (&timeline, &thread) {
        (Some(_), _) if dir < 0 => "new posts",
        (_, Some(_)) => "new replies",
        // The bottom of a feed loads older posts by itself.
        _ => return,
    };
    let again = LAST_EDGE.with(|e| {
        e.borrow().as_ref().is_some_and(|(w, d, when)| {
            w.upgrade().as_ref() == Some(sw) && *d == dir && when.elapsed() < AGAIN_WITHIN
        })
    });
    if !again {
        LAST_EDGE.with(|e| *e.borrow_mut() = Some((sw.downgrade(), dir, Instant::now())));
        let end = if dir < 0 { "top" } else { "end" };
        app.toast(&format!("At the {end}. Press again to check for {what}."));
        return;
    }
    LAST_EDGE.with(|e| *e.borrow_mut() = None);
    app.toast(&format!("Checking for {what}…"));
    if let Some(t) = timeline {
        t.check_new();
    } else if let Some(t) = thread {
        t.reload_at(if dir < 0 {
            ThreadFocus::First
        } else {
            ThreadFocus::Last
        });
    }
}

// ----- Animation -----

struct Anim {
    scroller: glib::WeakRef<gtk::ScrolledWindow>,
    to: Cell<f64>,
    tick: RefCell<Option<gtk::TickCallbackId>>,
}

thread_local! {
    /// The scroll animation in progress, if any.
    static ANIM: RefCell<Option<Rc<Anim>>> = const { RefCell::new(None) };
}

/// Scrolls `sw` to `target` with a short ease-out, or at once if the
/// desktop has animations turned off.
fn animate(sw: &gtk::ScrolledWindow, target: f64) {
    if let Some(old) = ANIM.with(|a| a.borrow_mut().take())
        && let Some(id) = old.tick.borrow_mut().take()
    {
        id.remove();
    }
    let adj = sw.vadjustment();
    let animations = gtk::Settings::default().is_none_or(|s| s.is_gtk_enable_animations());
    if !animations {
        adj.set_value(target);
        return;
    }
    let anim = Rc::new(Anim {
        scroller: sw.downgrade(),
        to: Cell::new(target),
        tick: RefCell::new(None),
    });
    let from = adj.value();
    let start: Cell<Option<i64>> = Cell::new(None);
    let last = Cell::new(from);
    let a = Rc::downgrade(&anim);
    let id = sw.add_tick_callback(move |sw, clock| {
        let adj = sw.vadjustment();
        let now = clock.frame_time();
        let t0 = *start.get().get_or_insert(now);
        start.set(Some(t0));
        // The user scrolled meanwhile: let them have it.
        if (adj.value() - last.get()).abs() > 1.0 {
            finish(&a);
            return glib::ControlFlow::Break;
        }
        let p = ((now - t0) as f64 / ANIMATION.as_micros() as f64).clamp(0.0, 1.0);
        let eased = 1.0 - (1.0 - p).powi(3);
        adj.set_value(from + (target - from) * eased);
        last.set(adj.value());
        if p >= 1.0 {
            finish(&a);
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
    *anim.tick.borrow_mut() = Some(id);
    ANIM.with(|a| *a.borrow_mut() = Some(anim));
}

/// Forgets the animation once it has stopped (its tick callback is gone).
fn finish(anim: &std::rc::Weak<Anim>) {
    let Some(anim) = anim.upgrade() else { return };
    anim.tick.borrow_mut().take();
    ANIM.with(|a| {
        let mut a = a.borrow_mut();
        if a.as_ref().is_some_and(|cur| Rc::ptr_eq(cur, &anim)) {
            *a = None;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    // Posts as (top, bottom); the third is taller than the 600px page.
    const ROWS: [(f64, f64); 6] = [
        (0.0, 300.0),
        (300.0, 500.0),
        (500.0, 1400.0),
        (1400.0, 1600.0),
        (1600.0, 2000.0),
        (2000.0, 2300.0),
    ];
    const PAGE: f64 = 600.0;
    const UPPER: f64 = 2350.0; // a footer below the last post; max value 1750

    fn down(v: f64) -> Option<f64> {
        page_target(&ROWS, v, PAGE, UPPER, 1)
    }

    fn up(v: f64) -> Option<f64> {
        page_target(&ROWS, v, PAGE, UPPER, -1)
    }

    #[test]
    fn page_down_brings_the_cut_post_to_the_top() {
        // View 0-600 cuts the tall post (500-1400): its start goes to the top,
        // so the part already seen is shown again with the rest.
        assert_eq!(down(0.0), Some(500.0));
        // View 1060-1660 cuts 1600-2000.
        assert_eq!(down(1060.0), Some(1600.0));
    }

    #[test]
    fn page_down_brings_a_cut_post_up_even_if_mostly_seen() {
        // Seen in the app: a 614px post starting 127px below the top of a
        // 721px view, cut 20px short.  It goes to the top, a boundary,
        // rather than a full page down into the middle of it.
        let rows = [(1439.0, 1566.0), (1566.0, 2180.0), (2180.0, 2600.0)];
        assert_eq!(page_target(&rows, 1439.0, 721.0, 3000.0, 1), Some(1566.0));
        // A post starting only a few pixels down is paged past instead of
        // making a move too small to notice.
        let rows = [(0.0, 10.0), (10.0, 900.0), (900.0, 2000.0)];
        assert_eq!(
            page_target(&rows, 0.0, 600.0, 2000.0, 1),
            Some(600.0 - OVERLAP)
        );
    }

    #[test]
    fn page_down_pages_through_a_tall_post() {
        // The tall post fills the view from its top: page within it, keeping
        // OVERLAP px of the old page.
        assert_eq!(down(500.0), Some(500.0 + PAGE - OVERLAP));
    }

    #[test]
    fn page_down_stops_at_the_end() {
        // 1600 would cut 2000-2300 and go to 2000, past the end: clamped.
        assert_eq!(down(1600.0), Some(1750.0));
        assert_eq!(down(1750.0), None);
        // Content shorter than the page: nothing to do.
        assert_eq!(page_target(&ROWS[..1], 0.0, PAGE, 300.0, 1), None);
    }

    #[test]
    fn page_up_starts_the_page_at_a_post() {
        // A plain page up from 1000 would go to 440; the first post starting
        // from there on is the tall one at 500.
        assert_eq!(up(1000.0), Some(500.0));
        // Inside the tall post, with no post starting in the page above: a
        // plain page up.
        assert_eq!(up(1300.0), Some(1300.0 - (PAGE - OVERLAP)));
        // Near the top: to the top, then nothing.
        assert_eq!(up(200.0), Some(0.0));
        assert_eq!(up(0.0), None);
    }

    #[test]
    fn paging_never_skips_content() {
        // All the way down and back up: every step moves, and never by more
        // than a page, so nothing goes unseen.
        let mut v = 0.0;
        while let Some(t) = down(v) {
            assert!(t > v && t <= v + PAGE, "down {v} -> {t}");
            v = t;
        }
        assert_eq!(v, UPPER - PAGE);
        while let Some(t) = up(v) {
            assert!(t < v && t >= v - PAGE, "up {v} -> {t}");
            v = t;
        }
        assert_eq!(v, 0.0);
    }
}
