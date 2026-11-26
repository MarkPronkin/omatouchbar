//! The Agents panel, laid out like Omarchy's: for each subscription its
//! mark, name and plan, then a meter per limit window (time to reset beside
//! it), banked resets, balances and accounts; then what the token counts add
//! up to across every agent, and the panel's actions.

use super::*;
use crate::agents::{
    AgentAccount, AgentLimit, AgentSummary, AgentUsage, AgentWindow, format_money,
    format_reset_duration, format_tokens,
};

/// Every meter card is at least this wide, so they line up from agent to agent.
const METER_W: f32 = 250.0;
/// How long the Refresh button shimmers waiting for new records.
pub(crate) const AGENTS_REFRESH: std::time::Duration = std::time::Duration::from_secs(12);
pub(crate) const AGENTS_REFRESH_PULSE: &str = "agents-refresh";

/// What a meter card shows.
struct Meter<'a> {
    title: &'a str,
    /// Dim, top right (`resets in 4h 59m`).
    aside: Option<&'a str>,
    value: &'a str,
    /// How full the track beside the value is; `None` draws no track.
    fraction: Option<f32>,
    fill: Rgba,
    /// Other allowances on the same clock, as ticks across the track.
    ticks: Vec<f32>,
}

impl Scene {
    pub(super) fn panel_agents(&self, x: &mut f32, nodes: &mut Vec<Node>) {
        let m = self.m();
        let t = &self.theme;
        let disabled: &[String] = match self.layout.find("agents") {
            Some(Widget::Agents { disabled }) => disabled,
            _ => &[],
        };
        let agents: Vec<&AgentUsage> = self
            .state
            .agents
            .iter()
            .filter(|a| !disabled.contains(&a.id))
            .collect();

        let mut sections = 0;
        for agent in agents.iter().filter(|a| a.subscribed()) {
            if sections > 0 {
                *x += m.gap * 2.0;
            }
            sections += 1;
            self.agent_section(agent, x, nodes);
        }
        let summary = AgentSummary::of(agents.iter().copied());
        if !summary.is_empty() {
            if sections > 0 {
                *x += m.gap * 2.0;
            }
            self.agent_summary(&summary, x, nodes);
        } else if sections == 0 {
            self.row_message(x, nodes, "No agent usage recorded");
        }
        *x += m.gap;

        let mut refresh = self.list_chip(
            Key::new("p/agents/refresh"),
            x,
            (glyph::UPDATE, t.fg_dim),
            "Refresh",
            None,
            false,
            Some(Cmd::Do(Action::RefreshAgents {
                disabled: disabled.to_vec(),
            })),
        );
        if self.pulses.contains_key(AGENTS_REFRESH_PULSE) {
            refresh.parts.insert(
                0,
                Part::Shimmer {
                    color: t.accent,
                    radius: m.radius,
                },
            );
        }
        nodes.push(refresh);
        nodes.push(self.list_chip(
            Key::new("p/agents/start"),
            x,
            (glyph::CONSOLE, t.fg_dim),
            "Start agent",
            None,
            false,
            Some(Cmd::Do(Action::Shell("omarchy-agent --pick".into()))),
        ));
        nodes.push(self.list_chip(
            Key::new("p/agents/desktop"),
            x,
            (glyph::MONITOR, t.fg_dim),
            "Show on desktop",
            None,
            false,
            Some(Cmd::Do(Action::Shell(
                "omarchy-shell shell summon omarchy.agents".into(),
            ))),
        ));
    }

