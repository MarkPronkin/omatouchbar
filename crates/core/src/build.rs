//! Builds the target node list for the current page.

mod agents;
mod display;
pub(crate) use agents::{AGENTS_REFRESH, AGENTS_REFRESH_PULSE};
mod weather;
mod widgets;
mod world_clock;

use crate::{
    canvas::VAlign,
    color::Rgba,
    config::{ButtonStyle, Mode, WorkspaceStyle},
    node::{Anchor, Fill, HitKind, Key, Motion, Node, Part, Rect},
    omarchy::{DEFAULT_CLOCK, Widget, format_qt},
    scene::{Action, Cmd, Page, Panel, QuickSlider, Scene, Slider, SoundTab, WifiPromptPhase},
    speedtest::SpeedPhase,
    state::{
        AudioDevice, AudioDeviceKind, BtDevice, BtKind, BtOp, Charge, DnsProvider, NetKind,
        format_bytes, format_eth_speed, format_packet_loss, format_ping_latency, format_rate,
        format_wifi_band,
    },
    text::Role,
};

pub mod glyph {
    pub const LOGO: &str = "\u{e900}";
    pub const BACK: &str = "\u{F0141}";
    pub const CHEVRON: &str = "\u{F0142}";
    pub const CHECK: &str = "\u{F012C}";
    pub const VOLUME_HIGH: &str = "\u{F057E}";
    pub const VOLUME_MEDIUM: &str = "\u{F0580}";
    pub const VOLUME_LOW: &str = "\u{F057F}";
    pub const VOLUME_OFF: &str = "\u{F0581}";
    pub const MIC: &str = "\u{F036C}";
    pub const MIC_OFF: &str = "\u{F036D}";
    pub const BRIGHTNESS: &str = "\u{F00E0}";
    pub const BRIGHTNESS_LOW: &str = "\u{F00DE}";
    pub const KEYBOARD: &str = "\u{F030C}";
    pub const BATTERY: [&str; 10] = [
        "\u{F007A}",
        "\u{F007B}",
        "\u{F007C}",
        "\u{F007D}",
        "\u{F007E}",
        "\u{F007F}",
        "\u{F0080}",
        "\u{F0081}",
        "\u{F0082}",
        "\u{F0079}",
    ];
    pub const CHARGING: [&str; 10] = [
        "\u{F089C}",
        "\u{F0086}",
        "\u{F0087}",
        "\u{F0088}",
        "\u{F089D}",
        "\u{F0089}",
        "\u{F089E}",
        "\u{F008A}",
        "\u{F008B}",
        "\u{F0085}",
    ];
    pub const WIFI: [&str; 5] = [
        "\u{F092F}",
        "\u{F091F}",
        "\u{F0922}",
        "\u{F0925}",
        "\u{F0928}",
    ];
    pub const ETHERNET: &str = "\u{F0200}";
    pub const ETHERNET_ALERT: &str = "\u{F0202}";
    pub const WIFI_ALERT: &str = "\u{F0929}";
    pub const NO_NETWORK: &str = "\u{F092E}";
    pub const QR_CODE: &str = "\u{F0432}";
    pub const PORTAL: &str = "\u{F03CC}";
    pub const BT_OFF: &str = "\u{F00B2}";
    pub const BT_ON: &str = "\u{F00AF}";
    pub const BT_CONNECTED: &str = "\u{F00B1}";
    pub const RADAR: &str = "\u{F0437}";
    pub const MOUSE: &str = "\u{F037D}";
    pub const GAMEPAD: &str = "\u{F0297}";
    pub const PHONE: &str = "\u{F011C}";
    pub const LAPTOP: &str = "\u{F0322}";
    pub const DND: &str = "\u{F009B}";
    pub const NIGHTLIGHT: &str = "\u{F050E}";
    pub const STAY_AWAKE: &str = "\u{EF59}";
    pub const RECORDING: &str = "\u{F0EC2}";
    pub const PASSWORDLESS_SUDO: &str = "\u{F07F5}";
    pub const REMOTE_SESSION: &str = "\u{F08B9}";
    pub const REMINDER: &str = "\u{F088C}";
    pub const TOGGLES: &str = "\u{F062E}";
    pub const PLUS: &str = "\u{F0415}";
    pub const TRASH: &str = "\u{F0A7A}";
    pub const GLOBE: &str = "\u{F01E7}";
    pub const UPDATE: &str = "\u{F06B0}";
    pub const SAVER: &str = "\u{F032A}";
    pub const BALANCED: &str = "\u{F029A}";
    pub const PERFORMANCE: &str = "\u{F04C5}";
    pub const DOWNLOAD: &str = "\u{F01DA}";
    pub const UPLOAD: &str = "\u{F0552}";
    pub const MUSIC: &str = "\u{F075A}";
    pub const PREVIOUS: &str = "\u{F04AE}";
    pub const NEXT: &str = "\u{F04AD}";
    pub const PLAY: &str = "\u{F040A}";
    pub const PAUSE: &str = "\u{F03E4}";
    pub const FINGERPRINT: &str = "\u{F0237}";
    pub const TRAY: &str = "󰘔";
    pub const WEATHER: &str = "";
    pub const AGENTS: &str = "󱚣";
    pub const LOCK: &str = "\u{F033E}";
    pub const SPEAKER: &str = "\u{F04C3}";
    pub const HEADPHONES: &str = "\u{F02CB}";
    pub const WEBCAM: &str = "\u{F0100}";
    pub const EYE: &str = "\u{F0208}";
    pub const EYE_OFF: &str = "\u{F0209}";
    pub const BACKSPACE: &str = "\u{F006E}";
    pub const CLOSE: &str = "\u{F0156}";
    pub const REFRESH: &str = "\u{F0450}";
    pub const ALERT: &str = "\u{F0026}";
    pub const MAP_MARKER: &str = "\u{F034E}";
    pub const CROSSHAIRS_GPS: &str = "\u{F01A4}";
    pub const MAGNIFY: &str = "\u{F0349}";
    pub const MONITOR: &str = "\u{F0379}";
    pub const CONSOLE: &str = "\u{F018D}";
    pub const UMBRELLA: &str = "\u{F054A}";
}

/// Layout metrics derived from the panel height and text scale.
#[derive(Clone, Copy)]
pub(crate) struct M {
    pub inset: f32,
    pub bh: f32,
    pub radius: f32,
    pub gap: f32,
    pub edge: f32,
    pub esc: f32,
    pub label: f32,
    pub icon: f32,
    pub small: f32,
    pub pad: f32,
}

/// Width of the Sound panel's output and microphone sliders.
const SOUND_SLIDER_W: f32 = 440.0;
/// Width of each app's volume slider on the Sound panel.
const STREAM_SLIDER_W: f32 = 300.0;
/// Width of the Sound panel's previous / play-pause / next buttons.
const MEDIA_BUTTON_W: f32 = 76.0;
/// Space between the control keys' groups, as on a keyboard's function row.
const CONTROL_GROUP_GAP: f32 = 26.0;
/// Keyboard mode's player, which the Sound panel grows out of.
const KEYBOARD_PLAYER: &str = "kb/player";
/// The player's width, fixed so the keys stay put when the track changes.
const KEYBOARD_PLAYER_W: f32 = 380.0;
/// Control keys and the player sink in place as they leave, so the F-keys
/// rising in cover them instead of their tops drifting out above them.
const KEY_EXIT: Motion = Motion {
    dx: 0.0,
    dy: 0.0,
    scale: 0.94,
    delay: 0.0,
};

fn hash(text: &str) -> i64 {
    Key::new(text).0 as i64
}

impl Scene {
    pub(crate) fn m(&self) -> M {
        let s = self.config.appearance.text_scale;
        let h = self.height;
        let inset = (h * 0.067).round().max(2.0);
        M {
            inset,
            bh: h - inset * 2.0,
            radius: (h * 0.17).round(),
            gap: 8.0,
            edge: 6.0,
            esc: 112.0,
            label: (h * 0.36 * s).round(),
            icon: (h * 0.44 * s).round(),
            small: (h * 0.27 * s).round(),
            pad: 18.0,
        }
    }

    fn surface(&self) -> Fill {
        let t = &self.theme;
        Fill {
            color: t.surface,
            emph: None,
            radius: self.m().radius,
            border: None,
            sheen: true,
        }
    }

    /// Fill for bar widgets (filled or flat per settings).
    fn widget_fill(&self) -> Fill {
        match self.config.appearance.button_style {
            ButtonStyle::Filled => self.surface(),
            ButtonStyle::Flat => Fill {
                color: self.theme.surface.alpha(0.0),
                emph: None,
                radius: self.m().radius,
                border: None,
                sheen: false,
            },
        }
    }

    fn flat(&self) -> Fill {
        Fill {
            color: self.theme.surface.alpha(0.0),
            emph: None,
            radius: self.m().radius,
            border: None,
            sheen: false,
        }
    }

    fn tw(&self, text: &str, size: f32, role: Role) -> f32 {
        self.fonts.measure(text, size, role)
    }

