//! The Weather panel: the report Omarchy's weather panel shows (conditions,
//! feels like, wind, humidity, forecast) plus the next hours, rain chance,
//! sunrise, sunset and UV — and its location search, typed on the keyboard.

use chrono::{NaiveDateTime, NaiveTime};

use super::*;
use crate::weather::{self, Location, Report};

impl Scene {
    fn clock_12h(&self) -> bool {
        let format = self.clock_format();
        format.contains("AP") || format.contains("ap")
    }

    fn weather_time(&self, time: NaiveTime) -> String {
        let fmt = if self.clock_12h() {
            "%-I:%M %p"
        } else {
            "%H:%M"
        };
        time.format(fmt).to_string()
    }

    fn weather_hour(&self, time: NaiveDateTime) -> String {
        let fmt = if self.clock_12h() { "%-I %p" } else { "%H:%M" };
        time.format(fmt).to_string()
    }

    pub(super) fn panel_weather(&self, x: &mut f32, nodes: &mut Vec<Node>) -> Option<(f32, f32)> {
        if self.weather_search.is_some() {
            return self.panel_weather_search(x, nodes);
        }
        let m = self.m();
        let t = self.theme.clone();
        let key = Key::new("p/weather");
        let row_start = *x;
        let weather = self.state.weather.as_ref();
        let loading = weather.is_some_and(|w| w.loading);
        let report = weather.filter(|_| !loading).and_then(|w| w.report.as_ref());

        if loading {
            let text = match weather
                .map(|w| w.place.name.as_str())
                .filter(|n| !n.is_empty())
            {
                Some(name) => format!("Loading {name}…"),
                None => "Finding your location…".into(),
            };
            let w = (self.tw(&text, m.label, Role::Text) + m.pad * 2.0)
                .max(300.0)
                .round();
            let node = Node::new(key.with("loading"), Rect::new(0.0, m.inset, w, m.bh))
                .fill(Fill::solid(t.well, m.radius))
                .part(Part::Shimmer {
                    color: t.accent,
                    radius: m.radius,
                })
                .part(Part::label(text, m.label, t.fg_dim, Anchor::Start, m.pad));
            self.row_node(x, nodes, node);
        } else if let Some(r) = report {
            self.row_node(x, nodes, self.weather_hero(key.with("now"), r));
            if weather.is_some_and(|w| w.stale) {
                let text = format!("Last report {}", self.weather_time(r.time.time()));
                self.row_message(x, nodes, &text);
            }
        } else if let Some(status) = weather.map(|w| w.status.as_str()).filter(|s| !s.is_empty()) {
            let node = self.chip(key, None, Some((status, t.fg)), self.flat());
            self.row_node(x, nodes, node);
            if weather.is_some_and(|w| w.stale) {
                self.row_message(x, nodes, "Last report");
            }
        } else if weather.is_none() {
            self.row_message(x, nodes, "Fetching forecast…");
        } else {
            self.row_message(x, nodes, "Weather unavailable");
        }

        self.row_node(x, nodes, self.weather_location_chip(key.with("place")));

        if let Some(r) = report {
            let stat = |x: &mut f32, nodes: &mut Vec<Node>, label: &str, value: &str, sample| {
                self.net_stat(
                    key.with("stat").with(label),
                    x,
                    label,
                    t.fg_dim,
                    value,
                    t.fg,
                    sample,
                    None,
                    None,
                    nodes,
                );
            };
            stat(x, nodes, "Feels like", &r.temp(r.now.feels), "-10°C");
            stat(x, nodes, "Wind", &r.wind(), "10 km/h");
            stat(
                x,
                nodes,
                "Humidity",
                &format!("{}%", r.now.humidity.round() as i32),
                "100%",
            );
            if let Some(rain) = r.rain_chance(12) {
                stat(x, nodes, "Rain", &format!("{rain}%"), "100%");
            }
            *x += m.gap;

            for day in r.upcoming_days().take(6) {
                let node = self.weather_day(key.with("day").with(&day.date.to_string()), r, day);
                self.row_node(x, nodes, node);
            }
            *x += m.gap;

            if let Some(node) = self.weather_hours(key.with("hours"), r) {
                self.row_node(x, nodes, node);
                *x += m.gap;
            }

            if let Some(today) = r.today() {
                if let Some(rise) = today.sunrise {
                    stat(x, nodes, "Sunrise", &self.weather_time(rise), "00:00");
                }
                if let Some(set) = today.sunset {
                    stat(x, nodes, "Sunset", &self.weather_time(set), "00:00");
                }
                if let Some(uv) = today.uv {
                    let value = format!("{} · {}", uv.round() as i32, weather::uv_level(uv));
                    stat(x, nodes, "UV index", &value, "0 · Low");
                }
                *x += m.gap;
            }
        }

        nodes.push(self.list_chip(
            key.with("refresh"),
            x,
            (glyph::UPDATE, t.fg_dim),
            "Refresh",
            None,
            false,
            Some(Cmd::Do(Action::RefreshWeather)),
        ));
        nodes.push(self.list_chip(
            key.with("desktop"),
            x,
            (glyph::MONITOR, t.fg_dim),
            "Show on desktop",
            None,
            false,
            Some(Cmd::Do(Action::Shell(
                "omarchy-shell shell summon omarchy.weather".into(),
            ))),
        ));
        Some((row_start, *x - m.gap))
    }

