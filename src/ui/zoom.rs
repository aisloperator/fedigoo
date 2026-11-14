//! Zooming the whole view, as in a web browser: Ctrl++ (or Ctrl+=), Ctrl+−
//! and Ctrl+0, and the Zoom submenu of the main menu.
//!
//! The zoom steps through fixed levels that include 100%, so zooming in and
//! back out always returns to exactly 100%.  Only the view of pages (posts,
//! profiles and so on: the widgets with the `ZOOMED` CSS class, the main
//! window's page stack) zooms; the header bar, the sidebar, menus and other
//! windows keep their size.  It sets a CSS `font-size` in percent on that
//! class, so text and everything sized from it (the post column's width,
//! paddings in `em`) follows.  Avatars zoom too (`set_pixel_size`); icons keep
//! their pixel sizes.  Images in posts never get larger than at 100%
//! (`fit::set_zoom`), but they shrink with a narrower column.

use super::fit;
use gtk::{glib, prelude::*};
use std::cell::{Cell, RefCell};

/// The zoom levels, in percent.
pub const LEVELS: [u32; 11] = [50, 67, 80, 90, 100, 110, 120, 133, 150, 170, 200];
pub const DEFAULT: u32 = 100;

/// The next level above `level`, or None at the top.
pub fn zoom_in(level: u32) -> Option<u32> {
    LEVELS.iter().copied().find(|&l| l > level)
}

/// The next level below `level`, or None at the bottom.
pub fn zoom_out(level: u32) -> Option<u32> {
    LEVELS.iter().rev().copied().find(|&l| l < level)
}

/// `level` if it's one of `LEVELS`, else the nearest one (for a hand-edited
/// or out-of-range setting).
pub fn nearest(level: u32) -> u32 {
    LEVELS
        .iter()
        .copied()
        .min_by_key(|&l| l.abs_diff(level))
        .unwrap_or(DEFAULT)
}

/// The CSS class of the widgets whose contents zoom.
pub const ZOOMED: &str = "zoomed";

/// The style sheet for `level`.
pub fn css(level: u32) -> String {
    format!(".{ZOOMED} {{ font-size: {level}%; }}")
}

/// `px` pixels at 100%, at `level`.
pub fn scaled(px: i32, level: u32) -> i32 {
    ((px as i64 * level as i64 + 50) / 100) as i32
}

/// Sets `img` (an avatar) to `px` pixels at 100%, scaled to the zoom now and
/// whenever it changes.
pub fn set_pixel_size(img: &gtk::Image, px: i32) {
    img.set_pixel_size(scaled(px, LEVEL.with(Cell::get)));
    SIZED.with(|v| {
        let mut v = v.borrow_mut();
        // Drop the images that are gone, now and then.
        if v.len() >= SIZED_CAP.with(Cell::get) {
            v.retain(|(w, _)| w.upgrade().is_some());
            SIZED_CAP.with(|c| c.set((v.len() * 2).max(256)));
        }
        v.push((img.downgrade(), px));
    });
}

thread_local! {
    /// Images sized by `set_pixel_size`, with their size at 100%.
    static SIZED: RefCell<Vec<(glib::WeakRef<gtk::Image>, i32)>> = const { RefCell::new(Vec::new()) };
    static SIZED_CAP: Cell<usize> = const { Cell::new(256) };
    static PROVIDER: RefCell<Option<gtk::CssProvider>> = const { RefCell::new(None) };
    static LEVEL: Cell<u32> = const { Cell::new(DEFAULT) };
}

/// Applies `level` to the zoomed view.
pub fn apply(level: u32) {
    let Some(display) = gtk::gdk::Display::default() else {
        return;
    };
    PROVIDER.with(|p| {
        p.borrow_mut()
            .get_or_insert_with(|| {
                let provider = gtk::CssProvider::new();
                gtk::style_context_add_provider_for_display(
                    &display,
                    &provider,
                    gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
                );
                provider
            })
            .load_from_string(&css(level));
    });
    LEVEL.with(|l| l.set(level));
    fit::set_zoom(level as f64 / 100.0);
    SIZED.with(|v| {
        v.borrow_mut().retain(|(w, px)| match w.upgrade() {
            Some(img) => {
                img.set_pixel_size(scaled(*px, level));
                true
            }
            None => false,
        })
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_return_to_100() {
        let mut l = DEFAULT;
        for _ in 0..3 {
            l = zoom_in(l).unwrap();
        }
        assert_eq!(l, 133);
        for _ in 0..3 {
            l = zoom_out(l).unwrap();
        }
        assert_eq!(l, DEFAULT);
        for _ in 0..2 {
            l = zoom_out(l).unwrap();
        }
        assert_eq!(l, 80);
        for _ in 0..2 {
            l = zoom_in(l).unwrap();
        }
        assert_eq!(l, DEFAULT);
    }

    #[test]
    fn stops_at_the_ends() {
        assert_eq!(zoom_in(200), None);
        assert_eq!(zoom_out(50), None);
        // Between levels (a hand-edited setting): the next one either way.
        assert_eq!(zoom_in(105), Some(110));
        assert_eq!(zoom_out(105), Some(100));
    }

    #[test]
    fn nearest_level() {
        assert_eq!(nearest(100), 100);
        assert_eq!(nearest(104), 100);
        assert_eq!(nearest(1000), 200);
        assert_eq!(nearest(0), 50);
    }

    #[test]
    fn pixel_sizes() {
        assert_eq!(scaled(44, 100), 44);
        assert_eq!(scaled(44, 150), 66);
        assert_eq!(scaled(44, 133), 59);
        assert_eq!(scaled(44, 50), 22);
    }

    #[test]
    fn style_sheet() {
        assert_eq!(css(133), ".zoomed { font-size: 133%; }");
    }
}
