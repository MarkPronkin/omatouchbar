//! Labelled brackets under the preview that name each part of the strip.
//!
//! Zones come from the real scene: a hidden copy is laid out with the same
//! config and settled, then its node rectangles are read back, so the
//! brackets sit exactly under the keys and widgets they name.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::{Duration, Instant},
};

use chrono::Utc;
use gtk::{glib, prelude::*};
use omatouchbar_core::{
    Config, DesktopState, Fonts, Palette, Scene, T1_HEIGHT, T1_WIDTH, config::Mode, menu::MenuTree,
    node::Key, omarchy::BarLayout,
};

use crate::{motion, style};

#[derive(Clone, Debug, PartialEq)]
pub struct Zone {
    /// Strip pixels.
    pub x0: f32,
    pub x1: f32,
    pub label: String,
}

/// Lays the strip out off screen to find where things land.
pub struct Measurer {
    scene: Scene,
}

impl Measurer {
    pub fn new(config: &Config) -> Measurer {
        let mut scene = Scene::new(
            T1_WIDTH,
            T1_HEIGHT,
            config.clone(),
            Palette::load().unwrap_or_else(Palette::fallback),
            Fonts::system(None),
            MenuTree::load(),
            BarLayout::from_omarchy(),
        );
        let model = std::fs::read_to_string("/sys/class/dmi/id/product_name").unwrap_or_default();
        scene.set_state(DesktopState::sample_for_model(&model));
        Measurer { scene }
    }

    fn settle(&mut self, start: Instant) {
        for frame in 1..150u64 {
            self.scene
                .update(start + Duration::from_millis(frame * 16), Utc::now());
        }
    }

    fn rect(&self, key: Key) -> Option<(f32, f32)> {
        self.scene
            .node_rect(key)
            .filter(|r| r.w > 1.0)
            .map(|r| (r.x, r.x + r.w))
    }

    fn union(&self, keys: &[Key]) -> Option<(f32, f32)> {
        keys.iter()
            .filter_map(|k| self.rect(*k))
            .reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)))
    }

    /// The zones for `config`, with Fn held or not.
    pub fn zones(&mut self, config: &Config, fn_held: bool) -> Vec<Zone> {
        self.scene.set_config(config.clone());
        let start = Instant::now();
        self.scene.input(fn_held, &[], start);
        self.settle(start);
        let mut zones = Vec::new();
        let mut push = |range: Option<(f32, f32)>, label: &str| {
            if let Some((x0, x1)) = range {
                zones.push(Zone {
                    x0,
                    x1,
                    label: label.into(),
                });
            }
        };
        let ctl = |names: &[&str]| -> Vec<Key> {
            names.iter().map(|n| Key::new("ctl").with(n)).collect()
        };
        push(self.rect(Key::new("esc")), "esc");
        let fkeys: Vec<Key> = (1..=12).map(|n| Key::new("fkey").index(n)).collect();
        // Whatever page is up decides the zones, so Fn layers work too.
        let page = if self.rect(fkeys[0]).is_some() {
            Mode::Function
        } else if self.rect(Key::new("ctl").with("bd")).is_some() {
            Mode::Controls
        } else {
            Mode::Bar
        };
        match page {
            Mode::Keyboard | Mode::Controls => {
                push(self.union(&ctl(&["bd", "bu"])), "brightness");
                push(self.union(&ctl(&["kd", "ku"])), "keyboard light");
                push(self.union(&ctl(&["prev", "play", "next"])), "media");
                let volume = self.union(&ctl(&["mute", "vd", "vu"]));
                push(volume, "volume");
                if config.mode == Mode::Keyboard
                    && !fn_held
                    && let Some((_, end)) = volume
                {
                    let right = T1_WIDTH as f32 - 8.0;
                    if right - end > 120.0 {
                        push(Some((end + 14.0, right)), "now playing · tap for sound");
                    }
                }
            }
            Mode::Function => {
                push(self.union(&fkeys), "F1 – F12");
            }
            Mode::Bar => {
                let named: [(&str, &str); 14] = [
                    ("w/menu", "menu"),
                    ("w/ind", "status"),
                    ("w/ws", "workspaces"),
                    ("w/clock", "clock"),
                    ("w/media", "now playing"),
                    ("w/world", "status"),
                    ("w/update", "status"),
                    ("w/keyboard", "status"),
                    ("w/bluetooth", "status"),
                    ("w/network", "status"),
                    ("w/audio", "status"),
                    ("w/mic", "status"),
                    ("w/brightness", "status"),
                    ("w/battery", "status"),
                ];
                let mut found: Vec<Zone> = named
                    .iter()
                    .filter_map(|(key, label)| {
                        self.rect(Key::new(key)).map(|(x0, x1)| Zone {
                            x0,
                            x1,
                            label: (*label).into(),
                        })
                    })
                    .collect();
                found.sort_by(|a, b| a.x0.total_cmp(&b.x0));
                // Status widgets read as one group until something else
                // (the clock, say) sits between them; anything unnamed in
                // between (weather, tray, agents) belongs to it anyway.
                let mut merged: Vec<Zone> = Vec::new();
                for zone in found {
                    if let Some(last) = merged.last_mut()
                        && last.label == zone.label
                    {
                        last.x1 = last.x1.max(zone.x1);
                        continue;
                    }
                    merged.push(zone);
                }
                zones.extend(merged);
            }
        }
        zones
    }
}

