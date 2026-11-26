//! The Touch ID overlay: a full-strip effect in the spirit of Omarchy's
//! screensaver (terminal text effects), sized for a 60 px strip.
//!
//! - Everything is anchored on the real Touch ID sensor, the power key just
//!   past the right end of the strip: the print sits next to it behind
//!   chevrons pointing at it, and everything moves outward from it (pings
//!   while authenticating, shockwaves on each enrollment step, burns and
//!   floods). Nothing travels back toward it, and while enrolling nothing
//!   pulses except progress.
//! - A field of scrambling glyphs boots out of the sensor and fills the
//!   strip. While enrolling, the whole strip is the progress bar: it fills
//!   from the sensor end with a glowing leading edge, rails and settled
//!   glyphs, and each step's slice flashes as it lands.
//! - The fingerprint is a dot matrix sampled from the icon font. Its dots
//!   swarm in from the field, a scanner beam sweeps them, they lock in as
//!   enrollment progresses, glitch and burn red on a miss, and explode
//!   along the strip on success.
//! - Status text decrypts itself character by character.
//!
//! Every frame is a pure function of the prompt's event times, so the
//! effect keeps no per-frame simulation state and works at any frame rate.

use std::{
    f32::consts::{PI, TAU},
    rc::Rc,
    time::{Duration, Instant},
};

use tiny_skia::PathBuilder;

use crate::{
    build::glyph,
    canvas::{Canvas, ClipRect, VAlign},
    color::Rgba,
    scene::Scene,
    state::{TouchId, TouchIdStage},
    text::{Fonts, Role, Run},
    theme::Theme,
};

/// Glyphs the field and the decrypting text scramble through.
const GLYPHS: &[&str] = &[
    "0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "A", "B", "C", "D", "E", "F", "X", "Z", "#",
    "%", "&", "@", "$", "?", "!", "+", "=", "*", "<", ">", "/", "\\", "|", "~", "^", ":", "░", "▒",
    "▓", "▀", "▄", "█",
];
/// Index of the full block in [`GLYPHS`] (the success wave front).
const BLOCK: usize = GLYPHS.len() - 1;
/// Shade blocks (░▒▓) for the burn front of a miss.
const BURN: [usize; 3] = [GLYPHS.len() - 6, GLYPHS.len() - 5, GLYPHS.len() - 4];

/// How long a success plays before it may fade, measured from the moment
/// it began (T1Bridge often clears the prompt sooner).
const SUCCESS_HOLD: f32 = 1.9;
/// Fade-out once the prompt is gone.
const FADE: f32 = 0.35;
/// Field glyph size and row pitch.
const FIELD_SIZE: f32 = 11.0;
const FIELD_ROW: f32 = 11.0;
/// Fingerprint dot pitch (dots are one pixel smaller).
const PITCH: f32 = 3.0;

// ------------------------------------------------------------------ state

/// The prompt's timeline, as the overlay sees it.
pub(crate) struct TouchFx {
    start: Instant,
    stage: TouchIdStage,
    prev: Option<TouchIdStage>,
    stage_at: Instant,
    /// The prompt enrolled a finger at some point (success says "added").
    enrolling: bool,
    progress: f32,
    progress_from: f32,
    progress_at: Instant,
    /// Recent progress steps: (when, seed).
    bumps: Vec<(Instant, u32)>,
    ended: Option<Instant>,
    seen: Option<TouchId>,
    seed: u32,
    art: Rc<Art>,
}

impl TouchFx {
    pub fn new(tid: &TouchId, now: Instant, fonts: &Fonts, height: f32) -> TouchFx {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0x5eed);
        let progress = tid.progress.map(percent).unwrap_or(0.0);
        TouchFx {
            start: now,
            stage: tid.stage,
            prev: None,
            stage_at: now,
            enrolling: tid.stage == TouchIdStage::Enrollment,
            progress,
            progress_from: progress,
            progress_at: now,
            bumps: Vec::new(),
            ended: None,
            seen: Some(tid.clone()),
            seed,
            art: Rc::new(Art::build(fonts, height)),
        }
    }

    /// Follows the prompt. Returns whether anything changed.
    pub fn sync(&mut self, tid: Option<&TouchId>, now: Instant) -> bool {
        if self.seen.as_ref() == tid {
            return false;
        }
        self.seen = tid.cloned();
        let Some(tid) = tid else {
            self.ended = Some(now);
            return true;
        };
        self.ended = None;
        if tid.stage != self.stage {
            self.prev = Some(self.stage);
            self.stage = tid.stage;
            self.stage_at = now;
            self.enrolling |= tid.stage == TouchIdStage::Enrollment;
        }
        if let Some(p) = tid.progress.map(percent)
            && (p - self.progress).abs() > 0.001
        {
            self.progress_from = self.progress_shown(now);
            if p > self.progress {
                let n = self.bumps.last().map_or(1, |b| b.1.wrapping_add(1));
                self.bumps.push((now, n));
                if self.bumps.len() > 4 {
                    self.bumps.remove(0);
                }
            }
            self.progress = p;
            self.progress_at = now;
        }
        true
    }

    /// Enrollment progress as drawn (counts up after each step).
    fn progress_shown(&self, now: Instant) -> f32 {
        let k = ease_out_cubic(secs(now, self.progress_at) / 0.6);
        self.progress_from + (self.progress - self.progress_from) * k
    }

    /// Progress as drawn this frame (no count-up with motion off).
    fn shown(&self, clock: &Clock) -> f32 {
        if clock.live {
            self.progress_shown(clock.now)
        } else {
            self.progress
        }
    }

    /// Overall opacity: 1 while the prompt is up, then fading out (a
    /// success first plays out in full).
    pub fn fade(&self, now: Instant) -> f32 {
        let Some(ended) = self.ended else {
            return 1.0;
        };
        let from = if self.stage == TouchIdStage::Success {
            ended.max(self.stage_at + Duration::from_secs_f32(SUCCESS_HOLD))
        } else {
            ended
        };
        if now < from {
            return 1.0;
        }
        (1.0 - secs(now, from) / FADE).max(0.0)
    }

    pub fn done(&self, now: Instant) -> bool {
        self.ended.is_some() && self.fade(now) <= 0.0
    }

    /// Whether the Cancel button belongs on screen.
    /// Whether frames change on their own (always, unless motion is off
    /// and the prompt is still up).
    pub fn animating(&self, motion: bool) -> bool {
        motion || self.ended.is_some()
    }

    pub fn cancellable(&self) -> bool {
        self.ended.is_none() && self.stage != TouchIdStage::Success
    }

    fn text(&self) -> &'static str {
        match self.stage {
            TouchIdStage::Authenticate => "Touch ID to authenticate",
            TouchIdStage::Approve => "Touch ID to approve",
            TouchIdStage::Retry => "Not recognized — try again",
            TouchIdStage::Success if self.enrolling => "Fingerprint added",
            TouchIdStage::Success => "Authenticated",
            TouchIdStage::Enrollment => "Lift and rest your finger on Touch ID",
        }
    }
}

fn percent(p: u8) -> f32 {
    p.min(100) as f32 / 100.0
}

// --------------------------------------------------------------- the print

/// The fingerprint as a dot matrix centered on (0, 0).
pub(crate) struct Art {
    dots: Vec<Dot>,
    w: f32,
    h: f32,
}