    /// Truncates `text` with an ellipsis to fit `max` px.
    pub(crate) fn fit(&self, text: &str, max: f32, size: f32, role: Role) -> String {
        if self.tw(text, size, role) <= max {
            return text.to_string();
        }
        let chars: Vec<char> = text.chars().collect();
        let (mut lo, mut hi) = (0usize, chars.len());
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            let candidate: String = chars[..mid].iter().collect::<String>() + "…";
            if self.tw(&candidate, size, role) <= max {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        chars[..lo]
            .iter()
            .collect::<String>()
            .trim_end()
            .to_string()
            + "…"
    }

    /// A button with an optional icon and label, laid out at x = 0.
    fn chip(
        &self,
        key: Key,
        icon: Option<(&str, Role, Rgba)>,
        label: Option<(&str, Rgba)>,
        fill: Fill,
    ) -> Node {
        let m = self.m();
        let mut node = Node::new(key, Rect::new(0.0, m.inset, 0.0, m.bh)).fill(fill);
        let (il, ir) = icon
            .map(|(g, role, _)| self.fonts.ink_hbounds(g, m.icon, role))
            .unwrap_or((0.0, 0.0));
        let icon_w = ir - il;
        let label_w = label
            .map(|(l, _)| self.tw(l, m.label, Role::Text))
            .unwrap_or(0.0);
        let width = match (icon, label) {
            (Some((g, role, color)), None) => {
                node = node.part(Part::icon(g, m.icon, color, Anchor::Center, 0.0).with_role(role));
                (icon_w + m.pad * 2.0).max(64.0)
            }
            (None, Some((l, color))) => {
                node = node.part(Part::label(l, m.label, color, Anchor::Center, 0.0));
                (label_w + m.pad * 2.0).max(64.0)
            }
            (Some((g, role, ic)), Some((l, lc))) => {
                let inner = 10.0;
                let start = m.pad - 2.0;
                node = node
                    .part(Part::icon(g, m.icon, ic, Anchor::Start, start - il).with_role(role))
                    .part(Part::label(
                        l,
                        m.label,
                        lc,
                        Anchor::Start,
                        start + icon_w + inner,
                    ));
                start + icon_w + inner + label_w + m.pad
            }
            (None, None) => 64.0,
        };
        node.rect.w = width.round();
        node
    }

    /// Ink width of an icon glyph.
    fn iw(&self, glyph: &str, size: f32, role: Role) -> f32 {
        let (l, r) = self.fonts.ink_hbounds(glyph, size, role);
        r - l
    }

    pub(crate) fn build(&mut self) -> Vec<Node> {
        let m = self.m();
        let mut nodes = Vec::new();
        let page = self.shown_page();
        let esc = self.config.show_escape;
        if esc {
            let t = &self.theme;
            nodes.push(
                Node::new(Key::new("esc"), Rect::new(m.edge, m.inset, m.esc, m.bh))
                    .fill(self.surface())
                    .part(Part::label("esc", m.label, t.fg, Anchor::Center, 0.0))
                    .hit(Cmd::Esc, HitKind::Down)
                    .enter(Motion::FADE),
            );
        }
        let left = if esc {
            m.edge + m.esc + m.gap * 1.5
        } else {
            m.edge
        };
        self.scroll.max = 0.0;
        self.scroll.x0 = f32::MAX;
        self.scroll.x1 = f32::MIN;
        if self.touch_fx.is_some() {
            self.build_touch_id(left, &mut nodes);
            return nodes;
        }
        match page {
            Page::Bar => self.build_bar(left, &mut nodes),
            Page::FnKeys => self.build_fkeys(left, &mut nodes),
            Page::Keyboard => self.build_keyboard(left, &mut nodes),
            Page::Controls => self.build_controls(left, &mut nodes),
            Page::Menu(id) => self.build_menu(&id, left, &mut nodes),
            Page::Panel(panel) => self.build_panel(panel, left, &mut nodes),
        }
        nodes
    }

    // ------------------------------------------------------------- the bar

    fn build_bar(&self, left: f32, nodes: &mut Vec<Node>) {
        // Widgets dropped first when the strip runs out of room.
        const PRIORITY: [&str; 15] = [
            "active-window",
            "media",
            "world-clock",
            "update",
            "keyboard",
            "tray",
            "weather",
            "agents",
            "indicators",
            "microphone",
            "bluetooth",
            "brightness",
            "network",
            "workspaces",
            "audio",
        ];
        let urgent_indicator = self
            .state
            .indicators
            .any_urgent(&self.layout.indicator_items());
        let mut hidden: Vec<&str> = Vec::new();
        let mut compact_indicators = false;
        loop {
            let mut trial = Vec::new();
            let mut spans = Vec::new();
            if self.layout_bar(left, &hidden, compact_indicators, &mut trial, &mut spans) {
                self.apply_quick_slider(trial, spans, nodes);
                return;
            }
            // If indicators are expanded inline, fold them to the compact
            // panel pill before dropping any widgets from the bar.
            if !compact_indicators && self.indicators_expanded_inline() {
                compact_indicators = true;
                continue;
            }
            let next = PRIORITY
                .iter()
                .find(|&&name| {
                    !hidden.contains(&name)
                        && self.layout.contains(name)
                        && !(urgent_indicator && name == "indicators")
                })
                .or_else(|| {
                    PRIORITY
                        .iter()
                        .find(|name| !hidden.contains(name) && self.layout.contains(name))
                });
            match next {
                Some(name) => hidden.push(name),
                None => {
                    self.apply_quick_slider(trial, spans, nodes);
                    return;
                }
            }
        }
    }

    fn indicators_expanded_inline(&self) -> bool {
        self.layout.sections().into_iter().flatten().any(|w| {
            matches!(
                w,
                Widget::Indicators { always_show, .. }
                    if *always_show == Some(true) || self.config.appearance.always_show_indicators
            )
        })
    }

    fn apply_quick_slider(
        &self,
        trial: Vec<Node>,
        spans: Vec<(usize, usize, f32, f32)>,
        nodes: &mut Vec<Node>,
    ) {
        let Some(qs) = &self.quick_slider else {
            if let Some((closing_key, _)) = self.quick_slider_closing {
                let mut promoted = None;
                for node in trial {
                    if node.key == closing_key {
                        promoted = Some(node);
                    } else {
                        nodes.push(node);
                    }
                }
                if let Some(node) = promoted {
                    nodes.push(node);
                }
            } else {
                nodes.extend(trial);
            }
            return;
        };
        let clear_x0 = qs.rect.x - self.m().gap;
        let clear_x1 = qs.rect.right() + self.m().gap;
        let mut keep = vec![false; trial.len()];
        for (s, e, wx, ww) in spans {
            let is_origin = trial[s..e].iter().any(|n| n.key == qs.key);
            let overlaps = wx < clear_x1 && wx + ww > clear_x0;
            if !is_origin && !overlaps {
                for slot in &mut keep[s..e] {
                    *slot = true;
                }
            }
        }
        for (mut node, kept) in trial.into_iter().zip(keep) {
            if kept {
                node.opacity *= 0.45;
                nodes.push(node);
            }
        }
        nodes.push(self.quick_slider_node(qs));
    }

    fn quick_slider_node(&self, qs: &QuickSlider) -> Node {
        let m = self.m();
        let t = &self.theme;
        let fraction = qs.fraction.clamp(0.0, 1.0);
        let value = (fraction * 100.0).round() as u8;
        let (icon, muted, accent) = match qs.slider {
            Slider::Volume => {
                let muted = !qs.changed && self.state.audio.as_ref().is_some_and(|a| a.muted);
                let (g, _) = volume_glyph(value, muted, t);
                let accent = if muted { t.fg_faint } else { t.accent };
                (g, muted, accent)
            }
            Slider::Display | Slider::Monitor(_) => {
                let g = if value < 35 {
                    glyph::BRIGHTNESS_LOW
                } else {
                    glyph::BRIGHTNESS
                };
                (g, false, t.yellow.mix(t.accent, 0.25))
            }
            Slider::Keyboard => (glyph::KEYBOARD, false, t.accent),
            Slider::Mic => (glyph::MIC, false, t.accent),
            Slider::Stream(_) => (glyph::VOLUME_HIGH, false, t.accent),
        };
        let icon_color = if muted {
            t.red
        } else if fraction > 0.08 {
            t.on_accent
        } else {
            t.fg
        };
        let pct_color = if fraction > 0.86 {
            t.on_accent
        } else if muted {
            t.fg_faint
        } else {
            t.fg
        };
        let icon_size = m.icon * 0.9;
        let (il, _) = self.fonts.ink_hbounds(icon, icon_size, Role::Icon);
        let fill = Fill {
            color: t.well,
            emph: None,
            radius: m.radius,
            border: Some(accent.alpha(0.55)),
            sheen: false,
        };
        let mut node = Node::new(qs.key, qs.rect)
            .fill(fill)
            .emph(1.0)
            .part(Part::Progress {
                value: fraction,
                color: accent,
                track: None,
                inset: 4.0,
                radius: m.radius - 3.0,
            })
            .part(Part::icon(
                icon,
                icon_size,
                icon_color,
                Anchor::Start,
                16.0 - il,
            ))
            .part(Part::label(
                format!("{value}%"),
                m.label,
                pct_color,
                Anchor::End,
                18.0,
            ))
            .no_ripple();
        node.snap_progress = true;
        node.press_glow = false;
        node
    }

    /// Lays out the bar sections; returns false when they overlap.
    fn layout_bar(
        &self,
        left: f32,
        hidden: &[&str],
        compact_indicators: bool,
        nodes: &mut Vec<Node>,
        spans: &mut Vec<(usize, usize, f32, f32)>,
    ) -> bool {
        let m = self.m();
        let gap = m.gap;
        let right = self.width - m.edge;
        let place = |widgets: &[Widget], nodes: &mut Vec<Node>| -> Vec<(usize, usize, f32)> {
            widgets
                .iter()
                .filter(|w| !hidden.contains(&w.name()))
                .map(|w| {
                    let start = nodes.len();
                    let width = self.widget(w, compact_indicators, nodes);
                    (start, nodes.len(), width)
                })
                .filter(|(_, _, w)| *w > 0.0)
                .collect()
        };
        let span = |items: &[(usize, usize, f32)]| -> f32 {
            items.iter().map(|c| c.2).sum::<f32>() + gap * items.len().saturating_sub(1) as f32
        };
        let shift = |nodes: &mut Vec<Node>, range: (usize, usize), dx: f32| {
            for n in &mut nodes[range.0..range.1] {
                n.shift(dx);
            }
        };
        let lefts = place(&self.layout.left, nodes);
        let rights = place(&self.layout.right, nodes);
        let begin = place(&self.layout.center_begin, nodes);
        let center = place(&self.layout.center, nodes);
        let end = place(&self.layout.center_end, nodes);

        let left_end = if lefts.is_empty() {
            left - gap
        } else {
            left + span(&lefts)
        };
        let right_start = if rights.is_empty() {
            right + gap
        } else {
            right - span(&rights)
        };
        let with_gap = |items: &[(usize, usize, f32)]| {
            if items.is_empty() {
                0.0
            } else {
                span(items) + gap
            }
        };
        // The center cluster: begin | center | end, with center centered.
        let center_w = span(&center);
        let mut c0 = (self.width / 2.0 - center_w / 2.0).round();
        let mut cluster0 = c0 - with_gap(&begin);
        let mut cluster1 = c0 + center_w + with_gap(&end);
        // Slide the cluster toward whichever side has room.
        let min0 = left_end + gap * 2.0;
        let max1 = right_start - gap * 2.0;
        if cluster1 > max1 {
            let dx = (cluster1 - max1).min((cluster0 - min0).max(0.0));
            c0 -= dx;
            cluster0 -= dx;
            cluster1 -= dx;
        }
        if cluster0 < min0 {
            let dx = (min0 - cluster0).min((max1 - cluster1).max(0.0));
            c0 += dx;
            cluster0 += dx;
            cluster1 += dx;
        }
        let fits = cluster0 >= min0 - 0.5 && cluster1 <= max1 + 0.5 && left_end <= right_start;

        let mut x = left;
        for (s, e, w) in &lefts {
            shift(nodes, (*s, *e), x);
            spans.push((*s, *e, x, *w));
            x += w + gap;
        }
        let mut x = right;
        for (s, e, w) in rights.iter().rev() {
            x -= w;
            shift(nodes, (*s, *e), x);
            spans.push((*s, *e, x, *w));
            x -= gap;
        }
        let mut x = c0;
        for (s, e, w) in &center {
            shift(nodes, (*s, *e), x);
            spans.push((*s, *e, x, *w));
            x += w + gap;
        }
        let mut x = c0 - gap;
        for (s, e, w) in begin.iter().rev() {
            x -= w;
            shift(nodes, (*s, *e), x);
            spans.push((*s, *e, x, *w));
            x -= gap;
        }
        let mut x = c0 + center_w + gap;
        for (s, e, w) in &end {
            shift(nodes, (*s, *e), x);
            spans.push((*s, *e, x, *w));
            x += w + gap;
        }
        fits
    }

    /// Pushes a widget's nodes at x = 0 and returns its width.
    fn widget(&self, widget: &Widget, compact_indicators: bool, nodes: &mut Vec<Node>) -> f32 {
        let m = self.m();
        let t = self.theme.clone();
        let s = &self.state;
        let push = |nodes: &mut Vec<Node>, node: Node| -> f32 {
            let w = node.rect.w;
            nodes.push(node);
            w
        };
        match widget {
            Widget::Menu => {
                let node = self
                    .chip(
                        Key::new("w/menu"),
                        Some((glyph::LOGO, Role::Logo, t.accent)),
                        None,
                        self.widget_fill(),
                    )
                    .tap(Cmd::OpenMenu);
                push(nodes, node)
            }
            Widget::Tray { .. } => push(nodes, self.panel_button(Panel::Tray, glyph::TRAY, t.fg)),
            Widget::Weather { .. } => {
                let icon = s
                    .weather
                    .as_ref()
                    .map(|w| w.icon.as_str())
                    .filter(|s| !s.is_empty())
                    .unwrap_or(glyph::WEATHER);
                let temp = self
                    .config
                    .appearance
                    .weather_temperature
                    .then(|| {
                        s.weather
                            .as_ref()
                            .and_then(crate::state::Weather::temperature)
                    })
                    .flatten();
                push(nodes, self.weather_button(icon, temp.as_deref(), t.fg))
            }
            Widget::Agents { disabled } => {
                let near_limit = s
                    .agents
                    .iter()
                    .filter(|a| !disabled.contains(&a.id))
                    .any(|a| a.alarming(self.utc));
                push(
                    nodes,
                    self.panel_button(
                        Panel::Agents,
                        glyph::AGENTS,
                        if near_limit { t.orange } else { t.fg },
                    ),
                )
            }
            Widget::Workspaces => self.workspaces(nodes),
            Widget::Clock { format, .. } => self.clock(format, nodes),
            Widget::WorldClock { .. } => {
                let node = self
                    .chip(
                        Key::new("w/world"),
                        Some((glyph::GLOBE, Role::Icon, t.fg)),
                        None,
                        self.widget_fill(),
                    )
                    .tap(Cmd::OpenPanel(Panel::Clock));
                push(nodes, node)
            }
            Widget::Indicators { items, always_show } => {
                self.indicators_widget(items, *always_show, compact_indicators, nodes)
            }
            Widget::Update => {
                if !s.update_available {
                    return 0.0;
                }
                let node = self
                    .chip(
                        Key::new("w/update"),
                        Some((glyph::UPDATE, Role::Icon, t.accent)),
                        None,
                        self.widget_fill(),
                    )
                    .tap(Cmd::Do(Action::Shell(
                        "omarchy-launch-floating-terminal-with-presentation omarchy-update".into(),
                    )));
                push(nodes, node)
            }
            Widget::Keyboard => {
                let Some(k) = s.keyboard.as_ref().filter(|k| k.count > 1) else {
                    return 0.0;
                };
                let code: String = k
                    .layout
                    .chars()
                    .filter(|c| c.is_alphabetic())
                    .take(2)
                    .collect::<String>()
                    .to_uppercase();
                let node = self
                    .chip(
                        Key::new("w/keyboard"),
                        None,
                        Some((&code, t.fg)),
                        self.widget_fill(),
                    )
                    .tap(Cmd::Do(Action::CycleKeyboardLayout));
                push(nodes, node)
            }
            Widget::Bluetooth => {
                if !s.bluetooth.available {
                    return 0.0;
                }
                let (g, c) = if !s.bluetooth.powered {
                    (glyph::BT_OFF, t.fg_faint)
                } else if !s.bluetooth.connected.is_empty() {
                    (glyph::BT_CONNECTED, t.accent)
                } else {
                    (glyph::BT_ON, t.fg)
                };
                let node = self
                    .chip(
                        Key::new("w/bluetooth"),
                        Some((g, Role::Icon, c)),
                        None,
                        self.widget_fill(),
                    )
                    .tap(Cmd::OpenPanel(Panel::Bluetooth));
                push(nodes, node)
            }
            Widget::Network => {
                let (g, c) = network_glyph(s, &t);
                let node = self
                    .chip(
                        Key::new("w/network"),
                        Some((g, Role::Icon, c)),
                        None,
                        self.widget_fill(),
                    )
                    .tap(Cmd::OpenPanel(Panel::Network));
                push(nodes, node)
            }
            Widget::Audio => {
                let Some(a) = &s.audio else { return 0.0 };
                let (g, c) = volume_glyph(a.volume, a.muted, &t);
                let label = format!("{}%", a.volume);
                let label = self
                    .config
                    .appearance
                    .volume_percentage
                    .then_some((label.as_str(), if a.muted { t.fg_faint } else { t.fg }));
                let mut fill = self.widget_fill();
                fill.emph = Some(t.well);
                let node = self
                    .chip(Key::new("w/audio"), Some((g, Role::Icon, c)), label, fill)
                    .hit(
                        Cmd::OpenPanel(Panel::Audio),
                        HitKind::TapSlide(Slider::Volume),
                    );
                let value = if a.muted {
                    0.0
                } else {
                    a.volume as f32 / 100.0
                };
                self.with_meter(node, "audio", value, nodes)
            }
            Widget::Microphone => {
                let Some(muted) = s.mic_muted else { return 0.0 };
                let (g, c) = if muted {
                    (glyph::MIC_OFF, t.red)
                } else {
                    (glyph::MIC, t.fg)
                };
                let node = self
                    .chip(
                        Key::new("w/mic"),
                        Some((g, Role::Icon, c)),
                        None,
                        self.widget_fill(),
                    )
                    .tap(Cmd::Do(Action::ToggleMicMute));
                push(nodes, node)
            }
            Widget::Brightness => {
                let label = s.display_brightness.map(|b| format!("{b}%"));
                let label = self
                    .config
                    .appearance
                    .brightness_percentage
                    .then_some(label.as_deref())
                    .flatten()
                    .map(|l| (l, t.fg));
                let mut fill = self.widget_fill();
                fill.emph = Some(t.well);
                let node = self
                    .chip(
                        Key::new("w/brightness"),
                        Some((glyph::BRIGHTNESS, Role::Icon, t.fg)),
                        label,
                        fill,
                    )
                    .hit(
                        Cmd::OpenPanel(Panel::Brightness),
                        HitKind::TapSlide(Slider::Display),
                    );
                let value = s.display_brightness.unwrap_or(0) as f32 / 100.0;
                self.with_meter(node, "brightness", value, nodes)
            }
            Widget::Battery { percentage } => {
                let Some(b) = &s.battery else { return 0.0 };
                let (g, c) = battery_glyph(b.percent, b.charge, &t);
                let show = percentage.unwrap_or(self.config.appearance.battery_percentage);
                let label = format!("{}%", b.percent.round() as i32);
                let label = show.then_some((label.as_str(), t.fg));
                let mut node = self
                    .chip(
                        Key::new("w/battery"),
                        Some((g, Role::Icon, c)),
                        label,
                        self.widget_fill(),
                    )
                    .tap(Cmd::OpenPanel(Panel::Power));
                if self.pulses.contains_key("battery") {
                    node.emph = 1.0;
                    if let Some(f) = node.fill.as_mut() {
                        f.emph = Some(t.green.alpha(0.35));
                    }
                }
                push(nodes, node)
            }
            Widget::ActiveWindow => {
                if s.window_title.is_empty() {
                    return 0.0;
                }
                let text = self.fit(&s.window_title, 420.0, m.label * 0.9, Role::Text);
                let w = self.tw(&text, m.label * 0.9, Role::Text) + m.pad;
                let node = Node::new(
                    Key::new("w/window").index(hash(&text)),
                    Rect::new(0.0, m.inset, w, m.bh),
                )
                .part(Part::label(
                    text,
                    m.label * 0.9,
                    t.fg_dim,
                    Anchor::Start,
                    m.pad / 2.0,
                ))
                .enter(Motion::rise(12.0))
                .exit(Motion::rise(-12.0));
                push(nodes, node)
            }
            Widget::Media => self.media_widget(nodes),
            Widget::Spacer => 24.0,
        }
    }

    /// Appends an inline level meter to a chip while its value just changed.
    fn with_meter(
        &self,
        mut chip: Node,
        name: &'static str,
        value: f32,
        nodes: &mut Vec<Node>,
    ) -> f32 {
        let t = &self.theme;
        if !self.pulses.contains_key(name) {
            let w = chip.rect.w;
            nodes.push(chip);
            return w;
        }
        let meter_w = 104.0;
        let x = chip.rect.w - 6.0;
        chip.rect.w += meter_w + 4.0;
        let w = chip.rect.w;
        let key = chip.key.with("meter");
        nodes.push(chip);
        nodes.push(
            Node::new(key, Rect::new(x, self.height / 2.0 - 5.0, meter_w, 10.0))
                .part(Part::Progress {
                    value,
                    color: t.fg,
                    track: Some(t.fg_faint.alpha(0.35)),
                    inset: 0.0,
                    radius: 5.0,
                })
                .enter(Motion {
                    dx: -24.0,
                    dy: 0.0,
                    scale: 0.9,
                    delay: 0.05,
                })
                .exit(Motion {
                    dx: -24.0,
                    dy: 0.0,
                    scale: 0.9,
                    delay: 0.0,
                }),
        );
        w
    }

    pub(crate) fn workspace_ids(&self) -> Vec<i32> {
        let ws = &self.state.workspaces;
        let base = ws.block * 10;
        let mut ids: Vec<i32> = (1..=self.config.appearance.min_workspaces as i32)
            .map(|i| base + i)
            .collect();
        let mut guests = Vec::new();
        for w in &ws.list {
            if w.id <= 0 {
                continue;
            }
            if (w.id - 1).div_euclid(10) == ws.block {
                if !ids.contains(&w.id) {
                    ids.push(w.id);
                }
            } else if w.monitor == ws.monitor && !guests.contains(&w.id) {
                guests.push(w.id);
            }
        }
        ids.sort_unstable();
        guests.sort_unstable();
        ids.extend(guests);
        ids
    }

    fn workspaces(&self, nodes: &mut Vec<Node>) -> f32 {
        let m = self.m();
        let t = &self.theme;
        let dots = self.config.appearance.workspace_style == WorkspaceStyle::Dots;
        let ids = self.workspace_ids();
        let slot = if dots { 44.0 } else { 56.0 };
        let pad = 4.0;
        let width = ids.len() as f32 * slot + pad * 2.0;
        let well_fill = match self.config.appearance.button_style {
            ButtonStyle::Filled => Fill::solid(t.well, m.radius),
            ButtonStyle::Flat => Fill::solid(t.well.alpha(0.0), m.radius),
        };
        nodes.push(
            Node::new(Key::new("w/ws"), Rect::new(0.0, m.inset, width, m.bh)).fill(well_fill),
        );
        let active = self.state.workspaces.active;
        if let Some(index) = ids.iter().position(|&id| id == active) {
            let r = m.radius - 3.0;
            nodes.push(
                Node::new(
                    Key::new("w/ws/pill"),
                    Rect::new(
                        pad + index as f32 * slot,
                        m.inset + pad,
                        slot,
                        m.bh - pad * 2.0,
                    ),
                )
                .fill(Fill {
                    color: t.accent,
                    emph: None,
                    radius: r,
                    border: None,
                    sheen: true,
                }),
            );
        }
        for (i, &id) in ids.iter().enumerate() {
            let occupied = self
                .state
                .workspaces
                .list
                .iter()
                .any(|w| w.id == id && w.windows > 0);
            let is_active = id == active;
            let base = if occupied { t.fg } else { t.fg_faint };
            let mut node = Node::new(
                Key::new("w/ws").index(id as i64),
                Rect::new(pad + i as f32 * slot, m.inset, slot, m.bh),
            )
            .hit(Cmd::Workspace(id), HitKind::Scrub)
            .emph(if is_active { 1.0 } else { 0.0 })
            .no_ripple();
            node.press_glow = false;
            if dots {
                let r = if occupied { 6.0 } else { 4.0 };
                node = node.part(Part::Dot {
                    anchor: Anchor::Center,
                    dx: 0.0,
                    dy: 0.0,
                    r,
                    color: if is_active { t.on_accent } else { base },
                });
            } else {
                // Match Omarchy's top bar: slot 10 is bound to Super+0, so label it "0"
                // while keeping it in slot 10's position at the end of the block.
                let label = if (id - 1).div_euclid(10) != self.state.workspaces.block {
                    id.to_string()
                } else {
                    match (id - 1).rem_euclid(10) + 1 {
                        10 => "0".into(),
                        n => n.to_string(),
                    }
                };
                node = node.part(Part::Text {
                    text: label,
                    size: m.label,
                    role: Role::Bold,
                    color: base,
                    emph: Some(t.on_accent),
                    anchor: Anchor::Center,
                    dx: 0.0,
                    dy: 0.0,
                    valign: VAlign::Cap,
                });
            }
            nodes.push(node);
        }
        width
    }

    pub(crate) fn clock_format(&self) -> String {
        for section in self.layout.sections() {
            for w in section {
                if let Widget::Clock { format, .. } = w {
                    return format.clone();
                }
            }
        }
        if self.config.appearance.clock_format.is_empty() {
            DEFAULT_CLOCK.into()
        } else {
            self.config.appearance.clock_format.clone()
        }
    }

    pub(crate) fn clock_text(&self) -> String {
        format_qt(&self.clock_format(), self.local)
    }

    pub(crate) fn clock_has_seconds(&self) -> bool {
        let mut quoted = false;
        for c in self.clock_format().chars() {
            match c {
                '\'' => quoted = !quoted,
                's' if !quoted => return true,
                _ => {}
            }
        }
        false
    }

    fn clock(&self, format: &str, nodes: &mut Vec<Node>) -> f32 {
        let m = self.m();
        let t = &self.theme;
        let text = format_qt(format, self.local);
        let run = self.fonts.layout(&text, m.label, Role::Text);
        let width = (run.width + m.pad * 2.0).round();
        nodes.push(
            Node::new(Key::new("w/clock"), Rect::new(0.0, m.inset, width, m.bh))
                .fill(self.flat())
                .tap(Cmd::OpenPanel(Panel::Clock)),
        );
        // One node per character so changed digits roll.
        let chars: Vec<char> = text.chars().filter(|c| *c != '\n').collect();
        for (i, (ch, g)) in chars.iter().zip(run.glyphs.iter()).enumerate() {
            if ch.is_whitespace() {
                continue;
            }
            let next_x = run.glyphs.get(i + 1).map(|n| n.x).unwrap_or(run.width);
            let key = Key::new("w/clock/c").index(i as i64).with(&ch.to_string());
            nodes.push(
                Node::new(key, Rect::new(m.pad + g.x, m.inset, next_x - g.x, m.bh))
                    .part(Part::label(
                        ch.to_string(),
                        m.label,
                        t.fg,
                        Anchor::Start,
                        0.0,
                    ))
                    .enter(Motion {
                        dx: 0.0,
                        dy: 22.0,
                        scale: 1.0,
                        delay: i as f32 * 0.018,
                    })
                    .exit(Motion {
                        dx: 0.0,
                        dy: -22.0,
                        scale: 1.0,
                        delay: 0.0,
                    }),
            );
        }
        width
    }

    fn media_widget(&self, nodes: &mut Vec<Node>) -> f32 {
        let m = self.m();
        let t = &self.theme;
        let Some(media) = self.state.media.as_ref().filter(|m| !m.title.is_empty()) else {
            return 0.0;
        };
        let text = if media.artist.is_empty() {
            media.title.clone()
        } else {
            format!("{} · {}", media.title, media.artist)
        };
        let size = m.label * 0.86;
        let text = self.fit(&text, 300.0, size, Role::Text);
        let icon_w = self.iw(glyph::MUSIC, m.icon * 0.85, Role::Icon);
        let text_w = self.tw(&text, size, Role::Text);
        let w = (m.pad + icon_w + 10.0 + text_w + m.pad).round();
        let icon_color = if media.playing { t.accent } else { t.fg_dim };
        let chip = Node::new(Key::new("w/media"), Rect::new(0.0, m.inset, w, m.bh))
            .fill(self.widget_fill())
            .part(Part::icon(
                glyph::MUSIC,
                m.icon * 0.85,
                icon_color,
                Anchor::Start,
                m.pad
                    - 2.0
                    - self
                        .fonts
                        .ink_hbounds(glyph::MUSIC, m.icon * 0.85, Role::Icon)
                        .0,
            ))
            .tap(Cmd::OpenPanel(Panel::Audio));
        nodes.push(chip);
        nodes.push(
            Node::new(
                Key::new("w/media/t").index(hash(&text)),
                Rect::new(m.pad - 2.0 + icon_w + 10.0, m.inset, text_w, m.bh),
            )
            .part(Part::label(
                text,
                size,
                if media.playing { t.fg } else { t.fg_dim },
                Anchor::Start,
                0.0,
            ))
            .enter(Motion::rise(18.0))
            .exit(Motion::rise(-18.0)),
        );
        let pp = if media.playing {
            glyph::PAUSE
        } else {
            glyph::PLAY
        };
        let mut button = self
            .chip(
                Key::new("w/media/pp"),
                Some((pp, Role::Icon, t.fg)),
                None,
                self.widget_fill(),
            )
            .tap(Cmd::Do(Action::MediaPlayPause));
        button.rect.x = w + m.gap;
        let bw = button.rect.w;
        nodes.push(button);
        w + m.gap + bw
    }

    // ----------------------------------------------------- other base pages

    fn build_fkeys(&self, left: f32, nodes: &mut Vec<Node>) {
        let m = self.m();
        let t = &self.theme;
        let right = self.width - m.edge;
        let gap = m.gap;
        let w = ((right - left) - gap * 11.0) / 12.0;
        for n in 1..=12u8 {
            let x = left + (n - 1) as f32 * (w + gap);
            nodes.push(
                Node::new(
                    Key::new("fkey").index(n as i64),
                    Rect::new(x.round(), m.inset, w.floor(), m.bh),
                )
                .fill(self.surface())
                .part(Part::label(
                    format!("F{n}"),
                    m.label,
                    t.fg,
                    Anchor::Center,
                    0.0,
                ))
                .hit(Cmd::FKey(n), HitKind::Down)
                .enter(self.animator.enter.delayed(n as f32 * 0.008)),
            );
        }
    }

    fn build_controls(&self, left: f32, nodes: &mut Vec<Node>) {
        let right = self.width - self.m().edge;
        self.control_keys(left, right, nodes);
    }

    /// Keyboard mode: the control keys like a keyboard's function row, and
    /// the now-playing player at the right end.
    fn build_keyboard(&self, left: f32, nodes: &mut Vec<Node>) {
        let right = self.width - self.m().edge;
        let player_x = right - KEYBOARD_PLAYER_W;
        let count = self.control_keys(left, player_x - CONTROL_GROUP_GAP, nodes);
        self.keyboard_player(player_x, count, nodes);
    }

    /// Brightness, keyboard light, media and volume keys in groups between
    /// `left` and `right`. A group's level shows on a thin meter across it
    /// for a moment after it changes. Returns how many keys were laid out.
    fn control_keys(&self, left: f32, right: f32, nodes: &mut Vec<Node>) -> usize {
        let m = self.m();
        let t = self.theme.clone();
        let s = &self.state;
        let playing = s.media.as_ref().is_some_and(|m| m.playing);
        let muted = s.audio.as_ref().is_some_and(|a| a.muted);
        let level = |v: Option<u8>| v.map(|v| v as f32 / 100.0);
        type Keys<'a> = Vec<(&'a str, &'a str, Option<&'a str>, Cmd, HitKind)>;
        let groups: Vec<(Option<(&'static str, f32)>, Keys)> = vec![
            (
                level(s.display_brightness).map(|v| ("brightness", v)),
                vec![
                    (
                        "bd",
                        glyph::BRIGHTNESS_LOW,
                        None,
                        Cmd::Do(Action::StepDisplayBrightness(false)),
                        HitKind::Repeat,
                    ),
                    (
                        "bu",
                        glyph::BRIGHTNESS,
                        None,
                        Cmd::Do(Action::StepDisplayBrightness(true)),
                        HitKind::Repeat,
                    ),
                ],
            ),
            (
                level(s.keyboard_backlight).map(|v| ("keyboard", v)),
                vec![
                    (
                        "kd",
                        glyph::KEYBOARD,
                        Some("−"),
                        Cmd::Do(Action::StepKeyboardBacklight(false)),
                        HitKind::Repeat,
                    ),
                    (
                        "ku",
                        glyph::KEYBOARD,
                        Some("+"),
                        Cmd::Do(Action::StepKeyboardBacklight(true)),
                        HitKind::Repeat,
                    ),
                ],
            ),
            (
                None,
                vec![
                    (
                        "prev",
                        glyph::PREVIOUS,
                        None,
                        Cmd::Do(Action::MediaPrevious),
                        HitKind::Tap,
                    ),
                    (
                        "play",
                        if playing { glyph::PAUSE } else { glyph::PLAY },
                        None,
                        Cmd::Do(Action::MediaPlayPause),
                        HitKind::Tap,
                    ),
                    (
                        "next",
                        glyph::NEXT,
                        None,
                        Cmd::Do(Action::MediaNext),
                        HitKind::Tap,
                    ),
                ],
            ),
            (
                s.audio.as_ref().map(|a| {
                    (
                        "audio",
                        if a.muted {
                            0.0
                        } else {
                            a.volume as f32 / 100.0
                        },
                    )
                }),
                vec![
                    (
                        "mute",
                        glyph::VOLUME_OFF,
                        None,
                        Cmd::Do(Action::ToggleMute),
                        HitKind::Tap,
                    ),
                    (
                        "vd",
                        glyph::VOLUME_LOW,
                        None,
                        Cmd::Do(Action::StepVolume(false)),
                        HitKind::Repeat,
                    ),
                    (
                        "vu",
                        glyph::VOLUME_HIGH,
                        None,
                        Cmd::Do(Action::StepVolume(true)),
                        HitKind::Repeat,
                    ),
                ],
            ),
        ];
        let count: usize = groups.iter().map(|(_, g)| g.len()).sum();
        let avail = right
            - left
            - CONTROL_GROUP_GAP * (groups.len() - 1) as f32
            - m.gap * (count - groups.len()) as f32;
        let w = (avail / count as f32).floor();
        let mut x = left;
        let mut index = 0;
        for (meter, group) in groups {
            let group_x = x;
            for (name, g, suffix, cmd, kind) in group {
                let mut node = Node::new(
                    Key::new("ctl").with(name),
                    Rect::new(x.round(), m.inset, w, m.bh),
                )
                .fill(self.surface())
                .hit(cmd, kind)
                .enter(self.animator.enter.delayed(index as f32 * 0.01))
                .exit(KEY_EXIT);
                let color = if name == "mute" && muted { t.red } else { t.fg };
                match suffix {
                    Some(sfx) => {
                        node = node
                            .part(Part::icon(g, m.icon, color, Anchor::Center, -10.0))
                            .part(Part::label(sfx, m.label, t.fg_dim, Anchor::Center, 20.0));
                    }
                    None => node = node.part(Part::icon(g, m.icon, color, Anchor::Center, 0.0)),
                }
                if name == "mute" && muted {
                    node.emph = 1.0;
                    node.fill.as_mut().unwrap().emph = Some(t.red.alpha(0.25).over(t.surface));
                }
                nodes.push(node);
                x += w + m.gap;
                index += 1;
            }
            if let Some((pulse, value)) = meter.filter(|(pulse, _)| self.pulses.contains_key(pulse))
            {
                let inset = 18.0;
                let group_w = x - m.gap - group_x;
                nodes.push(
                    Node::new(
                        Key::new("ctl/meter").with(pulse),
                        Rect::new(
                            (group_x + inset).round(),
                            m.inset + m.bh - 9.0,
                            (group_w - inset * 2.0).round(),
                            4.0,
                        ),
                    )
                    .part(Part::Progress {
                        value,
                        color: t.fg,
                        track: Some(t.fg_faint.alpha(0.35)),
                        inset: 0.0,
                        radius: 2.0,
                    })
                    .enter(Motion::rise(4.0))
                    .exit(Motion::rise(4.0)),
                );
            }
            x += CONTROL_GROUP_GAP - m.gap;
        }
        count
    }

    /// What Keyboard mode's player shows: the track and artist (or
    /// player), or the output when nothing plays.
    fn keyboard_player_text(&self) -> (&str, &str, bool) {
        match self.state.media.as_ref().filter(|m| !m.title.is_empty()) {
            Some(media) => (
                media.title.as_str(),
                if media.artist.is_empty() {
                    media.player.as_str()
                } else {
                    media.artist.as_str()
                },
                true,
            ),
            None => (
                "Not playing",
                self.state
                    .audio
                    .as_ref()
                    .map(|a| a.sink.as_str())
                    .filter(|sink| !sink.is_empty())
                    .unwrap_or("Sound"),
                false,
            ),
        }
    }

    /// The player's node. One node per track, so a new track crossfades in
    /// (as the Sound panel's now-playing card does).
    pub(crate) fn keyboard_player_key(&self) -> Key {
        let (title, subtitle, _) = self.keyboard_player_text();
        Key::new(KEYBOARD_PLAYER).index(hash(&format!("{title}\n{subtitle}")))
    }

    /// Keyboard mode's player: what's playing (or the output, when nothing
    /// is), opening the Sound panel when tapped. Its text is part of the
    /// card, so it moves, fades and presses with it like a key.
    fn keyboard_player(&self, x: f32, stagger: usize, nodes: &mut Vec<Node>) {
        let m = self.m();
        let t = &self.theme;
        let playing = self.state.media.as_ref().is_some_and(|m| m.playing);
        let (title, subtitle, has_media) = self.keyboard_player_text();
        let icon = m.icon * 1.05;
        let (il, ir) = self.fonts.ink_hbounds(glyph::MUSIC, icon, Role::Icon);
        let chevron = m.icon * 0.8;
        let text_x = 16.0 + (ir - il) + 14.0;
        let text_w =
            KEYBOARD_PLAYER_W - text_x - self.iw(glyph::CHEVRON, chevron, Role::Icon) - 24.0;
        let title_size = m.small * 1.12;
        nodes.push(
            Node::new(
                self.keyboard_player_key(),
                Rect::new(x, m.inset, KEYBOARD_PLAYER_W, m.bh),
            )
            .fill(self.surface())
            .part(Part::icon(
                glyph::MUSIC,
                icon,
                if playing { t.accent } else { t.fg_dim },
                Anchor::Start,
                16.0 - il,
            ))
            .part(Part::Text {
                text: self.fit(title, text_w, title_size, Role::Bold),
                size: title_size,
                role: Role::Bold,
                color: if has_media { t.fg } else { t.fg_dim },
                emph: None,
                anchor: Anchor::Start,
                dx: text_x,
                dy: -10.0,
                valign: VAlign::Cap,
            })
            .part(Part::Text {
                text: self.fit(subtitle, text_w, m.small, Role::Text),
                size: m.small,
                role: Role::Text,
                color: t.fg_dim,
                emph: None,
                anchor: Anchor::Start,
                dx: text_x,
                dy: 11.0,
                valign: VAlign::Cap,
            })
            .part(Part::icon(
                glyph::CHEVRON,
                chevron,
                t.fg_faint,
                Anchor::End,
                12.0,
            ))
            .tap(Cmd::OpenPanel(Panel::Audio))
            .enter(self.animator.enter.delayed(stagger as f32 * 0.01))
            .exit(KEY_EXIT),
        );
    }

    // --------------------------------------------------------------- menus

    fn nav(&self, left: f32, title: &str, icon: (&str, Role, Rgba)) -> Node {
        let t = &self.theme;
        let mut node = self
            .chip(
                Key::new("nav"),
                Some(icon),
                Some((title, t.fg)),
                self.surface(),
            )
            .tap(Cmd::Back);
        node.rect.x = left;
        node.enter = Some(Motion::slide(-30.0));
        node
    }

    fn build_menu(&mut self, id: &str, left: f32, nodes: &mut Vec<Node>) {
        let m = self.m();
        let t = self.theme.clone();
        let nav = if id.is_empty() {
            self.nav(left, "Menu", (glyph::LOGO, Role::Logo, t.accent))
        } else {
            self.nav(
                left,
                &self.fit(&self.menu.title(id), 260.0, m.label, Role::Text),
                (glyph::BACK, Role::Icon, t.fg_dim),
            )
        };
        let x0 = nav.rect.right() + m.gap * 1.5;
        nodes.push(nav);
        let x1 = self.width - m.edge;
        let mut items: Vec<crate::menu::MenuItem> = self.menu.children(id).cloned().collect();
        if let Some(rows) = self.provider_rows.get(id) {
            items.extend(rows.iter().cloned());
        }
        let stagger = self.config.motion.stagger_ms / 1000.0;
        let enter = self.animator.enter;
        let mut x = x0;
        let mut shown = 0;
        for item in items {
            let guard = self.guards.get(&item.id).copied().unwrap_or_default();
            if guard.when == Some(false) {
                continue;
            }
            let submenu = self.menu.is_submenu(&item);
            let disabled = guard.disabled == Some(true);
            let checked = guard.checked == Some(true) || item.current;
            let role = if item.logo_icon {
                Role::Logo
            } else {
                Role::Icon
            };
            let icon = if item.icon.is_empty() {
                None
            } else {
                Some((
                    item.icon.as_str(),
                    role,
                    if checked { t.accent } else { t.fg_dim },
                ))
            };
            let label = self.fit(&item.label, 360.0, m.label, Role::Text);
            let mut node = self.chip(
                Key::new("m").with(&item.id),
                icon,
                Some((&label, t.fg)),
                self.surface(),
            );
            let suffix = if checked {
                Some((glyph::CHECK, t.accent))
            } else if submenu {
                Some((glyph::CHEVRON, t.fg_faint))
            } else {
                None
            };
            if let Some((g, c)) = suffix {
                let sw = self.tw(g, m.icon * 0.8, Role::Icon);
                node.parts
                    .push(Part::icon(g, m.icon * 0.8, c, Anchor::End, m.pad - 6.0));
                node.rect.w += sw + 2.0;
            }
            node.rect.x = x;
            node = node.group(0).enter(enter.delayed(shown as f32 * stagger));
            if disabled {
                node.opacity = 0.42;
            } else {
                node = node.tap(if submenu {
                    Cmd::MenuOpen(item.id.clone())
                } else {
                    Cmd::MenuRun(item.id.clone())
                });
            }
            x += node.rect.w + m.gap;
            nodes.push(node);
            shown += 1;
        }
        self.scroll.x0 = x0;
        self.scroll.x1 = x1;
        self.scroll.max = (x - m.gap - x1).max(0.0);
    }

    // -------------------------------------------------------------- panels

    fn build_panel(&mut self, panel: Panel, left: f32, nodes: &mut Vec<Node>) {
        let m = self.m();
        let t = self.theme.clone();
        let nav_title = if panel == Panel::Clock && !self.world.nav.is_empty() {
            &self.world.nav
        } else if panel == Panel::Network && self.wifi_prompt.is_some() {
            "Cancel"
        } else if panel == Panel::Network && self.speedtest.is_some() {
            "Net"
        } else if panel == Panel::Weather && self.weather_search.is_some() {
            "Cancel"
        } else {
            panel.title()
        };
        let nav = self.nav(left, nav_title, (glyph::BACK, Role::Icon, t.fg_dim));
        let mut x = nav.rect.right() + m.gap * 2.0;
        nodes.push(nav);
        let start = nodes.len();
        let row = match panel {
            Panel::Audio => self.panel_audio(&mut x, nodes),
            Panel::Brightness => self.panel_display(&mut x, nodes),
            Panel::Power => {
                self.panel_power(&mut x, nodes);
                None
            }
            Panel::Weather => self.panel_weather(&mut x, nodes),
            Panel::Tray | Panel::Agents | Panel::Indicators => {
                let row_start = x;
                match panel {
                    Panel::Tray => self.panel_tray(&mut x, nodes),
                    Panel::Agents => self.panel_agents(&mut x, nodes),
                    _ => self.panel_indicators(&mut x, nodes),
                }
                Some((row_start, x - m.gap))
            }
            Panel::Network => self.panel_network(&mut x, nodes),
            Panel::Bluetooth => self.panel_bluetooth(&mut x, nodes),
            Panel::Clock => self.panel_clock(&mut x, nodes),
        };
        // Lists that overflow scroll like the menu row.
        if let Some((x0, end)) = row {
            self.scroll.x0 = x0;
            self.scroll.x1 = self.width - m.edge;
            self.scroll.max = (end - self.scroll.x1).max(0.0);
        }
        // Grow out of the widget the panel was opened from.
        let origin = self.panel_origin.unwrap_or(self.width / 2.0);
        let stagger = self.config.motion.stagger_ms / 1000.0;
        for (i, node) in nodes[start..].iter_mut().enumerate() {
            if node.enter.is_none() {
                let dx = ((origin - node.rect.cx()) * 0.3).clamp(-360.0, 360.0);
                node.enter = Some(Motion {
                    dx,
                    dy: 0.0,
                    scale: 0.9,
                    delay: i as f32 * stagger,
                });
            }
        }
    }

    fn slider(
        &self,
        key: Key,
        x: f32,
        w: f32,
        value: u8,
        icon: Option<&str>,
        slider: Slider,
        accent: Rgba,
    ) -> Node {
        let m = self.m();
        let t = &self.theme;
        let dragging = self
            .overrides
            .get(&slider)
            .is_some_and(|(_, release)| release.is_none());
        let fraction = value as f32 / 100.0;
        let icon_color = if fraction > 0.07 { t.on_accent } else { t.fg };
        let mut node = Node::new(key, Rect::new(x, m.inset, w, m.bh))
            .fill(Fill::solid(t.well, m.radius))
            .part(Part::Progress {
                value: fraction,
                color: accent,
                track: None,
                inset: 4.0,
                radius: m.radius - 3.0,
            });
        if let Some(icon) = icon {
            node = node.part(Part::icon(
                icon,
                m.icon * 0.9,
                icon_color,
                Anchor::Start,
                16.0,
            ));
        }
        // The percentage reads on the fill once the fill passes its middle,
        // whatever the slider's width.
        let label = format!("{value}%");
        let label_mid = w - 18.0 - self.tw(&label, m.label, Role::Text) / 2.0;
        let on_fill = 4.0 + fraction * (w - 8.0) > label_mid;
        node = node
            .part(Part::label(
                label,
                m.label,
                if on_fill { t.on_accent } else { t.fg },
                Anchor::End,
                18.0,
            ))
            .hit(Cmd::Slider(slider), HitKind::Slider)
            .no_ripple();
        node.snap_progress = dragging;
        node.press_glow = false;
        node
    }

    /// The Sound panel, in Omarchy's audio panel sections: Output (volume,
    /// output devices, now playing), Input (microphone volume, input devices)
    /// and Sources (per-app volume).
    fn panel_audio(&self, x: &mut f32, nodes: &mut Vec<Node>) -> Option<(f32, f32)> {
        let m = self.m();
        let tabs: Vec<(String, String, Cmd)> = SoundTab::ALL
            .iter()
            .map(|&tab| (tab.label().into(), tab.label().into(), Cmd::SoundTab(tab)))
            .collect();
        self.segmented(
            "p/snd/tab",
            x,
            None,
            &tabs,
            self.sound_tab.label(),
            None,
            nodes,
        );
        *x += m.gap;
        match self.sound_tab {
            SoundTab::Output => self.sound_output(x, nodes),
            SoundTab::Input => self.sound_input(x, nodes),
            SoundTab::Sources => self.sound_sources(x, nodes),
        }
    }

    /// A mute button: red with a tinted fill while muted.
    fn mute_chip(&self, key: Key, x: f32, glyph: &str, muted: bool, action: Action) -> Node {
        let t = &self.theme;
        let mut node = self
            .chip(
                key,
                Some((glyph, Role::Icon, if muted { t.red } else { t.fg })),
                None,
                self.surface(),
            )
            .tap(Cmd::Do(action));
        node.rect.x = x;
        node.emph = if muted { 1.0 } else { 0.0 };
        node.fill.as_mut().unwrap().emph = Some(t.red.alpha(0.22).over(t.surface));
        node
    }

    /// Output or input devices as a list; tapping one makes it the default.
    fn sound_devices(
        &self,
        base: &'static str,
        x: &mut f32,
        devices: &[AudioDevice],
        input: bool,
        nodes: &mut Vec<Node>,
    ) {
        let t = &self.theme;
        for d in devices {
            let color = if d.default { t.accent } else { t.fg_dim };
            let cmd = (!d.default).then(|| {
                Cmd::Do(if input {
                    Action::SetDefaultInput {
                        id: d.id,
                        name: d.name.clone(),
                    }
                } else {
                    Action::SetDefaultOutput {
                        id: d.id,
                        name: d.name.clone(),
                    }
                })
            });
            nodes.push(self.list_chip(
                Key::new(base).with(&d.name),
                x,
                (device_glyph(d.kind), color),
                &d.label,
                d.default.then_some((glyph::CHECK, t.accent)),
                d.default,
                cmd,
            ));
        }
    }

    /// A dim note in the panel's list row.
    fn panel_note(&self, key: Key, x: &mut f32, text: &str, nodes: &mut Vec<Node>) {
        let m = self.m();
        let mut node = self
            .chip(key, None, Some((text, self.theme.fg_dim)), self.flat())
            .group(0);
        node.rect.x = *x;
        *x += node.rect.w + m.gap;
        nodes.push(node);
    }

    fn sound_output(&self, x: &mut f32, nodes: &mut Vec<Node>) -> Option<(f32, f32)> {
        let m = self.m();
        let t = self.theme.clone();
        let s = &self.state;
        if let Some(a) = &s.audio {
            let (g, _) = volume_glyph(a.volume, a.muted, &t);
            let mute = self.mute_chip(Key::new("p/mute"), *x, g, a.muted, Action::ToggleMute);
            *x += mute.rect.w + m.gap;
            nodes.push(mute);
            nodes.push(self.slider(
                Key::new("p/volume"),
                *x,
                SOUND_SLIDER_W,
                if a.muted { 0 } else { a.volume },
                Some(glyph::VOLUME_HIGH),
                Slider::Volume,
                if a.muted { t.fg_faint } else { t.accent },
            ));
            *x += SOUND_SLIDER_W + m.gap * 2.0;
        }
        let row_start = *x;
        if !s.mixer.outputs.is_empty() {
            self.sound_devices("p/snd/out", x, &s.mixer.outputs, false, nodes);
        } else if let Some(a) = &s.audio {
            // Device details unavailable: still name the output.
            nodes.push(self.list_chip(
                Key::new("p/sink"),
                x,
                (glyph::SPEAKER, t.accent),
                &a.sink,
                Some((glyph::CHECK, t.accent)),
                true,
                None,
            ));
        } else {
            self.panel_note(Key::new("p/snd/out/none"), x, "No output devices", nodes);
        }
        if let Some(media) = s.media.as_ref().filter(|m| !m.title.is_empty()) {
            *x += m.gap;
            // Narrow the now-playing card so the transport stays on screen
            // when the outputs leave room.
            let transport_w = 3.0 * (MEDIA_BUTTON_W + m.gap) - m.gap;
            let room = self.width - m.edge - *x - transport_w - m.gap;
            let info_w = room.clamp(220.0, 320.0).floor();
            let title = self.fit(&media.title, info_w - 30.0, m.small * 1.12, Role::Bold);
            let artist = self.fit(
                if media.artist.is_empty() {
                    &media.player
                } else {
                    &media.artist
                },
                info_w - 30.0,
                m.small,
                Role::Text,
            );
            nodes.push(
                Node::new(
                    Key::new("p/media").index(hash(&title)),
                    Rect::new(*x, m.inset, info_w, m.bh),
                )
                .fill(Fill::solid(t.well, m.radius))
                .part(Part::Text {
                    text: title,
                    size: m.small * 1.12,
                    role: Role::Bold,
                    color: t.fg,
                    emph: None,
                    anchor: Anchor::Start,
                    dx: 16.0,
                    dy: -10.0,
                    valign: VAlign::Cap,
                })
                .part(Part::Text {
                    text: artist,
                    size: m.small,
                    role: Role::Text,
                    color: t.fg_dim,
                    emph: None,
                    anchor: Anchor::Start,
                    dx: 16.0,
                    dy: 11.0,
                    valign: VAlign::Cap,
                })
                .group(0)
                .tap(Cmd::Do(Action::MediaPlayPause)),
            );
            *x += info_w + m.gap;
            let pp = if media.playing {
                glyph::PAUSE
            } else {
                glyph::PLAY
            };
            for (name, g, action) in [
                ("prev", glyph::PREVIOUS, Action::MediaPrevious),
                ("pp", pp, Action::MediaPlayPause),
                ("next", glyph::NEXT, Action::MediaNext),
            ] {
                let mut b = self
                    .chip(
                        Key::new("p/media").with(name),
                        Some((g, Role::Icon, t.fg)),
                        None,
                        self.surface(),
                    )
                    .group(0)
                    .tap(Cmd::Do(action));
                b.rect.x = *x;
                b.rect.w = MEDIA_BUTTON_W;
                *x += b.rect.w + m.gap;
                nodes.push(b);
            }
        }
        Some((row_start, *x - m.gap))
    }

    fn sound_input(&self, x: &mut f32, nodes: &mut Vec<Node>) -> Option<(f32, f32)> {
        let m = self.m();
        let t = self.theme.clone();
        let s = &self.state;
        if let Some(muted) = s.mic_muted {
            let g = if muted { glyph::MIC_OFF } else { glyph::MIC };
            let mute = self.mute_chip(Key::new("p/mic/mute"), *x, g, muted, Action::ToggleMicMute);
            *x += mute.rect.w + m.gap;
            nodes.push(mute);
            nodes.push(self.slider(
                Key::new("p/mic"),
                *x,
                SOUND_SLIDER_W,
                s.mic_volume.unwrap_or(0),
                Some(glyph::MIC),
                Slider::Mic,
                if muted { t.fg_faint } else { t.accent },
            ));
            *x += SOUND_SLIDER_W + m.gap * 2.0;
        }
        let row_start = *x;
        if s.mixer.inputs.is_empty() {
            self.panel_note(Key::new("p/snd/in/none"), x, "No input devices", nodes);
        } else {
            self.sound_devices("p/snd/in", x, &s.mixer.inputs, true, nodes);
        }
        Some((row_start, *x - m.gap))
    }

    /// Per-app playback volume: a mute toggle naming the app, then its slider.
    fn sound_sources(&self, x: &mut f32, nodes: &mut Vec<Node>) -> Option<(f32, f32)> {
        let m = self.m();
        let t = self.theme.clone();
        let row_start = *x;
        let streams = &self.state.mixer.streams;
        if streams.is_empty() {
            self.panel_note(
                Key::new("p/snd/app/none"),
                x,
                "No apps are playing audio",
                nodes,
            );
        }
        for st in streams {
            let key = Key::new("p/snd/app").index(st.id as i64);
            let label = self.fit(&st.label, 200.0, m.label, Role::Text);
            let (g, c) = if st.muted {
                (glyph::VOLUME_OFF, t.red)
            } else {
                (glyph::VOLUME_HIGH, t.fg)
            };
            let mut chip = self
                .chip(
                    key,
                    Some((g, Role::Icon, c)),
                    Some((&label, if st.muted { t.fg_dim } else { t.fg })),
                    self.surface(),
                )
                .group(0)
                .tap(Cmd::Do(Action::ToggleStreamMute(st.id)));
            chip.rect.x = *x;
            chip.emph = if st.muted { 1.0 } else { 0.0 };
            chip.fill.as_mut().unwrap().emph = Some(t.red.alpha(0.22).over(t.surface));
            *x += chip.rect.w + m.gap;
            nodes.push(chip);
            nodes.push(
                self.slider(
                    key.with("volume"),
                    *x,
                    STREAM_SLIDER_W,
                    st.volume,
                    None,
                    Slider::Stream(st.id),
                    if st.muted { t.fg_faint } else { t.accent },
                )
                .group(0),
            );
            *x += STREAM_SLIDER_W + m.gap * 3.0;
        }
        *x -= m.gap * 2.0;
        if streams.is_empty() {
            *x += m.gap * 2.0;
        }
        Some((row_start, *x - m.gap))
    }

    fn info(&self, key: Key, x: f32, icon: (&str, Rgba), title: &str, subtitle: &str) -> Node {
        let m = self.m();
        let t = &self.theme;
        let (il, ir) = self.fonts.ink_hbounds(icon.0, m.icon * 1.1, Role::Icon);
        let icon_w = ir - il;
        let text_w =
            self.tw(title, m.small * 1.15, Role::Bold)
                .max(self.tw(subtitle, m.small, Role::Text));
        Node::new(
            key,
            Rect::new(x, m.inset, (icon_w + text_w + 52.0).round(), m.bh),
        )
        .fill(Fill::solid(t.well, m.radius))
        .part(Part::icon(
            icon.0,
            m.icon * 1.1,
            icon.1,
            Anchor::Start,
            16.0 - il,
        ))
        .part(Part::Text {
            text: title.into(),
            size: m.small * 1.15,
            role: Role::Bold,
            color: t.fg,
            emph: None,
            anchor: Anchor::Start,
            dx: 30.0 + icon_w,
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
            dx: 30.0 + icon_w,
            dy: 11.0,
            valign: VAlign::Cap,
        })
    }

    fn panel_power(&self, x: &mut f32, nodes: &mut Vec<Node>) {
        let m = self.m();
        let t = self.theme.clone();
        let s = &self.state;
        if let Some(b) = &s.battery {
            let (g, c) = battery_glyph(b.percent, b.charge, &t);
            let title = format!("{}%", b.percent.round() as i32);
            let subtitle = match b.charge {
                Charge::Charging => "Charging",
                Charge::Full => "Fully charged",
                Charge::Holding => "Holding at charge limit",
                Charge::NotCharging => "Plugged in, not charging",
                Charge::Discharging | Charge::Unknown => "On battery",
            };
            let node = self.info(Key::new("p/battery"), *x, (g, c), &title, subtitle);
            *x += node.rect.w + m.gap * 2.0;
            nodes.push(node);

            // Details, as in Omarchy's battery panel.
            let discharging = b.charge == Charge::Discharging;
            let holding = b.charge == Charge::Holding;
            let full = b.charge == Charge::Full;
            // Omarchy hides the time while the flow is negligible.
            let idle = b.rate_watts <= 0.2 || (!discharging && b.seconds_to_full >= 8 * 3600);
            let time = if !b.time.is_empty() {
                b.time.clone()
            } else if discharging && b.seconds_to_empty > 0 {
                duration(b.seconds_to_empty)
            } else if b.seconds_to_full > 0 {
                duration(b.seconds_to_full)
            } else {
                "—".into()
            };
            let stats: [(&str, String, Rgba); 4] = [
                if holding {
                    ("Charge limit", spaced(&b.threshold, "—"), t.fg)
                } else if discharging {
                    ("Time left", if idle { "—".into() } else { time }, t.fg)
                } else {
                    (
                        "Time to full",
                        if idle || full { "—".into() } else { time },
                        t.fg,
                    )
                },
                if holding {
                    ("Battery state", "Holding".into(), t.fg)
                } else if discharging {
                    ("Discharging", format!("{:.1} W", b.rate_watts), t.orange)
                } else {
                    (
                        "Charging",
                        if full {
                            "—".into()
                        } else {
                            format!("{:.1} W", b.rate_watts)
                        },
                        t.green,
                    )
                },
                ("Battery size", spaced(&b.size, "—"), t.fg),
                (
                    "Charge cycles",
                    if b.cycles.is_empty() {
                        "—".into()
                    } else {
                        b.cycles.clone()
                    },
                    t.fg,
                ),
            ];
            for (label, value, color) in stats {
                let w = (self.tw(label, m.small, Role::Text).max(self.tw(
                    &value,
                    m.label * 0.95,
                    Role::Bold,
                )) + m.pad * 2.0)
                    .round();
                nodes.push(
                    Node::new(
                        Key::new("p/battery/stat").with(label),
                        Rect::new(*x, m.inset, w, m.bh),
                    )
                    .fill(Fill::solid(t.well, m.radius))
                    .part(Part::Text {
                        text: label.into(),
                        size: m.small,
                        role: Role::Text,
                        color: t.fg_dim,
                        emph: None,
                        anchor: Anchor::Start,
                        dx: m.pad,
                        dy: -11.0,
                        valign: VAlign::Cap,
                    })
                    .part(Part::Text {
                        text: value,
                        size: m.label * 0.95,
                        role: Role::Bold,
                        color,
                        emph: None,
                        anchor: Anchor::Start,
                        dx: m.pad,
                        dy: 10.0,
                        valign: VAlign::Cap,
                    }),
                );
                *x += w + m.gap;
            }
            *x += m.gap * 2.0;
        }
        let profiles: Vec<String> = if s.power_profiles.is_empty() {
            vec![
                "power-saver".into(),
                "balanced".into(),
                "performance".into(),
            ]
        } else {
            s.power_profiles.clone()
        };
        let segments: Vec<(String, &str, String, f32)> = profiles
            .iter()
            .map(|p| {
                let (g, label) = match p.as_str() {
                    "power-saver" => (glyph::SAVER, "Saver"),
                    "balanced" => (glyph::BALANCED, "Balanced"),
                    "performance" => (glyph::PERFORMANCE, "Performance"),
                    other => (glyph::BALANCED, other),
                };
                let w = self.iw(g, m.icon, Role::Icon)
                    + 10.0
                    + self.tw(label, m.label, Role::Text)
                    + m.pad * 2.0;
                (p.clone(), g, label.to_string(), w.round())
            })
            .collect();
        let pad = 4.0;
        let total: f32 = segments.iter().map(|s| s.3).sum::<f32>() + pad * 2.0;
        nodes.push(
            Node::new(Key::new("p/profiles"), Rect::new(*x, m.inset, total, m.bh))
                .fill(Fill::solid(t.well, m.radius)),
        );
        let mut sx = *x + pad;
        let active = s.power_profile.clone().unwrap_or_default();
        if let Some(i) = segments.iter().position(|s| s.0 == active) {
            let px = *x + pad + segments[..i].iter().map(|s| s.3).sum::<f32>();
            nodes.push(
                Node::new(
                    Key::new("p/profiles/pill"),
                    Rect::new(px, m.inset + pad, segments[i].3, m.bh - pad * 2.0),
                )
                .fill(Fill {
                    color: t.accent,
                    emph: None,
                    radius: m.radius - 3.0,
                    border: None,
                    sheen: true,
                }),
            );
        }
        for (name, g, label, w) in segments {
            let on = name == active;
            let (il, ir) = self.fonts.ink_hbounds(g, m.icon, Role::Icon);
            let icon_w = ir - il;
            let mut node = Node::new(
                Key::new("p/profile").with(&name),
                Rect::new(sx, m.inset, w, m.bh),
            )
            .part(Part::icon(g, m.icon, t.fg_dim, Anchor::Start, m.pad - il).with_emph(t.on_accent))
            .part(
                Part::label(label, m.label, t.fg, Anchor::Start, m.pad + icon_w + 10.0)
                    .with_emph(t.on_accent),
            )
            .emph(if on { 1.0 } else { 0.0 })
            .tap(Cmd::Do(Action::SetPowerProfile(name.clone())))
            .no_ripple();
            node.press_glow = false;
            nodes.push(node);
            sx += w;
        }
        *x += total + m.gap;
    }

    fn toggle_chip(&self, key: Key, x: f32, label: &str, on: bool, action: Action) -> Node {
        let m = self.m();
        let t = &self.theme;
        let text_w = self.tw(label, m.label, Role::Text);
        let switch_w = 46.0;
        let w = (m.pad + text_w + 14.0 + switch_w + m.pad).round();
        // The switch itself is a separate node (see `switch`) so its knob
        // can spring independently.
        Node::new(key, Rect::new(x, m.inset, w, m.bh))
            .fill(Fill {
                color: t.surface,
                emph: Some(t.accent.alpha(0.18).over(t.surface)),
                radius: m.radius,
                border: None,
                sheen: true,
            })
            .part(Part::label(label, m.label, t.fg, Anchor::Start, m.pad))
            .tap(Cmd::Do(action))
            .emph(if on { 1.0 } else { 0.0 })
    }

    /// A switch track plus a knob that springs between its ends.
    fn switch(&self, parent: &Node, on: bool, nodes: &mut Vec<Node>) {
        self.switch_colored(parent, on, self.theme.accent, nodes);
    }

    fn switch_colored(&self, parent: &Node, on: bool, active_color: Rgba, nodes: &mut Vec<Node>) {
        let m = self.m();
        let t = &self.theme;
        let w = 46.0;
        let h = 26.0;
        let x = parent.rect.right() - m.pad - w;
        let y = self.height / 2.0 - h / 2.0;
        let mut track = Node::new(parent.key.with("switch"), Rect::new(x, y, w, h))
            .fill(Fill {
                color: t.fg_faint.alpha(0.45),
                emph: Some(active_color),
                radius: h / 2.0,
                border: None,
                sheen: false,
            })
            .emph(if on { 1.0 } else { 0.0 })
            .no_ripple();
        track.group = parent.group;
        track.press_glow = false;
        nodes.push(track);
        let k = h - 7.0;
        let kx = if on { x + w - k - 3.5 } else { x + 3.5 };
        let mut knob = Node::new(parent.key.with("knob"), Rect::new(kx, y + 3.5, k, k))
            .fill(Fill {
                color: t.fg,
                emph: Some(t.on_accent),
                radius: k / 2.0,
                border: None,
                sheen: false,
            })
            .emph(if on { 1.0 } else { 0.0 })
            .no_ripple();
        knob.group = parent.group;
        knob.press_glow = false;
        nodes.push(knob);
    }

    /// A list chip in the panel's scrolling row.
    fn list_chip(
        &self,
        key: Key,
        x: &mut f32,
        icon: (&str, Rgba),
        label: &str,
        suffix: Option<(&str, Rgba)>,
        active: bool,
        cmd: Option<Cmd>,
    ) -> Node {
        let m = self.m();
        let t = &self.theme;
        let fill = Fill {
            color: t.surface,
            emph: Some(t.accent.alpha(0.2).over(t.surface)),
            radius: m.radius,
            border: None,
            sheen: true,
        };
        let label = self.fit(label, 300.0, m.label, Role::Text);
        let mut node = self.chip(
            key,
            Some((icon.0, Role::Icon, icon.1)),
            Some((&label, t.fg)),
            fill,
        );
        if let Some((g, c)) = suffix {
            let sw = self.iw(g, m.icon * 0.75, Role::Icon);
            node.parts
                .push(Part::icon(g, m.icon * 0.75, c, Anchor::End, m.pad - 4.0));
            node.rect.w += sw + 6.0;
        }
        node.rect.x = *x;
        node.emph = if active { 1.0 } else { 0.0 };
        node.group = Some(0);
        if let Some(cmd) = cmd {
            node = node.tap(cmd);
        }
        *x += node.rect.w + m.gap;
        node
    }

    /// Truncates `text` from the left (keeping the tail) with a leading ellipsis to fit `max` px.
    fn fit_tail(&self, text: &str, max: f32, size: f32, role: Role) -> String {
        if self.tw(text, size, role) <= max {
            return text.to_string();
        }
        let chars: Vec<char> = text.chars().collect();
        let (mut lo, mut hi) = (0usize, chars.len());
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            let start = chars.len().saturating_sub(mid);
            let candidate = format!("…{}", chars[start..].iter().collect::<String>());
            if self.tw(&candidate, size, role) <= max {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        let start = chars.len().saturating_sub(lo);
        format!("…{}", chars[start..].iter().collect::<String>())
    }

    #[allow(clippy::too_many_arguments)]
    fn net_stat(
        &self,
        key: Key,
        x: &mut f32,
        label: &str,
        label_color: Rgba,
        value: &str,
        value_color: Rgba,
        min_sample: &str,
        border: Option<Rgba>,
        cmd: Option<Cmd>,
        nodes: &mut Vec<Node>,
    ) {
        let m = self.m();
        let t = &self.theme;
        let label_w = self.tw(label, m.small, Role::Text);
        let value_w = self.tw(value, m.label * 0.95, Role::Bold).max(self.tw(
            min_sample,
            m.label * 0.95,
            Role::Bold,
        ));
        let w = (label_w.max(value_w) + m.pad * 2.0).round();
        let mut node = Node::new(key, Rect::new(*x, m.inset, w, m.bh))
            .fill(Fill {
                color: t.well,
                emph: None,
                radius: m.radius,
                border,
                sheen: false,
            })
            .part(Part::Text {
                text: label.into(),
                size: m.small,
                role: Role::Text,
                color: label_color,
                emph: None,
                anchor: Anchor::Start,
                dx: m.pad,
                dy: -11.0,
                valign: VAlign::Cap,
            })
            .part(Part::Text {
                text: value.into(),
                size: m.label * 0.95,
                role: Role::Bold,
                color: value_color,
                emph: None,
                anchor: Anchor::Start,
                dx: m.pad,
                dy: 10.0,
                valign: VAlign::Cap,
            })
            .group(0);
        if let Some(cmd) = cmd {
            node = node.tap(cmd);
        } else {
            node = node.no_ripple();
            node.press_glow = false;
        }
        *x += w + m.gap;
        nodes.push(node);
    }

    /// A segmented pill selector (Sound panel sections; DNS provider and
    /// Wi-Fi band in the scrolling network strip, matching the battery
    /// profile pill). `prefix` is an optional icon and caption before the
    /// options; `group` puts it in a scrolling row.
    #[allow(clippy::too_many_arguments)]
    fn segmented(
        &self,
        base_key: &'static str,
        x: &mut f32,
        prefix: Option<(Option<&str>, &str)>,
        options: &[(String, String, Cmd)],
        active_id: &str,
        group: Option<u8>,
        nodes: &mut Vec<Node>,
    ) {
        if options.is_empty() {
            return;
        }
        let m = self.m();
        let t = &self.theme;
        let pad = 4.0;
        let seg_size = m.label * 0.92;
        let prefix_size = m.small * 1.02;

        let mut well = Node::new(Key::new(base_key), Rect::new(*x, m.inset, 0.0, m.bh))
            .fill(Fill::solid(t.well, m.radius))
            .no_ripple();
        well.group = group;
        well.press_glow = false;

        let seg_start = if let Some((prefix_icon, prefix_label)) = prefix {
            let mut cursor = 14.0f32;
            if let Some(icon) = prefix_icon {
                let icon_size = m.icon * 0.78;
                let (il, ir) = self.fonts.ink_hbounds(icon, icon_size, Role::Icon);
                well = well.part(Part::icon(
                    icon,
                    icon_size,
                    t.fg_dim,
                    Anchor::Start,
                    cursor - il,
                ));
                cursor += (ir - il) + 6.0;
            }
            let tag_w = self.tw(prefix_label, prefix_size, Role::Bold);
            well = well.part(Part::Text {
                text: prefix_label.into(),
                size: prefix_size,
                role: Role::Bold,
                color: t.fg_dim,
                emph: None,
                anchor: Anchor::Start,
                dx: cursor,
                dy: 0.0,
                valign: VAlign::Cap,
            });
            (cursor + tag_w + 12.0).round()
        } else {
            pad
        };

        let widths: Vec<f32> = options
            .iter()
            .map(|(_, label, _)| {
                (self.tw(label, seg_size, Role::Text) + 26.0)
                    .round()
                    .max(64.0)
            })
            .collect();
        let total = seg_start + widths.iter().sum::<f32>() + pad;
        well.rect.w = total;
        nodes.push(well);

        if let Some(i) = options.iter().position(|(id, _, _)| id == active_id) {
            let px = *x + seg_start + widths[..i].iter().sum::<f32>();
            let mut pill = Node::new(
                Key::new(base_key).with("pill"),
                Rect::new(px, m.inset + pad, widths[i], m.bh - pad * 2.0),
            )
            .fill(Fill {
                color: t.accent,
                emph: None,
                radius: m.radius - 3.0,
                border: None,
                sheen: true,
            })
            .no_ripple();
            pill.group = group;
            pill.press_glow = false;
            nodes.push(pill);
        }

        let mut sx = *x + seg_start;
        for ((id, label, cmd), w) in options.iter().zip(widths) {
            let on = id == active_id;
            let mut seg = Node::new(
                Key::new(base_key).with("opt").with(id),
                Rect::new(sx, m.inset, w, m.bh),
            )
            .part(Part::Text {
                text: label.clone(),
                size: seg_size,
                role: Role::Text,
                color: t.fg,
                emph: Some(t.on_accent),
                anchor: Anchor::Center,
                dx: 0.0,
                dy: 0.0,
                valign: VAlign::Cap,
            })
            .emph(if on { 1.0 } else { 0.0 })
            .tap(cmd.clone())
            .no_ripple();
            seg.group = group;
            seg.press_glow = false;
            nodes.push(seg);
            sx += w;
        }
        *x += total + m.gap;
    }

    fn panel_network(&self, x: &mut f32, nodes: &mut Vec<Node>) -> Option<(f32, f32)> {
        if self.wifi_prompt.is_some() {
            self.panel_wifi_prompt(*x, nodes);
            return None;
        }
        if self.speedtest.is_some() {
            self.panel_speedtest(*x, nodes);
            return None;
        }
        let m = self.m();
        let t = self.theme.clone();
        let n = &self.state.network;
        let row_start = *x;

        // 1. Controls & hero actions (Wi-Fi switch, Captive Portal, QR share, Speed Test).
        if n.wifi_available || n.wifi_enabled || !n.networks.is_empty() {
            let toggle = self
                .toggle_chip(
                    Key::new("p/wifi"),
                    *x,
                    "Wi-Fi",
                    n.wifi_enabled,
                    Action::SetWifi(!n.wifi_enabled),
                )
                .group(0);
            *x += toggle.rect.w + m.gap;
            nodes.push(toggle.clone());
            self.switch(&toggle, n.wifi_enabled, nodes);
        }

        if n.has_captive_portal() {
            let mut portal = self
                .chip(
                    Key::new("p/net/portal"),
                    Some((glyph::PORTAL, Role::Icon, t.red)),
                    Some(("Sign in", t.fg)),
                    Fill {
                        color: t.red.alpha(0.22).over(t.surface),
                        emph: None,
                        radius: m.radius,
                        border: Some(t.red.alpha(0.7)),
                        sheen: true,
                    },
                )
                .group(0)
                .tap(Cmd::Do(Action::OpenCaptivePortal));
            portal.rect.x = *x;
            *x += portal.rect.w + m.gap;
            nodes.push(portal);
        }

        if n.kind == NetKind::Wifi && n.details.can_share_qr {
            let mut qr = self
                .chip(
                    Key::new("p/net/qr"),
                    Some((glyph::QR_CODE, Role::Icon, t.fg)),
                    None,
                    self.surface(),
                )
                .group(0)
                .tap(Cmd::Do(Action::ShareWifiQr {
                    iface: n.details.iface.clone(),
                    ssid: n.name.clone(),
                }));
            qr.rect.x = *x;
            *x += qr.rect.w + m.gap;
            nodes.push(qr);
        }

        if matches!(n.kind, NetKind::Wifi | NetKind::Ethernet) {
            let mut speed = self
                .chip(
                    Key::new("p/net/speedtest"),
                    Some((glyph::PERFORMANCE, Role::Icon, t.cyan.mix(t.accent, 0.35))),
                    Some(("Speed Test", t.fg)),
                    Fill {
                        color: t.accent.alpha(0.18).over(t.surface),
                        emph: None,
                        radius: m.radius,
                        border: Some(t.cyan.mix(t.accent, 0.4).alpha(0.45)),
                        sheen: true,
                    },
                )
                .group(0)
                .tap(Cmd::SpeedTestStart);
            speed.rect.x = *x;
            *x += speed.rect.w + m.gap;
            nodes.push(speed);
        }

        if *x > row_start {
            *x += m.gap;
        }

        // 2. Ethernet & Wi-Fi network selection first for instant switching.
        let networks_start = *x;
        if n.kind == NetKind::Ethernet {
            let base_name = if n.name.is_empty() {
                "Ethernet"
            } else {
                n.name.as_str()
            };
            let eth_label = match n.details.speed_mbps.filter(|&s| s > 0) {
                Some(speed) => format!("{base_name} · {}", format_eth_speed(speed)),
                None => base_name.to_string(),
            };
            let (eth_icon, eth_color) = if n.restricted() {
                (glyph::ETHERNET_ALERT, t.red)
            } else {
                (glyph::ETHERNET, t.accent)
            };
            nodes.push(self.list_chip(
                Key::new("p/net/eth"),
                x,
                (eth_icon, eth_color),
                &eth_label,
                Some((glyph::CHECK, eth_color)),
                true,
                None,
            ));
        }
        if !n.wifi_enabled && n.wifi_available {
            let mut node = self
                .chip(
                    Key::new("p/net/off"),
                    None,
                    Some(("Wi-Fi is off", t.fg_dim)),
                    self.flat(),
                )
                .group(0);
            node.rect.x = *x;
            *x += node.rect.w + m.gap;
            nodes.push(node);
        } else if n.wifi_enabled && n.networks.is_empty() {
            let mut node = self
                .chip(
                    Key::new("p/net/scan"),
                    None,
                    Some(("Scanning for networks…", t.fg_dim)),
                    self.flat(),
                )
                .group(0);
            node.rect.x = *x;
            *x += node.rect.w + m.gap;
            nodes.push(node);
        }
        if n.wifi_enabled {
            const COLLAPSED_UNKNOWN_LIMIT: usize = 3;
            let known_nets = n.networks.iter().filter(|net| net.active || net.known);
            let unknown_nets: Vec<_> = n
                .networks
                .iter()
                .filter(|net| !net.active && !net.known)
                .collect();
            let has_more_unknown = unknown_nets.len() > COLLAPSED_UNKNOWN_LIMIT;
            let visible_unknown = if self.wifi_networks_expanded {
                unknown_nets.as_slice()
            } else {
                &unknown_nets[..unknown_nets.len().min(COLLAPSED_UNKNOWN_LIMIT)]
            };

            for network in known_nets.chain(visible_unknown.iter().copied()) {
                let restricted_active = network.active && n.kind == NetKind::Wifi && n.restricted();
                let strength = if restricted_active {
                    glyph::WIFI_ALERT
                } else {
                    glyph::WIFI[((network.signal as usize) / 20).min(4)]
                };
                let suffix = if network.active {
                    Some((
                        glyph::CHECK,
                        if restricted_active { t.red } else { t.accent },
                    ))
                } else if network.secure && !network.known {
                    Some((glyph::LOCK, t.fg_faint))
                } else {
                    None
                };
                let cmd = (!network.active).then(|| {
                    if network.secure && !network.known {
                        Cmd::WifiPromptOpen {
                            ssid: network.ssid.clone(),
                            signal: network.signal,
                        }
                    } else {
                        Cmd::Do(Action::ConnectWifi {
                            ssid: network.ssid.clone(),
                            known: network.known,
                            secure: network.secure,
                        })
                    }
                });
                let color = if restricted_active {
                    t.red
                } else if network.active {
                    t.accent
                } else if network.known {
                    t.fg
                } else {
                    t.fg_dim
                };
                nodes.push(self.list_chip(
                    Key::new("p/net/ssid").with(&network.ssid),
                    x,
                    (strength, color),
                    &network.ssid,
                    suffix,
                    network.active,
                    cmd,
                ));
            }

            if has_more_unknown {
                let mut toggle_more = if self.wifi_networks_expanded {
                    self.chip(
                        Key::new("p/net/wifi/more"),
                        Some((glyph::BACK, Role::Icon, t.fg_dim)),
                        None,
                        self.surface(),
                    )
                } else {
                    let hidden = unknown_nets.len() - COLLAPSED_UNKNOWN_LIMIT;
                    let label = format!("+{hidden}");
                    let label_size = m.label * 0.92;
                    let icon_size = m.icon * 0.85;
                    let lw = self.tw(&label, label_size, Role::Text);
                    let (il, ir) = self
                        .fonts
                        .ink_hbounds(glyph::CHEVRON, icon_size, Role::Icon);
                    let iw = ir - il;
                    let pad = m.pad - 3.0;
                    let w = (pad + lw + 6.0 + iw + pad).round().max(64.0);
                    Node::new(
                        Key::new("p/net/wifi/more"),
                        Rect::new(0.0, m.inset, w, m.bh),
                    )
                    .fill(self.surface())
                    .part(Part::label(label, label_size, t.fg_dim, Anchor::Start, pad))
                    .part(Part::icon(
                        glyph::CHEVRON,
                        icon_size,
                        t.fg,
                        Anchor::Start,
                        pad + lw + 6.0 - il,
                    ))
                }
                .group(0)
                .tap(Cmd::ToggleWifiNetworksExpanded);
                toggle_more.rect.x = *x;
                *x += toggle_more.rect.w + m.gap;
                nodes.push(toggle_more);
            }
        }
        if *x > networks_start {
            *x += m.gap;
        }

        // 3. Live connection details matching Omarchy's network panel:
        //    Ping, Packet loss, Receiving, Sending, Downloaded, Uploaded, IP address, Gateway.
        if n.has_details() {
            let d = &n.details;
            let ping_readable = n.internet_ping_readable();
            let ping_urgent = ping_readable && d.packet_loss > 0;
            let ping_color = if ping_urgent { t.red } else { t.fg };
            let has_transfer = n.has_transfer_stats();

            let ping_text = format_ping_latency(d.ping_ms, ping_readable);
            self.net_stat(
                Key::new("p/net/stat/ping"),
                x,
                "Ping",
                t.fg_dim,
                &ping_text,
                ping_color,
                "Timeout",
                None,
                None,
                nodes,
            );

            let loss_text = format_packet_loss(d.packet_loss, ping_readable);
            self.net_stat(
                Key::new("p/net/stat/loss"),
                x,
                "Packet loss",
                t.fg_dim,
                &loss_text,
                ping_color,
                "100%",
                None,
                None,
                nodes,
            );

            let rx_rate = if has_transfer {
                format_rate(d.download_rate)
            } else {
                "--".into()
            };
            self.net_stat(
                Key::new("p/net/stat/rx_rate"),
                x,
                "Receiving",
                t.fg_dim,
                &rx_rate,
                t.fg,
                "999.9 KB/s",
                None,
                None,
                nodes,
            );

            let tx_rate = if has_transfer {
                format_rate(d.upload_rate)
            } else {
                "--".into()
            };
            self.net_stat(
                Key::new("p/net/stat/tx_rate"),
                x,
                "Sending",
                t.fg_dim,
                &tx_rate,
                t.fg,
                "999.9 KB/s",
                None,
                None,
                nodes,
            );

            let rx_total = d.rx_bytes.map(format_bytes).unwrap_or_else(|| "--".into());
            self.net_stat(
                Key::new("p/net/stat/rx_total"),
                x,
                "Downloaded",
                t.fg_dim,
                &rx_total,
                t.fg,
                "999.9 MB",
                None,
                None,
                nodes,
            );

            let tx_total = d.tx_bytes.map(format_bytes).unwrap_or_else(|| "--".into());
            self.net_stat(
                Key::new("p/net/stat/tx_total"),
                x,
                "Uploaded",
                t.fg_dim,
                &tx_total,
                t.fg,
                "999.9 MB",
                None,
                None,
                nodes,
            );

            let ip_copied = self.is_copied("ip");
            let ip_text = if d.ip.is_empty() { "--" } else { d.ip.as_str() };
            let ip_cmd = (!d.ip.is_empty()).then(|| Cmd::CopyField {
                field: "ip",
                value: d.ip.clone(),
            });
            self.net_stat(
                Key::new("p/net/stat/ip"),
                x,
                if ip_copied { "Copied" } else { "IP address" },
                if ip_copied { t.green } else { t.fg_dim },
                ip_text,
                if ip_copied { t.green } else { t.fg },
                "000.000.0.00",
                ip_copied.then_some(t.green.alpha(0.7)),
                ip_cmd,
                nodes,
            );

            let gw_copied = self.is_copied("gateway");
            let gw_text = if d.gateway.is_empty() {
                "--"
            } else {
                d.gateway.as_str()
            };
            let gw_cmd = (!d.gateway.is_empty()).then(|| Cmd::CopyField {
                field: "gateway",
                value: d.gateway.clone(),
            });
            self.net_stat(
                Key::new("p/net/stat/gateway"),
                x,
                if gw_copied { "Copied" } else { "Gateway" },
                if gw_copied { t.green } else { t.fg_dim },
                gw_text,
                if gw_copied { t.green } else { t.fg },
                "000.000.0.0",
                gw_copied.then_some(t.green.alpha(0.7)),
                gw_cmd,
                nodes,
            );

            *x += m.gap;
        }

        // 4. Wi-Fi band selector (only on multi-band Wi-Fi or when pinned).
        if n.can_select_band() {
            let effective = n.effective_band();
            let auto_label = if effective == "auto" && !n.band_current.is_empty() {
                format!("Auto · {}", format_wifi_band(&n.band_current))
            } else {
                "Auto".into()
            };
            let mut options = vec![(
                "auto".to_string(),
                auto_label,
                Cmd::Do(Action::SetWifiBand("auto".into())),
            )];
            for band in &n.band_available {
                options.push((
                    band.clone(),
                    format_wifi_band(band),
                    Cmd::Do(Action::SetWifiBand(band.clone())),
                ));
            }
            if effective != "auto" && !options.iter().any(|(id, _, _)| id == effective) {
                options.push((
                    effective.to_string(),
                    format_wifi_band(effective),
                    Cmd::Do(Action::SetWifiBand(effective.to_string())),
                ));
            }
            self.segmented(
                "p/net/band",
                x,
                Some((None, "Band")),
                &options,
                effective,
                Some(0),
                nodes,
            );
            *x += m.gap;
        }

        // 5. DNS provider selector (DHCP, Cloudflare, Google, Custom).
        let dns_options: Vec<(String, String, Cmd)> = DnsProvider::ALL
            .iter()
            .map(|&provider| {
                let label = provider.label().to_string();
                (label.clone(), label, Cmd::Do(Action::SetDns(provider)))
            })
            .collect();
        self.segmented(
            "p/net/dns",
            x,
            Some((Some(glyph::GLOBE), "DNS")),
            &dns_options,
            n.effective_dns().label(),
            Some(0),
            nodes,
        );
        Some((row_start, *x - m.gap))
    }

    fn panel_wifi_prompt(&self, start_x: f32, nodes: &mut Vec<Node>) {
        let Some(prompt) = &self.wifi_prompt else {
            return;
        };
        let m = self.m();
        let t = self.theme.clone();
        let right = self.width - m.edge;

        let editable = matches!(
            prompt.phase,
            WifiPromptPhase::Editing | WifiPromptPhase::Failed(_)
        );
        let connecting = matches!(prompt.phase, WifiPromptPhase::Connecting);
        let connected = matches!(prompt.phase, WifiPromptPhase::Connected);
        let can_submit = editable && !prompt.password.is_empty();

        // 1. SSID badge on the left.
        let strength = glyph::WIFI[((prompt.signal as usize) / 20).min(4)];
        let ssid_label = self.fit(&prompt.ssid, 230.0, m.label * 0.95, Role::Bold);
        let badge_icon_color = if connected {
            t.green
        } else if matches!(prompt.phase, WifiPromptPhase::Failed(_)) {
            t.red
        } else {
            t.accent
        };
        let mut badge = self.chip(
            Key::new("p/wifi/ssid"),
            Some((strength, Role::Icon, badge_icon_color)),
            Some((&ssid_label, t.fg)),
            Fill::solid(t.well, m.radius),
        );
        let lock_w = self.iw(glyph::LOCK, m.icon * 0.72, Role::Icon);
        badge.parts.push(Part::icon(
            glyph::LOCK,
            m.icon * 0.72,
            t.fg_faint,
            Anchor::End,
            m.pad - 4.0,
        ));
        badge.rect.w += lock_w + 6.0;
        badge.rect.x = start_x;
        let field_x = badge.rect.right() + m.gap;
        nodes.push(badge);

        // 2. Right-edge action buttons: [ Show/Hide ] [ ⌫ ] [ ✓ Join ]
        let join_w = 152.0;
        let join_x = right - join_w;
        let (join_icon, join_label, join_fill, join_fg) = if connected {
            (
                glyph::CHECK,
                "Joined",
                Fill {
                    color: t.green,
                    emph: None,
                    radius: m.radius,
                    border: None,
                    sheen: true,
                },
                t.on_accent,
            )
        } else if connecting {
            (
                glyph::LOCK,
                "Joining…",
                Fill {
                    color: t.accent.alpha(0.32).over(t.surface),
                    emph: None,
                    radius: m.radius,
                    border: Some(t.accent.alpha(0.5)),
                    sheen: true,
                },
                t.fg_dim,
            )
        } else if can_submit {
            (
                glyph::CHECK,
                "Join",
                Fill {
                    color: t.accent,
                    emph: None,
                    radius: m.radius,
                    border: None,
                    sheen: true,
                },
                t.on_accent,
            )
        } else {
            (glyph::CHECK, "Join", self.surface(), t.fg_faint)
        };
        let mut join = Node::new(
            Key::new("p/wifi/join"),
            Rect::new(join_x, m.inset, join_w, m.bh),
        )
        .fill(join_fill);
        let j_icon_w = self.iw(join_icon, m.icon * 0.85, Role::Icon);
        let j_label_w = self.tw(join_label, m.label * 0.95, Role::Bold);
        let j_total = j_icon_w + 8.0 + j_label_w;
        let j_start = ((join_w - j_total) * 0.5).max(12.0);
        let (jil, _) = self.fonts.ink_hbounds(join_icon, m.icon * 0.85, Role::Icon);
        join = join
            .part(Part::icon(
                join_icon,
                m.icon * 0.85,
                join_fg,
                Anchor::Start,
                j_start - jil,
            ))
            .part(Part::Text {
                text: join_label.into(),
                size: m.label * 0.95,
                role: Role::Bold,
                color: join_fg,
                emph: None,
                anchor: Anchor::Start,
                dx: j_start + j_icon_w + 8.0,
                dy: 0.0,
                valign: VAlign::Cap,
            });
        if !can_submit && !connected && !connecting {
            join.opacity = 0.45;
        }
        if can_submit {
            join = join.tap(Cmd::WifiPromptSubmit);
        }

        // Backspace button
        let bs_w = 72.0;
        let bs_x = join_x - m.gap - bs_w;
        let mut bs = Node::new(Key::new("p/wifi/bs"), Rect::new(bs_x, m.inset, bs_w, m.bh))
            .fill(self.surface())
            .part(Part::icon(
                glyph::BACKSPACE,
                m.icon * 0.95,
                if editable && !prompt.password.is_empty() {
                    t.fg
                } else {
                    t.fg_faint
                },
                Anchor::Center,
                0.0,
            ));
        if editable && !prompt.password.is_empty() {
            bs = bs.hit(Cmd::WifiPromptBackspace, HitKind::Repeat);
        } else {
            bs.opacity = 0.45;
        }

        // Show / Hide password toggle button (fixed width so toggling never shifts layout!)
        let vis_w = 132.0;
        let vis_x = bs_x - m.gap - vis_w;
        let (vis_icon, vis_label, vis_color) = if prompt.revealed {
            (glyph::EYE_OFF, "Hide", t.accent)
        } else {
            (glyph::EYE, "Show", t.fg)
        };
        let mut vis = Node::new(
            Key::new("p/wifi/reveal"),
            Rect::new(vis_x, m.inset, vis_w, m.bh),
        )
        .fill(Fill {
            color: t.surface,
            emph: Some(t.accent.alpha(0.24).over(t.surface)),
            radius: m.radius,
            border: prompt.revealed.then_some(t.accent.alpha(0.6)),
            sheen: true,
        })
        .emph(if prompt.revealed { 1.0 } else { 0.0 });
        let (vil, vir) = self.fonts.ink_hbounds(vis_icon, m.icon * 0.9, Role::Icon);
        let v_icon_w = vir - vil;
        let v_label_w = self.tw(vis_label, m.label * 0.92, Role::Text);
        let v_start = ((vis_w - (v_icon_w + 9.0 + v_label_w)) * 0.5).max(12.0);
        vis = vis
            .part(Part::icon(
                vis_icon,
                m.icon * 0.9,
                vis_color,
                Anchor::Start,
                v_start - vil,
            ))
            .part(Part::label(
                vis_label,
                m.label * 0.92,
                if prompt.revealed { t.fg } else { t.fg_dim },
                Anchor::Start,
                v_start + v_icon_w + 9.0,
            ));
        if editable {
            vis = vis.tap(Cmd::WifiPromptToggleReveal);
        } else {
            vis.opacity = 0.45;
        }

        // 3. Center Password Input Field spanning the remaining Touch Bar width!
        let field_w = (vis_x - m.gap - field_x).max(200.0);
        let (field_fill, border) = match &prompt.phase {
            WifiPromptPhase::Editing => (
                t.well,
                Some(if prompt.revealed {
                    t.yellow.mix(t.accent, 0.45).alpha(0.75)
                } else {
                    t.accent.alpha(0.72)
                }),
            ),
            WifiPromptPhase::Connecting => (t.well, Some(t.accent.alpha(0.9))),
            WifiPromptPhase::Connected => {
                (t.green.alpha(0.18).over(t.well), Some(t.green.alpha(0.9)))
            }
            WifiPromptPhase::Failed(_) => (t.red.alpha(0.14).over(t.well), Some(t.red.alpha(0.88))),
        };
        let mut field = Node::new(
            Key::new("p/wifi/field"),
            Rect::new(field_x, m.inset, field_w, m.bh),
        )
        .fill(Fill {
            color: field_fill,
            emph: None,
            radius: m.radius,
            border,
            sheen: false,
        })
        .no_ripple();
        field.press_glow = false;

        let text_pad = 18.0;
        let text_size = m.label * 1.04;
        match &prompt.phase {
            WifiPromptPhase::Connecting => {
                field.parts.push(Part::Shimmer {
                    color: t.accent,
                    radius: m.radius,
                });
                let msg = format!("Connecting to {}…", prompt.ssid);
                let msg = self.fit(&msg, field_w - text_pad * 2.0, m.label, Role::Text);
                field
                    .parts
                    .push(Part::label(msg, m.label, t.fg, Anchor::Start, text_pad));
            }
            WifiPromptPhase::Connected => {
                let msg = format!("Connected to {}", prompt.ssid);
                let msg = self.fit(&msg, field_w - text_pad * 2.0 - 30.0, m.label, Role::Bold);
                let (cil, cir) = self
                    .fonts
                    .ink_hbounds(glyph::CHECK, m.icon * 0.9, Role::Icon);
                field.parts.push(Part::icon(
                    glyph::CHECK,
                    m.icon * 0.9,
                    t.green,
                    Anchor::Start,
                    text_pad - cil,
                ));
                field.parts.push(Part::Text {
                    text: msg,
                    size: m.label,
                    role: Role::Bold,
                    color: t.green,
                    emph: None,
                    anchor: Anchor::Start,
                    dx: text_pad + (cir - cil) + 10.0,
                    dy: 0.0,
                    valign: VAlign::Cap,
                });
            }
            WifiPromptPhase::Editing | WifiPromptPhase::Failed(_) => {
                let err_msg = match &prompt.phase {
                    WifiPromptPhase::Failed(err) => Some(err.as_str()),
                    _ => None,
                };
                // Right-side hint / error badge inside field.
                let right_badge_w = if let Some(err) = err_msg {
                    let err_short = self.fit(err, 290.0, m.small * 1.05, Role::Bold);
                    let ew = self.tw(&err_short, m.small * 1.05, Role::Bold);
                    let aw = self.iw(glyph::ALERT, m.small * 1.25, Role::Icon);
                    field.parts.push(Part::Text {
                        text: err_short,
                        size: m.small * 1.05,
                        role: Role::Bold,
                        color: t.red,
                        emph: None,
                        anchor: Anchor::End,
                        dx: 16.0,
                        dy: 0.0,
                        valign: VAlign::Cap,
                    });
                    field.parts.push(Part::icon(
                        glyph::ALERT,
                        m.small * 1.25,
                        t.red,
                        Anchor::End,
                        16.0 + ew + 14.0,
                    ));
                    ew + aw + 44.0
                } else if !prompt.password.is_empty() {
                    let count = prompt.password.chars().count();
                    let hint = if prompt.revealed {
                        format!("Visible · {count}")
                    } else {
                        format!("{count}")
                    };
                    let hw = self.tw(&hint, m.small * 1.05, Role::Text);
                    field.parts.push(Part::Text {
                        text: hint,
                        size: m.small * 1.05,
                        role: Role::Text,
                        color: if prompt.revealed {
                            t.accent.alpha(0.85)
                        } else {
                            t.fg_faint
                        },
                        emph: None,
                        anchor: Anchor::End,
                        dx: 16.0,
                        dy: 0.0,
                        valign: VAlign::Cap,
                    });
                    hw + 32.0
                } else {
                    16.0
                };

                let max_text_w = (field_w - text_pad - right_badge_w - 14.0).max(80.0);
                if prompt.password.is_empty() {
                    field.parts.push(Part::Cursor {
                        dx: text_pad,
                        height: m.bh * 0.56,
                        color: if err_msg.is_some() { t.red } else { t.accent },
                    });
                    let placeholder = if err_msg.is_some() {
                        "Re-enter password on keyboard…"
                    } else {
                        "Type Wi-Fi password on keyboard…"
                    };
                    let placeholder =
                        self.fit(placeholder, max_text_w - 10.0, m.label * 0.92, Role::Text);
                    field.parts.push(Part::label(
                        placeholder,
                        m.label * 0.92,
                        t.fg_faint,
                        Anchor::Start,
                        text_pad + 10.0,
                    ));
                } else {
                    let display = if prompt.revealed {
                        self.fit_tail(&prompt.password, max_text_w, text_size, Role::Text)
                    } else {
                        let bullets: String =
                            std::iter::repeat_n('•', prompt.password.chars().count()).collect();
                        self.fit_tail(&bullets, max_text_w, text_size * 1.08, Role::Text)
                    };
                    let draw_size = if prompt.revealed {
                        text_size
                    } else {
                        text_size * 1.08
                    };
                    let tw = self.tw(&display, draw_size, Role::Text);
                    field.parts.push(Part::label(
                        display,
                        draw_size,
                        t.fg,
                        Anchor::Start,
                        text_pad,
                    ));
                    field.parts.push(Part::Cursor {
                        dx: text_pad + tw + 4.0,
                        height: m.bh * 0.56,
                        color: if err_msg.is_some() { t.red } else { t.accent },
                    });
                }
            }
        }

        nodes.push(field);
        nodes.push(vis);
        nodes.push(bs);
        nodes.push(join);
    }

    /// The speed test, laid out like Omarchy's own: the connection under test
    /// and its status, Download and Upload gauges on one shared scale, the
    /// ping, and Stop / Run Again at the right edge.
    fn panel_speedtest(&self, start_x: f32, nodes: &mut Vec<Node>) {
        let Some(st) = &self.speedtest else {
            return;
        };
        let m = self.m();
        let t = self.theme.clone();
        let right = self.width - m.edge;
        let mut x = start_x;

        // 1. Connection and status, like the battery panel's summary card.
        let (icon, icon_color, status) = match &st.phase {
            SpeedPhase::Ignition => (glyph::PERFORMANCE, t.accent, "Starting…".to_string()),
            SpeedPhase::Download => (glyph::PERFORMANCE, t.accent, "Testing download…".into()),
            SpeedPhase::Upload => (glyph::PERFORMANCE, t.accent, "Testing upload…".into()),
            SpeedPhase::Complete => (glyph::CHECK, t.green, "Finished".into()),
            SpeedPhase::Error(err) => (
                glyph::ALERT,
                t.red,
                self.fit(err, 320.0, m.small, Role::Text),
            ),
        };
        let title = self.fit(&st.connection, 240.0, m.small * 1.15, Role::Bold);
        let key = Key::new("p/speed/info");
        let mut info = self.info(key, x, (icon, icon_color), &title, &status);
        // Hold the card steady while the status line changes.
        let steady = self.info(
            key,
            x,
            (glyph::PERFORMANCE, t.accent),
            &title,
            "Testing download…",
        );
        info.rect.w = info.rect.w.max(steady.rect.w);
        if matches!(st.phase, SpeedPhase::Error(_))
            && let Some(Part::Text { color, .. }) = info.parts.last_mut()
        {
            *color = t.red;
        }
        x += info.rect.w + m.gap * 2.0;
        nodes.push(info);

        // 2. Stop / Run Again, pinned right like the Wi-Fi prompt's Join.
        let running = st.is_running();
        let (icon, label, fill, fg, cmd) = if running {
            (
                glyph::CLOSE,
                "Stop",
                Fill {
                    color: t.red.alpha(0.20).over(t.surface),
                    emph: None,
                    radius: m.radius,
                    border: Some(t.red.alpha(0.55)),
                    sheen: true,
                },
                t.red.lighten(0.18),
                Cmd::SpeedTestStop,
            )
        } else {
            (
                glyph::REFRESH,
                "Run Again",
                Fill {
                    color: t.accent,
                    emph: None,
                    radius: m.radius,
                    border: None,
                    sheen: true,
                },
                t.on_accent,
                Cmd::SpeedTestStart,
            )
        };
        let btn_size = m.label * 0.95;
        let btn_icon = m.icon * 0.85;
        let (il, ir) = self.fonts.ink_hbounds(icon, btn_icon, Role::Icon);
        let icon_w = ir - il;
        let label_w = self.tw(label, btn_size, Role::Bold);
        // Sized for the longer label so the button keeps its place.
        let btn_w = (self.iw(glyph::REFRESH, btn_icon, Role::Icon)
            + 8.0
            + self.tw("Run Again", btn_size, Role::Bold)
            + m.pad * 2.0)
            .max(152.0)
            .round();
        let btn_x = right - btn_w;
        let inner_start = ((btn_w - (icon_w + 8.0 + label_w)) * 0.5).max(12.0);
        let action_btn = Node::new(
            Key::new("p/speed/action"),
            Rect::new(btn_x, m.inset, btn_w, m.bh),
        )
        .fill(fill)
        .part(Part::icon(
            icon,
            btn_icon,
            fg,
            Anchor::Start,
            inner_start - il,
        ))
        .part(Part::Text {
            text: label.into(),
            size: btn_size,
            role: Role::Bold,
            color: fg,
            emph: None,
            anchor: Anchor::Start,
            dx: inner_start + icon_w + 8.0,
            dy: 0.0,
            valign: VAlign::Cap,
        })
        .tap(cmd);

        // 3. Ping, as the network panel's stat card.
        let ping_text = match st.ping_ms {
            Some(ms) => format_ping_latency(Some(ms), true),
            None => "--".into(),
        };
        let ping_color = if st.ping_ms.is_some() {
            t.fg
        } else {
            t.fg_faint
        };
        let mut ping_nodes = Vec::new();
        let mut ping_w = 0.0;
        self.net_stat(
            Key::new("p/speed/ping"),
            &mut ping_w,
            "Ping",
            t.fg_dim,
            &ping_text,
            ping_color,
            "99.9 ms",
            None,
            None,
            &mut ping_nodes,
        );

        // 4. Download and Upload share what's left.
        let ping_x = btn_x - m.gap * 2.0 - (ping_w - m.gap);
        let gauge_w = ((ping_x - m.gap - x - m.gap) / 2.0).max(200.0).floor();
        let sweep = st.sweep();
        nodes.push(self.speed_gauge(
            Key::new("p/speed/down"),
            x,
            gauge_w,
            (glyph::DOWNLOAD, "Download"),
            st.download_mbps,
            st.scale,
            sweep,
            st.phase == SpeedPhase::Download,
        ));
        x += gauge_w + m.gap;
        nodes.push(self.speed_gauge(
            Key::new("p/speed/up"),
            x,
            gauge_w,
            (glyph::UPLOAD, "Upload"),
            st.upload_mbps,
            st.scale,
            sweep,
            st.phase == SpeedPhase::Upload,
        ));
        for mut ping in ping_nodes {
            ping.rect.x = ping_x;
            ping.group = None;
            nodes.push(ping);
        }
        nodes.push(action_btn);
    }

    /// A speed reading as a meter styled like the Sound panel's volume
    /// slider: the fill shows `value` against the run's shared `scale`.
    /// `sweep` overrides the fill during the opening sweep; `live` marks the
    /// phase being measured, which shimmers until its first reading.
    #[allow(clippy::too_many_arguments)]
    fn speed_gauge(
        &self,
        key: Key,
        x: f32,
        w: f32,
        (icon, label): (&str, &str),
        value: Option<f32>,
        scale: f32,
        sweep: Option<f32>,
        live: bool,
    ) -> Node {
        let m = self.m();
        let t = &self.theme;
        let fraction =
            sweep.unwrap_or_else(|| value.map_or(0.0, |v| (v / scale.max(1.0)).clamp(0.0, 1.0)));
        // Text reads on the fill once the fill passes its middle.
        let on_fill = |mid: f32| fraction > 0.0 && 4.0 + fraction * (w - 8.0) > mid;
        let lead_color = if live || value.is_some() {
            t.fg
        } else {
            t.fg_dim
        };

        let mut node = Node::new(key, Rect::new(x, m.inset, w, m.bh))
            .fill(Fill {
                color: t.well,
                emph: None,
                radius: m.radius,
                border: live.then(|| t.accent.alpha(0.5)),
                sheen: false,
            })
            .part(Part::Progress {
                value: fraction,
                color: t.accent,
                track: None,
                inset: 4.0,
                radius: m.radius - 3.0,
            });
        if live && value.is_none() {
            node = node.part(Part::Shimmer {
                color: t.accent,
                radius: m.radius,
            });
        }

        let icon_size = m.icon * 0.9;
        let (il, ir) = self.fonts.ink_hbounds(icon, icon_size, Role::Icon);
        let icon_w = ir - il;
        let label_dx = 16.0 + icon_w + 10.0;
        let label_w = self.tw(label, m.label, Role::Text);
        let value_text = value.map_or_else(|| "--".into(), |v| format!("{} Mbps", format_mbps(v)));
        let value_size = m.label * 0.95;
        let value_w = self.tw(&value_text, value_size, Role::Bold);
        let value_color = if value.is_some() { t.fg } else { t.fg_faint };
        // While sweeping, the whole fill moves at once: ease every color
        // with it through `emph` instead of switching ahead of the fill.
        let over = |mid: f32, off: Rgba| match sweep {
            Some(_) => off,
            None if on_fill(mid) => t.on_accent,
            None => off,
        };
        let ease = |part: Part| match sweep {
            Some(_) => part.with_emph(t.on_accent),
            None => part,
        };
        let mut node = node
            .part(ease(Part::icon(
                icon,
                icon_size,
                over(16.0 + icon_w * 0.5, lead_color),
                Anchor::Start,
                16.0 - il,
            )))
            .part(ease(Part::label(
                label,
                m.label,
                over(label_dx + label_w * 0.5, lead_color),
                Anchor::Start,
                label_dx,
            )))
            .part(ease(Part::Text {
                text: value_text,
                size: value_size,
                role: Role::Bold,
                color: over(w - 18.0 - value_w * 0.5, value_color),
                emph: None,
                anchor: Anchor::End,
                dx: 18.0,
                dy: 0.0,
                valign: VAlign::Cap,
            }))
            .no_ripple();
        node.emph = sweep.unwrap_or(0.0);
        node.press_glow = false;
        node
    }

    /// Omarchy's Bluetooth panel on the strip: the power switch, then
    /// connected and paired devices (tap to connect or disconnect, trash to
    /// forget), then devices a scan finds nearby (tap to pair). The host
    /// scans while the panel is open (see [`Scene::wants_bluetooth_scan`]).
    fn panel_bluetooth(&self, x: &mut f32, nodes: &mut Vec<Node>) -> Option<(f32, f32)> {
        let m = self.m();
        let t = self.theme.clone();
        let b = &self.state.bluetooth;
        if !b.available {
            let row_start = *x;
            self.bt_note("none", "No Bluetooth adapter", x, nodes);
            return Some((row_start, *x - m.gap));
        }
        let toggle = self.toggle_chip(
            Key::new("p/bt"),
            *x,
            "Bluetooth",
            b.powered,
            Action::SetBluetooth(!b.powered),
        );
        *x += toggle.rect.w + m.gap * 2.0;
        nodes.push(toggle.clone());
        self.switch(&toggle, b.powered, nodes);
        let row_start = *x;

        // Connected, then remembered devices (as Omarchy lists them).
        for device in &b.devices {
            let node = self.bt_device_chip(device);
            self.row_node(x, nodes, node);
            let busy = self.bt.pending.contains_key(&device.address);
            if device.paired && !busy {
                // Sits snug against its device.
                *x -= m.gap * 0.5;
                let node = self.bt_forget_chip(&device.address);
                self.row_node(x, nodes, node);
            }
        }
        if !b.devices.is_empty() {
            *x += m.gap;
        }

        // Pairing: what the scan finds.
        if !b.powered {
            self.bt_note("off", "Turn Bluetooth on to scan", x, nodes);
            return Some((row_start, *x - m.gap));
        }
        let nearby: &[BtDevice] = if b.discovering { &b.nearby } else { &[] };
        let subtitle = if !b.discovering {
            "Starting scan…"
        } else if nearby.is_empty() {
            "Scanning for devices…"
        } else {
            "Tap one to pair"
        };
        let mut scan = self
            .info(
                Key::new("p/bt/scan"),
                *x,
                (glyph::RADAR, t.accent),
                "Pair a device",
                subtitle,
            )
            .no_ripple();
        scan.press_glow = false;
        if b.discovering {
            scan.parts.insert(
                0,
                Part::Shimmer {
                    color: t.accent,
                    radius: m.radius,
                },
            );
        }
        self.row_node(x, nodes, scan);
        for device in nearby {
            let node = self.bt_device_chip(device);
            self.row_node(x, nodes, node);
        }
        Some((row_start, *x - m.gap))
    }

    /// A device as a two-line chip: its name over what it is doing.
    fn bt_device_chip(&self, device: &BtDevice) -> Node {
        let m = self.m();
        let t = &self.theme;
        let pending = self.bt.pending.get(&device.address).map(|(op, _)| *op);
        let failed = self.bt.failed.get(&device.address).map(|(op, _)| *op);
        let (status, status_color) = match (pending, failed) {
            (Some(op), _) => (
                match op {
                    BtOp::Pair => "Pairing…",
                    BtOp::Connect => "Connecting…",
                    BtOp::Disconnect => "Disconnecting…",
                    BtOp::Forget => "Forgetting…",
                }
                .to_string(),
                t.fg,
            ),
            (None, Some(op)) => (
                match op {
                    BtOp::Pair => "Couldn’t pair",
                    BtOp::Connect => "Couldn’t connect",
                    BtOp::Disconnect => "Couldn’t disconnect",
                    BtOp::Forget => "Couldn’t forget",
                }
                .to_string(),
                t.red,
            ),
            _ if device.connected => (
                match device.battery {
                    Some(level) => format!("Connected · {level}%"),
                    None => "Connected".into(),
                },
                t.fg_dim,
            ),
            _ if device.paired => ("Paired".into(), t.fg_dim),
            _ => (bt_kind_label(device.kind).into(), t.fg_dim),
        };
        let icon_color = if device.connected && pending.is_none() {
            t.accent
        } else if device.paired {
            t.fg
        } else {
            t.fg_dim
        };
        let name = self.fit(&device.name, 260.0, m.small * 1.15, Role::Bold);
        let mut node = self.two_line_chip_colored(
            Key::new("p/bt/dev").with(&device.address),
            (bt_glyph(device.kind, device.connected), icon_color),
            &name,
            (&status, status_color),
            (device.connected && pending.is_none()).then_some((glyph::CHECK, t.accent)),
        );
        node.emph = if device.connected { 1.0 } else { 0.0 };
        if pending.is_some() {
            node.parts.insert(
                0,
                Part::Shimmer {
                    color: t.accent,
                    radius: m.radius,
                },
            );
            node.ripple = false;
        } else {
            let op = if device.connected {
                BtOp::Disconnect
            } else if device.paired {
                BtOp::Connect
            } else {
                BtOp::Pair
            };
            node = node.tap(Cmd::Do(Action::BluetoothDevice {
                address: device.address.clone(),
                op,
            }));
        }
        node
    }

    /// Forget for a remembered device: a trash can that asks once.
    fn bt_forget_chip(&self, address: &str) -> Node {
        let m = self.m();
        let t = &self.theme;
        let armed = self
            .bt
            .forget_armed
            .as_ref()
            .is_some_and(|(a, _)| a == address);
        let key = Key::new("p/bt/forget").with(address);
        let node = if armed {
            self.chip(
                key,
                Some((glyph::TRASH, Role::Icon, t.red)),
                Some(("Forget", t.fg)),
                Fill {
                    color: t.red.alpha(0.22).over(t.surface),
                    emph: None,
                    radius: m.radius,
                    border: Some(t.red.alpha(0.7)),
                    sheen: true,
                },
            )
        } else {
            self.chip(
                key,
                Some((glyph::TRASH, Role::Icon, t.fg_dim)),
                None,
                self.surface(),
            )
        };
        node.tap(Cmd::BluetoothForget(address.to_string()))
    }

    /// A dim note in the Bluetooth panel's row.
    fn bt_note(&self, id: &str, text: &str, x: &mut f32, nodes: &mut Vec<Node>) {
        let node = self.chip(
            Key::new("p/bt/note").with(id),
            None,
            Some((text, self.theme.fg_dim)),
            self.flat(),
        );
        self.row_node(x, nodes, node);
    }

    // ------------------------------------------------------------ Touch ID

    /// The Touch ID prompt takes over the strip: Esc and Cancel on the left,
    /// then the screensaver-style effect (see `touchfx`) running up to the
    /// sensor at the right end.
    fn build_touch_id(&self, left: f32, nodes: &mut Vec<Node>) {
        let m = self.m();
        let Some(fx) = &self.touch_fx else {
            return;
        };
        let mut cancel = self
            .chip(
                Key::new("tid/cancel"),
                None,
                Some(("Cancel", self.theme.fg)),
                self.surface(),
            )
            .tap(Cmd::Do(Action::CancelTouchId))
            .enter(Motion::rise(8.0).delayed(0.35));
        // Cancel sits by Esc so the right end belongs to the sensor.
        cancel.rect.x = left;
        // The effect keeps its width when Cancel goes, so nothing shifts
        // mid-success.
        let x0 = left + cancel.rect.w + m.gap * 1.5;
        nodes.push(
            Node::new(
                Key::new("tid/fx"),
                Rect::new(x0, 0.0, self.width - x0, self.height),
            )
            .part(Part::TouchFx)
            .no_ripple()
            .enter(Motion {
                dx: 0.0,
                dy: 0.0,
                scale: 1.0,
                delay: 0.0,
            }),
        );
        if fx.cancellable() {
            nodes.push(cancel);
        }
    }

    /// Whether the scene is in a mode where the system bar should be hidden.
    pub fn shows_bar(&self) -> bool {
        self.config.mode == Mode::Bar
    }
}

/// "55Wh" → "55 Wh", "12.4W" → "12.4 W", as Omarchy's readings.
/// A speed test reading in Mbps, with fewer decimals as it grows.
fn format_mbps(mbps: f32) -> String {
    if mbps >= 1000.0 {
        format!("{mbps:.0}")
    } else if mbps >= 10.0 {
        format!("{mbps:.1}")
    } else {
        format!("{mbps:.2}")
    }
}

fn spaced(value: &str, empty: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        return empty.into();
    }
    match value.find(|c: char| c.is_alphabetic()) {
        Some(i) if i > 0 && !value[..i].ends_with(' ') => {
            format!("{} {}", &value[..i], &value[i..])
        }
        _ => value.replace('-', "–"),
    }
}

fn duration(seconds: u64) -> String {
    let h = seconds / 3600;
    let m = (seconds % 3600) / 60;
    if h > 0 {
        format!("{h}h {m:02}m")
    } else {
        format!("{m}m")
    }
}

/// A Bluetooth device's glyph: what it is, else the Bluetooth mark.
fn bt_glyph(kind: BtKind, connected: bool) -> &'static str {
    match kind {
        BtKind::Headphones => glyph::HEADPHONES,
        BtKind::Speaker => glyph::SPEAKER,
        BtKind::Keyboard => glyph::KEYBOARD,
        BtKind::Mouse => glyph::MOUSE,
        BtKind::Gamepad => glyph::GAMEPAD,
        BtKind::Phone => glyph::PHONE,
        BtKind::Computer => glyph::LAPTOP,
        BtKind::Display => glyph::MONITOR,
        BtKind::Other if connected => glyph::BT_CONNECTED,
        BtKind::Other => glyph::BT_ON,
    }
}