    /// Big condition icon and temperature, with the condition and today's
    /// high and low beside them.
    fn weather_hero(&self, key: Key, r: &Report) -> Node {
        let m = self.m();
        let t = &self.theme;
        let glyph = weather::icon(r.now.code, !r.now.day);
        let icon_size = (m.icon * 1.2).round();
        let (il, ir) = self.fonts.ink_hbounds(glyph, icon_size, Role::Icon);
        let temp = r.temp(r.now.temp);
        let temp_size = (m.label * 1.3).round();
        let temp_w = self.tw(&temp, temp_size, Role::Bold);
        let condition = weather::condition(r.now.code, r.now.day);
        let range = r
            .today()
            .map(|d| format!("H {}  L {}", r.deg(d.max), r.deg(d.min)))
            .unwrap_or_default();
        let title_size = m.small * 1.15;
        let text_w =
            self.tw(condition, title_size, Role::Bold)
                .max(self.tw(&range, m.small, Role::Text));
        let x_temp = 16.0 + (ir - il) + 12.0;
        let x_text = x_temp + temp_w + 16.0;
        let two_lines = !range.is_empty();
        let mut node = Node::new(
            key,
            Rect::new(0.0, m.inset, (x_text + text_w + m.pad).round(), m.bh),
        )
        .fill(Fill::solid(t.well, m.radius))
        .part(Part::icon(
            glyph,
            icon_size,
            t.accent,
            Anchor::Start,
            16.0 - il,
        ))
        .part(Part::Text {
            text: temp,
            size: temp_size,
            role: Role::Bold,
            color: t.fg,
            emph: None,
            anchor: Anchor::Start,
            dx: x_temp,
            dy: 0.0,
            valign: VAlign::Cap,
        })
        .part(Part::Text {
            text: condition.into(),
            size: title_size,
            role: Role::Bold,
            color: t.fg,
            emph: None,
            anchor: Anchor::Start,
            dx: x_text,
            dy: if two_lines { -10.0 } else { 0.0 },
            valign: VAlign::Cap,
        });
        if two_lines {
            node = node.part(Part::Text {
                text: range,
                size: m.small,
                role: Role::Text,
                color: t.fg_dim,
                emph: None,
                anchor: Anchor::Start,
                dx: x_text,
                dy: 11.0,
                valign: VAlign::Cap,
            });
        }
        node
    }

