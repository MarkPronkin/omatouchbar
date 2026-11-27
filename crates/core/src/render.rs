//! Draws the animated node list.

use std::collections::HashMap;

use crate::{
    anim::{Animated, Ripple},
    canvas::{Canvas, ClipRect, VAlign},
    color::Rgba,
    node::{Anchor, Key, Node, Part},
    scene::Scene,
    text::Role,
    theme::Theme,
};

fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

/// One key's share of a `Level`: a fill in the key's own shape from its
/// left up to the value, as a slider fills. It rises out of the key's
/// bottom edge as it shows and sinks back as it hides (`shown`), level
/// like liquid, so it never blends into the key half-faded. Returns where
/// the fill ends across and how high it reaches, for the key's `SplitText`.
fn draw_level(
    canvas: &mut Canvas,
    (x, y, w, h, r): (f32, f32, f32, f32, f32),
    value: f32,
    (from, to): (f32, f32),
    color: Rgba,
    (shown, opacity): (f32, f32),
) -> (f32, f32) {
    let local = ((value - from) / (to - from).max(1e-4)).clamp(0.0, 1.0);
    let edge = x + w * local;
    // Gone a touch before the spring settles, so no sliver lingers.
    let rise = ((shown - 0.1) / 0.9).clamp(0.0, 1.0);
    let top = y + h * (1.0 - rise);
    let prev = canvas.clip();
    let outer = prev.unwrap_or(ClipRect::band(0.0, canvas.width() as f32));
    let clip = ClipRect {
        x1: edge.min(outer.x1),
        y0: top.max(outer.y0),
        ..outer
    };
    if clip.x1 - clip.x0 >= 0.5 && clip.y1 - clip.y0 >= 0.5 {
        canvas.set_clip(Some(clip));
        canvas.fill_rrect_vgradient(
            x,
            y,
            w,
            h,
            r,
            color.lighten(0.08).fade(opacity),
            color.fade(opacity),
        );
        canvas.set_clip(prev);
    }
    (edge, top)
}

impl Scene {
    /// Draws the current frame.
    pub fn render(&self, canvas: &mut Canvas) {
        let Some(wipe) = &self.theme_wipe else {
            self.render_pass(canvas, &self.theme, None);
            return;
        };
        // Mid theme switch: the old theme, with the new one revealed in a
        // band opening out of the middle.
        self.render_pass(canvas, &wipe.from, Some(&wipe.nodes));
        wipe.composite(canvas, &mut |c| self.render_pass(c, &self.theme, None));
        wipe.draw_edges(canvas, self.theme.accent);
    }

    /// Draws every node in `t`, taking each node from `nodes` (the same
    /// layout built in another theme) where it has one.
    fn render_pass(&self, canvas: &mut Canvas, t: &Theme, nodes: Option<&HashMap<Key, Node>>) {
        canvas.clear(t.bar);
        let row = self.scroll.clip();
        for a in &self.animator.items {
            let offset = if a.node.group == Some(0) {
                self.scroll.offset
            } else {
                0.0
            };
            canvas.set_clip(if a.node.group == Some(0) { row } else { None });
            let node = nodes.and_then(|n| n.get(&a.node.key)).unwrap_or(&a.node);
            self.draw(canvas, a, node, t, offset);
        }
        canvas.set_clip(None);
        // Soft edges where the menu row continues out of view.
        if let Some(clip) = row {
            let fade = 36.0;
            let h = canvas.height() as f32;
            if self.scroll.offset > 1.0 {
                canvas.fill_hgradient(clip.x0, 0.0, fade, h, t.bar, t.bar.alpha(0.0));
            }
            if self.scroll.offset < self.scroll.max - 1.0 {
                canvas.fill_hgradient(clip.x1 - fade, 0.0, fade, h, t.bar.alpha(0.0), t.bar);
            }
        }
    }

