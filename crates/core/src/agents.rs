//! AI agent usage, read the way Omarchy's Agents panel reads it.
//!
//! Each agent is one record in `~/.local/state/omarchy/agents/usage/`,
//! written by `omarchy-agent-usage-update`. Parsing and normalization follow
//! `shell/plugins/agents/{Main,Panel}.qml`: limit windows get one vocabulary
//! (Session, Weekly, Monthly), model-scoped allowances ride on the window
//! they share a clock with, and token counts read as `148.1M`.

use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use serde_json::Value;

use crate::agent_icons::AgentIcons;

/// One rate-limit window.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentLimit {
    /// Display title: `Session`, `Weekly`, `Monthly`, or the collector's own
    /// (`Spend limit`, `Fable Weekly`).
    pub window: String,
    pub used: u8,
    pub resets_at: Option<DateTime<Utc>>,
}

impl AgentLimit {
    /// Whether this limit's reset timestamp has passed by `now`.
    pub fn expired(&self, now: DateTime<Utc>) -> bool {
        self.resets_at.is_some_and(|t| t <= now)
    }

    /// Effective percentage used at `now` (resets to 0 once `resets_at` passes).
    pub fn used_at(&self, now: DateTime<Utc>) -> u8 {
        if self.expired(now) { 0 } else { self.used }
    }

    /// Remaining time until reset formatted like Omarchy's top menu
    /// (`4d 19h`, `3h 25m`, `12m`), or `None` if unknown or already reset.
    pub fn resets_in(&self, now: DateTime<Utc>) -> Option<String> {
        let remaining = self.resets_at?.signed_duration_since(now);
        format_reset_duration(remaining)
    }

    /// Bucket of the currently displayed reset duration, used to detect
    /// countdown ticks without allocating strings.
    pub(crate) fn reset_display_bucket(&self, now: DateTime<Utc>) -> Option<(u8, i64)> {
        countdown_bucket(self.resets_at?, now)
    }

    /// Time until the displayed reset countdown or expiry changes, if any.
    pub(crate) fn next_reset_change(&self, now: DateTime<Utc>) -> Option<std::time::Duration> {
        next_countdown_change(self.resets_at?, now)
    }
}

pub(crate) fn countdown_bucket(at: DateTime<Utc>, now: DateTime<Utc>) -> Option<(u8, i64)> {
    let remaining = at.signed_duration_since(now);
    if remaining <= chrono::Duration::zero() {
        return None;
    }
    let minutes = remaining.num_minutes();
    let hours = minutes / 60;
    let days = hours / 24;
    if days > 0 {
        Some((2, hours))
    } else if hours > 0 {
        Some((1, minutes))
    } else {
        Some((0, minutes.max(1)))
    }
}

pub(crate) fn next_countdown_change(
    at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Option<std::time::Duration> {
    let ms = at.signed_duration_since(now).num_milliseconds();
    if ms <= 0 {
        return None;
    }
    const MINUTE_MS: i64 = 60_000;
    const HOUR_MS: i64 = 60 * MINUTE_MS;
    const DAY_MS: i64 = 24 * HOUR_MS;
    let wait_ms = if ms >= DAY_MS {
        // Displays "Xd Yh"; changes when crossing a whole hour (including 24h -> 23h 59m).
        (ms % HOUR_MS) + 1
    } else if ms >= 2 * MINUTE_MS {
        // Displays "Xh Ym" or "Nm" (N >= 2); changes when crossing a whole minute.
        (ms % MINUTE_MS) + 1
    } else {
        // Displays "1m" for (0, 2m); changes when reaching 0 (expires).
        ms
    };
    Some(std::time::Duration::from_millis(wait_ms as u64))
}

/// Formats a positive countdown duration identically to Omarchy's Agents panel
/// (`formatDuration` in `shell/plugins/agents/Panel.qml`).
pub fn format_reset_duration(remaining: chrono::Duration) -> Option<String> {
    if remaining <= chrono::Duration::zero() {
        return None;
    }
    let minutes = remaining.num_minutes();
    let hours = minutes / 60;
    let days = hours / 24;
    if days > 0 {
        Some(format!("{days}d {}h", hours % 24))
    } else if hours > 0 {
        Some(format!("{hours}h {}m", minutes % 60))
    } else {
        Some(format!("{}m", minutes.max(1)))
    }
}

/// A limit window as the panel lays it out: model-scoped allowances on the
/// same clock (Claude's `Fable Weekly`) ride along as ticks on its meter.
#[derive(Clone, Debug, PartialEq)]
pub struct AgentWindow<'a> {
    pub limit: &'a AgentLimit,
    /// `(model, limit)` for each scoped allowance attached to this window.
    pub scoped: Vec<(&'a str, &'a AgentLimit)>,
}