    /// Where the report is for; tap to search for another place or Auto.
    fn weather_location_chip(&self, key: Key) -> Node {
        let m = self.m();
        let t = &self.theme;
        let place = self.state.weather.as_ref().map(|w| &w.place);
        let auto = place.is_none_or(|p| p.auto);
        let name = place
            .map(|p| p.name.as_str())
            .filter(|n| !n.is_empty())
            .unwrap_or("Set location");
        let title = self.fit(name, 240.0, m.small * 1.15, Role::Bold);
        let subtitle = if auto {
            "Auto-detected"
        } else {
            "Saved location"
        };
        let icon_size = m.icon * 0.95;
        let (il, ir) = self
            .fonts
            .ink_hbounds(glyph::MAP_MARKER, icon_size, Role::Icon);
        let x_text = 14.0 + (ir - il) + 10.0;
        let text_w =
            self.tw(&title, m.small * 1.15, Role::Bold)
                .max(self.tw(subtitle, m.small, Role::Text));
        let chevron = m.icon * 0.75;
        let chevron_w = self.iw(glyph::CHEVRON, chevron, Role::Icon);
        let w = (x_text + text_w + 10.0 + chevron_w + m.pad - 4.0).round();
        Node::new(key, Rect::new(0.0, m.inset, w, m.bh))
            .fill(self.surface())
            .part(Part::icon(
                glyph::MAP_MARKER,
                icon_size,
                t.accent,
                Anchor::Start,
                14.0 - il,
            ))
            .part(Part::Text {
                text: title,
                size: m.small * 1.15,
                role: Role::Bold,
                color: t.fg,
                emph: None,
                anchor: Anchor::Start,
                dx: x_text,
                dy: -10.0,
                valign: VAlign::Cap,
            })
            .part(Part::Text {
                text: subtitle.into(),
                size: m.small,
                role: Role::Text,
                color: t.fg_dim,
                emph: None,
                anchor: Anchor::Start,
                dx: x_text,
                dy: 11.0,
                valign: VAlign::Cap,
            })
            .part(Part::icon(
                glyph::CHEVRON,
                chevron,
                t.fg_faint,
                Anchor::End,
                m.pad - 4.0,
            ))
            .tap(Cmd::WeatherSearch)
    }

    /// One forecast day: icon, day name, high and low, and rain chance when
    /// rain is likely.
    fn weather_day(&self, key: Key, r: &Report, day: &weather::Day) -> Node {
        let m = self.m();
        let t = &self.theme;
        let glyph = weather::icon(day.code, false);
        let icon_size = m.icon * 0.95;
        let (il, ir) = self.fonts.ink_hbounds(glyph, icon_size, Role::Icon);
        let x_text = m.pad - 2.0 + (ir - il) + 12.0;
        let name = day.date.format("%A").to_string();
        let (hi, lo) = (r.deg(day.max), r.deg(day.min));
        let size = m.label * 0.9;
        let hi_w = self.tw(&hi, size, Role::Bold);
        let lo_w = self.tw(&lo, size, Role::Text);
        let rain = day.rain.filter(|&p| p >= 20).map(|p| format!("{p}%"));
        let rain_icon = m.small * 1.05;
        let rain_w = rain.as_ref().map_or(0.0, |text| {
            12.0 + self.iw(glyph::UMBRELLA, rain_icon, Role::Icon)
                + 3.0
                + self.tw(text, m.small, Role::Text)
        });
        let line_w = hi_w + 8.0 + lo_w + rain_w;
        let w = (x_text + line_w.max(self.tw(&name, m.small, Role::Text)) + m.pad).round();
        let mut node = Node::new(key, Rect::new(0.0, m.inset, w, m.bh))
            .fill(Fill::solid(t.well, m.radius))
            .part(Part::icon(
                glyph,
                icon_size,
                t.fg,
                Anchor::Start,
                m.pad - 2.0 - il,
            ))
            .part(Part::Text {
                text: name,
                size: m.small,
                role: Role::Text,
                color: t.fg_dim,
                emph: None,
                anchor: Anchor::Start,
                dx: x_text,
                dy: -11.0,
                valign: VAlign::Cap,
            })
            .part(Part::Text {
                text: hi,
                size,
                role: Role::Bold,
                color: t.fg,
                emph: None,
                anchor: Anchor::Start,
                dx: x_text,
                dy: 10.0,
                valign: VAlign::Cap,
            })
            .part(Part::Text {
                text: lo,
                size,
                role: Role::Text,
                color: t.fg_dim,
                emph: None,
                anchor: Anchor::Start,
                dx: x_text + hi_w + 8.0,
                dy: 10.0,
                valign: VAlign::Cap,
            });
        if let Some(text) = rain {
            let rx = x_text + hi_w + 8.0 + lo_w + 12.0;
            let (ul, ur) = self
                .fonts
                .ink_hbounds(glyph::UMBRELLA, rain_icon, Role::Icon);
            node = node
                .part(Part::Text {
                    text: glyph::UMBRELLA.into(),
                    size: rain_icon,
                    role: Role::Icon,
                    color: t.cyan,
                    emph: None,
                    anchor: Anchor::Start,
                    dx: rx - ul,
                    dy: 10.0,
                    valign: VAlign::Ink,
                })
                .part(Part::Text {
                    text,
                    size: m.small,
                    role: Role::Text,
                    color: t.cyan,
                    emph: None,
                    anchor: Anchor::Start,
                    dx: rx + (ur - ul) + 3.0,
                    dy: 10.0,
                    valign: VAlign::Cap,
                });
        }
        node.no_ripple()
    }

