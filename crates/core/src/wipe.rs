//! The theme switch on the Touch Bar: the new theme opens out of the middle
//! of the strip in a slanted band while the old one stays outside it. It
//! repeats the wallpaper reveal Omarchy's shell plays on the desktop for the
//! same switch (`plugins/background/Background.qml`: the band's slant, 840 ms,
//! ease-out cubic), so the two read as one motion. A thin line in the new
//! accent rides each edge, since on a black strip only the buttons would
//! otherwise show where the band has reached.

use std::{cell::RefCell, collections::HashMap, time::Instant};

use crate::{
    canvas::Canvas,
    color::Rgba,
    node::{Key, Node},
    theme::Theme,
};

/// The desktop reveal's length (`revealAnimation.duration`).
pub const DURATION: f32 = 0.84;
/// Horizontal lean per unit of height (`revealMask.slant`).
const SLANT: f32 = -0.18;
/// How far the edge line's glow trails into the new theme, in pixels.
const GLOW: f32 = 26.0;

pub(crate) struct ThemeWipe {
    /// The outgoing theme, drawn outside the band.
    pub from: Theme,
    /// The current nodes built with `from`, by key.
    pub nodes: HashMap<Key, Node>,
    /// Set on the first update, so the reveal runs on the scene's clock.
    started: Option<Instant>,
    /// Eased progress, 0–1.
    progress: f32,
    /// The incoming frame, composited into the band.
    scratch: RefCell<Option<Canvas>>,
}

fn ease_out_cubic(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

impl ThemeWipe {
    pub fn new(from: Theme) -> ThemeWipe {
        ThemeWipe {
            from,
            nodes: HashMap::new(),
            started: None,
            progress: 0.0,
            scratch: RefCell::new(None),
        }
    }

    /// Advances the reveal. Returns `false` once it has finished.
    pub fn step(&mut self, now: Instant) -> bool {
        let started = *self.started.get_or_insert(now);
        let t = now.saturating_duration_since(started).as_secs_f32() / DURATION;
        self.progress = ease_out_cubic(t);
        t < 1.0
    }

    /// The band's left and right edges at height `y`.
    fn edges(&self, w: f32, h: f32, y: f32) -> (f32, f32) {
        let top = w / 2.0 - SLANT * h / 2.0;
        let bottom = w / 2.0 + SLANT * h / 2.0;
        let reach = w / 2.0 + SLANT.abs() * h / 2.0 + 4.0;
        let spread = reach * self.progress;
        let center = top + (bottom - top) * (y / h);
        (center - spread, center + spread)
    }

    /// Renders the incoming frame with `incoming` and copies it into
    /// `canvas` (showing the outgoing theme) inside the band.
    pub fn composite(&self, canvas: &mut Canvas, incoming: &mut dyn FnMut(&mut Canvas)) {
        let (w, h) = (canvas.width(), canvas.height());
        let mut slot = self.scratch.borrow_mut();
        let scratch = match slot.as_mut() {
            Some(c) if c.width() == w && c.height() == h => c,
            _ => slot.insert(Canvas::new(w, h)),
        };
        incoming(scratch);
        for y in 0..h {
            let (l, r) = self.edges(w as f32, h as f32, y as f32 + 0.5);
            canvas.copy_span(scratch, y, l, r);
        }
    }

    /// Strokes the band's edges in `accent`, fading as the reveal settles.
    pub fn draw_edges(&self, canvas: &mut Canvas, accent: Rgba) {
        let fade = (1.0 - self.progress).max(0.0).sqrt();
        if fade < 0.01 {
            return;
        }
        let (w, h) = (canvas.width() as f32, canvas.height());
        // `d` is the distance into the new theme: a crisp line on the edge
        // and a short glow behind it.
        let alpha = |d: f32| {
            let line = (1.0 - (d - 1.0).abs() / 1.5).clamp(0.0, 1.0) * 0.9;
            let trail = if (0.0..GLOW).contains(&d) {
                0.22 * (1.0 - d / GLOW).powi(2)
            } else {
                0.0
            };
            line.max(trail) * fade
        };
        for y in 0..h {
            let (l, r) = self.edges(w, h as f32, y as f32 + 0.5);
            // The pixels near either edge; one run while the band is narrow.
            let (x0, x1) = ((l - 1.0).floor() as i32, (r + 1.0).ceil() as i32);
            let (left_end, right_start) = ((l + GLOW).ceil() as i32, (r - GLOW).floor() as i32);
            let runs = if left_end >= right_start {
                [(x0, x1), (0, 0)]
            } else {
                [(x0, left_end), (right_start, x1)]
            };
            for (start, end) in runs {
                for px in start.max(0)..end.min(w as i32) {
                    let cx = px as f32 + 0.5;
                    let a = alpha(cx - l).max(alpha(r - cx));
                    canvas.blend_rect(px as f32, y as f32, 1.0, 1.0, accent.alpha(a));
                }
            }
        }
    }
}
