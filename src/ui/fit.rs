//! `FitLayout`: lays out a post's media thumbnails, each shown whole at its
//! own aspect ratio (scaled, never cropped), with small gaps between them:
//!
//! * 1 image: as large as fits the width and the height limit, centred.
//! * 2 images: side by side at a common height, filling the width.
//! * 3 images: the first in about half the width, the other two stacked
//!   beside it at half the height limit each.
//! * 4 or more: the first in about half the width, the next four in a
//!   2×2 grid beside it, and any more flowed below in centred rows, each at
//!   most a quarter of the width and half the height limit.
//!
//! The height limit is half the main window's height, shared by all
//! layouts and updated (with a re-layout) when the window resizes.
//!
//! A layout gets taller as it gets wider, which GTK's height-for-width
//! contract (and its containers, which allocate rows their minimum height)
//! doesn't allow for.  So instead the widget reports a constant size
//! computed from the width it was last allocated, and when that width
//! changes it re-queues a resize; the width never depends on the height, so
//! this settles after one extra layout pass.

use gtk::{glib, prelude::*, subclass::prelude::*};
use std::cell::{Cell, RefCell};

/// Aspect ratio assumed until the real one is known.
const DEFAULT_ASPECT: f64 = 16.0 / 9.0;
/// Gap between images.
const GAP: f64 = 4.0;
/// Images in the first block (the large one plus up to four beside it).
const BLOCK: usize = 5;

thread_local! {
    static MAX_HEIGHT: Cell<i32> = const { Cell::new(400) };
    /// Most recent layout width, used as the first guess for new layouts.
    static LAST_WIDTH: Cell<i32> = const { Cell::new(600) };
    static LAYOUTS: RefCell<Vec<glib::WeakRef<FitLayout>>> = const { RefCell::new(Vec::new()) };
}