/// A prepaid credit ledger.
#[derive(Clone, Debug, PartialEq)]
pub struct AgentBalance {
    pub remaining: f64,
    pub funded: f64,
    pub spent: f64,
    pub currency: String,
    pub estimated: bool,
}

impl AgentBalance {
    /// What is left of the funded credit, when the ledger knows the total.
    pub fn fraction(&self) -> Option<f32> {
        (self.funded > 0.0).then(|| (self.remaining / self.funded).clamp(0.0, 1.0) as f32)
    }

    /// The last 10% of funded credit lights the same alarm as a full window.
    pub fn alarming(&self) -> bool {
        self.funded > 0.0 && self.remaining / self.funded <= 0.1
    }
}

/// Codex's banked free limit resets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentResetCredits {
    pub available: u32,
    pub next_expires_at: Option<DateTime<Utc>>,
}

/// One of several subscription accounts for the same agent.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentAccount {
    pub id: String,
    pub label: String,
    pub plan: String,
    /// The account living in the agent's own home (`~/.claude`, …).
    pub primary: bool,
    /// The account new sessions start as.
    pub active: bool,
    pub limits: Vec<AgentLimit>,
    pub stale: bool,
    pub needs_sign_in: bool,
    pub note: Option<String>,
    pub free_resets: u32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AgentUsage {
    pub icons: AgentIcons,
    pub id: String,
    pub name: String,
    /// Plan or tier (`Pro`, `Max 20x`, `Prepaid`).
    pub plan: String,
    pub has_local_stats: bool,
    pub has_prompt_stats: bool,
    pub today_tokens: u64,
    pub today_prompts: u64,
    pub today_sessions: u64,
    /// Tokens per day for the last week, oldest first.
    pub recent_days: Vec<(NaiveDate, u64)>,
    /// All-time tokens per model id.
    pub models: Vec<(String, u64)>,
    pub limits: Vec<AgentLimit>,
    /// The limits are the last ones known; the latest check failed.
    pub limits_stale: bool,
    pub balance: Option<AgentBalance>,
    pub reset_credits: Option<AgentResetCredits>,
    pub accounts: Vec<AgentAccount>,
    /// The sign-in lapsed and can be redone from the panel.
    pub needs_sign_in: bool,
    /// Why the numbers aren't current, when they aren't.
    pub note: Option<String>,
}

impl AgentUsage {
    /// Whether the panel gives this agent a section of its own: limits, a
    /// balance, a plan or accounts. Local history alone only feeds the summary.
    pub fn subscribed(&self) -> bool {
        !self.limits.is_empty()
            || !self.accounts.is_empty()
            || self.balance.is_some()
            || !self.plan.is_empty()
    }

    pub fn windows(&self) -> Vec<AgentWindow<'_>> {
        display_windows(&self.limits)
    }

    /// Nearly out: the fullest window is at 90% or more, or a prepaid
    /// balance is down to its last 10% (what lights Omarchy's bar icon).
    pub fn alarming(&self, now: DateTime<Utc>) -> bool {
        self.limits.iter().any(|l| l.used_at(now) >= 90)
            || self.balance.as_ref().is_some_and(AgentBalance::alarming)
    }

    /// The agent's limits and every account's.
    pub fn all_limits(&self) -> impl Iterator<Item = &AgentLimit> {
        self.limits
            .iter()
            .chain(self.accounts.iter().flat_map(|a| &a.limits))
    }

    /// Tokens over the recent days the record covers.
    pub fn week_tokens(&self) -> u64 {
        self.recent_days.iter().map(|(_, n)| n).sum()
    }
}

/// What the token counts add up to across every agent (the hero line of
/// Omarchy's panel).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentSummary {
    pub today_tokens: u64,
    pub week_tokens: u64,
    pub prompts: u64,
    pub sessions: u64,
    pub top_model: Option<String>,
    pub busiest_day: Option<NaiveDate>,
}