struct Dot {
    x: f32,
    y: f32,
    /// When it locks in while enrolling (0–1, uniform).
    order: f32,
    /// Distance from the center, 0 (core) to 1 (rim).
    radius: f32,
    row: i32,
}

impl Art {
    fn build(fonts: &Fonts, height: f32) -> Art {
        let target = (height - 6.0).max(16.0);
        let ink = |size: f32| {
            let run = fonts.layout(glyph::FINGERPRINT, size, Role::Icon);
            run.glyphs.first().and_then(|g| fonts.bitmap(g, size, 0))
        };
        let probe = ink(64.0).filter(|b| b.height > 8 && b.rgba.is_none());
        let Some(bitmap) = probe
            .and_then(|b| ink((64.0 * target / b.height as f32).round()))
            .filter(|b| b.height > 8 && b.rgba.is_none())
        else {
            return Art::procedural(target);
        };
        let (bw, bh) = (bitmap.width as usize, bitmap.height as usize);
        let p = PITCH as usize;
        let (cols, rows) = (bw.div_ceil(p), bh.div_ceil(p));
        let mut cells = Vec::new();
        for r in 0..rows {
            for c in 0..cols {
                let (mut sum, mut n) = (0u32, 0u32);
                for y in r * p..((r + 1) * p).min(bh) {
                    for x in c * p..((c + 1) * p).min(bw) {
                        sum += bitmap.coverage[y * bw + x] as u32;
                        n += 1;
                    }
                }
                if n > 0 && sum as f32 / n as f32 > 0.4 * 255.0 {
                    cells.push((c as i32, r as i32));
                }
            }
        }
        Art::from_cells(cells, cols as i32, rows as i32)
    }

    /// Concentric ridges, for systems without the icon font.
    fn procedural(target: f32) -> Art {
        let rows = (target / PITCH) as i32;
        let cols = (rows as f32 * 0.82) as i32;
        let mut cells = Vec::new();
        for r in 0..rows {
            for c in 0..cols {
                let x = (c as f32 + 0.5) / cols as f32 * 2.0 - 1.0;
                let y = (r as f32 + 0.5) / rows as f32 * 2.0 - 1.0;
                let d = (x * x + y * y).sqrt();
                let opening = y > 0.35 && x.abs() < 0.25 + d * 0.2;
                if d < 1.0 && (d * 4.5).fract() < 0.5 && !opening {
                    cells.push((c, r));
                }
            }
        }
        Art::from_cells(cells, cols, rows)
    }

    fn from_cells(cells: Vec<(i32, i32)>, cols: i32, rows: i32) -> Art {
        let (w, h) = (cols as f32 * PITCH, rows as f32 * PITCH);
        let mut dots: Vec<Dot> = cells
            .into_iter()
            .map(|(c, r)| {
                let x = (c as f32 + 0.5) * PITCH - w / 2.0;
                let y = (r as f32 + 0.5) * PITCH - h / 2.0;
                let radius = ((x / (w / 2.0)).powi(2) + (y / (h / 2.0)).powi(2))
                    .sqrt()
                    .min(1.0);
                Dot {
                    x,
                    y,
                    order: 0.0,
                    radius,
                    row: r,
                }
            })
            .collect();
        // Lock in roughly from the core outward, in noisy patches.
        let mut keyed: Vec<(f32, usize)> = dots
            .iter()
            .enumerate()
            .map(|(i, d)| {
                let patch = unit(h2((d.x / 9.0) as i32 as u32, (d.y / 9.0) as i32 as u32));
                (d.radius * 0.5 + patch * 0.3 + unit(hash(i as u32)) * 0.2, i)
            })
            .collect();
        keyed.sort_by(|a, b| a.0.total_cmp(&b.0));
        let n = keyed.len().max(1) as f32;
        for (rank, (_, i)) in keyed.into_iter().enumerate() {
            dots[i].order = (rank as f32 + 0.5) / n;
        }
        Art { dots, w, h }
    }
}

// ------------------------------------------------------------------ colors

/// A stage's gradient (TTE-style): `a` near the print, through `b`, to `c`
/// at the far ends; `hot` for wave fronts and flashes.
#[derive(Clone, Copy)]
struct Pal {
    a: Rgba,
    b: Rgba,
    c: Rgba,
    hot: Rgba,
    /// The theme's text and faint colors.
    fg: Rgba,
    faint: Rgba,
}

impl Pal {
    fn of(stage: TouchIdStage, t: &Theme) -> Pal {
        let hot = t.fg.mix(Rgba::WHITE, 0.5);
        let (fg, faint) = (t.fg, t.fg_faint);
        match stage {
            TouchIdStage::Retry => Pal {
                a: t.red,
                b: t.orange,
                c: t.red.mix(t.magenta, 0.5),
                hot: t.red.mix(hot, 0.55),
                fg,
                faint,
            },
            TouchIdStage::Success => Pal {
                a: t.green,
                b: t.cyan,
                c: t.accent,
                hot,
                fg,
                faint,
            },
            _ => Pal {
                a: t.accent,
                b: t.cyan,
                c: t.magenta,
                hot,
                fg,
                faint,
            },
        }
    }

    fn mix(self, o: Pal, k: f32) -> Pal {
        Pal {
            a: self.a.mix(o.a, k),
            b: self.b.mix(o.b, k),
            c: self.c.mix(o.c, k),
            hot: self.hot.mix(o.hot, k),
            fg: self.fg.mix(o.fg, k),
            faint: self.faint.mix(o.faint, k),
        }
    }

    fn grad(&self, u: f32) -> Rgba {
        let u = u.clamp(0.0, 1.0);
        if u < 0.5 {
            self.a.mix(self.b, u * 2.0)
        } else {
            self.b.mix(self.c, u * 2.0 - 1.0)
        }
    }
}

// ----------------------------------------------------------------- drawing

/// Pixels from the panel's right edge to the middle of the Touch ID
/// sensor: the power key just past the right end of the Touch Bar.
const SENSOR_GAP: f32 = 90.0;
/// While waiting, the sensor sends a ping down the strip every
/// `PING_PERIOD` seconds at `PING_SPEED` px/s.
const PING_PERIOD: f32 = 2.0;
const PING_SPEED: f32 = 1150.0;
/// Room for the chevrons between the print and the sensor.
const POINTER_W: f32 = 46.0;

/// Where everything sits this frame. The hero (text, print, chevrons) is
/// anchored at the right end, next to the sensor, and the effects radiate
/// out of the sensor itself.
struct Hero {
    /// The print's center.
    cx: f32,
    cy: f32,
    /// Where the status text starts (it is right-aligned against the
    /// percentage or the print).
    text_x: f32,
    /// Right edge of the enrollment percentage.
    pct_right: f32,
    /// Where the chevrons start.
    pointer_x: f32,
    /// The sensor's center, off the right end of the strip.
    sx: f32,
    /// Left edge of the area the field keeps clear.
    x0: f32,
    /// Distance from the sensor to the far end of the strip.
    reach: f32,
    /// The enrollment progress bar, while enrolling.
    bar: Option<Bar>,
}

