//! Anti-aliased drawing onto an RGBA pixmap, plus XRGB8888 output.

use tiny_skia::{
    FillRule, GradientStop, LinearGradient, Mask, Paint, Path, PathBuilder, Pixmap, Point,
    RadialGradient, Shader, SpreadMode, Stroke, Transform,
};

use crate::{
    color::Rgba,
    text::{Fonts, Role, Run},
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum VAlign {
    /// Center the font's cap height (consistent baseline for labels).
    Cap,
    /// Center the run's actual ink (best for icon glyphs).
    Ink,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClipRect {
    pub x0: f32,
    pub x1: f32,
}

pub struct Canvas {
    pixmap: Pixmap,
    clip: Option<ClipRect>,
    clip_mask: Option<(ClipRect, Mask)>,
}

pub fn rrect_path(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<Path> {
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    let mut pb = PathBuilder::new();
    if r < 0.5 {
        pb.push_rect(tiny_skia::Rect::from_xywh(x, y, w, h)?);
        return pb.finish();
    }
    // Cubic approximation of quarter circles.
    let k = r * 0.552_284_8;
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.cubic_to(x + w - r + k, y, x + w, y + r - k, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.cubic_to(x + w, y + h - r + k, x + w - r + k, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.cubic_to(x + r - k, y + h, x, y + h - r + k, x, y + h - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    pb.close();
    pb.finish()
}

fn paint(color: Rgba) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color(color.to_skia());
    paint.anti_alias = true;
    paint
}

impl Canvas {
    pub fn new(width: u32, height: u32) -> Canvas {
        Canvas {
            pixmap: Pixmap::new(width.max(1), height.max(1)).expect("canvas size"),
            clip: None,
            clip_mask: None,
        }
    }

    pub fn width(&self) -> u32 {
        self.pixmap.width()
    }

    pub fn height(&self) -> u32 {
        self.pixmap.height()
    }

    pub fn pixmap(&self) -> &Pixmap {
        &self.pixmap
    }

    pub fn clear(&mut self, color: Rgba) {
        self.pixmap.fill(color.alpha(1.0).to_skia());
    }

    /// Restricts drawing to a horizontal band (full height).
    pub fn set_clip(&mut self, clip: Option<ClipRect>) {
        self.clip = clip;
    }

    pub fn clip(&self) -> Option<ClipRect> {
        self.clip
    }

    fn mask(&mut self) -> Option<&Mask> {
        let clip = self.clip?;
        let stale = !matches!(&self.clip_mask, Some((c, _)) if *c == clip);
        if stale {
            let mut mask = Mask::new(self.pixmap.width(), self.pixmap.height())?;
            let rect = tiny_skia::Rect::from_ltrb(
                clip.x0.max(0.0),
                0.0,
                clip.x1.max(clip.x0 + 1.0),
                self.pixmap.height() as f32,
            )?;
            mask.fill_path(
                &PathBuilder::from_rect(rect),
                FillRule::Winding,
                false,
                Transform::identity(),
            );
            self.clip_mask = Some((clip, mask));
        }
        self.clip_mask.as_ref().map(|(_, m)| m)
    }

    fn fill(&mut self, path: &Path, paint: &Paint) {
        let has_clip = self.clip.is_some();
        if has_clip {
            self.mask();
            let mask = self.clip_mask.as_ref().map(|(_, m)| m);
            self.pixmap
                .fill_path(path, paint, FillRule::Winding, Transform::identity(), mask);
        } else {
            self.pixmap
                .fill_path(path, paint, FillRule::Winding, Transform::identity(), None);
        }
    }

    pub(crate) fn draw_icon(&mut self, icon: &Pixmap, x: f32, cy: f32, size: f32, opacity: f32) {
        let scale = size / icon.width().max(icon.height()) as f32;
        let left = x + (size - icon.width() as f32 * scale) * 0.5;
        let top = cy - icon.height() as f32 * scale * 0.5;
        let transform = Transform::from_row(scale, 0.0, 0.0, scale, left, top);
        let paint = tiny_skia::PixmapPaint {
            opacity,
            quality: tiny_skia::FilterQuality::Bilinear,
            ..Default::default()
        };
        self.mask();
        let mask = self
            .clip
            .and_then(|_| self.clip_mask.as_ref().map(|(_, mask)| mask));
        self.pixmap
            .draw_pixmap(0, 0, icon.as_ref(), &paint, transform, mask);
    }

    pub fn fill_rrect(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, color: Rgba) {
        if color.a <= 0.002 {
            return;
        }
        if let Some(path) = rrect_path(x, y, w, h, r) {
            self.fill(&path, &paint(color));
        }
    }

    pub fn fill_rrect_vgradient(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        r: f32,
        top: Rgba,
        bottom: Rgba,
    ) {
        let Some(path) = rrect_path(x, y, w, h, r) else {
            return;
        };
        let shader = LinearGradient::new(
            Point::from_xy(0.0, y),
            Point::from_xy(0.0, y + h),
            vec![
                GradientStop::new(0.0, top.to_skia()),
                GradientStop::new(1.0, bottom.to_skia()),
            ],
            SpreadMode::Pad,
            Transform::identity(),
        );
        let mut p = Paint::default();
        p.anti_alias = true;
        p.shader = shader.unwrap_or(Shader::SolidColor(top.to_skia()));
        self.fill(&path, &p);
    }

    pub fn fill_hgradient(&mut self, x: f32, y: f32, w: f32, h: f32, left: Rgba, right: Rgba) {
        let Some(rect) = tiny_skia::Rect::from_xywh(x, y, w, h) else {
            return;
        };
        let shader = LinearGradient::new(
            Point::from_xy(x, 0.0),
            Point::from_xy(x + w, 0.0),
            vec![
                GradientStop::new(0.0, left.to_skia()),
                GradientStop::new(1.0, right.to_skia()),
            ],
            SpreadMode::Pad,
            Transform::identity(),
        );
        let mut p = Paint::default();
        p.shader = shader.unwrap_or(Shader::SolidColor(left.to_skia()));
        self.pixmap.fill_rect(rect, &p, Transform::identity(), None);
    }

    pub fn fill_rrect_hgradient(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        r: f32,
        stops: &[(f32, Rgba)],
    ) {
        if stops.is_empty() {
            return;
        }
        let Some(path) = rrect_path(x, y, w, h, r) else {
            return;
        };
        let grad_stops: Vec<GradientStop> = stops
            .iter()
            .map(|(pos, c)| GradientStop::new(pos.clamp(0.0, 1.0), c.to_skia()))
            .collect();
        let shader = LinearGradient::new(
            Point::from_xy(x, y),
            Point::from_xy(x + w, y),
            grad_stops,
            SpreadMode::Pad,
            Transform::identity(),
        );
        let p = Paint {
            anti_alias: true,
            shader: shader.unwrap_or(Shader::SolidColor(stops[0].1.to_skia())),
            ..Default::default()
        };
        self.fill(&path, &p);
    }

    pub fn fill_path_vgradient(&mut self, path: &Path, y0: f32, y1: f32, top: Rgba, bottom: Rgba) {
        let shader = LinearGradient::new(
            Point::from_xy(0.0, y0),
            Point::from_xy(0.0, y1),
            vec![
                GradientStop::new(0.0, top.to_skia()),
                GradientStop::new(1.0, bottom.to_skia()),
            ],
            SpreadMode::Pad,
            Transform::identity(),
        );
        let p = Paint {
            anti_alias: true,
            shader: shader.unwrap_or(Shader::SolidColor(top.to_skia())),
            ..Default::default()
        };
        self.fill(path, &p);
    }

    pub fn stroke_path_colored(&mut self, path: &Path, width: f32, color: Rgba) {
        if color.a <= 0.002 || width <= 0.0 {
            return;
        }
        let stroke = Stroke {
            width,
            line_cap: tiny_skia::LineCap::Round,
            line_join: tiny_skia::LineJoin::Round,
            ..Stroke::default()
        };
        let mask = if self.clip.is_some() {
            self.mask();
            self.clip_mask.as_ref().map(|(_, m)| m)
        } else {
            None
        };
        self.pixmap
            .stroke_path(path, &paint(color), &stroke, Transform::identity(), mask);
    }

    pub fn stroke_path_hgradient(
        &mut self,
        path: &Path,
        x0: f32,
        x1: f32,
        width: f32,
        stops: &[(f32, Rgba)],
    ) {
        if stops.is_empty() || width <= 0.0 {
            return;
        }
        let grad_stops: Vec<GradientStop> = stops
            .iter()
            .map(|(pos, c)| GradientStop::new(pos.clamp(0.0, 1.0), c.to_skia()))
            .collect();
        let shader = LinearGradient::new(
            Point::from_xy(x0, 0.0),
            Point::from_xy(x1, 0.0),
            grad_stops,
            SpreadMode::Pad,
            Transform::identity(),
        );
        let p = Paint {
            anti_alias: true,
            shader: shader.unwrap_or(Shader::SolidColor(stops[0].1.to_skia())),
            ..Default::default()
        };
        let stroke = Stroke {
            width,
            line_cap: tiny_skia::LineCap::Round,
            line_join: tiny_skia::LineJoin::Round,
            ..Stroke::default()
        };
        let mask = if self.clip.is_some() {
            self.mask();
            self.clip_mask.as_ref().map(|(_, m)| m)
        } else {
            None
        };
        self.pixmap
            .stroke_path(path, &p, &stroke, Transform::identity(), mask);
    }

    /// A soft elliptical glow: `color` at the center, fading out by radii
    /// `rx` × `ry` (a true gradient, so large glows have no visible rings).
    pub fn fill_radial_glow(&mut self, cx: f32, cy: f32, rx: f32, ry: f32, color: Rgba) {
        if color.a <= 0.002 || rx <= 0.5 || ry <= 0.5 {
            return;
        }
        let origin = Point::from_xy(0.0, 0.0);
        let Some(shader) = RadialGradient::new(
            origin,
            0.0,
            origin,
            1.0,
            vec![
                GradientStop::new(0.0, color.to_skia()),
                GradientStop::new(0.4, color.fade(0.45).to_skia()),
                GradientStop::new(1.0, color.alpha(0.0).to_skia()),
            ],
            SpreadMode::Pad,
            Transform::from_row(rx, 0.0, 0.0, ry, cx, cy),
        ) else {
            return;
        };
        let Some(rect) = tiny_skia::Rect::from_xywh(cx - rx, cy - ry, rx * 2.0, ry * 2.0) else {
            return;
        };
        let paint = Paint {
            shader,
            ..Default::default()
        };
        self.fill(&PathBuilder::from_rect(rect), &paint);
    }

    /// Soft radial-like glow built from concentric feathered circles.
    pub fn fill_glow_circle(&mut self, cx: f32, cy: f32, radius: f32, color: Rgba) {
        if radius <= 0.5 || color.a <= 0.002 {
            return;
        }
        for (scale, weight) in [(1.0, 0.14), (0.68, 0.24), (0.4, 0.42), (0.2, 0.78)] {
            self.fill_circle(cx, cy, radius * scale, color.fade(weight));
        }
    }

    pub fn stroke_rrect(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        r: f32,
        width: f32,
        color: Rgba,
    ) {
        if color.a <= 0.002 {
            return;
        }
        let inset = width / 2.0;
        let Some(path) = rrect_path(
            x + inset,
            y + inset,
            w - width,
            h - width,
            (r - inset).max(0.0),
        ) else {
            return;
        };
        let stroke = Stroke {
            width,
            ..Stroke::default()
        };
        let mask = if self.clip.is_some() {
            self.mask();
            self.clip_mask.as_ref().map(|(_, m)| m)
        } else {
            None
        };
        self.pixmap
            .stroke_path(&path, &paint(color), &stroke, Transform::identity(), mask);
    }

    /// Blends a pixel-snapped rectangle without a path: the fast path for
    /// the thousands of dots and sparks in the Touch ID effect.
    pub fn blend_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: Rgba) {
        if color.a <= 0.004 {
            return;
        }
        let width = self.pixmap.width() as i32;
        let height = self.pixmap.height() as i32;
        let (clip0, clip1) = match self.clip {
            Some(c) => (c.x0.max(0.0) as i32, (c.x1 as i32).min(width)),
            None => (0, width),
        };
        let x0 = (x.round() as i32).max(clip0);
        let x1 = ((x + w).round() as i32).min(clip1);
        let y0 = (y.round() as i32).max(0);
        let y1 = ((y + h).round() as i32).min(height);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let [cr, cg, cb] =
            [color.r, color.g, color.b].map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u32);
        let a = (color.a.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
        let inv = 255 - a;
        let data = self.pixmap.data_mut();
        for py in y0..y1 {
            for px in x0..x1 {
                let i = ((py * width + px) * 4) as usize;
                data[i] = ((cr * a + data[i] as u32 * inv + 127) / 255) as u8;
                data[i + 1] = ((cg * a + data[i + 1] as u32 * inv + 127) / 255) as u8;
                data[i + 2] = ((cb * a + data[i + 2] as u32 * inv + 127) / 255) as u8;
                data[i + 3] = ((255 * a + data[i + 3] as u32 * inv + 127) / 255) as u8;
            }
        }
    }

    /// Copies row `y` of `src` between `x0` and `x1` over this canvas,
    /// anti-aliasing the partly covered end pixels. Both canvases must be
    /// the same size.
    pub fn copy_span(&mut self, src: &Canvas, y: u32, x0: f32, x1: f32) {
        let width = self.pixmap.width();
        if src.pixmap.width() != width || y >= self.pixmap.height().min(src.pixmap.height()) {
            return;
        }
        let x0 = x0.max(0.0);
        let x1 = x1.min(width as f32);
        if x1 <= x0 {
            return;
        }
        let width = width as usize;
        let row = y as usize * width * 4;
        let from = &src.pixmap.data()[row..row + width * 4];
        let to = &mut self.pixmap.data_mut()[row..row + width * 4];
        let blend = |to: &mut [u8], px: usize| {
            let coverage = (x1.min(px as f32 + 1.0) - x0.max(px as f32)).clamp(0.0, 1.0);
            let k = (coverage * 256.0) as u32;
            for i in px * 4..px * 4 + 4 {
                to[i] = ((from[i] as u32 * k + to[i] as u32 * (256 - k)) >> 8) as u8;
            }
        };
        // Pixels the span touches, and the ones it fully covers.
        let (first, last) = (x0.floor() as usize, (x1.ceil() as usize).min(width));
        let (full0, full1) = (x0.ceil() as usize, (x1.floor() as usize).min(width));
        if full1 <= full0 {
            (first..last).for_each(|px| blend(to, px));
            return;
        }
        to[full0 * 4..full1 * 4].copy_from_slice(&from[full0 * 4..full1 * 4]);
        (first..full0).for_each(|px| blend(to, px));
        (full1..last).for_each(|px| blend(to, px));
    }

    /// Blends a pixel-snapped rectangle with a horizontal gradient through
    /// `stops` (positions 0–1, increasing). Much faster than a gradient
    /// shader for large areas: each column's color is computed once.
    pub fn blend_hgradient(&mut self, x: f32, y: f32, w: f32, h: f32, stops: &[(f32, Rgba)]) {
        if w <= 0.0 || stops.is_empty() {
            return;
        }
        let width = self.pixmap.width() as i32;
        let height = self.pixmap.height() as i32;
        let (clip0, clip1) = match self.clip {
            Some(c) => (c.x0.max(0.0) as i32, (c.x1 as i32).min(width)),
            None => (0, width),
        };
        let x0 = (x.round() as i32).max(clip0);
        let x1 = ((x + w).round() as i32).min(clip1);
        let y0 = (y.round() as i32).max(0);
        let y1 = ((y + h).round() as i32).min(height);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let at = |u: f32| -> Rgba {
            let first = stops[0];
            if u <= first.0 {
                return first.1;
            }
            for pair in stops.windows(2) {
                let (p0, c0) = pair[0];
                let (p1, c1) = pair[1];
                if u <= p1 {
                    let k = if p1 > p0 { (u - p0) / (p1 - p0) } else { 1.0 };
                    return c0.mix(c1, k);
                }
            }
            stops[stops.len() - 1].1
        };
        let data = self.pixmap.data_mut();
        for px in x0..x1 {
            let color = at((px as f32 + 0.5 - x) / w);
            let a = (color.a.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
            if a == 0 {
                continue;
            }
            let [cr, cg, cb] =
                [color.r, color.g, color.b].map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u32);
            let inv = 255 - a;
            for py in y0..y1 {
                let i = ((py * width + px) * 4) as usize;
                data[i] = ((cr * a + data[i] as u32 * inv + 127) / 255) as u8;
                data[i + 1] = ((cg * a + data[i + 1] as u32 * inv + 127) / 255) as u8;
                data[i + 2] = ((cb * a + data[i + 2] as u32 * inv + 127) / 255) as u8;
                data[i + 3] = ((255 * a + data[i + 3] as u32 * inv + 127) / 255) as u8;
            }
        }
    }

    pub fn fill_circle(&mut self, cx: f32, cy: f32, r: f32, color: Rgba) {
        if r <= 0.0 || color.a <= 0.002 {
            return;
        }
        if let Some(path) = PathBuilder::from_circle(cx, cy, r) {
            self.fill(&path, &paint(color));
        }
    }

    /// Draws a circle clipped to a rounded rectangle (touch ripples).
    pub fn fill_circle_in_rrect(
        &mut self,
        cx: f32,
        cy: f32,
        radius: f32,
        color: Rgba,
        rect: (f32, f32, f32, f32, f32),
    ) {
        if radius <= 0.0 || color.a <= 0.002 {
            return;
        }
        let (x, y, w, h, r) = rect;
        let Some(shape) = rrect_path(x, y, w, h, r) else {
            return;
        };
        let Some(circle) = PathBuilder::from_circle(cx, cy, radius) else {
            return;
        };
        let Some(mut mask) = Mask::new(self.pixmap.width(), self.pixmap.height()) else {
            return;
        };
        mask.fill_path(&shape, FillRule::Winding, true, Transform::identity());
        if let Some(clip) = self.clip {
            // Intersect with the band clip by clearing columns outside it.
            let width = self.pixmap.width() as usize;
            let data = mask.data_mut();
            for (i, value) in data.iter_mut().enumerate() {
                let px = (i % width) as f32;
                if px < clip.x0 || px >= clip.x1 {
                    *value = 0;
                }
            }
        }
        self.pixmap.fill_path(
            &circle,
            &paint(color),
            FillRule::Winding,
            Transform::identity(),
            Some(&mask),
        );
    }

    /// Draws a laid-out run with its left edge at `x`, vertically centered
    /// on `cy`. Returns the run width.
    pub fn draw_run(
        &mut self,
        fonts: &Fonts,
        run: &Run,
        x: f32,
        cy: f32,
        color: Rgba,
        valign: VAlign,
    ) -> f32 {
        if color.a <= 0.004 || run.glyphs.is_empty() {
            return run.width;
        }
        let baseline = match valign {
            VAlign::Cap => cy + fonts.cap_height(Fonts::role_of(run), run.size) / 2.0,
            VAlign::Ink => match fonts.ink_extent(run) {
                Some((top, bottom)) => cy - (top + bottom) as f32 / 2.0,
                None => cy + run.size * 0.35,
            },
        };
        let baseline = baseline.round() as i32;
        let [cr, cg, cb] =
            [color.r, color.g, color.b].map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u32);
        let alpha = (color.a.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
        let (clip0, clip1) = match self.clip {
            Some(c) => (
                c.x0.max(0.0) as i32,
                c.x1.min(self.pixmap.width() as f32) as i32,
            ),
            None => (0, self.pixmap.width() as i32),
        };
        let width = self.pixmap.width() as i32;
        let height = self.pixmap.height() as i32;
        let data = self.pixmap.data_mut();
        for g in &run.glyphs {
            let pen = x + g.x;
            let whole = pen.floor();
            let phase = ((pen - whole) * 4.0).floor().clamp(0.0, 3.0) as u8;
            let Some(bitmap) = fonts.bitmap(g, run.size, phase) else {
                continue;
            };
            let ox = whole as i32 + bitmap.left;
            let oy = baseline + bitmap.top;
            for row in 0..bitmap.height as i32 {
                let py = oy + row;
                if py < 0 || py >= height {
                    continue;
                }
                for col in 0..bitmap.width as i32 {
                    let px = ox + col;
                    if px < clip0 || px >= clip1 || px < 0 || px >= width {
                        continue;
                    }
                    let i = ((py * width + px) * 4) as usize;
                    let src = (row * bitmap.width as i32 + col) as usize;
                    if let Some(rgba) = &bitmap.rgba {
                        // Color glyph: premultiplied source over, faded by
                        // the run's alpha.
                        let s = &rgba[src * 4..src * 4 + 4];
                        let sa = s[3] as u32 * alpha / 255;
                        if sa == 0 {
                            continue;
                        }
                        let inv = 255 - sa;
                        for c in 0..3 {
                            let sc = s[c] as u32 * alpha / 255;
                            data[i + c] =
                                (sc + (data[i + c] as u32 * inv + 127) / 255).min(255) as u8;
                        }
                        data[i + 3] = (sa + (data[i + 3] as u32 * inv + 127) / 255).min(255) as u8;
                        continue;
                    }
                    let cov = bitmap.coverage[src] as u32;
                    if cov == 0 {
                        continue;
                    }
                    let a = (cov * alpha + 127) / 255;
                    let inv = 255 - a;
                    data[i] = ((cr * a + data[i] as u32 * inv + 127) / 255) as u8;
                    data[i + 1] = ((cg * a + data[i + 1] as u32 * inv + 127) / 255) as u8;
                    data[i + 2] = ((cb * a + data[i + 2] as u32 * inv + 127) / 255) as u8;
                    data[i + 3] = ((255 * a + data[i + 3] as u32 * inv + 127) / 255) as u8;
                }
            }
        }
        run.width
    }

    #[allow(clippy::too_many_arguments)]
    pub fn text(
        &mut self,
        fonts: &Fonts,
        text: &str,
        x: f32,
        cy: f32,
        size: f32,
        role: Role,
        color: Rgba,
        valign: VAlign,
    ) -> f32 {
        let run = fonts.layout(text, size, role);
        self.draw_run(fonts, &run, x, cy, color, valign)
    }

    /// Writes the canvas as XRGB8888 (`0x00RRGGBB` native u32) scaled by
    /// `brightness`.
    pub fn write_xrgb(&self, out: &mut [u32], brightness: f32) {
        let data = self.pixmap.data();
        let scale = (brightness.clamp(0.0, 1.0) * 256.0) as u32;
        if scale >= 256 {
            for (dst, src) in out.iter_mut().zip(data.chunks_exact(4)) {
                *dst = (src[0] as u32) << 16 | (src[1] as u32) << 8 | src[2] as u32;
            }
        } else {
            for (dst, src) in out.iter_mut().zip(data.chunks_exact(4)) {
                let r = (src[0] as u32 * scale) >> 8;
                let g = (src[1] as u32 * scale) >> 8;
                let b = (src[2] as u32 * scale) >> 8;
                *dst = r << 16 | g << 8 | b;
            }
        }
    }

    /// Premultiplied RGBA bytes (opaque), for previews.
    pub fn rgba(&self) -> &[u8] {
        self.pixmap.data()
    }

    pub fn save_png(&self, path: &std::path::Path) -> Result<(), String> {
        self.pixmap.save_png(path).map_err(|e| e.to_string())
    }
}