impl AgentSummary {
    pub fn of<'a>(agents: impl IntoIterator<Item = &'a AgentUsage>) -> AgentSummary {
        let mut summary = AgentSummary::default();
        let mut by_day: Vec<(NaiveDate, u64)> = Vec::new();
        let mut by_model: Vec<(String, u64)> = Vec::new();
        for agent in agents {
            summary.today_tokens += agent.today_tokens;
            if agent.has_prompt_stats {
                summary.prompts += agent.today_prompts;
                summary.sessions += agent.today_sessions;
            }
            for &(date, tokens) in &agent.recent_days {
                summary.week_tokens += tokens;
                match by_day.iter_mut().find(|(d, _)| *d == date) {
                    Some((_, n)) => *n += tokens,
                    None => by_day.push((date, tokens)),
                }
            }
            for (id, tokens) in &agent.models {
                let name = friendly_model_name(id);
                match by_model.iter_mut().find(|(m, _)| *m == name) {
                    Some((_, n)) => *n += tokens,
                    None => by_model.push((name, *tokens)),
                }
            }
        }
        summary.busiest_day = first_max(&by_day).map(|i| by_day[i].0);
        summary.top_model = first_max(&by_model).map(|i| by_model[i].0.clone());
        summary
    }

    pub fn is_empty(&self) -> bool {
        self.today_tokens == 0 && self.week_tokens == 0 && self.prompts == 0
    }
}

/// Index of the first largest positive count (the first of equals wins, as
/// in Omarchy).
fn first_max<T>(list: &[(T, u64)]) -> Option<usize> {
    let mut best: Option<usize> = None;
    for (i, (_, n)) in list.iter().enumerate() {
        if *n > 0 && best.is_none_or(|b| *n > list[b].1) {
            best = Some(i);
        }
    }
    best
}

/// `148121357` → `148.1M`, as Omarchy's `formatTokenCount`.
pub fn format_tokens(n: u64) -> String {
    // Halves round up, as JavaScript's number formatting does.
    let one = |v: f64| (v * 10.0).round() / 10.0;
    let n = n as f64;
    if n >= 1e9 {
        format!("{:.1}B", one(n / 1e9))
    } else if n >= 1e6 {
        format!("{:.1}M", one(n / 1e6))
    } else if n >= 1e3 {
        format!("{:.1}K", one(n / 1e3))
    } else {
        format!("{n}")
    }
}

/// `4.5, "USD"` → `$4.50`.
pub fn format_money(value: f64, currency: &str) -> String {
    let amount = if value.is_finite() { value } else { 0.0 };
    let code = currency.to_uppercase();
    let prefix = match code.as_str() {
        "" | "USD" => "$".to_string(),
        "EUR" => "€".into(),
        "GBP" => "£".into(),
        _ => format!("{code} "),
    };
    format!("{prefix}{amount:.2}")
}

/// `claude-opus-4-8` → `Opus 4.8`, `gpt-5.6-sol` → `GPT 5.6 Sol`.
pub fn friendly_model_name(id: &str) -> String {
    if id.is_empty() {
        return "Unknown".into();
    }
    let mut name = id.strip_prefix("claude-").unwrap_or(id);
    // A trailing date stamp (`-20250514`).
    if let Some((head, tail)) = name.rsplit_once('-')
        && tail.len() == 8
        && tail.bytes().all(|b| b.is_ascii_digit())
    {
        name = head;
    }
    let mut words: Vec<String> = Vec::new();
    let mut version: Vec<&str> = Vec::new();
    for part in name.split('-').filter(|p| !p.is_empty()) {
        if part.starts_with(|c: char| c.is_ascii_digit()) {
            version.push(part);
            continue;
        }
        if !version.is_empty() {
            words.push(version.join("."));
            version.clear();
        }
        words.push(match part {
            "gpt" => "GPT".into(),
            "deepseek" => "DeepSeek".into(),
            _ => {
                let mut chars = part.chars();
                chars
                    .next()
                    .map(|c| c.to_uppercase().chain(chars).collect())
                    .unwrap_or_default()
            }
        });
    }
    if !version.is_empty() {
        words.push(version.join("."));
    }
    if words.is_empty() {
        "Unknown".into()
    } else {
        words.join(" ")
    }
}

fn window_is_long(text: &str) -> bool {
    ["week", "7-day", "seven", "month", "30-day"]
        .iter()
        .any(|w| text.contains(w))
}

/// The length a label like `5h window`, `Session (5-hour)` or `30m window`
/// spells out, in minutes (0 when it names none).
fn window_span_minutes(label: &str) -> u64 {
    let text = label.to_lowercase();
    if text.contains("month") || text.contains("30-day") {
        return 30 * 24 * 60;
    }
    if window_is_long(&text) {
        return 7 * 24 * 60;
    }
    // `(\d+)\s*-?\s*<unit>\b`, first match of hours, then of minutes.
    let unit_span = |units: &[&str]| -> Option<u64> {
        let bytes = text.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if !bytes[i].is_ascii_digit() {
                i += 1;
                continue;
            }
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            let number: u64 = text[start..i].parse().ok()?;
            let mut j = i;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'-' {
                j += 1;
            }
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            let rest = &text[j..];
            for unit in units {
                if let Some(after) = rest.strip_prefix(unit)
                    && !after
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_alphanumeric() || c == '_')
                {
                    return Some(number);
                }
            }
        }
        None
    };
    if let Some(hours) = unit_span(&["hour", "h"]) {
        return hours * 60;
    }
    unit_span(&["minutes", "minute", "mins", "min", "m"]).unwrap_or(0)
}