fn bt_kind_label(kind: BtKind) -> &'static str {
    match kind {
        BtKind::Headphones => "Headphones",
        BtKind::Speaker => "Speaker",
        BtKind::Keyboard => "Keyboard",
        BtKind::Mouse => "Mouse",
        BtKind::Gamepad => "Controller",
        BtKind::Phone => "Phone",
        BtKind::Computer => "Computer",
        BtKind::Display => "Display",
        BtKind::Other => "New device",
    }
}

fn device_glyph(kind: AudioDeviceKind) -> &'static str {
    match kind {
        AudioDeviceKind::Speaker => glyph::SPEAKER,
        AudioDeviceKind::Headphones => glyph::HEADPHONES,
        AudioDeviceKind::Bluetooth => glyph::BT_ON,
        AudioDeviceKind::Display => glyph::MONITOR,
        AudioDeviceKind::Microphone => glyph::MIC,
        AudioDeviceKind::Camera => glyph::WEBCAM,
    }
}

pub(crate) fn volume_glyph(
    volume: u8,
    muted: bool,
    t: &crate::theme::Theme,
) -> (&'static str, Rgba) {
    if muted {
        (glyph::VOLUME_OFF, t.red)
    } else if volume == 0 {
        (glyph::VOLUME_LOW, t.fg_dim)
    } else if volume < 50 {
        (glyph::VOLUME_MEDIUM, t.fg)
    } else {
        (glyph::VOLUME_HIGH, t.fg)
    }
}