    fn draw(&self, canvas: &mut Canvas, a: &Animated, node: &Node, t: &Theme, offset: f32) {
        let opacity = a.opacity.value.clamp(0.0, 1.0);
        if opacity < 0.004 {
            return;
        }
        let press = a.press.value.clamp(0.0, 1.0);
        let emph = a.emph.value.clamp(0.0, 1.0);
        let base = a.rect();
        let scale = a.scale.value * (1.0 - 0.045 * press);
        let w = base.w * scale;
        let h = base.h * scale;
        let x = base.x - offset + (base.w - w) / 2.0;
        let y = base.y + (base.h - h) / 2.0;
        if x > canvas.width() as f32 + 4.0 || x + w < -4.0 {
            return;
        }

        if let Some(fill) = &node.fill {
            let mut color = match fill.emph {
                Some(e) => fill.color.mix(e, emph),
                None => fill.color,
            };
            if node.press_glow {
                color = color.mix(t.surface_hi, press * 0.75);
            }
            let r = fill.radius * scale.min(1.0);
            if fill.sheen && color.a > 0.05 {
                canvas.fill_rrect_vgradient(
                    x,
                    y,
                    w,
                    h,
                    r,
                    color.lighten(0.035).fade(opacity),
                    color.darken(0.06).fade(opacity),
                );
            } else {
                canvas.fill_rrect(x, y, w, h, r, color.fade(opacity));
            }
            if let Some(border) = fill.border {
                canvas.stroke_rrect(x, y, w, h, r, 1.5, border.fade(opacity));
            }
            for ripple in &a.ripples {
                self.ripple(canvas, ripple, (x, y, w, h, r), t, offset, opacity);
            }
        }

        // Where this node's Progress or Level fill ends (and, for a
        // rising Level, how high it reaches), for SplitText.
        let mut fill_edge = x;
        let mut fill_top = f32::NEG_INFINITY;
        for part in &node.parts {
            match part {
                Part::Text {
                    text,
                    size,
                    role,
                    color,
                    emph: emph_color,
                    anchor,
                    dx,
                    dy,
                    valign,
                } => {
                    let color = match emph_color {
                        Some(e) => color.mix(*e, emph),
                        None => *color,
                    };
                    let place = (x, y, w, h, scale);
                    let text = (text.as_str(), *size, *role, *anchor, *dx, *dy, *valign);
                    self.draw_text(canvas, text, place, color.fade(opacity));
                }
                Part::SplitText {
                    text,
                    size,
                    role,
                    color,
                    on_fill,
                    anchor,
                    dx,
                } => {
                    let valign = if matches!(role, Role::Icon | Role::Logo) {
                        VAlign::Ink
                    } else {
                        VAlign::Cap
                    };
                    let place = (x, y, w, h, scale);
                    let text = (text.as_str(), *size, *role, *anchor, *dx, 0.0, valign);
                    let prev = canvas.clip();
                    let outer = prev.unwrap_or(ClipRect::band(0.0, canvas.width() as f32));
                    let edge = fill_edge.clamp(outer.x0, outer.x1);
                    let top = fill_top.clamp(outer.y0, outer.y1);
                    // On the fill, above a rising fill, and past its edge.
                    for (clip, color) in [
                        (
                            ClipRect {
                                x1: edge,
                                y0: top,
                                ..outer
                            },
                            on_fill,
                        ),
                        (
                            ClipRect {
                                x1: edge,
                                y1: top,
                                ..outer
                            },
                            color,
                        ),
                        (ClipRect { x0: edge, ..outer }, color),
                    ] {
                        let rows = clip.y0.max(y) < clip.y1.min(y + h);
                        if clip.x1 - clip.x0 >= 0.5 && rows {
                            canvas.set_clip(Some(clip));
                            self.draw_text(canvas, text, place, color.fade(opacity));
                        }
                    }
                    canvas.set_clip(prev);
                }
                Part::ClockTrack {
                    value,
                    daylight,
                    scrub,
                    color,
                    sun,
                    track,
                } => {
                    let left = x + 12.0;
                    let width = (w - 24.0).max(0.0);
                    let top = y + h - 5.0;
                    canvas.fill_rrect(left, top, width, 3.0, 1.5, track.fade(opacity));
                    if *scrub {
                        for [a, b] in daylight {
                            canvas.fill_rrect(
                                left + a * width,
                                top,
                                (b - a) * width,
                                3.0,
                                1.5,
                                sun.fade(opacity),
                            );
                        }
                        canvas.fill_circle(
                            left + value * width,
                            top + 1.5,
                            3.5,
                            color.fade(opacity),
                        );
                    } else {
                        canvas.fill_rrect(left, top, width * value, 3.0, 1.5, color.fade(opacity));
                    }
                }
                Part::IconImage { icon, size, dx } => {
                    canvas.draw_icon(&icon.0, x + dx * scale, y + h * 0.5, size * scale, opacity);
                }
                Part::Progress {
                    color,
                    track,
                    inset,
                    radius,
                    ..
                } => {
                    let value = a.progress.value.clamp(0.0, 1.0);
                    let (px, py, pw, ph) = (x + inset, y + inset, w - inset * 2.0, h - inset * 2.0);
                    if let Some(track) = track {
                        canvas.fill_rrect(px, py, pw, ph, *radius, track.fade(opacity));
                    }
                    let raw = pw * value;
                    let cap = if value > 0.0 { radius * 2.0 } else { 0.0 };
                    let fw = raw.max(cap).min(pw);
                    // Draining to empty, fade the end cap out rather than
                    // parking a sliver at the edge while the spring settles.
                    let fade = if a.progress.target <= 0.0 && raw < cap {
                        opacity * raw / cap
                    } else {
                        opacity
                    };
                    if fw > 0.5 && fade > 0.004 {
                        fill_edge = px + fw;
                        canvas.fill_rrect_vgradient(
                            px,
                            py,
                            fw,
                            ph,
                            *radius,
                            color.lighten(0.08).fade(fade),
                            color.fade(fade),
                        );
                    }
                }
                Part::Level { span, color, .. } => {
                    let shown = a.level.value.clamp(0.0, 1.0);
                    let r = node.fill.as_ref().map_or(0.0, |f| f.radius) * scale.min(1.0);
                    let value = a.progress.value.clamp(0.0, 1.0);
                    let place = (x, y, w, h, r);
                    let show = (shown, opacity);
                    (fill_edge, fill_top) = draw_level(canvas, place, value, *span, *color, show);
                }
                Part::Dot {
                    anchor,
                    dx,
                    dy,
                    r,
                    color,
                } => {
                    let cx = match anchor {
                        Anchor::Start => x + dx,
                        Anchor::Center => x + w / 2.0 + dx,
                        Anchor::End => x + w - dx,
                    };
                    canvas.fill_circle(cx, y + h / 2.0 + dy, *r * scale, color.fade(opacity));
                }
                Part::Cursor { dx, height, color } => {
                    let last_edit = self
                        .wifi_prompt
                        .as_ref()
                        .map(|p| p.last_edit)
                        .or(self.weather_search.as_ref().map(|w| w.last_edit));
                    let age = last_edit
                        .map(|t| self.now.saturating_duration_since(t).as_secs_f32())
                        .unwrap_or(0.0);
                    // Stay solid right after typing, then pulse smoothly.
                    let blink = if age < 0.45 {
                        1.0
                    } else {
                        0.5 + 0.5 * ((age - 0.45) * 5.8).cos()
                    };
                    let ch = height * scale;
                    let cx = x + dx * scale;
                    let cy = y + (h - ch) * 0.5;
                    canvas.fill_rrect(
                        cx,
                        cy,
                        2.5 * scale,
                        ch,
                        1.25,
                        color.alpha(color.a * blink * opacity),
                    );
                }
                Part::Shimmer { color, radius } => {
                    let since = self
                        .wifi_prompt
                        .as_ref()
                        .map(|p| p.opened_at)
                        .unwrap_or(self.entered_at);
                    let elapsed = self.now.saturating_duration_since(since).as_secs_f32();
                    let t = (elapsed * 0.85).fract();
                    let band_w = (w * 0.32).clamp(120.0, 280.0);
                    let sx = x - band_w + (w + band_w * 2.0) * t;
                    let prev_clip = canvas.clip();
                    let mut clip = ClipRect::band(x + 2.0, x + w - 2.0);
                    if let Some(row) = prev_clip {
                        clip.x0 = clip.x0.max(row.x0);
                        clip.x1 = clip.x1.min(row.x1);
                    }
                    if clip.x0 >= clip.x1 {
                        continue;
                    }
                    canvas.set_clip(Some(clip));
                    canvas.fill_rrect_hgradient(
                        sx,
                        y + 2.0,
                        band_w,
                        h - 4.0,
                        *radius,
                        &[
                            (0.0, color.alpha(0.0)),
                            (0.5, color.alpha(0.28 * opacity)),
                            (1.0, color.alpha(0.0)),
                        ],
                    );
                    // Slim glowing progress rail along the bottom edge.
                    let rail_w = (w * 0.24).clamp(90.0, 220.0);
                    let rx = x - rail_w + (w + rail_w) * t;
                    canvas.fill_rrect_hgradient(
                        rx,
                        y + h - 4.5,
                        rail_w,
                        3.0,
                        1.5,
                        &[
                            (0.0, color.alpha(0.0)),
                            (0.5, color.lighten(0.25).alpha(0.9 * opacity)),
                            (1.0, color.alpha(0.0)),
                        ],
                    );
                    canvas.set_clip(prev_clip);
                }
                Part::TouchFx => self.draw_touch_fx(canvas, (x, y, w, h), t, opacity),
                Part::Screensaver => {
                    // The desktop's screensaver terminal is black.
                    canvas.blend_rect(x, y, w, h, Rgba::BLACK.alpha(opacity));
                    if let Some(saver) = &self.saver
                        && let Some(frame) = &saver.frame
                    {
                        crate::screensaver::draw(
                            canvas,
                            frame,
                            &saver.geometry,
                            &saver.glyphs,
                            &self.fonts,
                            opacity,
                        );
                    }
                }
            }
        }
    }

