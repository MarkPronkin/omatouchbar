//! Touch-first versions of the shell's tray, weather button and indicators.

use super::*;
use crate::{scene::TrayMenu, state::Indicator};

impl Scene {
    pub(super) fn panel_button(&self, panel: Panel, icon: &str, color: Rgba) -> Node {
        self.chip(
            panel.origin(),
            Some((icon, Role::Icon, color)),
            None,
            self.widget_fill(),
        )
        .tap(Cmd::OpenPanel(panel))
    }

    pub(super) fn weather_button(&self, icon: &str, temp: Option<&str>, color: Rgba) -> Node {
        let Some(temp) = temp else {
            return self.panel_button(Panel::Weather, icon, color);
        };
        let m = self.m();
        let icon_size = (m.icon * 0.78).round();
        let (il, ir) = self.fonts.ink_hbounds(icon, icon_size, Role::Icon);
        let icon_w = ir - il;
        let label_w = self.tw(temp, m.label, Role::Text);
        let inner = 8.0;
        let start = m.pad - 2.0;
        let width = (start + icon_w + inner + label_w + m.pad).round();
        Node::new(
            Panel::Weather.origin(),
            Rect::new(0.0, m.inset, width, m.bh),
        )
        .fill(self.widget_fill())
        .part(Part::icon(
            icon,
            icon_size,
            color,
            Anchor::Start,
            start - il,
        ))
        .part(Part::label(
            temp,
            m.label,
            color,
            Anchor::Start,
            start + icon_w + inner,
        ))
        .tap(Cmd::OpenPanel(Panel::Weather))
    }

    pub(super) fn row_node(&self, x: &mut f32, nodes: &mut Vec<Node>, mut node: Node) {
        node.rect.x = *x;
        *x += node.rect.w + self.m().gap;
        nodes.push(node.group(0));
    }

    pub(super) fn row_message(&self, x: &mut f32, nodes: &mut Vec<Node>, text: &str) {
        let node = self.chip(
            Key::new("p/message"),
            None,
            Some((text, self.theme.fg_dim)),
            self.flat(),
        );
        self.row_node(x, nodes, node);
    }

    pub(super) fn panel_tray(&self, x: &mut f32, nodes: &mut Vec<Node>) {
        let t = &self.theme;
        if let Some(menu) = &self.tray_menu {
            let Some(entries) = &self.tray_entries else {
                self.row_message(x, nodes, "Loading menu…");
                return;
            };
            if entries.is_empty() {
                self.row_message(x, nodes, "No menu actions available");
            }
            for e in entries {
                let cmd = if e.submenu {
                    let mut next = menu.clone();
                    next.parents.push(e.id);
                    Cmd::TrayMenu(next)
                } else {
                    Cmd::Do(Action::ClickTrayMenu {
                        menu: menu.clone(),
                        id: e.id,
                    })
                };
                let key = Key::new("p/tray/entry")
                    .with(&menu.address)
                    .index(e.id as i64);
                let mut node = self.list_chip(
                    key,
                    x,
                    (if e.checked { glyph::CHECK } else { glyph::TRAY }, t.fg_dim),
                    &e.label,
                    e.submenu.then_some((glyph::CHEVRON, t.accent)),
                    e.checked,
                    e.enabled.then_some(cmd),
                );
                if !e.enabled {
                    node.opacity = 0.45;
                }
                nodes.push(node);
            }
            return;
        }

        let (pinned, hidden): (&[String], &[String]) = match self.layout.find("tray") {
            Some(Widget::Tray { pinned, hidden }) => (pinned, hidden),
            _ => (&[], &[]),
        };
        let mut items: Vec<_> = self
            .state
            .tray
            .iter()
            .filter(|i| !hidden.contains(&i.id))
            .collect();
        items.sort_by_key(|i| {
            pinned
                .iter()
                .position(|id| id == &i.id)
                .unwrap_or(usize::MAX)
        });
        let start = nodes.len();
        for item in items {
            let key = Key::new("p/tray/item").with(&item.address);
            let menu = item.menu_path.as_ref().map(|path| {
                Cmd::TrayMenu(TrayMenu {
                    address: item.address.clone(),
                    path: path.clone(),
                    parents: vec![0],
                })
            });
            let activate = Cmd::Do(Action::ActivateTray {
                address: item.address.clone(),
                context_menu: item.menu_only,
            });
            let primary = if item.menu_only {
                menu.clone().unwrap_or(activate)
            } else {
                activate
            };
            let mut node = self.list_chip(
                key,
                x,
                (
                    glyph::TRAY,
                    if item.attention { t.orange } else { t.fg_dim },
                ),
                &item.title,
                None,
                item.attention,
                Some(primary),
            );
            if let Some(icon) = item.icon.clone() {
                // Reuse the icon slot reserved by list_chip so labels keep their alignment.
                let size = self.iw(glyph::TRAY, self.m().icon, Role::Icon);
                node.parts[0] = Part::IconImage {
                    icon,
                    size,
                    dx: self.m().pad - 2.0,
                };
            }
            nodes.push(node);
            if !item.menu_only
                && let Some(menu) = menu
            {
                let node = self
                    .chip(
                        key.with("menu"),
                        Some((glyph::CHEVRON, Role::Icon, t.fg_dim)),
                        None,
                        self.surface(),
                    )
                    .tap(menu);
                self.row_node(x, nodes, node);
            }
        }
        // The shell lets users put ordinary bar widgets inside the tray drawer.
        for widget in &self.layout.tray {
            if matches!(widget, Widget::Tray { .. }) {
                continue;
            }
            let begin = nodes.len();
            let width = self.widget(widget, true, nodes);
            for node in &mut nodes[begin..] {
                node.shift(*x);
                node.group = Some(0);
            }
            if width > 0.0 {
                *x += width + self.m().gap;
            }
        }
        if start == nodes.len() {
            self.row_message(x, nodes, "No tray items");
        }
    }

