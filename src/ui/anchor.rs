//! Keeps the post at the top of the view in place when the view changes size.
//!
//! A new width rewraps every post and a new height changes the image limit
//! (see `fit`), so the rows above move while the scroll position, a plain
//! pixel offset, stays put: what's in view jumps to somewhere else in the feed.
//!
//! So just before a resize takes effect, `capture` notes the row at the top
//! edge and how far through it that edge is, and after each layout `restore`
//! scrolls back to the same place.  That has to happen in the layout itself
//! (the surface's `layout` signal), since a tick callback would paint one frame
//! at the wrong place.  Rows can need more than one layout to settle, so it's
//! repeated for a moment, or until the user scrolls.
//!
//! Each resize starts afresh from wherever the view is by then, on purpose: if
//! one couldn't keep its place (the window got taller than the feed), the next
//! doesn't try to get back to the old one.

use super::paging;
use gtk::{glib, prelude::*};
use std::cell::RefCell;
use std::time::{Duration, Instant};

/// How long after a resize layouts are followed.
const SETTLE: Duration = Duration::from_secs(1);

/// Where the top edge of the view is, given the rows' (top, bottom) in content
/// coordinates, in order: which row, and how far through it (0 to 1).  None if
/// that's not on a post, or at the very top, which should stay the top even if
/// a new post arrives above.
fn anchor_at(rows: &[(f64, f64)], value: f64) -> Option<(usize, f64)> {
    if value < 1.0 {
        return None;
    }
    let i = rows.iter().position(|(_, bottom)| *bottom > value)?;
    let (top, bottom) = rows[i];
    // Above the first post: a profile's header, say.
    if i == 0 && top > value {
        return None;
    }
    Some((i, ((value - top) / (bottom - top)).clamp(0.0, 1.0)))
}

/// The scroll position for that same place in a row now at (top, bottom).
fn value_within(top: f64, bottom: f64, within: f64) -> f64 {
    top + within * (bottom - top)
}

struct Hold {
    scroller: glib::WeakRef<gtk::ScrolledWindow>,
    row: glib::WeakRef<gtk::ListBoxRow>,
    within: f64,
    /// The scroll position this last left, to notice anyone else moving it.
    value: f64,
    until: Instant,
}

impl Hold {
    /// Scrolls back to the held place; false once there's nothing left to do.
    fn restore(&mut self) -> bool {
        let (Some(sw), Some(row)) = (self.scroller.upgrade(), self.row.upgrade()) else {
            return false;
        };
        if Instant::now() >= self.until || !row.is_mapped() {
            return false;
        }
        let adj = sw.vadjustment();
        let max = (adj.upper() - adj.page_size()).max(0.0);
        // The user scrolled.  GTK also moves it, down to the new end of the
        // content, when that shrinks.
        if (adj.value() - self.value).abs() > 1.0 && adj.value() < max - 1.0 {
            return false;
        }
        let bounds = paging::scrolled_content(&sw).and_then(|c| row.compute_bounds(&c));
        if let Some(b) = bounds.filter(|b| b.height() > 0.0) {
            let (top, height) = (b.y() as f64, b.height() as f64);
            let target = value_within(top, top + height, self.within).clamp(0.0, max);
            if (target - adj.value()).abs() >= 0.5 {
                adj.set_value(target);
            }
        }
        self.value = adj.value();
        true
    }
}

thread_local! {
    static HOLD: RefCell<Option<Hold>> = const { RefCell::new(None) };
}

/// Notes what's at the top of `page`, which must still have its old layout
/// (the new one comes with the next frame).  This replaces any earlier note.
pub fn capture(page: &gtk::Widget) {
    let hold = paging::main_scroller(page).and_then(|sw| {
        let rows = paging::post_rows(&sw);
        let spans: Vec<(f64, f64)> = rows.iter().map(|r| (r.1, r.2)).collect();
        let value = sw.vadjustment().value();
        let (i, within) = anchor_at(&spans, value)?;
        Some(Hold {
            scroller: sw.downgrade(),
            row: rows[i].0.downgrade(),
            within,
            value,
            until: Instant::now() + SETTLE,
        })
    });
    HOLD.with(|h| *h.borrow_mut() = hold);
}

/// After a layout: keeps the noted post in place.
pub fn restore() {
    // Out of the cell, since moving the scroll position runs other code.
    let Some(mut hold) = HOLD.with(|h| h.borrow_mut().take()) else {
        return;
    };
    if hold.restore() {
        HOLD.with(|h| {
            h.borrow_mut().get_or_insert(hold);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROWS: [(f64, f64); 3] = [(0.0, 100.0), (100.0, 300.0), (300.0, 400.0)];

    #[test]
    fn finds_the_row_at_the_top_edge() {
        assert_eq!(anchor_at(&ROWS, 150.0), Some((1, 0.25)));
        assert_eq!(anchor_at(&ROWS, 50.0), Some((0, 0.5)));
        // On a boundary it's the row below, from its start.
        assert_eq!(anchor_at(&ROWS, 100.0), Some((1, 0.0)));
    }

    #[test]
    fn the_very_top_stays_the_top() {
        assert_eq!(anchor_at(&ROWS, 0.0), None);
    }

    #[test]
    fn nothing_to_hold_above_or_below_the_posts() {
        let below_a_header = [(250.0, 300.0), (300.0, 400.0)];
        assert_eq!(anchor_at(&below_a_header, 100.0), None);
        assert_eq!(anchor_at(&ROWS, 450.0), None);
        assert_eq!(anchor_at(&[], 100.0), None);
    }

    #[test]
    fn a_gap_between_rows_counts_as_the_start_of_the_next() {
        assert_eq!(
            anchor_at(&[(0.0, 100.0), (120.0, 200.0)], 110.0),
            Some((1, 0.0))
        );
    }

    #[test]
    fn same_place_in_a_row_of_another_size() {
        // A quarter of the way through a row that moved and got taller.
        assert_eq!(value_within(1000.0, 1800.0, 0.25), 1200.0);
    }
}