    /// The next twelve hours in one well: time over icon and temperature.
    fn weather_hours(&self, key: Key, r: &Report) -> Option<Node> {
        let m = self.m();
        let t = &self.theme;
        let hours: Vec<_> = r.hours.iter().take(12).collect();
        if hours.is_empty() {
            return None;
        }
        let labels: Vec<String> = hours.iter().map(|h| self.weather_hour(h.time)).collect();
        let icon_size = m.icon * 0.72;
        let temp_size = m.label * 0.86;
        let col_w = labels
            .iter()
            .map(|l| self.tw(l, m.small, Role::Text))
            .fold(0.0f32, f32::max)
            .max(
                self.iw("\u{e302}", icon_size, Role::Icon)
                    + 5.0
                    + self.tw("-10°", temp_size, Role::Bold),
            )
            + 20.0;
        let pad = 8.0;
        let mut node = Node::new(
            key,
            Rect::new(
                0.0,
                m.inset,
                (pad * 2.0 + col_w * hours.len() as f32).round(),
                m.bh,
            ),
        )
        .fill(Fill::solid(t.well, m.radius))
        .no_ripple();
        for (i, (hour, label)) in hours.iter().zip(labels).enumerate() {
            let cx = pad + col_w * (i as f32 + 0.5);
            let lw = self.tw(&label, m.small, Role::Text);
            node.parts.push(Part::Text {
                text: label,
                size: m.small,
                role: Role::Text,
                color: t.fg_dim,
                emph: None,
                anchor: Anchor::Start,
                dx: cx - lw / 2.0,
                dy: -11.0,
                valign: VAlign::Cap,
            });
            let glyph = weather::icon(hour.code, !hour.day);
            let (il, ir) = self.fonts.ink_hbounds(glyph, icon_size, Role::Icon);
            let temp = r.deg(hour.temp);
            let tw = self.tw(&temp, temp_size, Role::Bold);
            let start = cx - ((ir - il) + 5.0 + tw) / 2.0;
            node.parts.push(Part::Text {
                text: glyph.into(),
                size: icon_size,
                role: Role::Icon,
                color: t.fg,
                emph: None,
                anchor: Anchor::Start,
                dx: start - il,
                dy: 10.0,
                valign: VAlign::Ink,
            });
            node.parts.push(Part::Text {
                text: temp,
                size: temp_size,
                role: Role::Bold,
                color: t.fg,
                emph: None,
                anchor: Anchor::Start,
                dx: start + (ir - il) + 5.0,
                dy: 10.0,
                valign: VAlign::Cap,
            });
        }
        node.press_glow = false;
        Some(node)
    }

    // ------------------------------------------------------ location search