    fn indicator_meta(&self, indicator: Indicator) -> (&'static str, Rgba) {
        let t = &self.theme;
        match indicator {
            Indicator::PasswordlessSudo => (glyph::PASSWORDLESS_SUDO, t.red),
            Indicator::ScreenRecording => (glyph::RECORDING, t.red),
            Indicator::RemoteSession => (glyph::REMOTE_SESSION, t.red),
            Indicator::Reminder => (glyph::REMINDER, t.cyan),
            Indicator::NightLight => (glyph::NIGHTLIGHT, t.yellow),
            Indicator::Dnd => (glyph::DND, t.orange),
            Indicator::StayAwake => (glyph::STAY_AWAKE, t.accent),
        }
    }

    pub(super) fn indicators_widget(
        &self,
        configured: &[Indicator],
        always_show: Option<bool>,
        compact_only: bool,
        nodes: &mut Vec<Node>,
    ) -> f32 {
        let m = self.m();
        let t = &self.theme;
        let ind_state = &self.state.indicators;
        let items: &[Indicator] = if configured.is_empty() {
            &Indicator::ALL
        } else {
            configured
        };
        let inline = !compact_only
            && (always_show == Some(true) || self.config.appearance.always_show_indicators);

        if inline {
            let mut x = 0.0f32;
            let slot_w = 54.0;
            let gap = 6.0;
            for &ind in items {
                let active = ind_state.is_active(ind);
                if !active && !ind.reveal_when_inactive() {
                    continue;
                }
                let (g, active_color) = self.indicator_meta(ind);
                let fill = if active {
                    let tint = if ind.urgent() { t.red } else { active_color };
                    Fill {
                        color: tint
                            .alpha(if ind.urgent() { 0.24 } else { 0.16 })
                            .over(t.surface),
                        emph: None,
                        radius: m.radius,
                        border: Some(tint.alpha(if ind.urgent() { 0.65 } else { 0.42 })),
                        sheen: true,
                    }
                } else {
                    self.widget_fill()
                };
                let mut node = Node::new(
                    Key::new("w/ind").with(ind.key()),
                    Rect::new(x, m.inset, slot_w, m.bh),
                )
                .fill(fill)
                .part(Part::icon(
                    g,
                    m.icon * if active { 0.94 } else { 0.88 },
                    if active { active_color } else { t.fg_dim },
                    Anchor::Center,
                    0.0,
                ))
                .tap(Cmd::TriggerIndicator(ind));
                if !active {
                    node.opacity = 0.48;
                }
                nodes.push(node);
                x += slot_w + gap;
            }
            return if x > 0.0 { x - gap } else { 0.0 };
        }

        // Compact pill: always visible so inactive toggles (Stay Awake,
        // Passwordless Sudo, Night Light, DND, Record, Reminder) remain one
        // tap away inside Panel::Indicators, while active indicators glow
        // directly on the bar.
        let active: Vec<Indicator> = items
            .iter()
            .copied()
            .filter(|&ind| ind_state.is_active(ind))
            .collect();
        if active.is_empty() {
            let node = self
                .chip(
                    Panel::Indicators.origin(),
                    Some((glyph::TOGGLES, Role::Icon, t.fg_dim)),
                    None,
                    self.widget_fill(),
                )
                .tap(Cmd::OpenPanel(Panel::Indicators));
            let w = node.rect.w;
            nodes.push(node);
            return w;
        }

        let urgent = active.iter().any(|i| i.urgent());
        let fill = if urgent {
            Fill {
                color: t.red.alpha(0.22).over(t.surface),
                emph: None,
                radius: m.radius,
                border: Some(t.red.alpha(0.65)),
                sheen: true,
            }
        } else {
            self.widget_fill()
        };

        if active.len() == 1 {
            let (g, color) = self.indicator_meta(active[0]);
            let node = self
                .chip(
                    Panel::Indicators.origin(),
                    Some((g, Role::Icon, color)),
                    None,
                    fill,
                )
                .tap(Cmd::OpenPanel(Panel::Indicators));
            let w = node.rect.w;
            nodes.push(node);
            return w;
        }

        let icon_size = m.icon * 0.92;
        let inner = 12.0;
        let mut cursor = m.pad - 2.0;
        let mut parts = Vec::with_capacity(active.len());
        for (idx, ind) in active.into_iter().enumerate() {
            if idx > 0 {
                cursor += inner;
            }
            let (g, color) = self.indicator_meta(ind);
            let (il, ir) = self.fonts.ink_hbounds(g, icon_size, Role::Icon);
            parts.push(Part::icon(g, icon_size, color, Anchor::Start, cursor - il));
            cursor += (ir - il).max(16.0);
        }
        let width = (cursor + m.pad - 2.0).round().max(64.0);
        let mut node = Node::new(
            Panel::Indicators.origin(),
            Rect::new(0.0, m.inset, width, m.bh),
        )
        .fill(fill)
        .tap(Cmd::OpenPanel(Panel::Indicators));
        node.parts = parts;
        nodes.push(node);
        width
    }

