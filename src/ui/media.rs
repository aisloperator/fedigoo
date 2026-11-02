//! Media attachment thumbnails and the full-size image viewer.

use super::app::App;
use super::fit::FitLayout;
use crate::api;
use crate::model::Media;
use gtk::{glib, prelude::*};
use std::cell::Cell;
use std::rc::Rc;

pub fn grid(app: &Rc<App>, media: &[Media], sensitive: bool) -> gtk::Widget {
    let grid = FitLayout::new();
    grid.add_css_class("media-grid");
    for (i, m) in media.iter().enumerate() {
        // Whole image, scaled to fit: the FitLayout sizes each button to its
        // image's aspect ratio, so nothing is cropped.
        let pic = gtk::Picture::new();
        pic.set_content_fit(gtk::ContentFit::Contain);
        pic.set_can_shrink(true);
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&pic));
        overlay.set_overflow(gtk::Overflow::Hidden);
        overlay.add_css_class("media-thumb");
        if !m.is_image() {
            let badge = gtk::Label::new(Some(match m.kind.as_str() {
                "gifv" => "GIF",
                "video" => "▶ Video",
                "audio" => "♪ Audio",
                _ => "File",
            }));
            badge.add_css_class("media-badge");
            badge.set_halign(gtk::Align::Start);
            badge.set_valign(gtk::Align::End);
            badge.set_margin_start(6);
            badge.set_margin_bottom(6);
            overlay.add_overlay(&badge);
        }
        if m.description.is_some() {
            let alt = gtk::Label::new(Some("ALT"));
            alt.add_css_class("media-badge");
            alt.set_halign(gtk::Align::End);
            alt.set_valign(gtk::Align::End);
            alt.set_margin_end(6);
            alt.set_margin_bottom(6);
            overlay.add_overlay(&alt);
        }
        let btn = gtk::Button::builder().child(&overlay).build();
        btn.add_css_class("flat");
        btn.add_css_class("media-button");
        // One click opens the viewer: show the same hand as over a link.
        btn.set_cursor_from_name(Some("pointer"));
        btn.set_tooltip_text(Some(m.description.as_deref().unwrap_or("No description")));
        let a = app.clone();
        let all = media.to_vec();
        btn.connect_clicked(move |_| open(&a, &all, i));
        grid.append(&btn, m.aspect);
        let (pic_w, grid_w, btn_w) = (pic.downgrade(), grid.downgrade(), btn.downgrade());
        api::with_texture(&m.preview_url, move |t| {
            if let (Some(r), Some(b)) = (grid_w.upgrade(), btn_w.upgrade()) {
                r.set_aspect_if_unknown(&b, t.width() as f64 / t.height().max(1) as f64);
            }
            if let Some(p) = pic_w.upgrade() {
                p.set_paintable(Some(t));
            }
        });
    }

    if !sensitive || app.config.borrow().settings.show_sensitive_media {
        return grid.upcast();
    }
    // Hide sensitive media behind a click-through cover.
    let stack = gtk::Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    let cover = gtk::Button::with_label("Sensitive content — click to show");
    cover.add_css_class("sensitive-cover");
    cover.set_size_request(-1, 120);
    stack.add_child(&cover);
    stack.add_child(&grid);
    let hide = gtk::Button::from_icon_name("view-conceal-symbolic");
    hide.set_tooltip_text(Some("Hide media"));
    hide.add_css_class("osd");
    hide.set_halign(gtk::Align::End);
    hide.set_valign(gtk::Align::Start);
    hide.set_margin_top(6);
    hide.set_margin_end(6);
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&stack));
    overlay.add_overlay(&hide);
    hide.set_visible(false);
    cover.connect_clicked(glib::clone!(
        #[weak]
        stack,
        #[weak]
        grid,
        #[weak]
        hide,
        move |_| {
            stack.set_visible_child(&grid);
            hide.set_visible(true);
        }
    ));
    hide.connect_clicked(glib::clone!(
        #[weak]
        stack,
        #[weak]
        cover,
        move |h| {
            stack.set_visible_child(&cover);
            h.set_visible(false);
        }
    ));
    overlay.upcast()
}

/// Opens attachment `index` of `media`: images in the built-in viewer,
/// everything else in the default external application.
pub fn open(app: &Rc<App>, media: &[Media], index: usize) {
    let Some(m) = media.get(index) else { return };
    if !m.is_image() {
        app.open_url(&m.url);
        return;
    }
    let images: Vec<Media> = media.iter().filter(|m| m.is_image()).cloned().collect();
    let start = images.iter().position(|x| x.id == m.id).unwrap_or(0);
    Viewer::open(app, images, start);
}