    fn panel_weather_search(&self, x: &mut f32, nodes: &mut Vec<Node>) -> Option<(f32, f32)> {
        let search = self.weather_search.as_ref()?;
        let m = self.m();
        let t = self.theme.clone();
        let key = Key::new("p/wsearch");
        let weather = self.state.weather.as_ref();
        let auto = weather.is_none_or(|w| w.place.auto);

        // Auto: follow this network's location, as when nothing is stored.
        let detected = weather
            .and_then(|w| w.detected.clone())
            .or_else(|| {
                weather
                    .filter(|w| w.place.auto && !w.place.name.is_empty())
                    .map(|w| w.place.name.clone())
            })
            .unwrap_or_else(|| "From your network".into());
        let detected = self.fit(&detected, 200.0, m.small, Role::Text);
        let mut auto_node = self.two_line_chip(
            key.with("auto"),
            (
                glyph::CROSSHAIRS_GPS,
                if auto { t.accent } else { t.fg_dim },
            ),
            "Auto",
            &detected,
            auto.then_some((glyph::CHECK, t.accent)),
        );
        auto_node.rect.x = *x;
        auto_node.emph = if auto { 1.0 } else { 0.0 };
        let auto_node = auto_node.tap(Cmd::WeatherPick(None));
        *x += auto_node.rect.w + m.gap;
        nodes.push(auto_node);

        // The search field, fed by the physical keyboard.
        let field_w = 420.0;
        let mut field = Node::new(key.with("field"), Rect::new(*x, m.inset, field_w, m.bh))
            .fill(Fill {
                color: t.well,
                emph: None,
                radius: m.radius,
                border: Some(t.accent.alpha(0.72)),
                sheen: false,
            })
            .no_ripple();
        field.press_glow = false;
        let icon_size = m.icon * 0.8;
        let (il, ir) = self
            .fonts
            .ink_hbounds(glyph::MAGNIFY, icon_size, Role::Icon);
        field = field.part(Part::icon(
            glyph::MAGNIFY,
            icon_size,
            t.fg_dim,
            Anchor::Start,
            16.0 - il,
        ));
        let text_x = 16.0 + (ir - il) + 10.0;
        let max_text = field_w - text_x - 18.0;
        if search.query.is_empty() {
            field = field
                .part(Part::Cursor {
                    dx: text_x,
                    height: m.bh * 0.56,
                    color: t.accent,
                })
                .part(Part::label(
                    "Type a city…",
                    m.label * 0.92,
                    t.fg_faint,
                    Anchor::Start,
                    text_x + 10.0,
                ));
        } else {
            let text = self.fit_tail(&search.query, max_text, m.label, Role::Text);
            let tw = self.tw(&text, m.label, Role::Text);
            field = field
                .part(Part::label(text, m.label, t.fg, Anchor::Start, text_x))
                .part(Part::Cursor {
                    dx: text_x + tw + 3.0,
                    height: m.bh * 0.56,
                    color: t.accent,
                });
        }
        *x += field_w + m.gap;
        nodes.push(field);

        let has_text = !search.query.is_empty();
        let mut bs = Node::new(key.with("bs"), Rect::new(*x, m.inset, 72.0, m.bh))
            .fill(self.surface())
            .part(Part::icon(
                glyph::BACKSPACE,
                m.icon * 0.95,
                if has_text { t.fg } else { t.fg_faint },
                Anchor::Center,
                0.0,
            ));
        if has_text {
            bs = bs.hit(Cmd::WeatherSearchBackspace, HitKind::Repeat);
        } else {
            bs.opacity = 0.45;
        }
        *x += 72.0 + m.gap * 2.0;
        nodes.push(bs);

        // Suggestions scroll beside the fixed field.
        let row_start = *x;
        let query = search.query.trim();
        let long_enough = query.chars().count() >= 2;
        let current = search.results_for == query;
        let place = weather
            .filter(|w| !w.place.auto)
            .map(|w| w.place.name.as_str());
        if long_enough && !search.results.is_empty() && !search.failed {
            for (i, loc) in search.results.iter().enumerate() {
                let mut node = self.weather_suggestion(key.with("loc").index(i as i64), loc, place);
                if i == 0
                    && current
                    && let Some(fill) = node.fill.as_mut()
                {
                    // Enter picks this one.
                    fill.border = Some(t.accent.alpha(0.6));
                }
                if !current {
                    node.opacity = 0.5;
                }
                self.row_node(x, nodes, node);
            }
        }
        let message = if search.failed {
            Some("Search unavailable — check the connection".to_string())
        } else if query.is_empty() {
            Some("Search for a city, or tap Auto".into())
        } else if !long_enough {
            Some("Keep typing…".into())
        } else if search.searching && search.results.is_empty() {
            Some("Searching…".into())
        } else if current && search.results.is_empty() {
            Some(format!("No places match “{query}”"))
        } else {
            None
        };
        if let Some(text) = message {
            self.row_message(x, nodes, &self.fit(&text, 600.0, m.label, Role::Text));
        }
        Some((row_start, *x - m.gap))
    }