struct State {
    zones: Vec<Zone>,
    started: Option<i64>,
    /// Zones fading out while the new ones come in.
    leaving: Vec<Zone>,
    left_at: Option<i64>,
    tick: Option<gtk::TickCallbackId>,
}

/// The bracket strip itself, drawn under `picture`.
#[derive(Clone)]
pub struct Callouts {
    pub widget: gtk::DrawingArea,
    state: Rc<RefCell<State>>,
    picture: Rc<RefCell<Option<gtk::Picture>>>,
    visible: Rc<Cell<bool>>,
}

const STAGGER_US: i64 = 70_000;
const GROW_US: i64 = 520_000;
const LEAVE_US: i64 = 160_000;

impl Callouts {
    pub fn new() -> Callouts {
        let widget = gtk::DrawingArea::new();
        widget.set_content_height(54);
        widget.set_hexpand(true);
        let callouts = Callouts {
            widget,
            state: Rc::new(RefCell::new(State {
                zones: Vec::new(),
                started: None,
                leaving: Vec::new(),
                left_at: None,
                tick: None,
            })),
            picture: Rc::new(RefCell::new(None)),
            visible: Rc::new(Cell::new(true)),
        };
        let this = callouts.clone();
        callouts
            .widget
            .set_draw_func(move |area, cr, w, h| this.draw(area, cr, w, h));
        callouts
    }

    /// The preview picture the zones are measured against.
    pub fn track(&self, picture: &gtk::Picture) {
        self.picture.replace(Some(picture.clone()));
    }

    pub fn set_zones(&self, zones: Vec<Zone>, delay_ms: i64) {
        let now = self
            .widget
            .frame_clock()
            .map(|c| c.frame_time())
            .unwrap_or_else(glib::monotonic_time);
        {
            let mut s = self.state.borrow_mut();
            if s.zones == zones && s.started.is_some() {
                return;
            }
            s.leaving = std::mem::take(&mut s.zones);
            s.left_at = Some(now);
            s.zones = zones;
            s.started = Some(now + delay_ms * 1000);
        }
        self.run();
    }

    pub fn clear(&self) {
        self.set_zones(Vec::new(), 0);
    }

    pub fn set_shown(&self, shown: bool) {
        self.visible.set(shown);
        self.widget.queue_draw();
    }