/// Enrollment progress as a bar the length of the strip, filling from the
/// sensor end (right) toward the far end.
#[derive(Clone, Copy)]
struct Bar {
    /// The leading edge: the strip right of it is filled.
    edge: f32,
    /// Where the latest step's slice ends, and how much it still flashes.
    step_end: f32,
    flash: f32,
    /// Overall strength (fades into the success flood).
    show: f32,
}

impl Hero {
    /// How far a ping travels before it is gone.
    fn span(&self) -> f32 {
        self.reach + 60.0
    }

    /// Ages of the pings in flight.
    fn pings(&self, t: f32) -> impl Iterator<Item = f32> {
        ping_ages(t, self.span() / PING_SPEED)
    }

    /// How brightly the sensor flashes as it sends a ping (`emit`), and the
    /// print as the ping passes it (`pass`).
    fn ping_flash(&self, t: f32) -> (f32, f32) {
        let at_print = (self.sx - self.cx) / PING_SPEED;
        self.pings(t).fold((0.0f32, 0.0f32), |(emit, pass), age| {
            (
                emit.max(burst(age)),
                pass.max(gauss((age - at_print) / 0.09)),
            )
        })
    }
}

impl TouchFx {
    /// The enrollment bar across `x0..x1`, if this prompt enrolls.
    fn bar(&self, clock: &Clock, x0: f32, x1: f32) -> Option<Bar> {
        if !self.enrolling {
            return None;
        }
        let (p, show) = match self.stage {
            TouchIdStage::Enrollment | TouchIdStage::Retry => (self.shown(clock), 1.0),
            TouchIdStage::Success => (1.0, 1.0 - smoothstep(0.1, 0.9, clock.since(self.stage_at))),
            _ => return None,
        };
        if show <= 0.0 {
            return None;
        }
        let at = |p: f32| x1 - p.clamp(0.0, 1.0) * (x1 - x0);
        let flash = match self.bumps.last() {
            Some(b) if clock.live => (1.0 - clock.since(b.0) / 0.7).max(0.0),
            _ => 0.0,
        };
        Some(Bar {
            edge: at(p),
            step_end: at(self.progress_from),
            flash,
            show,
        })
    }

    /// The sensor pings on a timer while waiting to authenticate. While
    /// enrolling it stays quiet between steps: every pulse is progress.
    fn pinging(&self, clock: &Clock) -> bool {
        self.waiting(clock) && !self.enrolling
    }

    /// Waiting for a finger.
    fn waiting(&self, clock: &Clock) -> bool {
        self.ended.is_none()
            && clock.live
            && matches!(
                self.stage,
                TouchIdStage::Authenticate
                    | TouchIdStage::Approve
                    | TouchIdStage::Enrollment
                    | TouchIdStage::Retry
            )
    }
}

/// Ages of the pings still travelling `life` seconds after leaving the
/// sensor (the first one leaves 0.6 s after the prompt opens).
fn ping_ages(t: f32, life: f32) -> impl Iterator<Item = f32> {
    let phase = t - 0.6;
    (0..2).filter_map(move |k| {
        let age = phase.rem_euclid(PING_PERIOD) + k as f32 * PING_PERIOD;
        (phase >= 0.0 && age <= phase && age <= life).then_some(age)
    })
}

impl Scene {
    /// Draws the Touch ID overlay into the node's rectangle.
    pub(crate) fn draw_touch_fx(
        &self,
        canvas: &mut Canvas,
        rect: (f32, f32, f32, f32),
        theme: &Theme,
        opacity: f32,
    ) {
        let Some(fx) = self.touch_fx.as_ref() else {
            return;
        };
        let (rx, ry, rw, rh) = rect;
        let alpha = opacity * fx.fade(self.now);
        if alpha < 0.004 || rw < 40.0 {
            return;
        }
        let live = self.config.motion.enabled;
        let clock = Clock {
            now: self.now,
            live,
        };
        let ts = clock.since(fx.stage_at);
        let pal = {
            let cur = Pal::of(fx.stage, theme);
            match fx.prev {
                Some(prev) if ts < 0.4 => Pal::of(prev, theme).mix(cur, ease_out_cubic(ts / 0.4)),
                _ => cur,
            }
        };

        // From the right: chevrons, the print, the percentage while
        // enrolling, then the text, right-aligned.
        let m = self.m();
        let art = &fx.art;
        // A miss shakes the hero, like a wrong password.
        let shake = if fx.stage == TouchIdStage::Retry && live && ts < 0.55 {
            (ts * 46.0).sin() * 9.0 * (1.0 - ts / 0.55).powi(2)
        } else {
            0.0
        };
        let right = rx + rw;
        let cx = right - 8.0 - POINTER_W - art.w / 2.0 + shake;
        let pct_right = cx - art.w / 2.0 - 22.0;
        let pct_w = if fx.stage == TouchIdStage::Enrollment {
            self.fonts.measure("100%", m.label, Role::Bold) + 18.0
        } else {
            0.0
        };
        let text_right = pct_right - pct_w;
        let text_x = text_right - self.fonts.measure(fx.text(), m.label, Role::Bold);
        let sx = self.width + SENSOR_GAP;
        let hero = Hero {
            cx,
            cy: ry + rh / 2.0,
            text_x,
            pct_right,
            pointer_x: cx + art.w / 2.0 + 10.0,
            sx,
            x0: text_x - 14.0,
            reach: (sx - rx).max(1.0),
            bar: fx.bar(&clock, rx + 2.0, right),
        };

        canvas.set_clip(Some(ClipRect { x0: rx, x1: right }));
        self.fx_bar_fill(canvas, &hero, &pal, rect, alpha);
        self.fx_field(canvas, fx, &hero, &pal, rect, &clock, alpha);
        self.fx_bar_edge(canvas, &hero, &pal, rect, alpha);
        self.fx_waves(canvas, fx, &hero, &pal, &clock, alpha);
        self.fx_print(canvas, fx, &hero, &pal, rect, &clock, alpha);
        self.fx_pointer(canvas, fx, &hero, &pal, &clock, alpha);
        self.fx_text(canvas, fx, &hero, &pal, &clock, alpha);
        canvas.set_clip(None);
    }