    /// One subscription: its card, then its meters.
    fn agent_section(&self, agent: &AgentUsage, x: &mut f32, nodes: &mut Vec<Node>) {
        let m = self.m();
        let t = &self.theme;
        let key = Key::new("p/agents").with(&agent.id);
        let multi = agent.accounts.len() > 1;

        let mut detail = Vec::new();
        if multi {
            detail.push(format!("{} accounts", agent.accounts.len()));
        } else if !agent.plan.is_empty() {
            detail.push(agent.plan.clone());
        }
        if agent.has_local_stats {
            detail.push(format!(
                "{} tokens today",
                format_tokens(agent.today_tokens)
            ));
        }
        let (subtitle, color) = if agent.needs_sign_in && !multi {
            ("Sign-in required".to_string(), t.red)
        } else if let Some(note) = agent.note.as_ref().filter(|_| !multi) {
            (note.clone(), t.orange)
        } else if detail.is_empty() {
            ("Subscription".to_string(), t.fg_dim)
        } else {
            (detail.join(" · "), t.fg_dim)
        };
        let mut card = self.agent_card(key, agent, &subtitle, color);
        if agent.needs_sign_in && !multi {
            card = card.tap(sign_in(&agent.id, None));
        }
        self.row_node(x, nodes, card);

        if multi {
            for (i, account) in agent.accounts.iter().enumerate() {
                if i > 0 {
                    *x += m.gap;
                }
                self.agent_account(agent, account, x, nodes);
            }
        } else {
            if agent.needs_sign_in {
                nodes.push(self.list_chip(
                    key.with("signin"),
                    x,
                    (glyph::ALERT, t.red),
                    "Sign in",
                    None,
                    false,
                    Some(sign_in(&agent.id, None)),
                ));
            }
            self.agent_windows(key, &agent.windows(), agent.limits_stale, x, nodes);
            if let Some(credits) = &agent.reset_credits {
                let count = credits.available.to_string();
                let expires = credits
                    .next_expires_at
                    .and_then(|at| format_reset_duration(at.signed_duration_since(self.utc)));
                self.agent_meter(
                    key.with("resets"),
                    x,
                    Meter {
                        title: if credits.available == 1 {
                            "Free reset"
                        } else {
                            "Free resets"
                        },
                        value: &count,
                        aside: expires.map(|e| format!("next expires in {e}")).as_deref(),
                        fraction: None,
                        fill: t.accent,
                        ticks: Vec::new(),
                    },
                    nodes,
                );
            }
        }

        if let Some(balance) = &agent.balance {
            let remaining = format_money(balance.remaining, &balance.currency);
            let spent = format!(
                "{} spent{}",
                format_money(balance.spent, &balance.currency),
                if balance.estimated { " (est.)" } else { "" }
            );
            // The meter shows what is left: prepaid credit drains toward empty.
            self.agent_meter(
                key.with("balance"),
                x,
                Meter {
                    title: "Balance",
                    value: &remaining,
                    aside: balance.fraction().map(|_| spent.as_str()),
                    fraction: balance.fraction(),
                    fill: if balance.alarming() { t.red } else { t.accent },
                    ticks: Vec::new(),
                },
                nodes,
            );
        }
    }

    /// One of several accounts: its name and plan, its meters, and a way to
    /// make it the one new sessions use.
    fn agent_account(
        &self,
        agent: &AgentUsage,
        account: &AgentAccount,
        x: &mut f32,
        nodes: &mut Vec<Node>,
    ) {
        let t = &self.theme;
        let key = Key::new("p/agents").with(&agent.id).with(&account.id);
        let mut caption = if account.plan.is_empty() {
            "Account".to_string()
        } else {
            account.plan.clone()
        };
        if account.free_resets > 0 {
            caption += &format!(" · {} free", account.free_resets);
        }
        let (caption, caption_color) = if account.needs_sign_in {
            ("Sign-in required".to_string(), t.red)
        } else if let Some(note) = &account.note {
            (note.clone(), t.orange)
        } else if account.active {
            (format!("{caption} · Active"), t.accent)
        } else {
            (caption, t.fg_dim)
        };
        let m = self.m();
        let caption = self.fit(&caption, 260.0, m.small, Role::Text);
        let label = self.fit(&account.label, 220.0, m.label * 0.95, Role::Bold);
        // Like the desktop panel's account line: a link to sign the account
        // in again, or to make it the one new sessions use.
        let link = if account.needs_sign_in {
            Some(("Sign in", t.red, sign_in(&agent.id, Some(account))))
        } else if !account.active {
            Some((
                "Use",
                t.accent,
                Cmd::Do(Action::Shell(format!(
                    "omarchy-agent-account-use {} {}",
                    shell_word(&agent.id),
                    shell_word(&account.id)
                ))),
            ))
        } else {
            None
        };
        let left_w =
            self.tw(&caption, m.small, Role::Text)
                .max(self.tw(&label, m.label * 0.95, Role::Bold));
        let link_w = link
            .as_ref()
            .map_or(0.0, |(text, ..)| 24.0 + self.tw(text, m.label, Role::Text));
        let w = (m.pad * 2.0 + left_w + link_w).round();
        let mut card = Node::new(key, Rect::new(*x, m.inset, w, m.bh))
            .fill(Fill {
                color: t.well,
                emph: None,
                radius: m.radius,
                border: account.active.then(|| t.accent.alpha(0.5)),
                sheen: false,
            })
            .part(Part::Text {
                text: caption,
                size: m.small,
                role: Role::Text,
                color: caption_color,
                emph: None,
                anchor: Anchor::Start,
                dx: m.pad,
                dy: -11.0,
                valign: VAlign::Cap,
            })
            .part(Part::Text {
                text: label,
                size: m.label * 0.95,
                role: Role::Bold,
                color: t.fg,
                emph: None,
                anchor: Anchor::Start,
                dx: m.pad,
                dy: 10.0,
                valign: VAlign::Cap,
            })
            .group(0);
        match link {
            Some((text, color, cmd)) => {
                card = card
                    .part(Part::label(text, m.label, color, Anchor::End, m.pad))
                    .tap(cmd);
            }
            None => {
                card = card.no_ripple();
                card.press_glow = false;
            }
        }
        *x += w + m.gap;
        nodes.push(card);
        let windows = crate::agents::display_windows(&account.limits);
        self.agent_windows(key, &windows, account.stale, x, nodes);
    }