    fn run(&self) {
        if self.state.borrow().tick.is_some() {
            return;
        }
        let this = self.clone();
        let id = self.widget.add_tick_callback(move |area, clock| {
            area.queue_draw();
            let now = clock.frame_time();
            let mut s = this.state.borrow_mut();
            let end = s.started.unwrap_or(now) + GROW_US + STAGGER_US * s.zones.len() as i64;
            if now > end && now > s.left_at.unwrap_or(0) + LEAVE_US {
                s.leaving.clear();
                s.tick = None;
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });
        self.state.borrow_mut().tick = Some(id);
    }

    fn draw(&self, area: &gtk::DrawingArea, cr: &gtk::cairo::Context, w: i32, _h: i32) {
        if !self.visible.get() {
            return;
        }
        let picture = self.picture.borrow().clone();
        // Map strip pixels into this widget through the picture's bounds.
        let (origin, span) = picture
            .as_ref()
            .and_then(|p| p.compute_bounds(area))
            .filter(|b| b.width() > 10.0)
            .map(|b| (b.x() as f64, b.width() as f64))
            .unwrap_or((12.0, w as f64 - 24.0));
        let map = |x: f32| origin + x as f64 / T1_WIDTH as f64 * span;
        let t = style::tokens();
        let now = area
            .frame_clock()
            .map(|c| c.frame_time())
            .unwrap_or_else(glib::monotonic_time);
        let s = self.state.borrow();

        let set = |c: omatouchbar_core::color::Rgba, a: f64| {
            cr.set_source_rgba(c.r as f64, c.g as f64, c.b as f64, c.a as f64 * a);
        };
        let font = gtk::pango::FontDescription::from_string(&format!(
            "monospace {}px",
            (t.text * 0.84).round()
        ));

        // Leaving zones fade straight out.
        let leave = s
            .left_at
            .map(|at| 1.0 - ((now - at) as f64 / LEAVE_US as f64).clamp(0.0, 1.0))
            .unwrap_or(0.0);
        if leave > 0.0 {
            for zone in &s.leaving {
                bracket(cr, &set, map(zone.x0), map(zone.x1), 1.0, leave * 0.6, t.fg);
            }
        }

        let Some(started) = s.started else {
            return;
        };
        let mut label_ends: Vec<f64> = Vec::new();
        for (i, zone) in s.zones.iter().enumerate() {
            let local = (now - started - i as i64 * STAGGER_US) as f64 / GROW_US as f64;
            if local <= 0.0 {
                continue;
            }
            let grow = motion::ease_out_cubic(local);
            let fade = motion::ease_out_cubic(local * 1.6);
            let (x0, x1) = (map(zone.x0) + 2.0, map(zone.x1) - 2.0);
            bracket(cr, &set, x0, x1, grow, fade, t.fg);

            let layout = pangocairo::functions::create_layout(cr);
            layout.set_font_description(Some(&font));
            layout.set_text(&zone.label);
            let (lw, lh) = layout.pixel_size();
            let center = (x0 + x1) / 2.0;
            let mut lx = (center - lw as f64 / 2.0)
                .max(0.0)
                .min(w as f64 - lw as f64);
            // Labels wider than their zone drop to a second line when they
            // would run into the previous one.
            let mut row = 0.0;
            if let Some(end) = label_ends.last()
                && lx < end + 10.0
            {
                row = 1.0;
                if label_ends.len() >= 2 && lx < label_ends[label_ends.len() - 2] + 10.0 {
                    lx = label_ends[label_ends.len() - 2] + 10.0;
                }
            }
            let ly = 16.0 + row * (lh as f64 + 4.0) + (1.0 - fade) * 6.0;
            if row > 0.0 {
                // A short leader from the bracket down to the dropped label.
                set(t.fg, 0.22 * fade);
                cr.set_line_width(1.0);
                cr.move_to(center.round() + 0.5, 12.0);
                cr.line_to(center.round() + 0.5, ly - 1.0);
                let _ = cr.stroke();
            }
            set(t.secondary, fade);
            cr.move_to(lx, ly);
            pangocairo::functions::show_layout(cr, &layout);
            label_ends.push(lx + lw as f64);
        }
    }
}

/// A bracket under [x0, x1] that grows out from its middle.
fn bracket(
    cr: &gtk::cairo::Context,
    set: &dyn Fn(omatouchbar_core::color::Rgba, f64),
    x0: f64,
    x1: f64,
    grow: f64,
    alpha: f64,
    color: omatouchbar_core::color::Rgba,
) {
    let center = (x0 + x1) / 2.0;
    let half = (x1 - x0) / 2.0 * grow;
    let y = 8.5;
    set(color, 0.38 * alpha);
    cr.set_line_width(1.0);
    cr.move_to((center - half).round() + 0.5, 3.0);
    cr.line_to((center - half).round() + 0.5, y);
    cr.line_to((center + half).round() - 0.5, y);
    cr.line_to((center + half).round() - 0.5, 3.0);
    let _ = cr.stroke();
    // The tick that points at the label.
    cr.move_to(center.round() + 0.5, y);
    cr.line_to(center.round() + 0.5, y + 3.0 * grow);
    let _ = cr.stroke();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(zones: &[Zone]) -> Vec<&str> {
        zones.iter().map(|z| z.label.as_str()).collect()
    }

    #[test]
    fn every_mode_names_its_parts_in_order() {
        let mut config = Config::default();
        let mut measurer = Measurer::new(&config);
        for mode in Mode::ALL {
            config.mode = mode;
            for fn_held in [false, true] {
                let zones = measurer.zones(&config, fn_held);
                assert!(!zones.is_empty(), "{mode:?} fn={fn_held} found nothing");
                assert_eq!(zones[0].label, "esc", "{mode:?}: {:?}", labels(&zones));
                for pair in zones.windows(2) {
                    assert!(
                        pair[0].x1 <= pair[1].x0 + 1.0,
                        "{mode:?} fn={fn_held}: overlapping {:?}",
                        labels(&zones)
                    );
                }
                for zone in &zones {
                    assert!(zone.x0 >= 0.0 && zone.x1 <= T1_WIDTH as f32 && zone.x1 > zone.x0);
                }
            }
        }
        config.mode = Mode::Keyboard;
        let keyboard = measurer.zones(&config, false);
        assert_eq!(
            labels(&keyboard)[..5],
            ["esc", "brightness", "keyboard light", "media", "volume"]
        );
        assert!(
            labels(&keyboard)
                .iter()
                .any(|l| l.starts_with("now playing"))
        );
        // Fn in Keyboard mode shows F1–F12 by default.
        assert_eq!(labels(&measurer.zones(&config, true)), ["esc", "F1 – F12"]);
        config.mode = Mode::Function;
        assert_eq!(labels(&measurer.zones(&config, false)), ["esc", "F1 – F12"]);
        // …and the controls when Fn is held there.
        assert!(labels(&measurer.zones(&config, true)).contains(&"volume"));
        config.mode = Mode::Bar;
        let bar = measurer.zones(&config, false);
        assert!(labels(&bar).contains(&"workspaces"), "{:?}", labels(&bar));
    }
}