/// Share of the image's width, from its left edge, where a click goes back.
const BACK_ZONE: f64 = 0.2;
/// Horizontal drag distance (px) that flips to the next or previous image.
const SWIPE_MIN: f64 = 60.0;

/// The full-size image viewer window.  Two pictures take turns in a stack,
/// so flipping slides the new image in like a phone's photo gallery.
struct Viewer {
    win: glib::WeakRef<gtk::Window>,
    images: Vec<Media>,
    idx: Cell<usize>,
    stage: gtk::Stack,
    pics: [gtk::Picture; 2],
    /// Which of `pics` is showing.
    front: Cell<usize>,
    /// Bumped on every flip, so late downloads for an earlier image are dropped.
    generation: Rc<Cell<u32>>,
    alt: gtk::Label,
    counter: gtk::Label,
    prev: gtk::Button,
    next: gtk::Button,
}

impl Viewer {
    fn open(app: &Rc<App>, images: Vec<Media>, start: usize) {
        let win = gtk::Window::builder()
            .title("Image")
            .transient_for(&app.window)
            .default_width(1000)
            .default_height(760)
            .build();
        let pic = || {
            let p = gtk::Picture::new();
            p.set_content_fit(gtk::ContentFit::Contain);
            p.set_can_shrink(true);
            p
        };
        let pics = [pic(), pic()];
        let stage = gtk::Stack::new();
        stage.set_vexpand(true);
        stage.set_transition_duration(180);
        stage.add_css_class("viewer");
        // The image area takes keyboard focus (not the alt text), so the
        // arrow keys flip images as soon as the window opens.
        stage.set_focusable(true);
        stage.add_child(&pics[0]);
        stage.add_child(&pics[1]);

        let alt = gtk::Label::new(None);
        alt.set_wrap(true);
        alt.set_selectable(true);
        alt.set_margin_top(6);
        alt.set_margin_bottom(6);
        alt.set_margin_start(12);
        alt.set_margin_end(12);
        let prev = gtk::Button::from_icon_name("go-previous-symbolic");
        prev.set_tooltip_text(Some("Previous image (Left, Backspace)"));
        let next = gtk::Button::from_icon_name("go-next-symbolic");
        next.set_tooltip_text(Some("Next image (Right, Space)"));
        let counter = gtk::Label::new(None);
        let close = gtk::Button::with_label("Close");
        close.set_tooltip_text(Some("Close (Esc, Ctrl+W)"));
        close.set_margin_start(6);
        let browser = gtk::Button::with_label("Open in browser");
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        bar.set_margin_start(8);
        bar.set_margin_end(8);
        bar.set_margin_bottom(8);
        bar.append(&prev);
        bar.append(&counter);
        bar.append(&next);
        bar.append(&close);
        let sp = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        sp.set_hexpand(true);
        bar.append(&sp);
        bar.append(&browser);
        let vb = gtk::Box::new(gtk::Orientation::Vertical, 0);
        vb.append(&stage);
        vb.append(&alt);
        vb.append(&bar);
        win.set_child(Some(&vb));

        let v = Rc::new(Viewer {
            win: win.downgrade(),
            images,
            idx: Cell::new(start),
            stage: stage.clone(),
            pics,
            front: Cell::new(0),
            generation: Rc::default(),
            alt: alt.clone(),
            counter,
            prev: prev.clone(),
            next: next.clone(),
        });
        v.show(0);

        // The window holds the viewer; the handlers only refer to it weakly.
        let weak = Rc::downgrade(&v);
        let with = move |f: &dyn Fn(&Viewer)| {
            if let Some(v) = weak.upgrade() {
                f(&v);
            }
        };
        let w = with.clone();
        prev.connect_clicked(move |_| w(&|v| v.go(-1)));
        let w = with.clone();
        next.connect_clicked(move |_| w(&|v| v.go(1)));
        close.connect_clicked(glib::clone!(
            #[weak]
            win,
            move |_| win.close()
        ));
        let a = app.clone();
        let w = with.clone();
        browser.connect_clicked(move |_| {
            w(&|v| {
                let m = &v.images[v.idx.get()];
                a.open_url(m.remote_url.as_deref().unwrap_or(&m.url));
            })
        });

        // Keys are caught before the focused widget sees them, so they work
        // wherever focus is; a focused button keeps Space, and the alt text
        // keeps the keys it uses for selecting text.
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let w = with.clone();
        keys.connect_key_pressed(glib::clone!(
            #[weak]
            win,
            #[weak]
            alt,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, mods| {
                use gtk::gdk::{Key, ModifierType};
                let focus = gtk::prelude::GtkWindowExt::focus(&win);
                let in_alt = focus
                    .as_ref()
                    .is_some_and(|f| f == alt.upcast_ref::<gtk::Widget>());
                let on_button = focus.as_ref().is_some_and(|f| f.is::<gtk::Button>());
                let ctrl = mods.contains(ModifierType::CONTROL_MASK);
                match key {
                    Key::Escape => win.close(),
                    Key::w | Key::W if ctrl => win.close(),
                    Key::Left | Key::KP_Left if !in_alt => w(&|v| v.go(-1)),
                    Key::BackSpace if !in_alt => w(&|v| v.go(-1)),
                    Key::Right | Key::KP_Right if !in_alt => w(&|v| v.go(1)),
                    Key::space if !in_alt && !on_button => w(&|v| v.go(1)),
                    _ => return glib::Propagation::Proceed,
                }
                glib::Propagation::Stop
            }
        ));
        win.add_controller(keys);

        // A click on the left part of the image goes back, anywhere else
        // forward.  (A drag cancels the click, so the two don't mix.)
        let click = gtk::GestureClick::new();
        let w = with.clone();
        click.connect_released(move |g, n_press, x, _| {
            if n_press != 1 {
                return;
            }
            w(&|v| {
                v.stage.grab_focus();
                v.go(if v.in_back_zone(x) { -1 } else { 1 });
            });
            g.set_state(gtk::EventSequenceState::Claimed);
        });
        stage.add_controller(click);

        // Swiping, with a mouse or a finger: drag left for the next image,
        // right for the previous one.
        let drag = gtk::GestureDrag::new();
        let w = with.clone();
        drag.connect_drag_end(move |_, dx, dy| {
            if dx.abs() >= SWIPE_MIN && dx.abs() > dy.abs() {
                w(&|v| v.go(if dx < 0.0 { 1 } else { -1 }));
            }
        });
        stage.add_controller(drag);

        // The window keeps the viewer alive (the viewer only holds it weakly).
        win.connect_close_request(move |_| {
            let _keep = &v;
            glib::Propagation::Proceed
        });
        win.present();
        stage.grab_focus();
    }