    /// The glyph field. Distances are measured from the sensor.
    #[allow(clippy::too_many_arguments)]
    fn fx_field(
        &self,
        canvas: &mut Canvas,
        fx: &TouchFx,
        hero: &Hero,
        pal: &Pal,
        rect: (f32, f32, f32, f32),
        clock: &Clock,
        alpha: f32,
    ) {
        let (rx, ry, rw, rh) = rect;
        let runs: Vec<Run> = GLYPHS
            .iter()
            .map(|g| self.fonts.layout(g, FIELD_SIZE, Role::Text))
            .collect();
        let adv = runs[0].width;
        let cw = (adv + 2.5).round().max(6.0);
        let rows = ((rh - 3.0) / FIELD_ROW).floor().max(1.0) as i32;
        let top = ry + (rh - rows as f32 * FIELD_ROW) / 2.0 + FIELD_ROW / 2.0;
        let cols = (rw / cw).floor() as i32;
        let x0 = rx + (rw - cols as f32 * cw) / 2.0;

        let t = clock.since(fx.start);
        let ts = clock.since(fx.stage_at);
        let pings: Vec<f32> = if fx.pinging(clock) {
            hero.pings(t).collect()
        } else {
            Vec::new()
        };
        let glitch = if fx.stage == TouchIdStage::Retry && clock.live {
            (1.0 - ts / 0.75).max(0.0)
        } else {
            0.0
        };
        let success = (fx.stage == TouchIdStage::Success && clock.live).then_some(ts);
        let bumps: Vec<f32> = fx
            .bumps
            .iter()
            .map(|b| clock.since(b.0))
            .filter(|age| *age < 1.6)
            .collect();
        let boot = if clock.live { t * 1700.0 } else { f32::MAX };

        for col in 0..cols {
            let x = x0 + (col as f32 + 0.5) * cw;
            let mask = smoothstep(2.0, 90.0, hero.x0 - x);
            if mask <= 0.0 {
                continue;
            }
            let dxs = (hero.sx - x).max(0.0);
            let tint = pal.grad(dxs / hero.reach);
            let hc = h2(fx.seed, col as u32);
            for row in 0..rows {
                let y = top + row as f32 * FIELD_ROW;
                let hr = h2(hc, row as u32);
                let dy = (y - hero.cy) * 5.0;
                let dist = (dxs * dxs + dy * dy).sqrt();
                if dist > boot {
                    continue;
                }
                // Sparse ambient glyphs that slowly come and go.
                let epoch = (t * (0.2 + 0.5 * unit(hr)) + unit(hash(hr))) as u32;
                let mut e = if unit(h2(hr, epoch)) < 0.38 {
                    0.05 + 0.08 * unit(h2(hr, epoch ^ 0xa5))
                } else {
                    0.0
                };
                let mut hot = 0.0f32;
                let mut block = false;
                let mut burn = false;
                // Boot front, out of the sensor.
                let g = gauss((dist - boot) / 40.0);
                e += 0.9 * g;
                hot += g;
                // Waiting: the sensor pings down the strip, fading as it goes.
                for &age in &pings {
                    let r = age * PING_SPEED;
                    let life = 1.0 - 0.65 * (r / hero.span()).min(1.0);
                    let g = gauss((dist - r) / 30.0) * life;
                    e += 0.95 * g;
                    hot += 0.55 * g;
                    // The wake behind the front keeps scrambling.
                    if dist < r {
                        e += 0.22 * (-(r - dist) / 80.0).exp() * life;
                    }
                }
                // Enrollment steps: hard shockwaves out of the sensor.
                for &age in &bumps {
                    let g = gauss((dist - age * 1300.0) / 52.0) * (1.0 - age / 1.6);
                    e += 1.1 * g;
                    hot += 0.9 * g;
                }
                // The enrolled share of the strip is dense with settled
                // glyphs; they scramble hot along the bar's leading edge.
                let mut settled = false;
                if let Some(bar) = hero.bar {
                    if x >= bar.edge {
                        e = e.max((0.15 + 0.12 * unit(hash(hr ^ 0x0f))) * bar.show);
                        settled = x - bar.edge > 50.0;
                    }
                    let g = gauss((x - bar.edge) / 22.0) * bar.show;
                    e += 0.7 * g;
                    hot += 0.6 * g;
                }
                // Success: a flood of blocks rolls down the strip.
                if let Some(age) = success {
                    let passed = age - dist / 1900.0;
                    if passed >= 0.0 {
                        e += 0.95 * (-passed * 2.4).exp();
                        hot += (-passed * 7.0).exp();
                        block = passed < 0.08;
                    }
                }
                // A miss: a red burn rolls out, with static and torn rows.
                let mut x = x;
                if fx.stage == TouchIdStage::Retry && clock.live {
                    let passed = ts - dist / 2600.0;
                    if passed >= 0.0 {
                        e += 0.85 * (-passed * 4.5).exp();
                        hot += 0.6 * (-passed * 12.0).exp();
                        if passed < 0.06 {
                            burn = true;
                        }
                    }
                }
                if glitch > 0.0 {
                    let slice = h3(fx.seed, row as u32, (ts * 22.0) as u32);
                    if unit(slice) < 0.55 {
                        x += (unit(hash(slice)) - 0.5) * 70.0 * glitch;
                    }
                    if unit(h3(hr, (ts * 30.0) as u32, 3)) < 0.3 * glitch {
                        e += 0.75;
                        hot += 0.4;
                    }
                }
                let e = (e * mask).min(1.0);
                if e < 0.03 {
                    continue;
                }
                let index = if block {
                    BLOCK
                } else if burn {
                    BURN[(hr % 3) as usize]
                } else {
                    // Energized glyphs scramble faster.
                    let rate = 1.5 + 5.0 * unit(hash(hr ^ 0x55)) + 34.0 * (e - 0.12).max(0.0);
                    let frame = if clock.live && !settled {
                        (t * rate) as u32
                    } else {
                        0
                    };
                    h2(hr, frame) as usize % GLYPHS.len()
                };
                let color = tint.mix(pal.hot, hot.min(1.0) * 0.85);
                canvas.draw_run(
                    &self.fonts,
                    &runs[index],
                    x - adv / 2.0,
                    y,
                    color.alpha(e * alpha),
                    VAlign::Cap,
                );
            }
        }
    }

    /// The enrollment bar's fill, under the glyph field: a tint from the
    /// leading edge to the sensor end, and the latest step flashing in.
    fn fx_bar_fill(
        &self,
        canvas: &mut Canvas,
        hero: &Hero,
        pal: &Pal,
        rect: (f32, f32, f32, f32),
        alpha: f32,
    ) {
        let Some(bar) = hero.bar else {
            return;
        };
        let (rx, ry, rw, rh) = rect;
        let right = rx + rw;
        let a = bar.show * alpha;
        let w = right - bar.edge;
        if w > 0.5 {
            let near = (40.0 / w).min(0.45);
            canvas.blend_hgradient(
                bar.edge,
                ry,
                w,
                rh,
                &[
                    (0.0, pal.hot.mix(pal.a, 0.4).alpha(0.34 * a)),
                    (
                        near,
                        pal.grad((hero.sx - bar.edge) / hero.reach).alpha(0.2 * a),
                    ),
                    (1.0, pal.a.alpha(0.13 * a)),
                ],
            );
        }
        // The slice the latest step added flashes as the edge sweeps it.
        let slice = bar.step_end - bar.edge;
        if bar.flash > 0.0 && slice > 0.5 {
            canvas.blend_hgradient(
                bar.edge,
                ry,
                slice,
                rh,
                &[
                    (0.0, pal.hot.alpha(0.45 * bar.flash * a)),
                    (1.0, pal.hot.alpha(0.2 * bar.flash * a)),
                ],
            );
        }
    }