    fn weather_suggestion(&self, key: Key, loc: &Location, place: Option<&str>) -> Node {
        let t = &self.theme;
        let chosen = place.is_some_and(|p| p.eq_ignore_ascii_case(&loc.name));
        let region = match (loc.region.is_empty(), loc.coords) {
            (true, Some((lat, lon))) => format!("{lat:.2}, {lon:.2}"),
            _ => loc.region.clone(),
        };
        let name = self.fit(&loc.name, 260.0, self.m().small * 1.15, Role::Bold);
        let region = self.fit(&region, 260.0, self.m().small, Role::Text);
        let mut node = self
            .two_line_chip(
                key,
                (glyph::MAP_MARKER, if chosen { t.accent } else { t.fg_dim }),
                &name,
                &region,
                chosen.then_some((glyph::CHECK, t.accent)),
            )
            .tap(Cmd::WeatherPick(Some(loc.clone())));
        node.emph = if chosen { 1.0 } else { 0.0 };
        node
    }

    /// A raised, tappable chip with a bold title over a dim subtitle.
    fn two_line_chip(
        &self,
        key: Key,
        icon: (&str, Rgba),
        title: &str,
        subtitle: &str,
        suffix: Option<(&str, Rgba)>,
    ) -> Node {
        let dim = self.theme.fg_dim;
        self.two_line_chip_colored(key, icon, title, (subtitle, dim), suffix)
    }

    /// [`Self::two_line_chip`] with the subtitle in its own color.
    pub(super) fn two_line_chip_colored(
        &self,
        key: Key,
        icon: (&str, Rgba),
        title: &str,
        (subtitle, subtitle_color): (&str, Rgba),
        suffix: Option<(&str, Rgba)>,
    ) -> Node {
        let m = self.m();
        let t = &self.theme;
        let icon_size = m.icon * 0.95;
        let (il, ir) = self.fonts.ink_hbounds(icon.0, icon_size, Role::Icon);
        let x_text = 14.0 + (ir - il) + 10.0;
        let text_w =
            self.tw(title, m.small * 1.15, Role::Bold)
                .max(self.tw(subtitle, m.small, Role::Text));
        let suffix_size = m.icon * 0.75;
        let suffix_w = suffix.map_or(0.0, |(g, _)| self.iw(g, suffix_size, Role::Icon) + 10.0);
        let w = (x_text + text_w + suffix_w + m.pad).round();
        let mut node = Node::new(key, Rect::new(0.0, m.inset, w, m.bh))
            .fill(Fill {
                color: t.surface,
                emph: Some(t.accent.alpha(0.2).over(t.surface)),
                radius: m.radius,
                border: None,
                sheen: true,
            })
            .part(Part::icon(
                icon.0,
                icon_size,
                icon.1,
                Anchor::Start,
                14.0 - il,
            ))
            .part(Part::Text {
                text: title.into(),
                size: m.small * 1.15,
                role: Role::Bold,
                color: t.fg,
                emph: None,
                anchor: Anchor::Start,
                dx: x_text,
                dy: -10.0,
                valign: VAlign::Cap,
            })
            .part(Part::Text {
                text: subtitle.into(),
                size: m.small,
                role: Role::Text,
                color: subtitle_color,
                emph: None,
                anchor: Anchor::Start,
                dx: x_text,
                dy: 11.0,
                valign: VAlign::Cap,
            });
        if let Some((g, c)) = suffix {
            node = node.part(Part::icon(g, suffix_size, c, Anchor::End, m.pad - 4.0));
        }
        node
    }
}