    fn agent_windows(
        &self,
        key: Key,
        windows: &[AgentWindow],
        stale: bool,
        x: &mut f32,
        nodes: &mut Vec<Node>,
    ) {
        let t = &self.theme;
        for window in windows {
            let used = window.limit.used_at(self.utc);
            let value = format!("{used}%");
            let resets = window
                .limit
                .resets_in(self.utc)
                .map(|r| format!("resets in {r}"));
            let mut title = window.limit.window.clone();
            for (model, limit) in &window.scoped {
                title += &format!(" · {model} {}%", limit.used_at(self.utc));
            }
            let fill = if stale {
                t.fg_faint
            } else if used >= 90 {
                t.red
            } else {
                t.accent
            };
            self.agent_meter(
                key.with("limit").with(&window.limit.window),
                x,
                Meter {
                    title: &title,
                    value: &value,
                    aside: resets.as_deref(),
                    fraction: Some(f32::from(used) / 100.0),
                    fill,
                    ticks: window
                        .scoped
                        .iter()
                        .map(|(_, l): &(&str, &AgentLimit)| f32::from(l.used_at(self.utc)) / 100.0)
                        .collect(),
                },
                nodes,
            );
        }
    }

    /// The provider's mark (or the agents glyph), name and one line below.
    fn agent_card(&self, key: Key, agent: &AgentUsage, subtitle: &str, color: Rgba) -> Node {
        let m = self.m();
        let icon = (glyph::AGENTS, self.theme.accent);
        let name = self.fit(&agent.name, 240.0, m.small * 1.15, Role::Bold);
        let subtitle = self.fit(subtitle, 300.0, m.small, Role::Text);
        let mut node = self.info(key, 0.0, icon, &name, &subtitle);
        if let Some(image) = agent.icons.on_surface(self.theme.well) {
            node.parts[0] = Part::IconImage {
                icon: image.clone(),
                size: self.iw(icon.0, m.icon * 1.1, Role::Icon),
                dx: 16.0,
            };
        }
        if let Some(Part::Text { color: c, .. }) = node.parts.get_mut(2) {
            *c = color;
        }
        node
    }