    /// The enrollment bar's frame, over the glyph field: rails along the
    /// top and bottom, a dotted track ahead, and a glowing leading edge
    /// that surges on each step.
    fn fx_bar_edge(
        &self,
        canvas: &mut Canvas,
        hero: &Hero,
        pal: &Pal,
        rect: (f32, f32, f32, f32),
        alpha: f32,
    ) {
        let Some(bar) = hero.bar else {
            return;
        };
        let (rx, ry, rw, rh) = rect;
        let right = rx + rw;
        let a = bar.show * alpha;
        let w = right - bar.edge;
        for y in [ry + 1.0, ry + rh - 3.0] {
            if w > 0.5 {
                canvas.blend_hgradient(
                    bar.edge,
                    y,
                    w,
                    2.0,
                    &[
                        (0.0, pal.hot.alpha(0.95 * a)),
                        ((60.0 / w).min(0.5), pal.a.alpha(0.85 * a)),
                        (1.0, pal.a.alpha(0.6 * a)),
                    ],
                );
            }
            let mut x = rx + 3.0;
            while x < bar.edge - 5.0 {
                canvas.blend_rect(x, y, 2.0, 2.0, pal.faint.alpha(0.4 * a));
                x += 6.0;
            }
        }
        // Hidden at 0 %, where the edge sits on the end of the strip, and
        // softened while it passes behind the text and print.
        let behind = 0.35 + 0.65 * smoothstep(0.0, 40.0, hero.x0 - bar.edge);
        let edge = a * (w / 20.0).min(1.0) * behind;
        let surge = bar.flash;
        let glow = 18.0 + 46.0 * surge;
        canvas.blend_hgradient(
            bar.edge - glow * 0.4,
            ry,
            glow * 0.4,
            rh,
            &[
                (0.0, pal.hot.alpha(0.0)),
                (1.0, pal.hot.alpha((0.22 + 0.3 * surge) * edge)),
            ],
        );
        canvas.blend_hgradient(
            bar.edge,
            ry,
            glow,
            rh,
            &[
                (0.0, pal.hot.alpha((0.3 + 0.4 * surge) * edge)),
                (1.0, pal.hot.alpha(0.0)),
            ],
        );
        canvas.blend_rect(bar.edge - 1.0, ry, 2.0, rh, pal.hot.alpha(0.95 * edge));
    }

    /// Arcs centered on the sensor, and sparks off the print.
    fn fx_waves(
        &self,
        canvas: &mut Canvas,
        fx: &TouchFx,
        hero: &Hero,
        pal: &Pal,
        clock: &Clock,
        alpha: f32,
    ) {
        if !clock.live {
            return;
        }
        let t = clock.since(fx.start);
        let ts = clock.since(fx.stage_at);
        // Arcs and sparks skip the text and percentage so they stay
        // readable: draw in the strip left of them, then around the print.
        let full = canvas.clip();
        let (x0, x1) = full.map_or((0.0, canvas.width() as f32), |c| (c.x0, c.x1));
        let zones = [(x0, hero.x0 - 6.0), (hero.pct_right + 6.0, x1)];
        let ring = |canvas: &mut Canvas, r: f32, width: f32, color: Rgba| {
            if color.a <= 0.004 {
                return;
            }
            let Some(path) = PathBuilder::from_circle(hero.sx, hero.cy, r.max(1.0)) else {
                return;
            };
            for (a, b) in zones {
                if b > a + 1.0 {
                    canvas.set_clip(Some(ClipRect { x0: a, x1: b }));
                    canvas.stroke_path_colored(&path, width, color);
                }
            }
            canvas.set_clip(full);
        };
        // Waiting: each ping leaves the sensor as an arc across the print.
        if fx.pinging(clock) {
            let near = hero.sx - hero.pct_right;
            for age in hero.pings(t) {
                let r = age * PING_SPEED;
                if r < near && r > 40.0 {
                    let k = r / near;
                    ring(
                        canvas,
                        r,
                        2.5 - 1.5 * k,
                        pal.hot.mix(pal.a, k).alpha((0.75 - 0.5 * k) * alpha),
                    );
                }
            }
        }
        for &(at, n) in &fx.bumps {
            let age = clock.since(at);
            let k = age / 0.5;
            if k < 1.0 {
                ring(
                    canvas,
                    70.0 + ease_out_cubic(k) * 420.0,
                    2.0 * (1.0 - k) + 0.6,
                    pal.hot.mix(pal.a, k).alpha((1.0 - k) * 0.85 * alpha),
                );
            }
            // Sparks fly off the print down the strip.
            if age < 0.9 {
                let outside = |x: f32| zones.iter().any(|&(a, b)| x >= a && x < b);
                let fade = (1.0 - age / 0.9).powf(1.5);
                for s in 0..30u32 {
                    let hs = h3(fx.seed, n, s);
                    let ang = PI + (unit(hs) - 0.5) * 2.4;
                    let speed = 380.0 + 1100.0 * unit(hash(hs));
                    let (vx, vy) = (ang.cos() * speed, ang.sin() * speed * 0.25);
                    let color = pal.hot.mix(pal.a, unit(hs ^ 0x77) * 0.6);
                    for j in 0..4 {
                        let tj = (age - j as f32 * 0.018).max(0.0) * 3.4;
                        let tr = (1.0 - (-tj).exp()) / 3.4;
                        let a = fade * (1.0 - j as f32 * 0.24) * alpha;
                        if !outside(hero.cx + vx * tr) {
                            continue;
                        }
                        canvas.blend_rect(
                            hero.cx + vx * tr - 1.0,
                            hero.cy + vy * tr - 1.0,
                            2.0,
                            2.0,
                            color.alpha(a),
                        );
                    }
                }
            }
        }
        // Success: one shockwave out of the sensor.
        if fx.stage == TouchIdStage::Success && ts < 0.7 {
            let k = ts / 0.7;
            ring(
                canvas,
                70.0 + ease_out_cubic(k) * 700.0,
                4.0 * (1.0 - k) + 0.5,
                pal.hot.mix(pal.a, k).alpha((1.0 - k) * alpha),
            );
        }
    }