/// Called when the main window's height changes.
pub fn set_window_height(height: i32) {
    let max = (height / 2).max(120);
    if MAX_HEIGHT.with(|m| m.replace(max)) == max {
        return;
    }
    LAYOUTS.with(|r| {
        r.borrow_mut().retain(|w| match w.upgrade() {
            Some(l) => {
                l.queue_resize();
                true
            }
            None => false,
        })
    });
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// A placed size in floating point, before rounding.
#[derive(Clone, Copy, Default)]
struct Place {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

impl Place {
    fn rect(self) -> Rect {
        let (x, y) = (self.x.round(), self.y.round());
        Rect {
            x: x as i32,
            y: y as i32,
            w: ((self.x + self.w).round() - x).max(1.0) as i32,
            h: ((self.y + self.h).round() - y).max(1.0) as i32,
        }
    }
}

fn aspect_or_default(aspect: f64) -> f64 {
    if aspect > 0.0 { aspect } else { DEFAULT_ASPECT }
}

/// Largest (width, height) with `aspect` that fits a `bw` × `bh` box.
fn fit(aspect: f64, bw: f64, bh: f64) -> (f64, f64) {
    let a = aspect_or_default(aspect);
    let h = (bw.max(1.0) / a).min(bh.max(1.0));
    (h * a, h)
}

/// Places images with the given aspect ratios (width / height; 0 if
/// unknown) in `width`, with height limit `max_h`.  Returns one rectangle
/// per image, in order, and the total height.
pub fn layout(aspects: &[f64], width: i32, max_h: i32) -> (Vec<Rect>, i32) {
    let (w, max_h) = (width.max(1) as f64, max_h.max(1) as f64);
    let mut places = vec![Place::default(); aspects.len()];
    let mut height = match aspects.len() {
        0 => 0.0,
        1 => {
            let (iw, ih) = fit(aspects[0], w, max_h);
            places[0] = Place {
                x: (w - iw) / 2.0,
                y: 0.0,
                w: iw,
                h: ih,
            };
            ih
        }
        2 => {
            // A common height at which both fill the width.
            let sum: f64 = aspects.iter().map(|&a| aspect_or_default(a)).sum();
            let h = ((w - GAP) / sum).min(max_h);
            let mut x = (w - (h * sum + GAP)) / 2.0;
            for (p, &a) in places.iter_mut().zip(aspects) {
                let iw = h * aspect_or_default(a);
                *p = Place {
                    x,
                    y: 0.0,
                    w: iw,
                    h,
                };
                x += iw + GAP;
            }
            h
        }
        _ => block(aspects, &mut places, w, max_h),
    };
    if aspects.len() > BLOCK {
        height = flow(
            &aspects[BLOCK..],
            &mut places[BLOCK..],
            w,
            max_h,
            height + GAP,
        );
    }
    let rects = places.into_iter().map(Place::rect).collect();
    (rects, height.round() as i32)
}

/// The first image in about half the width, and up to four more beside it:
/// stacked in one column if there are two, otherwise a 2×2 grid filled
/// left to right.  Returns the block's height.
fn block(aspects: &[f64], places: &mut [Place], w: f64, max_h: f64) -> f64 {
    let half = (w - GAP) / 2.0;
    let (lw, lh) = fit(aspects[0], half, max_h);

    let side = &aspects[1..aspects.len().min(BLOCK)];
    let cols = if side.len() <= 2 { 1 } else { 2 };
    let rows = side.len().div_ceil(cols);
    let cell_w = (half - GAP * (cols - 1) as f64) / cols as f64;
    let cell_h = (max_h - GAP * (rows - 1) as f64) / rows as f64;
    let sizes: Vec<_> = side.iter().map(|&a| fit(a, cell_w, cell_h)).collect();
    // Shrink each column and row to its largest image, so the gaps
    // between images stay small.
    let mut col_w = vec![0.0f64; cols];
    let mut row_h = vec![0.0f64; rows];
    for (i, &(iw, ih)) in sizes.iter().enumerate() {
        col_w[i % cols] = col_w[i % cols].max(iw);
        row_h[i / cols] = row_h[i / cols].max(ih);
    }
    let side_w = col_w.iter().sum::<f64>() + GAP * (cols - 1) as f64;
    let side_h = row_h.iter().sum::<f64>() + GAP * (rows - 1) as f64;

    let height = lh.max(side_h);
    let x0 = (w - (lw + GAP + side_w)) / 2.0;
    places[0] = Place {
        x: x0,
        y: (height - lh) / 2.0,
        w: lw,
        h: lh,
    };
    let side_x = x0 + lw + GAP;
    let side_y = (height - side_h) / 2.0;
    for (i, &(iw, ih)) in sizes.iter().enumerate() {
        let (c, r) = (i % cols, i / cols);
        let cx = side_x + col_w[..c].iter().map(|cw| cw + GAP).sum::<f64>();
        let cy = side_y + row_h[..r].iter().map(|rh| rh + GAP).sum::<f64>();
        places[1 + i] = Place {
            x: cx + (col_w[c] - iw) / 2.0,
            y: cy + (row_h[r] - ih) / 2.0,
            w: iw,
            h: ih,
        };
    }
    height
}

/// Flows images in rows starting at `y`, each at most a quarter of the
/// width and half the height limit; rows are centred horizontally and
/// images centred vertically within their row.  Returns the bottom.
fn flow(aspects: &[f64], places: &mut [Place], w: f64, max_h: f64, mut y: f64) -> f64 {
    let box_w = (w - 3.0 * GAP) / 4.0;
    let sizes: Vec<_> = aspects
        .iter()
        .map(|&a| fit(a, box_w, max_h / 2.0))
        .collect();
    let mut start = 0;
    while start < sizes.len() {
        // Take images while they fit the width (always at least one).
        let mut end = start + 1;
        let mut row_w = sizes[start].0;
        while end < sizes.len() && row_w + GAP + sizes[end].0 <= w {
            row_w += GAP + sizes[end].0;
            end += 1;
        }
        let row_h = sizes[start..end].iter().map(|s| s.1).fold(0.0, f64::max);
        let mut x = (w - row_w) / 2.0;
        for i in start..end {
            let (iw, ih) = sizes[i];
            places[i] = Place {
                x,
                y: y + (row_h - ih) / 2.0,
                w: iw,
                h: ih,
            };
            x += iw + GAP;
        }
        y += row_h + GAP;
        start = end;
    }
    y - GAP
}

/// Natural width: about what shows the images at the full height limit.
fn natural_width(aspects: &[f64], max_h: f64) -> f64 {
    let a = |i: usize| aspect_or_default(aspects[i]);
    match aspects.len() {
        0 => 0.0,
        1 => max_h * a(0),
        2 => max_h * (a(0) + a(1)) + GAP,
        _ => 2.0 * max_h * a(0) + GAP,
    }
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct FitLayout {
        /// Children and their aspect ratios (width / height; 0 = unknown).
        pub children: RefCell<Vec<(gtk::Widget, f64)>>,
        /// Width of the last allocation; 0 before the first.
        pub width: Cell<i32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FitLayout {
        const NAME: &'static str = "FediGooFitLayout";
        type Type = super::FitLayout;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for FitLayout {
        fn dispose(&self) {
            for (child, _) in self.children.take() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for FitLayout {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::ConstantSize
        }

        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let children = self.children.borrow();
            let child_min = children
                .iter()
                .map(|(c, _)| c.measure(orientation, -1).0)
                .max()
                .unwrap_or(0);
            let aspects = self.aspects();
            match orientation {
                gtk::Orientation::Horizontal => {
                    let across = children.len().min(3) as i32;
                    let min = child_min * across + GAP as i32 * (across - 1).max(0);
                    let nat = natural_width(&aspects, MAX_HEIGHT.with(Cell::get) as f64);
                    (min, (nat.round() as i32).max(min), -1, -1)
                }
                _ => {
                    let width = match self.width.get() {
                        0 => LAST_WIDTH.with(Cell::get),
                        w => w,
                    };
                    let h = layout(&aspects, width, MAX_HEIGHT.with(Cell::get)).1;
                    let h = h.max(child_min);
                    (h, h, -1, -1)
                }
            }
        }

        fn size_allocate(&self, width: i32, height: i32, _baseline: i32) {
            if self.width.replace(width) != width {
                LAST_WIDTH.with(|w| w.set(width));
                // Our height depends on this width: measure again, outside
                // of this allocation pass.
                let obj = self.obj().downgrade();
                glib::idle_add_local_once(move || {
                    if let Some(o) = obj.upgrade() {
                        o.queue_resize();
                    }
                });
            }
            let (rects, _) = layout(&self.aspects(), width, MAX_HEIGHT.with(Cell::get));
            for ((child, _), r) in self.children.borrow().iter().zip(rects) {
                let min_w = child.measure(gtk::Orientation::Horizontal, -1).0;
                let min_h = child.measure(gtk::Orientation::Vertical, -1).0;
                let w = r.w.max(min_w);
                let h = r.h.min(height - r.y).max(min_h);
                child.size_allocate(&gtk::Allocation::new(r.x, r.y, w, h), -1);
            }
        }
    }

    impl FitLayout {
        fn aspects(&self) -> Vec<f64> {
            self.children.borrow().iter().map(|(_, a)| *a).collect()
        }
    }
}

glib::wrapper! {
    pub struct FitLayout(ObjectSubclass<imp::FitLayout>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl FitLayout {
    pub fn new() -> FitLayout {
        let l: FitLayout = glib::Object::new();
        LAYOUTS.with(|r| {
            let mut r = r.borrow_mut();
            if r.len() >= 256 {
                r.retain(|w| w.upgrade().is_some());
            }
            r.push(l.downgrade());
        });
        l
    }

    /// Adds a child with aspect ratio `aspect` (width / height; 0 if unknown).
    pub fn append(&self, child: &impl IsA<gtk::Widget>, aspect: f64) {
        child.set_parent(self);
        self.imp()
            .children
            .borrow_mut()
            .push((child.clone().upcast(), aspect));
        self.queue_resize();
    }

    /// Sets `child`'s aspect ratio if it wasn't known, e.g. once it loads.
    pub fn set_aspect_if_unknown(&self, child: &impl IsA<gtk::Widget>, aspect: f64) {
        let child = child.as_ref();
        let mut children = self.imp().children.borrow_mut();
        if let Some(entry) = children.iter_mut().find(|(c, _)| c == child)
            && entry.1 <= 0.0
            && aspect > 0.0
        {
            entry.1 = aspect;
            drop(children);
            self.queue_resize();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_overlap(rects: &[Rect]) {
        for (i, a) in rects.iter().enumerate() {
            for b in &rects[i + 1..] {
                let apart =
                    a.x + a.w <= b.x || b.x + b.w <= a.x || a.y + a.h <= b.y || b.y + b.h <= a.y;
                assert!(apart, "{a:?} overlaps {b:?}");
            }
        }
    }

    fn inside(rects: &[Rect], width: i32, height: i32) {
        for r in rects {
            assert!(r.x >= 0 && r.y >= 0, "{r:?}");
            assert!(
                r.x + r.w <= width && r.y + r.h <= height,
                "{r:?} outside {width}×{height}"
            );
        }
    }

    #[test]
    fn one_image_fits_and_centres() {
        let (r, h) = layout(&[2.0], 600, 400);
        assert_eq!(
            r,
            vec![Rect {
                x: 0,
                y: 0,
                w: 600,
                h: 300
            }]
        );
        assert_eq!(h, 300);
        let (r, h) = layout(&[0.5], 600, 400);
        assert_eq!(
            r,
            vec![Rect {
                x: 200,
                y: 0,
                w: 200,
                h: 400
            }]
        );
        assert_eq!(h, 400);
    }

    #[test]
    fn two_images_share_a_height_and_fill_width() {
        let (r, h) = layout(&[1.0, 2.0], 604, 400);
        assert_eq!(h, 200);
        assert_eq!(
            r[0],
            Rect {
                x: 0,
                y: 0,
                w: 200,
                h: 200
            }
        );
        assert_eq!(
            r[1],
            Rect {
                x: 204,
                y: 0,
                w: 400,
                h: 200
            }
        );
    }

    #[test]
    fn three_images_stack_beside_first() {
        let (r, h) = layout(&[1.0, 1.0, 1.0], 604, 400);
        inside(&r, 604, h);
        no_overlap(&r);
        // First takes the left half; the others stack on the right.
        assert_eq!((r[0].w, r[0].h), (300, 300));
        assert_eq!(r[1].x, r[2].x);
        assert!(r[1].x >= r[0].x + r[0].w + 4);
        assert!(r[2].y >= r[1].y + r[1].h + 4);
        assert!(r[1].h <= 198);
    }

    #[test]
    fn five_images_grid_beside_first() {
        let (r, h) = layout(&[1.0; 5], 604, 400);
        inside(&r, 604, h);
        no_overlap(&r);
        assert_eq!(r[1].y, r[2].y);
        assert_eq!(r[3].y, r[4].y);
        assert_eq!(r[1].x, r[3].x);
        assert!(r[2].x > r[1].x && r[3].y > r[1].y);
    }

    #[test]
    fn four_images_leave_last_cell_empty() {
        let (r, h) = layout(&[1.5, 1.0, 1.0, 1.0], 800, 400);
        inside(&r, 800, h);
        no_overlap(&r);
        assert_eq!(r[1].y, r[2].y);
        assert_eq!(r[3].x, r[1].x);
    }

    #[test]
    fn extras_flow_below_in_centred_rows() {
        let aspects = [1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 2.0, 0.5, 1.0, 1.0];
        let (r, h) = layout(&aspects, 800, 400);
        inside(&r, 800, h);
        no_overlap(&r);
        let top = r[..5].iter().map(|r| r.y + r.h).max().unwrap();
        for x in &r[5..] {
            assert!(x.y >= top + 4);
            assert!(x.w <= 197 && x.h <= 200, "{x:?}");
        }
        // The first flowed row is centred.
        let row: Vec<_> = r[5..].iter().filter(|x| x.y < r[5].y + r[5].h).collect();
        let left = row.first().unwrap().x;
        let right = 800 - (row.last().unwrap().x + row.last().unwrap().w);
        assert!((left - right).abs() <= 1, "{left} vs {right}");
    }
}
