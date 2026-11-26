//! A swipeable row for the clock, its editors and the six-week calendar.
use super::*;
use crate::{state::IconImage, world_clock::Item};
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Transform};

fn text_line(label: &str, size: f32, color: Rgba, role: Role, dx: f32, dy: f32) -> Part {
    Part::Text {
        text: label.into(),
        size,
        role,
        color,
        emph: None,
        anchor: Anchor::Start,
        dx,
        dy,
        valign: VAlign::Cap,
    }
}

impl Scene {
    pub(super) fn panel_clock(&self, x: &mut f32, nodes: &mut Vec<Node>) -> Option<(f32, f32)> {
        let start = *x;
        if !self.world.error.is_empty() {
            self.row_message(x, nodes, &self.world.error);
            let node = self.list_chip(
                Key::new("time/desktop"),
                x,
                (glyph::MONITOR, self.theme.fg_dim),
                "Show on desktop",
                None,
                false,
                Some(Cmd::Do(Action::Shell(
                    "omarchy-shell shell summon omarchy.elsewhen".into(),
                ))),
            );
            nodes.push(node);
            for city in self.layout.world_cities() {
                if let Ok(zone) = city.zone.parse::<chrono_tz::Tz>() {
                    let time = format_qt("HH:mm", self.utc.with_timezone(&zone).naive_local());
                    let node = self.info(
                        Key::new("time/fallback").with(&city.zone).with(&city.name),
                        0.0,
                        (glyph::GLOBE, self.theme.fg_dim),
                        &city.name,
                        &time,
                    );
                    self.row_node(x, nodes, node);
                }
            }
        }
        if self.world.items.is_empty() {
            // The model loads asynchronously. The date is available immediately.
            let node = self.info(
                Key::new("time/date"),
                0.0,
                ("󰃭", self.theme.accent),
                &self.clock_text(),
                &format_qt("dddd, d MMMM", self.local),
            );
            self.row_node(x, nodes, node);
        }
        for item in &self.world.items {
            let node = self.world_card(item);
            self.row_node(x, nodes, node);
        }
        Some((start, *x - self.m().gap))
    }

    fn world_card(&self, item: &Item) -> Node {
        let m = self.m();
        let t = &self.theme;
        let key = Key::new("time").with(&self.world.view).with(&item.id);
        if !item.week.is_empty() {
            let cell = (m.small * 1.55).max(25.0);
            let prefix = self.tw(&item.title, m.small, Role::Bold) + 21.0;
            let width = prefix + cell * item.week.len() as f32 + 6.0;
            let mut node = Node::new(key, Rect::new(0.0, m.inset, width, m.bh))
                .fill(self.surface())
                .part(text_line(
                    &item.title,
                    m.small,
                    t.accent,
                    Role::Bold,
                    8.0,
                    0.0,
                ));
            for (i, day) in item.week.iter().enumerate() {
                let center = prefix + (i as f32 + 0.5) * cell;
                let color = if day.dim {
                    t.fg_faint
                } else if day.weekend {
                    t.fg_dim
                } else {
                    t.fg
                };
                let caption = m.small * 0.84;
                node.parts.push(text_line(
                    &day.label,
                    caption,
                    t.fg_dim,
                    Role::Text,
                    center - self.tw(&day.label, caption, Role::Text) / 2.0,
                    -11.0,
                ));
                if day.today {
                    node.parts.push(Part::Dot {
                        anchor: Anchor::Start,
                        dx: center,
                        dy: 10.0,
                        r: (cell * 0.46).min(13.0),
                        color: t.accent,
                    });
                }
                node.parts.push(text_line(
                    &day.day,
                    m.small,
                    if day.today { t.on_accent } else { color },
                    Role::Bold,
                    center - self.tw(&day.day, m.small, Role::Bold) / 2.0,
                    10.0,
                ));
            }
            return node;
        }
        let mut fill = if item.action || item.drag {
            self.surface()
        } else {
            Fill::solid(t.well, m.radius)
        };
        if item.active {
            fill.border = Some(t.accent);
        }
        let mut node = Node::new(key, Rect::new(0.0, m.inset, 0.0, m.bh)).fill(fill);
        let mut left = 14.0;
        if !item.land.is_empty() {
            if let Some(icon) = globe_image(item, t) {
                node.parts.push(Part::IconImage {
                    icon,
                    size: m.bh - 2.0,
                    dx: left,
                });
            }
            left += m.bh + 8.0;
        } else if !item.icon.is_empty() {
            let size = m.icon * 0.87;
            let (il, ir) = self.fonts.ink_hbounds(&item.icon, size, Role::Icon);
            node.parts.push(Part::icon(
                &item.icon,
                size,
                t.accent,
                Anchor::Start,
                left - il,
            ));
            left += ir - il + 10.0;
        }
        for (i, face) in item.avatars.iter().enumerate() {
            if let Some(icon) = face {
                node.parts.push(Part::IconImage {
                    icon: icon.clone(),
                    size: 24.0,
                    dx: left,
                });
            } else if let Some(initial) = item.initials.get(i) {
                let size = m.small * 0.86;
                node.parts.push(Part::Dot {
                    anchor: Anchor::Start,
                    dx: left + 12.0,
                    dy: 0.0,
                    r: 12.0,
                    color: t.accent.alpha(0.3).over(t.well),
                });
                node.parts.push(text_line(
                    initial,
                    size,
                    t.fg,
                    Role::Bold,
                    left + 12.0 - self.tw(initial, size, Role::Bold) / 2.0,
                    0.0,
                ));
            }
            left += 21.0;
        }
        if !item.avatars.is_empty() {
            left += 9.0;
        }
        let title_size = if item.sub.is_empty() {
            m.label
        } else {
            m.small * 1.2
        };
        let title = if self.world.editing && item.id == "field" {
            self.fit_tail(&item.title, 590.0, title_size, Role::Bold)
        } else {
            self.fit(&item.title, 400.0, title_size, Role::Bold)
        };
        let sub = self.fit(&item.sub, 430.0, m.small, Role::Text);
        let width = (left
            + self
                .tw(&title, title_size, Role::Bold)
                .max(self.tw(&sub, m.small, Role::Text))
            + 14.0)
            .max(if item.drag { 300.0 } else { 58.0 })
            .ceil();
        node.rect.w = width;
        let meter = item.fraction.is_some();
        let top = if sub.is_empty() { 0.0 } else { -10.0 };
        node.parts.push(text_line(
            &title,
            title_size,
            t.fg,
            Role::Bold,
            left,
            top - if meter { 2.0 } else { 0.0 },
        ));
        if !sub.is_empty() {
            node.parts.push(text_line(
                &sub,
                m.small,
                t.fg_dim,
                Role::Text,
                left,
                if meter { 7.0 } else { 11.0 },
            ));
        }
        if let Some(value) = item.fraction {
            node.parts.push(Part::ClockTrack {
                value,
                daylight: item.daylight.clone(),
                scrub: item.drag,
                color: t.accent,
                sun: t.yellow,
                track: t.fg_faint.alpha(0.45),
            });
        }
        if item.drag {
            node = node
                .hit(Cmd::WorldTap(item.id.clone()), HitKind::WorldDrag)
                .no_ripple();
        } else if item.action {
            node = node.tap(Cmd::WorldTap(item.id.clone()));
        }
        node
    }
}