    /// The dot-matrix fingerprint, and the sensor's glow at the strip's end.
    #[allow(clippy::too_many_arguments)]
    fn fx_print(
        &self,
        canvas: &mut Canvas,
        fx: &TouchFx,
        hero: &Hero,
        pal: &Pal,
        rect: (f32, f32, f32, f32),
        clock: &Clock,
        alpha: f32,
    ) {
        let (rx, ry, rw, rh) = rect;
        let art = &fx.art;
        let t = clock.since(fx.start);
        let ts = clock.since(fx.stage_at);
        let shown = fx.shown(clock);
        let last_bump = fx.bumps.last().map(|b| clock.since(b.0)).unwrap_or(9.0);
        let scanning = clock.live
            && fx.ended.is_none()
            && matches!(
                fx.stage,
                TouchIdStage::Authenticate | TouchIdStage::Approve | TouchIdStage::Enrollment
            );
        let beam = scanning.then(|| {
            let sweep = (t * TAU / 1.9).sin();
            hero.cy + (art.h / 2.0 + 3.0) * sweep
        });
        let beam_k = if fx.stage == TouchIdStage::Enrollment {
            0.45
        } else {
            1.0
        };
        let glitch = if fx.stage == TouchIdStage::Retry && clock.live {
            (1.0 - ts / 0.6).max(0.0)
        } else {
            0.0
        };
        let boom = (fx.stage == TouchIdStage::Success).then_some(ts);
        // The sensor flashes as it pings or an enrollment step lands, and
        // the print as a ping passes.
        let (emit, pass) = flashes(fx, hero, clock, t);
        let grown = if clock.live {
            smoothstep(0.4, 1.2, t)
        } else {
            1.0
        };

        // Light spilling from the sensor onto the end of the strip, and a
        // soft halo that breathes behind the print.
        let spill = if boom.is_some() {
            1.0 - smoothstep(0.0, 1.2, ts)
        } else {
            0.22 + 0.45 * emit
        };
        canvas.fill_radial_glow(
            hero.sx,
            hero.cy,
            270.0,
            70.0,
            pal.a.mix(pal.hot, emit).alpha(spill * grown * alpha),
        );
        if boom.is_none() {
            let pulse = if clock.live {
                0.5 + 0.5 * (t * 2.2).sin()
            } else {
                0.5
            };
            canvas.fill_radial_glow(
                hero.cx,
                hero.cy,
                art.h * (1.0 + 0.15 * pulse),
                art.h * (0.8 + 0.1 * pulse),
                pal.a
                    .alpha((0.16 + 0.1 * pulse + 0.2 * pass) * grown * alpha),
            );
        }

        for (i, d) in art.dots.iter().enumerate() {
            let hi = h2(fx.seed ^ 0xf1, i as u32);
            let (tx, ty) = (hero.cx + d.x, hero.cy + d.y);

            // Intro: dots swarm in from across the strip.
            let delay = 0.1 + unit(hi) * 0.45 + d.radius * 0.15;
            let k = if clock.live {
                ((t - delay) / 0.65).clamp(0.0, 1.0)
            } else {
                1.0
            };
            if k <= 0.0 {
                continue;
            }
            if k < 1.0 {
                let sx = rx + unit(hash(hi)) * rw;
                let sy = ry + unit(hash(hi ^ 0x3c)) * rh;
                let swirl = (unit(hash(hi ^ 0x99)) - 0.5) * 50.0;
                let at = |k: f32| {
                    let e = ease_out_cubic(k);
                    (
                        sx + (tx - sx) * e,
                        sy + (ty - sy) * e + (k * PI).sin() * swirl,
                    )
                };
                let color = pal.hot.mix(pal.a, k);
                for j in 0..4 {
                    let (px, py) = at((k - j as f32 * 0.03).max(0.0));
                    let a = (1.0 - j as f32 * 0.25) * (0.35 + 0.65 * k) * alpha;
                    canvas.blend_rect(px - 1.0, py - 1.0, 2.0, 2.0, color.alpha(a));
                }
                continue;
            }

            // Success: the print bursts apart down the strip.
            if let Some(age) = boom
                && clock.live
            {
                let ang = PI + (unit(hi) - 0.5) * 2.0 + d.y * 0.02;
                let speed = 260.0 + 760.0 * unit(hash(hi ^ 0x21));
                let fade = 1.0 - smoothstep(0.25, 1.25, age);
                if fade <= 0.0 {
                    continue;
                }
                let color = pal.hot.mix(pal.a, smoothstep(0.0, 0.5, age));
                for j in 0..4 {
                    let tj = (age - j as f32 * 0.02).max(0.0);
                    let tr = speed * (1.0 - (-tj * 3.0).exp()) / 3.0;
                    let (px, py) = (tx + ang.cos() * tr * 3.2, ty + ang.sin() * tr * 0.3);
                    let a = fade * (1.0 - j as f32 * 0.25) * alpha;
                    canvas.blend_rect(px - 1.0, py - 1.0, 2.0, 2.0, color.alpha(a));
                }
                continue;
            }
            if boom.is_some() {
                continue;
            }

            let flicker = unit(h2(hi, (t * 9.0) as u32));
            let (mut b, mut color, mut hot) = match fx.stage {
                TouchIdStage::Enrollment if d.order < shown => {
                    let fresh = d.order >= fx.progress_from && last_bump < 0.55;
                    let hot = if fresh { 1.0 - last_bump / 0.55 } else { 0.0 };
                    (1.0, pal.a, hot)
                }
                TouchIdStage::Enrollment => (0.2 + 0.14 * flicker, pal.b, 0.0),
                TouchIdStage::Retry => (0.6 + 0.25 * (t * 5.0).sin().abs(), pal.a, 0.0),
                _ => {
                    let breath = 0.5 + 0.5 * (t * 2.2 - d.radius * 3.0).sin();
                    (0.68 + 0.27 * breath, pal.a, 0.0)
                }
            };
            b += 0.45 * pass;
            hot += 0.6 * pass;
            if let Some(by) = beam {
                let g = gauss((ty - by) / 3.2) * beam_k;
                b += 0.6 * g;
                hot += 0.8 * g;
            }
            if clock.live && unit(h2(hi, (t * 4.0) as u32 ^ 0xbeef)) < 0.02 {
                hot = hot.max(0.7);
                b = b.max(0.9);
            }
            color = color.mix(pal.hot, hot.min(1.0));
            let mut px = tx;
            if glitch > 0.0 {
                let slice = h3(fx.seed, d.row as u32 / 2, (ts * 26.0) as u32);
                if unit(slice) < 0.5 {
                    px += (unit(hash(slice)) - 0.5) * 16.0 * glitch;
                }
                // Chromatic ghosts.
                let ghost = 0.55 * glitch * alpha;
                canvas.blend_rect(px - 3.0, ty - 1.0, 2.0, 2.0, pal.b.alpha(ghost));
                canvas.blend_rect(px + 1.0, ty - 1.0, 2.0, 2.0, pal.c.alpha(ghost));
            }
            canvas.blend_rect(
                px - 1.0,
                ty - 1.0,
                2.0,
                2.0,
                color.alpha(b.min(1.0) * alpha),
            );
        }

        // The scanner beam itself, with a laser line running down the
        // strip from the hero.
        if let Some(by) = beam
            && t > 0.7
        {
            let w = art.w + 22.0;
            let x = hero.cx - w / 2.0;
            let a = beam_k * alpha * smoothstep(0.7, 1.1, t);
            canvas.fill_rrect_hgradient(
                hero.x0 - 560.0,
                by - 0.5,
                560.0,
                1.0,
                0.0,
                &[(0.0, pal.c.alpha(0.0)), (1.0, pal.a.alpha(0.42 * a))],
            );
            canvas.fill_rrect_hgradient(
                x,
                by - 3.5,
                w,
                7.0,
                3.5,
                &[
                    (0.0, pal.a.alpha(0.0)),
                    (0.5, pal.a.alpha(0.22 * a)),
                    (1.0, pal.a.alpha(0.0)),
                ],
            );
            canvas.fill_rrect_hgradient(
                x,
                by - 0.75,
                w,
                1.5,
                0.75,
                &[
                    (0.0, pal.hot.alpha(0.0)),
                    (0.5, pal.hot.alpha(0.95 * a)),
                    (1.0, pal.hot.alpha(0.0)),
                ],
            );
        }

        // Success: a check mark draws itself where the print was.
        if let Some(age) = boom {
            let k = if clock.live {
                ((age - 0.18) / 0.35).clamp(0.0, 1.0)
            } else {
                1.0
            };
            if k > 0.0 {
                let s = art.h * 0.62;
                let pts = [(-0.45, 0.02), (-0.13, 0.34), (0.47, -0.3)];
                let pts = pts.map(|(x, y)| (hero.cx + x * s, hero.cy + y * s));
                if let Some(path) = partial_polyline(&pts, ease_out_cubic(k)) {
                    let pop = 1.0 + 0.6 * (1.0 - smoothstep(0.35, 0.8, age));
                    canvas.stroke_path_colored(&path, 12.0 * pop, pal.a.alpha(0.16 * alpha));
                    canvas.stroke_path_colored(&path, 7.5 * pop, pal.a.alpha(0.35 * alpha));
                    canvas.stroke_path_colored(&path, 4.0, pal.hot.mix(pal.a, 0.3).alpha(alpha));
                }
            }
        }
    }

