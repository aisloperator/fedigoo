//! The image viewer's layout: the image on top, the alt text under it, then
//! the button bar. The alt text (in a `ScrolledWindow`) gets the height it
//! needs to show all its text, but never more than half the box, and it
//! never makes the window taller: the image takes whatever is left.
//!
//! A vertical `GtkBox` can't do this: a wrapped label's minimum height
//! grows the window past the screen, and the box shares spare height
//! between the image and the alt text by their natural sizes.

use gtk::{glib, prelude::*, subclass::prelude::*};
use std::cell::RefCell;

/// Heights of the image and the alt text in a box `height` tall whose bar
/// takes `bar`: the alt text gets its natural height `alt_nat` up to half
/// the box (but at least its minimum `alt_min`), the image the rest.
pub fn split(height: i32, bar: i32, alt_min: i32, alt_nat: i32) -> (i32, i32) {
    let alt = alt_nat.min(height / 2).max(alt_min);
    ((height - bar - alt).max(0), alt)
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct ViewerBox {
        /// Image, alt text and bar, top to bottom.
        pub children: RefCell<Vec<gtk::Widget>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ViewerBox {
        const NAME: &'static str = "FediGooViewerBox";
        type Type = super::ViewerBox;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for ViewerBox {
        fn dispose(&self) {
            for child in self.children.take() {
                child.unparent();
            }
        }
    }

    /// `(min, natural)` of `c` along `o`; zero if it's hidden.
    fn size(c: &gtk::Widget, o: gtk::Orientation, for_size: i32) -> (i32, i32) {
        if c.is_visible() {
            let (min, nat, _, _) = c.measure(o, for_size);
            (min, nat)
        } else {
            (0, 0)
        }
    }

    impl WidgetImpl for ViewerBox {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let children = self.children.borrow();
            let sizes = children.iter().map(|c| size(c, orientation, for_size));
            let (min, nat) = match orientation {
                gtk::Orientation::Horizontal => {
                    sizes.fold((0, 0), |(a, b), (m, n)| (a.max(m), b.max(n)))
                }
                _ => sizes.fold((0, 0), |(a, b), (m, n)| (a + m, b + n)),
            };
            (min, nat, -1, -1)
        }

        fn size_allocate(&self, width: i32, height: i32, _baseline: i32) {
            let children = self.children.borrow();
            let [image, alt, bar] = &children[..] else {
                return;
            };
            let v = gtk::Orientation::Vertical;
            let bar_h = size(bar, v, width).0;
            let (alt_min, alt_nat) = size(alt, v, width);
            let (image_h, alt_h) = split(height, bar_h, alt_min, alt_nat);
            let image_h = image_h.max(size(image, v, width).0);
            image.size_allocate(&gtk::Allocation::new(0, 0, width, image_h), -1);
            if alt.is_visible() {
                alt.size_allocate(&gtk::Allocation::new(0, image_h, width, alt_h), -1);
            }
            let y = image_h + if alt.is_visible() { alt_h } else { 0 };
            bar.size_allocate(&gtk::Allocation::new(0, y, width, bar_h), -1);
        }
    }
}

glib::wrapper! {
    pub struct ViewerBox(ObjectSubclass<imp::ViewerBox>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl ViewerBox {
    pub fn new(
        image: &impl IsA<gtk::Widget>,
        alt: &impl IsA<gtk::Widget>,
        bar: &impl IsA<gtk::Widget>,
    ) -> ViewerBox {
        let b: ViewerBox = glib::Object::new();
        let children: Vec<gtk::Widget> = vec![
            image.clone().upcast(),
            alt.clone().upcast(),
            bar.clone().upcast(),
        ];
        for c in &children {
            c.set_parent(&b);
        }
        b.imp().children.replace(children);
        b
    }
}

#[cfg(test)]
mod tests {
    use super::split;

    #[test]
    fn alt_text_takes_what_it_needs_up_to_half() {
        // Short alt text: its natural height, the rest to the image.
        assert_eq!(split(800, 50, 20, 60), (690, 60));
        // Long alt text: half the box at most; it scrolls.
        assert_eq!(split(800, 50, 20, 3000), (350, 400));
        // No alt text.
        assert_eq!(split(800, 50, 0, 0), (750, 0));
        // Never below the scroller's minimum, and the image never negative.
        assert_eq!(split(40, 50, 30, 3000), (0, 30));
    }
}