    fn indicator_toggle_chip(
        &self,
        key: Key,
        x: &mut f32,
        icon: (&str, Rgba),
        label: &str,
        on: bool,
        active_color: Rgba,
        urgent: bool,
        cmd: Cmd,
        nodes: &mut Vec<Node>,
    ) {
        let m = self.m();
        let t = &self.theme;
        let icon_size = m.icon * 0.94;
        let (il, ir) = self.fonts.ink_hbounds(icon.0, icon_size, Role::Icon);
        let icon_w = ir - il;
        let label_w = self.tw(label, m.label, Role::Text);
        let switch_w = 46.0;
        let start = m.pad - 2.0;
        let w = (start + icon_w + 10.0 + label_w + 14.0 + switch_w + m.pad).round();
        let emph_bg = active_color
            .alpha(if urgent { 0.24 } else { 0.18 })
            .over(t.surface);
        let border = on.then_some(active_color.alpha(if urgent { 0.65 } else { 0.45 }));
        let node = Node::new(key, Rect::new(*x, m.inset, w, m.bh))
            .fill(Fill {
                color: t.surface,
                emph: Some(emph_bg),
                radius: m.radius,
                border,
                sheen: true,
            })
            .part(Part::icon(
                icon.0,
                icon_size,
                if on { icon.1 } else { t.fg_dim },
                Anchor::Start,
                start - il,
            ))
            .part(Part::label(
                label,
                m.label,
                t.fg,
                Anchor::Start,
                start + icon_w + 10.0,
            ))
            .emph(if on { 1.0 } else { 0.0 })
            .group(0)
            .tap(cmd);
        *x += w + m.gap;
        nodes.push(node.clone());
        self.switch_colored(&node, on, active_color, nodes);
    }

