//! Omarchy's Display panel. Monitor/DDC reads run once at startup, then
//! only while the panel is visible; the last reading stays in between. A
//! single worker orders writes, including the last-screen check.

use std::{
    path::Path,
    process::{Command, Stdio},
    sync::mpsc::{self, RecvTimeoutError},
    thread,
    time::Duration,
};

use omatouchbar_core::{
    display::{DisplayChange, DisplayPanel},
    paths,
};

use crate::bus::Bus;

fn monitor_state(raw: &str) -> Option<DisplayPanel> {
    // Blank lines are fields too (no external monitor, fixed brightness, etc).
    let mut lines = raw.lines();
    let brightness = lines
        .next()?
        .trim()
        .parse::<u8>()
        .ok()
        .filter(|b| *b <= 100);
    let focused = lines.nth(4)?.trim();
    let scale = lines
        .next()?
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|s| s.is_finite() && *s > 0.0);
    let mut panel = DisplayPanel {
        monitors: serde_json::from_str(lines.next()?).ok()?,
        ..Default::default()
    };
    if panel.focused().is_some_and(|m| m.name == focused) {
        panel.brightness = brightness;
        panel.scale = scale;
    }
    Some(panel)
}

fn base_size(path: &Path) -> Option<u8> {
    let text = std::fs::read_to_string(path).ok()?;
    let values: toml::Table = toml::from_str(&text).ok()?;
    values
        .get("font")?
        .get("base-size")?
        .as_integer()?
        .try_into()
        .ok()
}

// Apple's display helper uses asdcontrol through sudo. The renderer's
// NoNewPrivileges would otherwise make a controllable screen appear fixed.
// Use argv throughout, including when the command is a Hyprland expression.
fn desktop_output(program: &str, args: &[&str]) -> Option<String> {
    let result = Command::new("systemd-run")
        .args([
            "--user",
            "--quiet",
            "--collect",
            "--wait",
            "--pipe",
            "--",
            program,
        ])
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !result.status.success() {
        log::warn!("{program} exited with {}", result.status);
        return None;
    }
    Some(String::from_utf8_lossy(&result.stdout).into_owned())
}

fn read_monitors() -> Option<DisplayPanel> {
    monitor_state(&desktop_output("omarchy-monitor-state", &[])?)
}

/// Names each screen by its maker and model, read straight from Hyprland's
/// socket (Omarchy's state helper only reports connectors).
fn name_monitors(panel: &mut DisplayPanel, models: Vec<(String, String, String)>) {
    for (name, make, model) in models {
        if let Some(monitor) = panel.monitors.iter_mut().find(|m| m.name == name) {
            monitor.make = make;
            monitor.model = model;
        }
    }
}

fn read_panel() -> DisplayPanel {
    let mut panel = read_monitors().unwrap_or_default();
    if panel.monitors.len() > 1 || panel.monitors.iter().any(|m| !m.internal()) {
        name_monitors(&mut panel, super::hyprland::monitor_models());
    }
    panel.text_size = base_size(&paths::config_home().join("omarchy/shell.toml"))
        .or_else(|| base_size(&paths::state_home().join("omarchy/current/theme/shell.toml")))
        .unwrap_or(12);
    panel
}

fn apply(change: DisplayChange, osd: bool) {
    match change {
        DisplayChange::TextSize(size) => {
            desktop_output("omarchy-display-text-size", &[&size.to_string()]);
        }
        DisplayChange::Brightness { monitor, percent } => {
            let value = format!("{percent}%");
            let mut args = vec!["--monitor", &monitor, &value];
            if !osd {
                args.insert(0, "--no-osd");
            }
            desktop_output("omarchy-brightness-display", &args);
        }
        DisplayChange::Scale { monitor, preset } => {
            // This version of Omarchy's scale helper targets the focused screen.
            if super::system::output("omarchy-hyprland-monitor-focused", &[])
                .is_some_and(|focused| focused.trim() == monitor)
            {
                let scale = (f64::from(preset) / 100.0).to_string();
                desktop_output("omarchy-hyprland-monitor-scaling", &[&scale]);
            }
        }
        DisplayChange::Enabled { monitor, enabled } => {
            // Recheck immediately before the write in case outputs changed
            // since the row was drawn or two disable taps arrived together.
            if read_monitors().is_some_and(|p| p.can_set_enabled(&monitor, enabled))
                && !monitor.chars().any(char::is_control)
            {
                let name = monitor.replace('\\', "\\\\").replace('"', "\\\"");
                let expr = format!(
                    "hl.monitor({{ output = \"{name}\", disabled = {} }})",
                    !enabled
                );
                desktop_output("hyprctl", &["eval", &expr]);
            }
        }
    }
}

