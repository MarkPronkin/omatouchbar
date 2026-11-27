//! Entrance motion for any widget.
//!
//! [`Motion`] wraps one child and draws it partway between hidden and shown:
//! risen a few pixels, scaled down, blurred and faded. The child keeps its
//! full size the whole time, so revealing things never reflows the layout.
//! Animations run on libadwaita's timed and spring animations, which follow
//! the desktop's "reduce motion" setting by skipping straight to the end.

use std::cell::{Cell, RefCell};

use adw::prelude::*;
use gtk::{glib, graphene, subclass::prelude::*};

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Motion {
        pub child: RefCell<Option<gtk::Widget>>,
        /// 0 is hidden, 1 is in place; springs may briefly overshoot.
        pub progress: Cell<f64>,
        pub rise: Cell<f64>,
        pub scale_from: Cell<f64>,
        pub blur: Cell<f64>,
        pub animation: RefCell<Option<adw::Animation>>,
        pub pending: RefCell<Option<glib::SourceId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Motion {
        const NAME: &'static str = "OmtMotion";
        type Type = super::Motion;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk::BinLayout>();
        }
    }

    impl ObjectImpl for Motion {
        fn constructed(&self) {
            self.parent_constructed();
            self.progress.set(1.0);
            self.rise.set(14.0);
            self.scale_from.set(1.0);
        }

        fn dispose(&self) {
            if let Some(source) = self.pending.take() {
                source.remove();
            }
            if let Some(child) = self.child.take() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for Motion {
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let Some(child) = self.child.borrow().clone() else {
                return;
            };
            let p = self.progress.get();
            if p <= 0.002 {
                return;
            }
            let obj = self.obj();
            if (p - 1.0).abs() < 0.0005 {
                obj.snapshot_child(&child, snapshot);
                return;
            }
            let (w, h) = (obj.width() as f32, obj.height() as f32);
            let shown = p.clamp(0.0, 1.0);
            let blur = self.blur.get() * (1.0 - shown);
            let from = self.scale_from.get();
            let scale = (from + (1.0 - from) * p) as f32;
            let dy = (self.rise.get() * (1.0 - p)) as f32;

            snapshot.save();
            snapshot.push_opacity(shown);
            let blurred = blur > 0.05;
            if blurred {
                snapshot.push_blur(blur);
            }
            snapshot.translate(&graphene::Point::new(w / 2.0, h / 2.0 + dy));
            snapshot.scale(scale, scale);
            snapshot.translate(&graphene::Point::new(-w / 2.0, -h / 2.0));
            obj.snapshot_child(&child, snapshot);
            if blurred {
                snapshot.pop();
            }
            snapshot.pop();
            snapshot.restore();
        }
    }
}

glib::wrapper! {
    pub struct Motion(ObjectSubclass<imp::Motion>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

/// How a [`Motion`] travels.
#[derive(Clone, Copy)]
pub enum Curve {
    /// A strong ease-out over this many milliseconds, Omarchy's default feel.
    Ease(u32),
    /// A spring that overshoots a little: (damping ratio, stiffness).
    Spring(f64, f64),
}

impl Curve {
    pub const SOFT: Curve = Curve::Ease(420);
    pub const POP: Curve = Curve::Spring(0.62, 260.0);
}

impl Motion {
    pub fn new(child: &impl IsA<gtk::Widget>) -> Motion {
        let motion: Motion = glib::Object::new();
        child.set_parent(&motion);
        motion.imp().child.replace(Some(child.clone().upcast()));
        motion
    }

    /// Pixels the child rises through while appearing.
    pub fn rise(self, px: f64) -> Self {
        self.imp().rise.set(px);
        self
    }

    /// The scale the child grows from.
    pub fn scale_from(self, scale: f64) -> Self {
        self.imp().scale_from.set(scale);
        self
    }

    /// The blur radius the child sharpens from.
    pub fn blur(self, radius: f64) -> Self {
        self.imp().blur.set(radius);
        self
    }

    pub fn progress(&self) -> f64 {
        self.imp().progress.get()
    }

    fn set_progress(&self, value: f64) {
        self.imp().progress.set(value);
        self.queue_draw();
    }

    fn cancel(&self) {
        if let Some(source) = self.imp().pending.take() {
            source.remove();
        }
        if let Some(animation) = self.imp().animation.take() {
            animation.pause();
        }
    }

    /// Hides the child at once, ready for [`Motion::reveal`].
    pub fn hide(&self) {
        self.cancel();
        self.set_progress(0.0);
    }

    /// Shows the child at once.
    pub fn show_now(&self) {
        self.cancel();
        self.set_progress(1.0);
    }

    /// Hides, then animates in after `delay_ms`.
    pub fn reveal(&self, delay_ms: u32, curve: Curve) {
        self.hide();
        self.animate_to(1.0, delay_ms, curve);
    }

    /// Animates from wherever the child is now to `to`.
    pub fn animate_to(&self, to: f64, delay_ms: u32, curve: Curve) {
        self.cancel();
        let weak = self.downgrade();
        let start = move || {
            let Some(this) = weak.upgrade() else {
                return;
            };
            this.imp().pending.take();
            let target = {
                let weak = this.downgrade();
                adw::CallbackAnimationTarget::new(move |value| {
                    if let Some(this) = weak.upgrade() {
                        this.set_progress(value);
                    }
                })
            };
            let from = this.progress();
            let animation: adw::Animation = match curve {
                Curve::Ease(ms) => {
                    let a = adw::TimedAnimation::new(&this, from, to, ms, target);
                    a.set_easing(adw::Easing::EaseOutExpo);
                    a.upcast()
                }
                Curve::Spring(damping, stiffness) => {
                    let a = adw::SpringAnimation::new(
                        &this,
                        from,
                        to,
                        adw::SpringParams::new(damping, 1.0, stiffness),
                        target,
                    );
                    a.set_epsilon(0.0008);
                    a.upcast()
                }
            };
            this.imp().animation.replace(Some(animation.clone()));
            animation.play();
        };
        if delay_ms == 0 {
            start();
        } else {
            let source = glib::timeout_add_local_once(
                std::time::Duration::from_millis(delay_ms as u64),
                start,
            );
            self.imp().pending.replace(Some(source));
        }
    }
}

/// Reveals each widget in turn, `step_ms` apart, starting after `delay_ms`.
pub fn stagger(items: &[Motion], delay_ms: u32, step_ms: u32, curve: Curve) {
    for (i, item) in items.iter().enumerate() {
        item.reveal(delay_ms + i as u32 * step_ms, curve);
    }
}

/// Ease-out curves for hand-drawn animations.
pub fn ease_out_cubic(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

pub fn ease_in_out(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}