    pub(super) fn panel_indicators(&self, x: &mut f32, nodes: &mut Vec<Node>) {
        let m = self.m();
        let t = &self.theme;
        let ind_state = &self.state.indicators;

        for ind in self.layout.indicator_items() {
            let active = ind_state.is_active(ind);
            let (g, color) = self.indicator_meta(ind);
            match ind {
                Indicator::RemoteSession => {
                    if !active {
                        continue;
                    }
                    let subtitle = if ind_state.remote_peers.is_empty() {
                        "Active".to_string()
                    } else {
                        self.fit(
                            &ind_state.remote_peers.join(", "),
                            240.0,
                            m.small,
                            Role::Text,
                        )
                    };
                    let mut badge = self.info(
                        Key::new("p/ind/remote"),
                        *x,
                        (g, t.red),
                        "Remote Session",
                        &subtitle,
                    );
                    badge.fill = Some(Fill {
                        color: t.red.alpha(0.20).over(t.well),
                        emph: None,
                        radius: m.radius,
                        border: Some(t.red.alpha(0.65)),
                        sheen: false,
                    });
                    self.row_node(x, nodes, badge);
                }
                Indicator::PasswordlessSudo => {
                    self.indicator_toggle_chip(
                        Key::new("p/ind/sudo"),
                        x,
                        (g, t.red),
                        "Passwordless Sudo",
                        active,
                        t.red,
                        true,
                        Cmd::TriggerIndicator(Indicator::PasswordlessSudo),
                        nodes,
                    );
                }
                Indicator::ScreenRecording => {
                    let label = if active {
                        "Stop Recording"
                    } else {
                        "Screen Record"
                    };
                    let suffix = if active {
                        Some((glyph::CLOSE, t.red))
                    } else {
                        Some((glyph::CHEVRON, t.fg_faint))
                    };
                    let mut chip = self.list_chip(
                        Key::new("p/ind/rec"),
                        x,
                        (g, if active { t.red } else { t.fg_dim }),
                        label,
                        suffix,
                        active,
                        Some(Cmd::TriggerIndicator(Indicator::ScreenRecording)),
                    );
                    if active {
                        chip.fill = Some(Fill {
                            color: t.surface,
                            emph: Some(t.red.alpha(0.24).over(t.surface)),
                            radius: m.radius,
                            border: Some(t.red.alpha(0.65)),
                            sheen: true,
                        });
                    }
                    nodes.push(chip);
                }
                Indicator::Reminder => {
                    let label = if ind_state.reminder_count > 0 {
                        format!("Reminders · {}", ind_state.reminder_count)
                    } else {
                        "Reminder".into()
                    };
                    let mut chip = self.list_chip(
                        Key::new("p/ind/reminder"),
                        x,
                        (g, if active { color } else { t.fg_dim }),
                        &label,
                        None,
                        active,
                        Some(Cmd::TriggerIndicator(Indicator::Reminder)),
                    );
                    if active {
                        chip.fill = Some(Fill {
                            color: t.surface,
                            emph: Some(color.alpha(0.18).over(t.surface)),
                            radius: m.radius,
                            border: Some(color.alpha(0.45)),
                            sheen: true,
                        });
                    }
                    nodes.push(chip);
                    if ind_state.reminder_count > 0 {
                        let add = self
                            .chip(
                                Key::new("p/ind/reminder/add"),
                                Some((glyph::PLUS, Role::Icon, t.fg)),
                                None,
                                self.surface(),
                            )
                            .tap(Cmd::Do(Action::SetReminder));
                        self.row_node(x, nodes, add);
                        let clear = self
                            .chip(
                                Key::new("p/ind/reminder/clear"),
                                Some((glyph::TRASH, Role::Icon, t.red)),
                                None,
                                self.surface(),
                            )
                            .tap(Cmd::Do(Action::ClearReminders));
                        self.row_node(x, nodes, clear);
                    }
                }
                Indicator::NightLight => {
                    self.indicator_toggle_chip(
                        Key::new("p/ind/night"),
                        x,
                        (g, t.yellow),
                        "Night Light",
                        active,
                        t.yellow,
                        false,
                        Cmd::TriggerIndicator(Indicator::NightLight),
                        nodes,
                    );
                }
                Indicator::Dnd => {
                    self.indicator_toggle_chip(
                        Key::new("p/ind/dnd"),
                        x,
                        (g, t.orange),
                        "DND",
                        active,
                        t.orange,
                        false,
                        Cmd::TriggerIndicator(Indicator::Dnd),
                        nodes,
                    );
                }
                Indicator::StayAwake => {
                    self.indicator_toggle_chip(
                        Key::new("p/ind/awake"),
                        x,
                        (g, t.accent),
                        "Stay Awake",
                        active,
                        t.accent,
                        false,
                        Cmd::TriggerIndicator(Indicator::StayAwake),
                        nodes,
                    );
                }
            }
        }
    }
}