    /// Flips by `step` images if there is one that way.
    fn go(&self, step: i32) {
        let i = self.idx.get() as i64 + step as i64;
        if i < 0 || i >= self.images.len() as i64 {
            return;
        }
        self.idx.set(i as usize);
        self.show(step);
    }

    /// Shows the current image, sliding it in from the side `step` points to.
    fn show(&self, step: i32) {
        let i = self.idx.get();
        let m = &self.images[i];
        let target = if step == 0 {
            self.front.get()
        } else {
            1 - self.front.get()
        };
        let pic = self.pics[target].clone();
        pic.set_paintable(None::<&gtk::gdk::Paintable>);
        pic.set_alternative_text(m.description.as_deref());
        // Show the cached preview first, then the full image; drop either if
        // the user has flipped on, and never let the preview replace the
        // full image.
        let generation = self.generation.get().wrapping_add(1);
        self.generation.set(generation);
        let full_shown = Rc::new(Cell::new(false));
        for (url, full) in [(&m.preview_url, false), (&m.url, true)] {
            let (pic, full_shown) = (pic.downgrade(), full_shown.clone());
            let current = self.generation.clone();
            api::with_texture(url, move |t| {
                if current.get() != generation || (!full && full_shown.get()) {
                    return;
                }
                if let Some(p) = pic.upgrade() {
                    p.set_paintable(Some(t));
                    full_shown.set(full_shown.get() || full);
                }
            });
        }
        self.stage.set_transition_type(match step {
            s if s > 0 => gtk::StackTransitionType::SlideLeft,
            s if s < 0 => gtk::StackTransitionType::SlideRight,
            _ => gtk::StackTransitionType::None,
        });
        self.stage.set_visible_child(&pic);
        self.front.set(target);
        self.alt.set_text(m.description.as_deref().unwrap_or(""));
        self.alt.set_visible(m.description.is_some());
        let n = self.images.len();
        self.counter.set_text(&format!("{} / {}", i + 1, n));
        self.prev.set_sensitive(i > 0);
        self.next.set_sensitive(i + 1 < n);
        if let Some(win) = self.win.upgrade() {
            win.set_title(Some(&format!("Image {} of {}", i + 1, n)));
        }
    }

    /// Whether `x` (in the stage's coordinates) is in the left part of the
    /// image as drawn, which is letterboxed inside the stage.
    fn in_back_zone(&self, x: f64) -> bool {
        let w = self.stage.width() as f64;
        let h = self.stage.height() as f64;
        let aspect = self.pics[self.front.get()]
            .paintable()
            .map(|p| p.intrinsic_aspect_ratio())
            .filter(|a| *a > 0.0)
            .unwrap_or(w / h.max(1.0));
        let img_w = w.min(h * aspect);
        let left = (w - img_w) / 2.0;
        x < left + img_w * BACK_ZONE
    }
}