enum Request {
    Visible(bool),
    Change(DisplayChange, bool),
}

impl Request {
    fn brightness_target(&self) -> Option<&str> {
        match self {
            Self::Change(DisplayChange::Brightness { monitor, .. }, _) => Some(monitor),
            _ => None,
        }
    }
}

fn watch(bus: Bus, rx: mpsc::Receiver<Request>) {
    // Read once up front and keep the last reading while the panel is
    // closed: the panel then opens complete, like Sound and Network, and
    // refreshes in place instead of reflowing mid-animation.
    let panel = read_panel();
    bus.patch(move |s| s.display_panel = Some(panel));
    let mut visible = false;
    let mut interval = Duration::ZERO;
    loop {
        let received = if visible {
            rx.recv_timeout(interval)
        } else {
            rx.recv().map_err(|_| RecvTimeoutError::Disconnected)
        };
        let first = match received {
            Ok(request) => request,
            Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) => {
                let panel = read_panel();
                bus.patch(move |s| s.display_panel = Some(panel));
                interval = Duration::from_secs(5);
                continue;
            }
        };
        let mut requests = std::iter::once(first).chain(rx.try_iter()).peekable();
        while let Some(request) = requests.next() {
            // A slow DDC write may span several drag samples. Keep the
            // newest consecutive value, then let it settle before readback.
            interval = if let Some(name) = request.brightness_target() {
                if requests.peek().and_then(Request::brightness_target) == Some(name) {
                    continue;
                }
                Duration::from_millis(220)
            } else {
                Duration::ZERO
            };
            match request {
                Request::Change(change, osd) => apply(change, osd),
                Request::Visible(next) => visible = next,
            }
        }
    }
}

pub struct Provider {
    tx: mpsc::Sender<Request>,
    visible: bool,
}

impl Provider {
    pub fn start(bus: Bus) -> Self {
        let (tx, rx) = mpsc::channel();
        thread::Builder::new()
            .name("display".into())
            .spawn(move || watch(bus, rx))
            .expect("display worker");
        Self { tx, visible: false }
    }

    pub fn set_visible(&mut self, visible: bool) {
        if self.visible != visible {
            self.visible = visible;
            let _ = self.tx.send(Request::Visible(visible));
        }
    }

    pub fn change(&self, change: DisplayChange, osd: bool) {
        let _ = self.tx.send(Request::Change(change, osd));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_focused_output_and_keeps_fixed_brightness_unavailable() {
        let raw = "unavailable\neDP-1\nHDMI-A-1\neDP-1\n\nHDMI-A-1\n1.25\n[{\"name\":\"eDP-1\",\"enabled\":true,\"focused\":false,\"width\":2880,\"height\":1800},{\"name\":\"HDMI-A-1\",\"enabled\":true,\"focused\":true,\"width\":1920,\"height\":1080}]";
        let panel = monitor_state(raw).unwrap();
        assert_eq!(panel.focused().unwrap().name, "HDMI-A-1");
        assert_eq!(panel.brightness, None);
        assert_eq!(panel.scale, Some(1.25));
        assert!(panel.can_set_enabled("eDP-1", false));
        assert_eq!(
            monitor_state(&raw.replace("unavailable", "38"))
                .unwrap()
                .brightness,
            Some(38)
        );
        assert!(monitor_state("38\neDP-1\n").is_none());

        let mut panel = monitor_state(raw).unwrap();
        name_monitors(
            &mut panel,
            vec![
                ("HDMI-A-1".into(), "Dell Inc.".into(), "DELL U2720Q".into()),
                (
                    "DP-9".into(),
                    "LG Electronics".into(),
                    "LG ULTRAFINE".into(),
                ),
            ],
        );
        assert_eq!(panel.monitors[0].label(), "Built-in Display");
        assert_eq!(panel.focused().unwrap().label(), "DELL U2720Q");
    }
}