// Render the installed Elsewhen coastline at the strip's pixel size.
fn globe_image(item: &Item, theme: &crate::Theme) -> Option<IconImage> {
    let mut image = Pixmap::new(60, 60)?;
    let mut paint = Paint::default();
    let set_color = |paint: &mut Paint, c: Rgba| {
        paint.set_color(tiny_skia::Color::from_rgba(c.r, c.g, c.b, c.a).unwrap());
    };
    let disc = PathBuilder::from_circle(30.0, 30.0, 29.0)?;
    set_color(&mut paint, theme.accent.alpha(0.20).over(theme.well));
    image.fill_path(
        &disc,
        &paint,
        FillRule::Winding,
        Transform::identity(),
        None,
    );
    set_color(&mut paint, theme.fg_dim);
    for ring in &item.land {
        let Some(([x, y], rest)) = ring.split_first() else {
            continue;
        };
        let mut shape = PathBuilder::new();
        shape.move_to(30.0 + x * 29.0, 30.0 + y * 29.0);
        for [x, y] in rest {
            shape.line_to(30.0 + x * 29.0, 30.0 + y * 29.0);
        }
        shape.close();
        if let Some(shape) = shape.finish() {
            image.fill_path(
                &shape,
                &paint,
                FillRule::Winding,
                Transform::identity(),
                None,
            );
        }
    }
    if let Some([sx, sy, sz]) = item.light {
        for (i, pixel) in image.pixels_mut().iter_mut().enumerate() {
            let x = ((i % 60) as f32 + 0.5 - 30.0) / 29.0;
            let y = (30.0 - (i / 60) as f32 - 0.5) / 29.0;
            let z = (1.0 - x * x - y * y).max(0.0).sqrt();
            let shade = (0.68 + 1.7 * (x * sx + y * sy + z * sz)).clamp(0.45, 1.0);
            *pixel = tiny_skia::PremultipliedColorU8::from_rgba(
                (f32::from(pixel.red()) * shade) as u8,
                (f32::from(pixel.green()) * shade) as u8,
                (f32::from(pixel.blue()) * shade) as u8,
                pixel.alpha(),
            )
            .unwrap();
        }
    }
    set_color(&mut paint, theme.yellow);
    for [x, y] in &item.markers {
        if let Some(pin) = PathBuilder::from_circle(30.0 + x * 29.0, 30.0 + y * 29.0, 2.4) {
            image.fill_path(&pin, &paint, FillRule::Winding, Transform::identity(), None);
        }
    }
    Some(IconImage(image.into()))
}