    /// Chevrons between the print and the sensor, pointing the way. They
    /// breathe together and flash as the sensor pings.
    fn fx_pointer(
        &self,
        canvas: &mut Canvas,
        fx: &TouchFx,
        hero: &Hero,
        pal: &Pal,
        clock: &Clock,
        alpha: f32,
    ) {
        if fx.stage == TouchIdStage::Success {
            return;
        }
        let size = (self.m().icon * 0.85).round();
        let run = self.fonts.layout(glyph::CHEVRON, size, Role::Icon);
        if run.glyphs.is_empty() {
            return;
        }
        let (l, r) = self.fonts.ink_hbounds(glyph::CHEVRON, size, Role::Icon);
        let step = (r - l + 3.0).max(9.0);
        let t = clock.since(fx.start);
        let appear = if clock.live {
            smoothstep(0.8, 1.2, t)
        } else {
            1.0
        };
        let g = if fx.waiting(clock) {
            let breath = 0.5 + 0.5 * (t * 2.2).sin();
            (0.35 * breath + 0.65 * flashes(fx, hero, clock, t).0).min(1.0)
        } else if clock.live {
            0.0
        } else {
            0.5
        };
        for i in 0..3 {
            canvas.draw_run(
                &self.fonts,
                &run,
                hero.pointer_x + i as f32 * step - l,
                hero.cy,
                pal.a
                    .mix(pal.hot, g)
                    .alpha((0.3 + 0.7 * g) * appear * alpha),
                VAlign::Ink,
            );
        }
    }

    /// Status text that decrypts itself (from the print outward), plus the
    /// enrollment percentage.
    fn fx_text(
        &self,
        canvas: &mut Canvas,
        fx: &TouchFx,
        hero: &Hero,
        pal: &Pal,
        clock: &Clock,
        alpha: f32,
    ) {
        let size = self.m().label;
        let ts = clock.since(fx.stage_at);
        // Let the swarm get going before the first text.
        let lead = if fx.prev.is_none() { 0.3 } else { 0.0 };
        let glitch = if fx.stage == TouchIdStage::Retry && clock.live {
            (1.0 - ts / 0.6).max(0.0)
        } else {
            0.0
        };
        let text = fx.text();
        let len = text.chars().count();
        let mut x = hero.text_x;
        if glitch > 0.0 {
            x += (unit(h2(fx.seed, (ts * 30.0) as u32)) - 0.5) * 10.0 * glitch;
        }
        let mut buf = [0u8; 4];
        for (k, ch) in text.chars().enumerate() {
            let run = self
                .fonts
                .layout(ch.encode_utf8(&mut buf), size, Role::Bold);
            let adv = run.width;
            let hk = h3(fx.seed, k as u32, fx.stage as u32);
            let from_print = (len - 1 - k) as f32;
            let appear = lead + from_print * 0.012;
            let resolve = lead + 0.12 + from_print * 0.018 + unit(hk) * 0.45;
            if ts >= appear {
                if ts < resolve {
                    if ch != ' ' || unit(hash(hk)) < 0.5 {
                        let g = h2(hk, (ts * 26.0) as u32) as usize % GLYPHS.len();
                        let scramble = self.fonts.layout(GLYPHS[g], size, Role::Bold);
                        canvas.draw_run(
                            &self.fonts,
                            &scramble,
                            x,
                            hero.cy,
                            pal.b.alpha(0.55 * alpha),
                            VAlign::Cap,
                        );
                    }
                } else if ch != ' ' {
                    let flash = (1.0 - (ts - resolve) / 0.22).max(0.0);
                    let color = pal
                        .grad(from_print / len.max(1) as f32 * 0.5)
                        .mix(pal.fg, 0.55)
                        .mix(pal.hot, flash);
                    canvas.draw_run(
                        &self.fonts,
                        &run,
                        x,
                        hero.cy,
                        color.alpha(alpha),
                        VAlign::Cap,
                    );
                }
            }
            x += adv;
        }

        if fx.stage == TouchIdStage::Enrollment {
            let shown = fx.shown(clock);
            let pct = format!("{}%", (shown * 100.0).round() as u32);
            let w = self.fonts.measure(&pct, size, Role::Bold);
            let last_bump = fx.bumps.last().map(|b| clock.since(b.0)).unwrap_or(9.0);
            let flash = (1.0 - last_bump / 0.4).max(0.0);
            let appear = smoothstep(lead + 0.2, lead + 0.5, ts);
            canvas.text(
                &self.fonts,
                &pct,
                hero.pct_right - w,
                hero.cy,
                size,
                Role::Bold,
                pal.a.mix(pal.hot, flash).alpha(appear * alpha),
                VAlign::Cap,
            );
        }
    }
}

/// A flash's brightness `age` seconds after it fires: a quick attack and
/// a fast decay.
fn burst(age: f32) -> f32 {
    (age / 0.05).min(1.0) * (-age / 0.25).exp()
}

/// How brightly the sensor flashes (`emit`) and the print as a ping passes
/// it (`pass`): from timed pings while authenticating, and from each
/// enrollment step.
fn flashes(fx: &TouchFx, hero: &Hero, clock: &Clock, t: f32) -> (f32, f32) {
    let (mut emit, pass) = if fx.pinging(clock) {
        hero.ping_flash(t)
    } else {
        (0.0, 0.0)
    };
    if clock.live
        && let Some(&(at, _)) = fx.bumps.last()
    {
        emit = emit.max(burst(clock.since(at)));
    }
    (emit, pass)
}

/// Seconds since an event, or a settled time when motion is off.
struct Clock {
    now: Instant,
    live: bool,
}

impl Clock {
    fn since(&self, at: Instant) -> f32 {
        if self.live { secs(self.now, at) } else { 60.0 }
    }
}

/// The first `k` (0–1) of a polyline's length.
fn partial_polyline(pts: &[(f32, f32)], k: f32) -> Option<tiny_skia::Path> {
    let lens: Vec<f32> = pts
        .windows(2)
        .map(|w| ((w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2)).sqrt())
        .collect();
    let mut left = lens.iter().sum::<f32>() * k.clamp(0.0, 1.0);
    let mut pb = PathBuilder::new();
    pb.move_to(pts[0].0, pts[0].1);
    for (w, len) in pts.windows(2).zip(&lens) {
        if left <= 0.0 {
            break;
        }
        let f = (left / len).min(1.0);
        pb.line_to(
            w[0].0 + (w[1].0 - w[0].0) * f,
            w[0].1 + (w[1].1 - w[0].1) * f,
        );
        left -= len;
    }
    pb.finish()
}

// ----------------------------------------------------------------- helpers

fn secs(now: Instant, at: Instant) -> f32 {
    now.saturating_duration_since(at).as_secs_f32()
}