/// Claude spells its windows out (`Session (5-hour)`), Codex abbreviates
/// them (`5h window`); both land on the same title, as in Omarchy.
pub fn window_title(label: &str) -> String {
    let text = label.to_lowercase();
    if text.contains("month") {
        return "Monthly".into();
    }
    if window_is_long(&text) {
        return "Weekly".into();
    }
    if text.contains("session") || window_span_minutes(label) > 0 {
        return "Session".into();
    }
    let plain = match (label.find('('), label.rfind(')')) {
        (Some(open), Some(close)) if close > open => {
            let head = label[..open].trim_end();
            let tail = label[close + 1..].trim_start();
            format!("{head}{tail}")
        }
        _ => label.to_string(),
    };
    let plain = plain.trim();
    if plain.is_empty() {
        "Limit".into()
    } else {
        plain.into()
    }
}

/// `Fable Weekly` → `("Fable", "Weekly")`.
fn scoped_part(title: &str) -> Option<(&str, &str)> {
    let (model, window) = title.rsplit_once(' ')?;
    (!model.is_empty() && matches!(window, "Session" | "Weekly" | "Monthly"))
        .then_some((model, window))
}

/// Lays limits out as Omarchy does: a model-scoped window joins the window
/// it is named for; one with nothing to join keeps a row of its own.
pub fn display_windows(limits: &[AgentLimit]) -> Vec<AgentWindow<'_>> {
    let mut out: Vec<AgentWindow> = limits
        .iter()
        .filter(|l| scoped_part(&l.window).is_none())
        .map(|limit| AgentWindow {
            limit,
            scoped: Vec::new(),
        })
        .collect();
    for limit in limits {
        let Some((model, window)) = scoped_part(&limit.window) else {
            continue;
        };
        match out.iter_mut().find(|w| w.limit.window == window) {
            Some(base) => base.scoped.push((model, limit)),
            None => out.push(AgentWindow {
                limit,
                scoped: Vec::new(),
            }),
        }
    }
    out
}

fn parse_time(value: &Value) -> Option<DateTime<Utc>> {
    if let Some(raw) = value.as_str() {
        let s = raw.trim();
        if s.is_empty() {
            return None;
        }
        if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
            return Some(dt.with_timezone(&Utc));
        }
        for fmt in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%d %H:%M:%S%.f"] {
            if let Ok(naive) = NaiveDateTime::parse_from_str(s, fmt) {
                return Some(naive.and_utc());
            }
        }
        if let Ok(date) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
            return date.and_hms_opt(0, 0, 0).map(|dt| dt.and_utc());
        }
        return None;
    }
    let ts = value
        .as_i64()
        .or_else(|| value.as_f64().filter(|n| n.is_finite()).map(|n| n as i64))?;
    if ts <= 0 {
        return None;
    }
    if ts >= 1_000_000_000_000 {
        DateTime::from_timestamp_millis(ts)
    } else {
        DateTime::from_timestamp(ts, 0)
    }
}

fn parse_limits(list: &Value, now: DateTime<Utc>) -> Vec<AgentLimit> {
    list.as_array()
        .into_iter()
        .flatten()
        .filter_map(|limit| {
            let percent = limit["percent"].as_f64().filter(|p| *p >= 0.0)?;
            let resets_at = parse_time(&limit["resetsAt"]);
            let (used, resets_at) = if resets_at.is_some_and(|t| t <= now) {
                (0, None)
            } else {
                ((percent * 100.0).round().min(255.0) as u8, resets_at)
            };
            let window = match limit["title"].as_str().filter(|s| !s.is_empty()) {
                Some(title) => title.to_owned(),
                None => window_title(limit["label"].as_str().unwrap_or_default()),
            };
            Some(AgentLimit {
                window,
                used,
                resets_at,
            })
        })
        .collect()
}

/// Only the agents `omarchy-agent-account-add` signs in can be signed in
/// again from the panel.
pub fn supports_sign_in(id: &str) -> bool {
    matches!(id, "claude" | "codex" | "grok")
}

fn sign_in_lapsed(status: &str) -> bool {
    matches!(status, "Sign-in expired" | "Waiting for auth")
}