pub(crate) fn battery_glyph(
    percent: f32,
    charge: Charge,
    t: &crate::theme::Theme,
) -> (&'static str, Rgba) {
    let index = ((percent / 10.0).ceil() as usize).clamp(1, 10) - 1;
    match charge {
        Charge::Charging | Charge::Full => (glyph::CHARGING[index], t.green),
        // A charge limit holds the battery: plugged in but not charging.
        Charge::Holding | Charge::NotCharging => (glyph::BATTERY[index], t.fg),
        _ if percent <= 15.0 => (glyph::BATTERY[index], t.red),
        _ if percent <= 30.0 => (glyph::BATTERY[index], t.yellow),
        _ => (glyph::BATTERY[index], t.fg),
    }
}

pub(crate) fn network_glyph(
    s: &crate::state::DesktopState,
    t: &crate::theme::Theme,
) -> (&'static str, Rgba) {
    let restricted = s.network.restricted();
    match s.network.kind {
        NetKind::Wifi if restricted => (glyph::WIFI_ALERT, t.red),
        NetKind::Wifi => (glyph::WIFI[((s.network.signal as usize) / 20).min(4)], t.fg),
        NetKind::Ethernet if restricted => (glyph::ETHERNET_ALERT, t.red),
        NetKind::Ethernet => (glyph::ETHERNET, t.fg),
        NetKind::Disconnected | NetKind::Unknown => (glyph::NO_NETWORK, t.fg_faint),
    }
}
