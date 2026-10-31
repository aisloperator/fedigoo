//! `FitRow`: lays out a row of children (media thumbnails) side by side in
//! equal-width cells, each shown at its own aspect ratio, as large as fits
//! its cell and a height limit, and centred.  Images are scaled, never
//! cropped.
//!
//! An aspect-ratio box gets taller as it gets wider, which GTK's
//! height-for-width contract (and its containers, which allocate rows their
//! minimum height) doesn't allow for.  So instead the row reports a constant
//! size computed from the width it was last allocated, and when that width
//! changes it re-queues a resize; the width never depends on the height, so
//! this settles after one extra layout pass.
//!
//! The height limit is a fraction of half the main window's height, shared
//! by all rows and updated (with a re-layout) when the window resizes.

use gtk::{glib, prelude::*, subclass::prelude::*};
use std::cell::{Cell, RefCell};

/// Aspect ratio assumed until the real one is known.
const DEFAULT_ASPECT: f64 = 16.0 / 9.0;
const SPACING: i32 = 4;

thread_local! {
    static MAX_HEIGHT: Cell<i32> = const { Cell::new(400) };
    /// Most recent row width, used as the first guess for new rows.
    static LAST_WIDTH: Cell<i32> = const { Cell::new(600) };
    static ROWS: RefCell<Vec<glib::WeakRef<FitRow>>> = const { RefCell::new(Vec::new()) };
}

/// Called when the main window's height changes.
pub fn set_window_height(height: i32) {
    let max = (height / 2).max(120);
    if MAX_HEIGHT.with(|m| m.replace(max)) == max {
        return;
    }
    ROWS.with(|r| {
        r.borrow_mut().retain(|w| match w.upgrade() {
            Some(row) => {
                row.queue_resize();
                true
            }
            None => false,
        })
    });
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct FitRow {
        /// Children and their aspect ratios (width / height; 0 = unknown).
        pub children: RefCell<Vec<(gtk::Widget, f64)>>,
        /// Number of equal cells; may exceed the number of children.
        pub cells: Cell<usize>,
        /// Fraction of the shared height limit this row may use.
        pub fraction: Cell<f64>,
        /// Width of the last allocation; 0 before the first.
        pub width: Cell<i32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FitRow {
        const NAME: &'static str = "FediGooFitRow";
        type Type = super::FitRow;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for FitRow {
        fn dispose(&self) {
            for (child, _) in self.children.take() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for FitRow {
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
            let cells = self.cells.get().max(1) as i32;
            match orientation {
                gtk::Orientation::Horizontal => {
                    // Natural: wide enough to show every image at full height.
                    let max_h = self.max_height();
                    let widest = children
                        .iter()
                        .map(|(_, a)| (max_h * aspect_or_default(*a)).round() as i32)
                        .max()
                        .unwrap_or(0);
                    let min = child_min * cells + SPACING * (cells - 1);
                    let nat = widest * cells + SPACING * (cells - 1);
                    (min, nat.max(min), -1, -1)
                }
                _ => {
                    let width = match self.width.get() {
                        0 => LAST_WIDTH.with(Cell::get),
                        w => w,
                    };
                    let h = self.height_for(width).max(child_min);
                    (h, h, -1, -1)
                }
            }
        }

        fn size_allocate(&self, width: i32, height: i32, _baseline: i32) {
            if self.width.replace(width) != width {
                LAST_WIDTH.with(|w| w.set(width));
                // Our height depends on this width: measure again, outside
                // of this allocation pass.
                let row = self.obj().downgrade();
                glib::idle_add_local_once(move || {
                    if let Some(r) = row.upgrade() {
                        r.queue_resize();
                    }
                });
            }
            let cell_w = self.cell_width(width);
            for (i, (child, aspect)) in self.children.borrow().iter().enumerate() {
                let min_w = child.measure(gtk::Orientation::Horizontal, -1).0;
                let min_h = child.measure(gtk::Orientation::Vertical, -1).0;
                let (w, h) = self.fit(cell_w, *aspect);
                let w = w.clamp(min_w, cell_w.max(min_w));
                let h = h.min(height).max(min_h);
                let x = i as i32 * (cell_w + SPACING) + (cell_w - w) / 2;
                child.size_allocate(&gtk::Allocation::new(x, 0, w, h), -1);
            }
        }
    }

    fn aspect_or_default(aspect: f64) -> f64 {
        if aspect > 0.0 { aspect } else { DEFAULT_ASPECT }
    }

    impl FitRow {
        fn max_height(&self) -> f64 {
            MAX_HEIGHT.with(Cell::get) as f64 * self.fraction.get()
        }

        fn cell_width(&self, width: i32) -> i32 {
            let cells = self.cells.get().max(1) as i32;
            ((width - SPACING * (cells - 1)) / cells).max(1)
        }

        /// Row height for a row `width` wide: its tallest fitted child.
        fn height_for(&self, width: i32) -> i32 {
            let cell_w = self.cell_width(width);
            self.children
                .borrow()
                .iter()
                .map(|(_, a)| self.fit(cell_w, *a).1)
                .max()
                .unwrap_or(0)
        }

        /// Largest (width, height) with `aspect` that fits `width` and this
        /// row's height limit.
        fn fit(&self, width: i32, aspect: f64) -> (i32, i32) {
            let aspect = aspect_or_default(aspect);
            let h = (width as f64 / aspect).min(self.max_height());
            ((h * aspect).round() as i32, h.round() as i32)
        }
    }
}

glib::wrapper! {
    pub struct FitRow(ObjectSubclass<imp::FitRow>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl FitRow {
    /// A row of `cells` equal cells using `fraction` of the shared height
    /// limit (1.0 for a lone image).
    pub fn new(cells: usize, fraction: f64) -> FitRow {
        let row: FitRow = glib::Object::new();
        row.imp().cells.set(cells.max(1));
        row.imp().fraction.set(fraction);
        ROWS.with(|r| {
            let mut r = r.borrow_mut();
            if r.len() >= 256 {
                r.retain(|w| w.upgrade().is_some());
            }
            r.push(row.downgrade());
        });
        row
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