    /// One run of text placed on a node. Text keeps its size while the node
    /// scales (entering, leaving, pressed): only its place follows the card.
    /// Scaling it would step through whole-pixel sizes and visibly pop, and
    /// would miss the glyph cache.
    #[allow(clippy::type_complexity)]
    fn draw_text(
        &self,
        canvas: &mut Canvas,
        (text, size, role, anchor, dx, dy, valign): (&str, f32, Role, Anchor, f32, f32, VAlign),
        (x, y, w, h, scale): (f32, f32, f32, f32, f32),
        color: Rgba,
    ) {
        let run = self.fonts.layout(text, size, role);
        let tx = match anchor {
            Anchor::Start => x + dx * scale,
            Anchor::Center if matches!(role, Role::Icon | Role::Logo) => {
                let (l, r) = self.fonts.ink_hbounds(text, size, role);
                x + w / 2.0 - (l + r) / 2.0 + dx * scale
            }
            Anchor::Center => x + (w - run.width) / 2.0 + dx * scale,
            Anchor::End => x + w - dx * scale - run.width,
        };
        // Unscaled, so the rounded baseline can't hop a pixel.
        canvas.draw_run(&self.fonts, &run, tx, y + h / 2.0 + dy, color, valign);
    }

    fn ripple(
        &self,
        canvas: &mut Canvas,
        ripple: &Ripple,
        shape: (f32, f32, f32, f32, f32),
        t: &Theme,
        offset: f32,
        opacity: f32,
    ) {
        let (x, y, w, h, _) = shape;
        let grow = ease_out(ripple.age / Ripple::GROW);
        let reach = (w.max(h)) * 1.1;
        let radius = 18.0 + (reach - 18.0) * grow;
        let fade = if ripple.released {
            (1.0 - ripple.fade / 0.32).max(0.0)
        } else {
            1.0
        };
        let alpha = 0.20 * fade * opacity;
        let cx = (x + ripple.x - offset).clamp(x, x + w);
        let cy = (y + ripple.y).clamp(y, y + h);
        canvas.fill_circle_in_rrect(cx, cy, radius, t.fg.alpha(alpha), shape);
    }
}