/// Whatever keeps the numbers from being current, said briefly: the
/// collector's status, else its help line.
fn usage_note(record: &Value, stale: bool) -> Option<String> {
    let status = record["usageStatusText"]
        .as_str()
        .unwrap_or_default()
        .trim();
    let help = record["authHelpText"].as_str().unwrap_or_default().trim();
    if status.is_empty() && !(stale && !help.is_empty()) {
        return None;
    }
    Some(if status.is_empty() { help } else { status }.replace('`', ""))
}

fn plan_label(tier: &str) -> String {
    let mut chars = tier.trim().chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// Parses one usage record, or `None` when the agent has nothing to show
/// (Omarchy's `providerHasData`).
pub fn from_record(record: &Value, now: DateTime<Utc>) -> Option<AgentUsage> {
    let id = record["id"].as_str().filter(|s| !s.is_empty())?;
    let count = |key: &str| {
        record[key]
            .as_u64()
            .or_else(|| record[key].as_f64().filter(|n| *n > 0.0).map(|n| n as u64))
            .unwrap_or(0)
    };
    let limits = parse_limits(&record["limits"], now);
    let plan = plan_label(record["tierLabel"].as_str().unwrap_or_default());
    let ledger = &record["balance"];
    let balance = ledger["remaining"]
        .as_f64()
        .filter(|n| n.is_finite() && *n >= 0.0)
        .map(|remaining| AgentBalance {
            remaining,
            funded: ledger["funded"]
                .as_f64()
                .filter(|n| n.is_finite() && *n > 0.0)
                .unwrap_or(0.0),
            spent: ledger["spent"].as_f64().unwrap_or(0.0).max(0.0),
            currency: ledger["currency"].as_str().unwrap_or("USD").to_owned(),
            estimated: ledger["estimated"] == true,
        });
    let supports = supports_sign_in(id);
    let accounts: Vec<AgentAccount> = record["accounts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| {
            let account_id = a["id"].as_str().filter(|s| !s.is_empty())?;
            let limits = parse_limits(&a["limits"], now);
            let stale = a["stale"] == true;
            let status = a["usageStatusText"].as_str().unwrap_or_default();
            let needs_sign_in = supports && sign_in_lapsed(status);
            Some(AgentAccount {
                id: account_id.into(),
                label: a["label"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .unwrap_or(account_id)
                    .into(),
                plan: plan_label(a["plan"].as_str().unwrap_or_default()),
                primary: a["primary"] == true,
                active: a["active"] == true,
                limits,
                stale,
                needs_sign_in,
                note: (!needs_sign_in).then(|| usage_note(a, stale)).flatten(),
                free_resets: a["resetCredits"]["available"].as_u64().unwrap_or(0) as u32,
            })
        })
        .collect();

    let has_data = [
        "totalPrompts",
        "totalSessions",
        "activeDays",
        "todayPrompts",
        "todaySessions",
        "todayTotalTokens",
    ]
    .into_iter()
    .any(|k| count(k) > 0)
        || !limits.is_empty()
        || !accounts.is_empty()
        || balance.is_some()
        || (record["ready"] == true && !plan.is_empty());
    if !has_data {
        return None;
    }

    let status = record["usageStatusText"].as_str().unwrap_or_default();
    let needs_sign_in = supports && sign_in_lapsed(status);
    let limits_stale = record["limitsStale"] == true;
    let credits = &record["resetCredits"];
    let reset_credits = credits["available"]
        .as_u64()
        .filter(|n| *n > 0)
        .map(|available| AgentResetCredits {
            available: available as u32,
            next_expires_at: parse_time(&credits["nextExpiresAt"]),
        });
    let recent_days = record["recentDays"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|day| {
            let date = NaiveDate::parse_from_str(day["date"].as_str()?, "%Y-%m-%d").ok()?;
            Some((date, day["messageCount"].as_u64().unwrap_or(0)))
        })
        .collect();
    let models = record["modelUsage"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(model, bucket)| {
            let total = [
                "inputTokens",
                "outputTokens",
                "cacheReadInputTokens",
                "cacheCreationInputTokens",
            ]
            .into_iter()
            .map(|k| bucket[k].as_u64().unwrap_or(0))
            .sum();
            (model.clone(), total)
        })
        .collect();
    Some(AgentUsage {
        icons: AgentIcons::default(),
        id: id.into(),
        name: record["name"]
            .as_str()
            .filter(|s| !s.is_empty())
            .unwrap_or(id)
            .into(),
        plan,
        has_local_stats: record["hasLocalStats"] != false,
        has_prompt_stats: record["hasPromptStats"] != false,
        today_tokens: count("todayTotalTokens"),
        today_prompts: count("todayPrompts"),
        today_sessions: count("todaySessions"),
        recent_days,
        models,
        limits,
        limits_stale,
        balance,
        reset_credits,
        accounts,
        needs_sign_in,
        note: (!needs_sign_in)
            .then(|| usage_note(record, limits_stale))
            .flatten(),
    })
}

/// Reads every record in Omarchy's agents state directory
/// (`~/.local/state/omarchy/agents`), in the panel's order.
pub fn load(
    dir: &std::path::Path,
    icons: &mut crate::agent_icons::AgentIconCache,
) -> Vec<AgentUsage> {
    let now = Utc::now();
    let mut agents: Vec<_> = std::fs::read_dir(dir.join("usage"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|file| {
            let path = file.path();
            if path.extension()? != "json" || !file.file_type().ok()?.is_file() {
                return None;
            }
            let record = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
            let mut agent = from_record(&record, now)?;
            agent.icons = icons.load(&agent.id);
            Some(agent)
        })
        .collect();
    agents.sort_by(|a, b| a.id.cmp(&b.id));
    // The order someone dragged the agents into on Omarchy's panel.
    let order: Vec<String> = std::fs::read(dir.join("order.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    apply_order(&mut agents, &order);
    agents
}

/// Plausible agents for previews and snapshots.
pub fn sample(now: DateTime<Utc>) -> Vec<AgentUsage> {
    let mut icons = crate::agent_icons::AgentIconCache::default();
    let today = now.date_naive();
    let week = |tokens: [u64; 7]| {
        (0..7)
            .map(|i| (today - chrono::Days::new(6 - i as u64), tokens[i]))
            .collect()
    };
    let limit = |window: &str, used: u8, minutes: i64| AgentLimit {
        window: window.into(),
        used,
        resets_at: Some(now + chrono::Duration::minutes(minutes) + chrono::Duration::seconds(30)),
    };
    vec![
        AgentUsage {
            icons: icons.load("claude"),
            id: "claude".into(),
            name: "Claude Code".into(),
            plan: "Max 5x".into(),
            has_local_stats: true,
            has_prompt_stats: true,
            today_tokens: 48_200_000,
            today_prompts: 24,
            today_sessions: 5,
            recent_days: week([
                0, 31_000_000, 52_000_000, 12_000_000, 0, 64_000_000, 48_200_000,
            ]),
            models: vec![("claude-opus-5-5".into(), 180_000_000)],
            limits: vec![
                limit("Session", 38, 3 * 60 + 25),
                limit("Weekly", 62, (4 * 24 + 19) * 60 + 30),
                limit("Fable Weekly", 41, (4 * 24 + 19) * 60 + 30),
            ],
            ..AgentUsage::default()
        },
        AgentUsage {
            icons: icons.load("codex"),
            id: "codex".into(),
            name: "Codex".into(),
            plan: "Plus".into(),
            has_local_stats: true,
            has_prompt_stats: true,
            today_tokens: 6_400_000,
            today_prompts: 9,
            today_sessions: 2,
            recent_days: week([0, 0, 8_000_000, 0, 3_000_000, 0, 6_400_000]),
            models: vec![("gpt-6.2-sol".into(), 17_400_000)],
            limits: vec![limit("Weekly", 71, 36 * 60 + 10)],
            reset_credits: Some(AgentResetCredits {
                available: 2,
                next_expires_at: Some(now + chrono::Duration::days(16)),
            }),
            ..AgentUsage::default()
        },
    ]
}

/// Puts agents in the order dragged into Omarchy's panel (`order.json`);
/// ones the order doesn't name keep their place after the ones it does.
pub fn apply_order(agents: &mut [AgentUsage], order: &[String]) {
    let rank = |a: &AgentUsage| {
        order
            .iter()
            .position(|id| *id == a.id)
            .unwrap_or(order.len())
    };
    agents.sort_by_key(rank);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn formats_agent_reset_countdowns_and_handles_expiry() {
        let now = at("2026-11-26T12:00:00Z");
        let limit = |delta: chrono::Duration| AgentLimit {
            window: "Session".into(),
            used: 75,
            resets_at: Some(now + delta),
        };
        let cases = [
            (
                chrono::Duration::hours(4 * 24 + 19) + chrono::Duration::minutes(40),
                "4d 19h",
            ),
            (chrono::Duration::hours(24), "1d 0h"),
            (
                chrono::Duration::hours(24) - chrono::Duration::seconds(1),
                "23h 59m",
            ),
            (chrono::Duration::minutes(3 * 60 + 25), "3h 25m"),
            (chrono::Duration::minutes(60), "1h 0m"),
            (
                chrono::Duration::minutes(59) + chrono::Duration::seconds(59),
                "59m",
            ),
            (chrono::Duration::seconds(30), "1m"),
            (chrono::Duration::milliseconds(1), "1m"),
        ];
        for (delta, text) in cases {
            assert_eq!(limit(delta).resets_in(now).as_deref(), Some(text));
        }

        let at_reset = limit(chrono::Duration::zero());
        assert!(at_reset.expired(now));
        assert_eq!(at_reset.used_at(now), 0);
        assert_eq!(at_reset.resets_in(now), None);
        assert_eq!(at_reset.next_reset_change(now), None);

        let no_reset = AgentLimit {
            window: "Daily".into(),
            used: 42,
            resets_at: None,
        };
        assert!(!no_reset.expired(now));
        assert_eq!(no_reset.used_at(now), 42);
        assert_eq!(no_reset.resets_in(now), None);

        // next_reset_change hits the exact display transition boundary.
        for remaining_ms in [
            2 * 24 * 3600 * 1000,
            24 * 3600 * 1000 + 30_000,
            24 * 3600 * 1000,
            3600 * 1000,
            2 * 60_000,
            90_000,
            5_000,
        ] {
            let l = limit(chrono::Duration::milliseconds(remaining_ms));
            let wait = l.next_reset_change(now).unwrap();
            let before = l.resets_in(now);
            let after = l.resets_in(now + chrono::Duration::from_std(wait).unwrap());
            assert_ne!(
                before, after,
                "expected display change after {wait:?} for remaining_ms={remaining_ms}"
            );
        }
    }

    #[test]
    fn names_windows_like_omarchy() {
        for (label, title) in [
            ("Session (5-hour)", "Session"),
            ("Weekly (7-day)", "Weekly"),
            ("5h window", "Session"),
            ("30m window", "Session"),
            ("Monthly allowance", "Monthly"),
            ("Daily", "Daily"),
            ("Spend limit", "Spend limit"),
            ("Credits (included)", "Credits"),
            // Omarchy reads "1M" as a minute; collectors title such limits.
            ("Opus 5 (1M context)", "Session"),
            ("", "Limit"),
        ] {
            assert_eq!(window_title(label), title, "{label:?}");
        }
    }

    #[test]
    fn formats_counts_money_and_models() {
        assert_eq!(format_tokens(0), "0");
        assert_eq!(format_tokens(125), "125");
        assert_eq!(format_tokens(65_062_969), "65.1M");
        assert_eq!(format_tokens(148_121_357), "148.1M");
        assert_eq!(format_tokens(1_250), "1.3K");
        assert_eq!(format_tokens(2_400_000_000), "2.4B");
        assert_eq!(format_money(4.5, "USD"), "$4.50");
        assert_eq!(format_money(12.0, "eur"), "€12.00");
        assert_eq!(format_money(3.0, "CHF"), "CHF 3.00");
        assert_eq!(friendly_model_name("claude-opus-4-8"), "Opus 4.8");
        assert_eq!(
            friendly_model_name("claude-haiku-5-5-20260101"),
            "Haiku 5.5"
        );
        assert_eq!(friendly_model_name("gpt-6.2-astra"), "GPT 6.2 Astra");
        assert_eq!(
            friendly_model_name("gemini-4-neon-xhigh"),
            "Gemini 4 Neon Xhigh"
        );
        assert_eq!(friendly_model_name(""), "Unknown");
    }

    #[test]
    fn reads_a_record_and_handles_expired_and_invalid_limits() {
        let now = at("2026-11-26T12:00:00Z");
        let record = json!({
            "id":"new-provider", "todayPrompts":1, "todayTotalTokens":125, "tierLabel":"max 20x",
            "limits":[
                {"label":"Session (5-hour)","title":"Session","percent":0.42,"resetsAt":"2026-11-26T15:25:00+00:00"},
                {"label":"Weekly (7-day)","percent":0.62,"resetsAt":"2026-12-01T07:00:00Z"},
                {"label":"Weekly (7-day)","title":"Fable Weekly","percent":0.2,"resetsAt":"2026-12-01T07:00:00Z"},
                {"label":"Daily","percent":0.15,"resetsAt":""},
                {"label":"Expired","percent":0.98,"resetsAt":"2020-01-01T00:00:00Z"},
                {"label":"Missing","percent":null}
            ],
            "balance":{"remaining":4.5,"funded":50,"spent":45.5,"currency":"EUR"},
            "resetCredits":{"available":2,"nextExpiresAt":"2026-12-12T22:21:36+00:00"},
            "recentDays":[{"date":"2026-11-25","messageCount":10},{"date":"2026-11-26","messageCount":125}],
            "modelUsage":{"claude-opus-5-5":{"inputTokens":3,"outputTokens":7}}
        });
        let agent = from_record(&record, now).unwrap();
        assert_eq!(agent.name, "new-provider");
        assert_eq!(agent.plan, "Max 20x");
        assert_eq!((agent.today_prompts, agent.today_tokens), (1, 125));
        let windows: Vec<_> = agent
            .limits
            .iter()
            .map(|l| (l.window.as_str(), l.used, l.resets_at.is_some()))
            .collect();
        assert_eq!(
            windows,
            [
                ("Session", 42, true),
                ("Weekly", 62, true),
                ("Fable Weekly", 20, true),
                ("Daily", 15, false),
                ("Expired", 0, false),
            ]
        );
        let shown = agent.windows();
        assert_eq!(shown.len(), 4);
        assert_eq!(shown[1].limit.window, "Weekly");
        assert_eq!(shown[1].scoped.len(), 1);
        assert_eq!(shown[1].scoped[0].0, "Fable");
        let balance = agent.balance.as_ref().unwrap();
        assert!(balance.alarming());
        assert_eq!(format_money(balance.remaining, &balance.currency), "€4.50");
        assert!(agent.alarming(now));
        assert_eq!(agent.reset_credits.as_ref().unwrap().available, 2);
        assert_eq!(agent.week_tokens(), 135);
        assert!(agent.subscribed());

        assert!(from_record(&json!({"id":"unused","ready":false}), now).is_none());
        let local = from_record(&json!({"id":"local","totalPrompts":3}), now).unwrap();
        assert!(!local.subscribed());
    }

    #[test]
    fn flags_a_lapsed_sign_in_and_explains_stale_limits() {
        let now = at("2026-11-26T12:00:00Z");
        let lapsed = from_record(
            &json!({"id":"claude","ready":true,"tierLabel":"pro","usageStatusText":"Sign-in expired"}),
            now,
        )
        .unwrap();
        assert!(lapsed.needs_sign_in);
        assert_eq!(lapsed.note, None);

        let paused = from_record(
            &json!({"id":"codex","tierLabel":"plus","limitsStale":true,
                    "limits":[{"label":"5h window","percent":0.3}],
                    "authHelpText":"Run `codex login` to authenticate."}),
            now,
        )
        .unwrap();
        assert!(!paused.needs_sign_in);
        assert_eq!(
            paused.note.as_deref(),
            Some("Run codex login to authenticate.")
        );

        let other = from_record(
            &json!({"id":"kimi","ready":true,"tierLabel":"pro","usageStatusText":"Sign-in expired"}),
            now,
        )
        .unwrap();
        assert!(!other.needs_sign_in);
        assert_eq!(other.note.as_deref(), Some("Sign-in expired"));
    }

    #[test]
    fn sums_the_summary_across_agents() {
        let day = |d: u32| NaiveDate::from_ymd_opt(2026, 11, d).unwrap();
        let a = AgentUsage {
            today_tokens: 100,
            today_prompts: 2,
            has_prompt_stats: true,
            recent_days: vec![(day(25), 50), (day(26), 100)],
            models: vec![("claude-opus-5-5".into(), 900)],
            ..AgentUsage::default()
        };
        let b = AgentUsage {
            today_tokens: 10,
            today_prompts: 7,
            has_prompt_stats: false,
            recent_days: vec![(day(25), 80), (day(26), 10)],
            models: vec![("gpt-6.2-astra".into(), 400), ("gpt-6.2-sol".into(), 600)],
            ..AgentUsage::default()
        };
        let summary = AgentSummary::of([&a, &b]);
        assert_eq!(summary.today_tokens, 110);
        assert_eq!(summary.week_tokens, 240);
        assert_eq!(summary.prompts, 2);
        assert_eq!(summary.busiest_day, Some(day(25)));
        assert_eq!(summary.top_model.as_deref(), Some("Opus 5.5"));
    }

    #[test]
    fn follows_the_panel_order() {
        let agent = |id: &str| AgentUsage {
            id: id.into(),
            ..AgentUsage::default()
        };
        let mut agents = vec![agent("agy"), agent("claude"), agent("codex")];
        apply_order(&mut agents, &["codex".into(), "missing".into()]);
        let ids: Vec<_> = agents.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["codex", "agy", "claude"]);
    }
}