    /// A limit meter like the desktop panel's: caption and countdown on
    /// top, the value beside a thin track below.
    fn agent_meter(&self, key: Key, x: &mut f32, meter: Meter, nodes: &mut Vec<Node>) {
        let m = self.m();
        let t = &self.theme;
        let small = m.small;
        let big = m.label * 0.95;
        let title_w = self.tw(meter.title, small, Role::Text);
        let aside_w = meter.aside.map_or(0.0, |a| self.tw(a, small, Role::Text));
        let value_w = self.tw(meter.value, big, Role::Bold);
        // Values sit in a column as wide as "100%", so tracks start together.
        let column = value_w.max(self.tw("100%", big, Role::Bold)) + 12.0;
        let top_w = title_w + if aside_w > 0.0 { 20.0 + aside_w } else { 0.0 };
        let bottom_w = match meter.fraction {
            Some(_) => column + 80.0,
            None => value_w,
        };
        let w = (m.pad * 2.0 + top_w.max(bottom_w)).max(METER_W).round();

        let text = |text: &str, size: f32, role: Role, color: Rgba, anchor, dy: f32| Part::Text {
            text: text.into(),
            size,
            role,
            color,
            emph: None,
            anchor,
            dx: m.pad,
            dy,
            valign: VAlign::Cap,
        };
        let mut card = Node::new(key, Rect::new(*x, m.inset, w, m.bh))
            .fill(Fill::solid(t.well, m.radius))
            .part(text(
                meter.title,
                small,
                Role::Text,
                t.fg_dim,
                Anchor::Start,
                -11.0,
            ))
            .part(text(
                meter.value,
                big,
                Role::Bold,
                t.fg,
                Anchor::Start,
                10.0,
            ));
        if let Some(aside) = meter.aside {
            card = card.part(text(aside, small, Role::Text, t.fg_dim, Anchor::End, -11.0));
        }
        let mut card = card.no_ripple().group(0);
        card.press_glow = false;
        nodes.push(card);

        if let Some(fraction) = meter.fraction {
            let track_x = *x + m.pad + column;
            let track_w = w - m.pad * 2.0 - column;
            let cy = m.inset + m.bh / 2.0 + 10.0;
            let h = 6.0;
            let mut track = Node::new(
                key.with("track"),
                Rect::new(track_x, cy - h / 2.0, track_w, h),
            )
            .part(Part::Progress {
                value: fraction.clamp(0.0, 1.0),
                color: meter.fill,
                track: Some(t.fg_faint.alpha(0.35)),
                inset: 0.0,
                radius: h / 2.0,
            })
            .no_ripple()
            .group(0);
            track.press_glow = false;
            nodes.push(track);
            for (i, tick) in meter.ticks.iter().enumerate() {
                let tw = 3.0;
                let along = (tick.clamp(0.0, 1.0) * track_w - tw / 2.0).clamp(0.0, track_w - tw);
                let mut node = Node::new(
                    key.with("tick").index(i as i64),
                    Rect::new(track_x + along, cy - h * 1.25, tw, h * 2.5),
                )
                .fill(Fill::solid(meter.fill, tw / 2.0))
                .no_ripple()
                .group(0);
                node.press_glow = false;
                nodes.push(node);
            }
        }
        *x += w + m.gap;
    }

    /// What the token counts add up to across every agent.
    fn agent_summary(&self, summary: &AgentSummary, x: &mut f32, nodes: &mut Vec<Node>) {
        let t = &self.theme;
        let key = Key::new("p/agents/summary");
        let (title, week_in_title) = if summary.today_tokens > 0 {
            (
                format!("{} tokens today", format_tokens(summary.today_tokens)),
                false,
            )
        } else {
            (
                format!("{} tokens this week", format_tokens(summary.week_tokens)),
                true,
            )
        };
        let mut detail = Vec::new();
        if summary.prompts > 0 {
            detail.push(plural(summary.prompts, "prompt"));
        }
        if summary.sessions > 0 {
            detail.push(plural(summary.sessions, "session"));
        }
        let subtitle = if detail.is_empty() {
            "All agents".to_string()
        } else {
            detail.join(" · ")
        };
        let card = self.info(key, 0.0, (glyph::AGENTS, t.accent), &title, &subtitle);
        self.row_node(x, nodes, card);

        let mut stat = |label: &str, value: &str| {
            self.net_stat(
                key.with(label),
                x,
                label,
                t.fg_dim,
                value,
                t.fg,
                "",
                None,
                None,
                nodes,
            );
        };
        if !week_in_title && summary.week_tokens > 0 {
            stat(
                "This week",
                &format!("{} tokens", format_tokens(summary.week_tokens)),
            );
        }
        if let Some(model) = &summary.top_model {
            stat(
                "Mostly",
                &self.fit(model, 220.0, self.m().label * 0.95, Role::Bold),
            );
        }
        if let Some(day) = summary.busiest_day {
            stat("Busiest day", &day.format("%A").to_string());
        }
    }
}

/// Signs an agent's account in again in a terminal, as the desktop panel's
/// _Sign-in required_ link does.
fn sign_in(agent: &str, account: Option<&AgentAccount>) -> Cmd {
    let target = match account {
        Some(a) if !a.primary => shell_word(&a.id),
        _ => ":primary".into(),
    };
    Cmd::Do(Action::Shell(format!(
        "omarchy-launch-floating-terminal-with-presentation omarchy-agent-account-add --reauth {target} {}",
        shell_word(agent)
    )))
}

/// Quotes a record-supplied word for the shell.
fn shell_word(word: &str) -> String {
    if !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'))
    {
        word.into()
    } else {
        format!("'{}'", word.replace('\'', r"'\''"))
    }
}

fn plural(n: u64, word: &str) -> String {
    format!("{n} {word}{}", if n == 1 { "" } else { "s" })
}