fn ease_out_cubic(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn gauss(x: f32) -> f32 {
    if x.abs() > 3.5 { 0.0 } else { (-x * x).exp() }
}

fn hash(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    x
}

fn h2(a: u32, b: u32) -> u32 {
    hash(a ^ hash(b).wrapping_add(0x9e37_79b9))
}

fn h3(a: u32, b: u32, c: u32) -> u32 {
    h2(h2(a, b), c)
}

/// A hash as a value in 0–1.
fn unit(h: u32) -> f32 {
    (h >> 8) as f32 / (1u32 << 24) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tid(stage: TouchIdStage, progress: Option<u8>) -> TouchId {
        TouchId { stage, progress }
    }

    #[test]
    fn success_plays_out_before_fading() {
        let fonts = Fonts::from_files(None, None, None, None, None);
        let t0 = Instant::now();
        let mut fx = TouchFx::new(&tid(TouchIdStage::Authenticate, None), t0, &fonts, 60.0);
        assert!(
            !fx.art.dots.is_empty(),
            "procedural print when no icon font"
        );
        let t1 = t0 + Duration::from_secs(2);
        assert!(fx.sync(Some(&tid(TouchIdStage::Success, None)), t1));
        // T1Bridge clears the prompt almost at once…
        let t2 = t1 + Duration::from_millis(200);
        assert!(fx.sync(None, t2));
        assert!(!fx.cancellable());
        // …but the success keeps playing, then fades.
        assert_eq!(
            fx.fade(t1 + Duration::from_secs_f32(SUCCESS_HOLD - 0.1)),
            1.0
        );
        assert!(!fx.done(t1 + Duration::from_secs_f32(SUCCESS_HOLD + 0.1)));
        assert!(fx.done(t1 + Duration::from_secs_f32(SUCCESS_HOLD + FADE + 0.01)));
    }

    #[test]
    fn cancel_fades_quickly_and_progress_counts_up() {
        let fonts = Fonts::from_files(None, None, None, None, None);
        let t0 = Instant::now();
        let mut fx = TouchFx::new(&tid(TouchIdStage::Enrollment, Some(0)), t0, &fonts, 60.0);
        let t1 = t0 + Duration::from_secs(1);
        fx.sync(Some(&tid(TouchIdStage::Enrollment, Some(40))), t1);
        assert_eq!(fx.bumps.len(), 1);
        let mid = fx.progress_shown(t1 + Duration::from_millis(150));
        assert!(mid > 0.0 && mid < 0.4);
        assert!((fx.progress_shown(t1 + Duration::from_secs(1)) - 0.4).abs() < 1e-4);
        // Unchanged state is not a change.
        assert!(!fx.sync(Some(&tid(TouchIdStage::Enrollment, Some(40))), t1));
        let t2 = t1 + Duration::from_secs(1);
        fx.sync(None, t2);
        assert!(fx.done(t2 + Duration::from_secs_f32(FADE + 0.01)));
    }

    #[test]
    fn enrollment_bar_fills_from_the_sensor_end() {
        let fonts = Fonts::from_files(None, None, None, None, None);
        let t0 = Instant::now();
        let clock = |now| Clock { now, live: true };
        // Authenticating never shows a bar.
        let auth = TouchFx::new(&tid(TouchIdStage::Authenticate, None), t0, &fonts, 60.0);
        assert!(auth.bar(&clock(t0), 100.0, 2100.0).is_none());

        let mut fx = TouchFx::new(&tid(TouchIdStage::Enrollment, Some(0)), t0, &fonts, 60.0);
        let bar = fx.bar(&clock(t0), 100.0, 2100.0).unwrap();
        assert_eq!(bar.edge, 2100.0, "empty at the sensor end");
        let t1 = t0 + Duration::from_secs(1);
        fx.sync(Some(&tid(TouchIdStage::Enrollment, Some(50))), t1);
        let bar = fx
            .bar(&clock(t1 + Duration::from_millis(100)), 100.0, 2100.0)
            .unwrap();
        assert!(bar.flash > 0.5, "a fresh step flashes");
        assert!(bar.edge > 1100.0 && bar.edge < 2100.0, "still sweeping");
        let bar = fx
            .bar(&clock(t1 + Duration::from_secs(1)), 100.0, 2100.0)
            .unwrap();
        assert!((bar.edge - 1100.0).abs() < 0.5, "half the strip");
        assert_eq!(bar.flash, 0.0);
        // A miss keeps the progress made so far.
        let t2 = t1 + Duration::from_secs(2);
        fx.sync(Some(&tid(TouchIdStage::Retry, Some(50))), t2);
        assert!((fx.bar(&clock(t2), 100.0, 2100.0).unwrap().edge - 1100.0).abs() < 0.5);
        // Success fills it and fades it into the flood.
        let t3 = t2 + Duration::from_secs(1);
        fx.sync(Some(&tid(TouchIdStage::Success, None)), t3);
        assert_eq!(fx.bar(&clock(t3), 100.0, 2100.0).unwrap().edge, 100.0);
        assert!(
            fx.bar(&clock(t3 + Duration::from_secs(1)), 100.0, 2100.0)
                .is_none()
        );
    }

    #[test]
    fn enrolling_pulses_only_when_progress_advances() {
        let fonts = Fonts::from_files(None, None, None, None, None);
        let t0 = Instant::now();
        let live = |now| Clock { now, live: true };
        let hero = Hero {
            cx: 2060.0,
            cy: 30.0,
            text_x: 1400.0,
            pct_right: 2010.0,
            pointer_x: 2100.0,
            sx: 2260.0,
            x0: 1386.0,
            reach: 2000.0,
            bar: None,
        };
        let emit_over = |fx: &TouchFx, from: Instant, secs: f32| {
            (0..(secs * 60.0) as u32)
                .map(|i| {
                    let now = from + Duration::from_secs_f32(i as f32 / 60.0);
                    let t = secs_since(now, t0);
                    flashes(fx, &hero, &live(now), t).0
                })
                .fold(0.0f32, f32::max)
        };
        // Authenticating pings on a timer.
        let auth = TouchFx::new(&tid(TouchIdStage::Authenticate, None), t0, &fonts, 60.0);
        assert!(auth.pinging(&live(t0)));
        assert!(emit_over(&auth, t0, 3.0) > 0.5);

        // Enrolling stays quiet until a step lands…
        let mut fx = TouchFx::new(&tid(TouchIdStage::Enrollment, Some(0)), t0, &fonts, 60.0);
        assert!(!fx.pinging(&live(t0)));
        assert_eq!(emit_over(&fx, t0, 3.0), 0.0);
        // …then flashes once for it and goes quiet again.
        let t1 = t0 + Duration::from_secs(3);
        fx.sync(Some(&tid(TouchIdStage::Enrollment, Some(20))), t1);
        assert!(emit_over(&fx, t1, 0.2) > 0.5);
        assert!(emit_over(&fx, t1 + Duration::from_secs(2), 3.0) < 0.01);
        // A miss mid-enrollment does not start the timer either.
        let t2 = t1 + Duration::from_secs(6);
        fx.sync(Some(&tid(TouchIdStage::Retry, Some(20))), t2);
        assert!(!fx.pinging(&live(t2)));
    }

    fn secs_since(now: Instant, at: Instant) -> f32 {
        now.saturating_duration_since(at).as_secs_f32()
    }

    #[test]
    fn enrollment_order_is_uniform() {
        let art = Art::procedural(54.0);
        let n = art.dots.len() as f32;
        let half = art.dots.iter().filter(|d| d.order < 0.5).count() as f32;
        assert!((half / n - 0.5).abs() < 0.02);
    }
}
