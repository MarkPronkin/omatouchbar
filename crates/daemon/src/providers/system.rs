//! Network (NetworkManager), brightness (sysfs), indicators, Touch ID state
//! and Omarchy update availability.

use std::{
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use omatouchbar_core::{
    paths,
    state::{
        Connectivity, Indicators, NetKind, NetworkDetails, TouchId, TouchIdStage, WifiNetwork,
    },
};

use crate::{bus::Bus, providers::network as net_details};

pub(super) fn output(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program)
        .args(args)
        .env("LC_ALL", "C")
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).to_string())
}

/// Splits nmcli terse output, honoring `\:` escapes.
fn terse(line: &str) -> Vec<String> {
    let mut fields = vec![String::new()];
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(n) = chars.next() {
                    fields.last_mut().unwrap().push(n);
                }
            }
            ':' => fields.push(String::new()),
            _ => fields.last_mut().unwrap().push(c),
        }
    }
    fields
}

struct NetworkSummary {
    kind: NetKind,
    name: String,
    signal: u8,
    wifi_enabled: bool,
    wifi_available: bool,
    connectivity: Connectivity,
    can_share_qr: bool,
    networks: Vec<WifiNetwork>,
}

fn network_summary() -> NetworkSummary {
    let wifi_enabled =
        output("nmcli", &["-t", "radio", "wifi"]).is_some_and(|s| s.trim() == "enabled");
    let mut summary = NetworkSummary {
        kind: NetKind::Disconnected,
        name: String::new(),
        signal: 0,
        wifi_enabled,
        wifi_available: false,
        connectivity: Connectivity::None,
        can_share_qr: false,
        networks: Vec::new(),
    };
    let Some(devices) = output("nmcli", &["-t", "-f", "TYPE,STATE,CONNECTION", "device"]) else {
        summary.kind = NetKind::Unknown;
        summary.connectivity = Connectivity::Unknown;
        return summary;
    };
    for line in devices.lines() {
        let f = terse(line);
        if f.len() < 3 {
            continue;
        }
        if f[0] == "wifi" {
            summary.wifi_available = true;
        }
        if f[1] != "connected" {
            continue;
        }
        // Prefer wired when both are up, matching Omarchy's default-route behavior.
        match f[0].as_str() {
            "ethernet" => {
                summary.kind = NetKind::Ethernet;
                summary.name = f[2].clone();
            }
            "wifi" if summary.kind != NetKind::Ethernet => {
                summary.kind = NetKind::Wifi;
                summary.name = f[2].clone();
            }
            _ => {}
        }
    }
    summary.connectivity = if summary.kind == NetKind::Disconnected {
        Connectivity::None
    } else {
        net_details::read_connectivity()
    };
    if wifi_enabled && summary.wifi_available {
        let known: Vec<String> = output("nmcli", &["-t", "-f", "NAME,TYPE", "connection", "show"])
            .unwrap_or_default()
            .lines()
            .map(terse)
            .filter(|f| f.len() >= 2 && f[1] == "802-11-wireless")
            .map(|f| f[0].clone())
            .collect();
        let mut networks: Vec<WifiNetwork> = Vec::new();
        if let Some(list) = output(
            "nmcli",
            &[
                "-t",
                "-f",
                "ACTIVE,SSID,SIGNAL,SECURITY",
                "device",
                "wifi",
                "list",
                "--rescan",
                "no",
            ],
        ) {
            for line in list.lines() {
                let f = terse(line);
                if f.len() < 4 || f[1].is_empty() {
                    continue;
                }
                let active = f[0] == "yes";
                let signal: u8 = f[2].parse().unwrap_or(0);
                let sec = f[3].trim();
                let is_enterprise = sec.to_ascii_uppercase().contains("802.1X");
                if active && summary.kind == NetKind::Wifi {
                    summary.name = f[1].clone();
                    summary.signal = signal;
                    summary.can_share_qr = !is_enterprise;
                }
                let secure = !sec.is_empty() && sec != "--" && !sec.eq_ignore_ascii_case("OWE");
                let entry = WifiNetwork {
                    ssid: f[1].clone(),
                    signal,
                    secure,
                    known: known.contains(&f[1]),
                    active,
                };
                match networks.iter_mut().find(|n| n.ssid == entry.ssid) {
                    Some(existing) => {
                        existing.active |= entry.active;
                        existing.signal = existing.signal.max(entry.signal);
                    }
                    None => networks.push(entry),
                }
            }
        }
        networks.sort_by_key(|n| (!n.active, !n.known, std::cmp::Reverse(n.signal)));
        networks.truncate(24);
        summary.networks = networks;
    }
    summary
}

pub fn spawn_network(bus: Bus) -> mpsc::Sender<()> {
    let (tx, rx) = mpsc::channel::<()>();
    let poke = tx.clone();
    // `nmcli monitor` prints a line on every change.
    let monitor_tx = tx.clone();
    thread::Builder::new()
        .name("nm-monitor".into())
        .spawn(move || {
            loop {
                let Ok(mut child) = Command::new("nmcli")
                    .arg("monitor")
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .spawn()
                else {
                    thread::sleep(Duration::from_secs(10));
                    continue;
                };
                if let Some(stdout) = child.stdout.take() {
                    for line in BufReader::new(stdout).lines() {
                        if line.is_err() || monitor_tx.send(()).is_err() {
                            break;
                        }
                    }
                }
                let _ = child.wait();
                thread::sleep(Duration::from_secs(5));
            }
        })
        .ok();
    thread::Builder::new()
        .name("network".into())
        .spawn(move || {
            let mut last_scan = std::time::Instant::now() - Duration::from_secs(60);
            loop {
                let summary = network_summary();
                bus.patch(move |s| {
                    let n = &mut s.network;
                    n.kind = summary.kind;
                    n.name = summary.name;
                    n.signal = summary.signal;
                    n.wifi_enabled = summary.wifi_enabled;
                    n.wifi_available = summary.wifi_available;
                    n.connectivity = summary.connectivity;
                    n.networks = summary.networks;
                    n.details.can_share_qr = summary.can_share_qr;
                    if n.kind == NetKind::Disconnected && n.pending_band.is_none() {
                        n.details = NetworkDetails::default();
                        n.band_current.clear();
                        n.band_available.clear();
                    }
                });
                match rx.recv_timeout(Duration::from_secs(30)) {
                    Ok(()) => {
                        thread::sleep(Duration::from_millis(300));
                        while rx.try_recv().is_ok() {}
                        // Opening the panel asks for a fresh scan; results land
                        // a few seconds later through `nmcli monitor`.
                        if last_scan.elapsed() > Duration::from_secs(20) {
                            last_scan = std::time::Instant::now();
                            let _ = Command::new("nmcli")
                                .args(["device", "wifi", "rescan"])
                                .stdout(Stdio::null())
                                .stderr(Stdio::null())
                                .spawn()
                                .map(|mut c| c.wait());
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => return,
                }
            }
        })
        .ok();
    poke
}

fn read_number(path: &Path) -> Option<f64> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn percent(dir: &Path) -> Option<u8> {
    let value = read_number(&dir.join("brightness"))?;
    let max = read_number(&dir.join("max_brightness"))?;
    (max > 0.0).then(|| ((value / max) * 100.0).round().clamp(0.0, 100.0) as u8)
}

fn backlights() -> (Option<PathBuf>, Option<PathBuf>) {
    let list = |dir: &str| -> Vec<PathBuf> {
        std::fs::read_dir(dir)
            .map(|r| r.flatten().map(|e| e.path()).collect())
            .unwrap_or_default()
    };
    let mut display: Vec<PathBuf> = list("/sys/class/backlight")
        .into_iter()
        .filter(|p| !p.to_string_lossy().contains("kbd"))
        .collect();
    // Prefer the firmware/gmux device the desktop controls.
    display.sort_by_key(|p| {
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if name.contains("gmux") {
            0
        } else if name.starts_with("intel") || name.starts_with("amdgpu") {
            1
        } else {
            2
        }
    });
    let keyboard = list("/sys/class/leds")
        .into_iter()
        .chain(list("/sys/class/backlight"))
        .find(|p| p.to_string_lossy().contains("kbd_backlight"));
    (display.into_iter().next(), keyboard)
}

pub fn spawn_brightness(bus: Bus) -> mpsc::Sender<()> {
    let (tx, rx) = mpsc::channel::<()>();
    thread::Builder::new()
        .name("brightness".into())
        .spawn(move || {
            let (display, keyboard) = backlights();
            let mut last = (None, None);
            let mut fast_until = Instant::now();
            loop {
                let now = (
                    display.as_deref().and_then(percent),
                    keyboard.as_deref().and_then(percent),
                );
                if now != last {
                    last = now;
                    bus.patch(move |s| {
                        s.display_brightness = now.0;
                        s.keyboard_backlight = now.1;
                        // The Display panel keeps its last reading while
                        // closed; brightness keys move the laptop panel.
                        if let (Some(level), Some(panel)) = (now.0, s.display_panel.as_mut())
                            && panel.brightness.is_some()
                            && panel.focused().is_some_and(|m| m.internal())
                        {
                            panel.brightness = Some(level);
                        }
                    });
                }
                // Sysfs has no change event here. Sample between the 90 ms
                // key repeats so the meter and tick follow the actual step.
                let interval = if Instant::now() < fast_until {
                    Duration::from_millis(35)
                } else {
                    Duration::from_millis(1500)
                };
                match rx.recv_timeout(interval) {
                    Ok(()) => {
                        while rx.try_recv().is_ok() {}
                        fast_until = Instant::now() + Duration::from_millis(720);
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => return,
                }
            }
        })
        .ok();
    tx
}

/// How thoroughly to refresh Omarchy's status indicators.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndicatorRefresh {
    /// Fast, zero-sudo checks (stay awake, night light, DND, recording,
    /// remote session, reminders, plus read-only timer check for sudo expiry).
    Fast,
    /// Same as `Fast`, plus an explicit `omarchy-sudo-passwordless --active`
    /// probe (run outside `NoNewPrivileges` via `systemd-run --user`).
    Full,
}

fn current_uid() -> u32 {
    rustix::process::getuid().as_raw()
}

/// Checks for this user's running `gliff-server` processes and their SSH peer
/// addresses, mirroring Omarchy's `services/remote-session/probe.sh` without
/// spawning a shell.
fn probe_remote_session() -> (bool, Vec<String>) {
    use std::os::unix::fs::MetadataExt;
    let uid = current_uid();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return (false, Vec::new());
    };
    let mut active = false;
    let mut peers = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(pid_str) = name.to_str() else {
            continue;
        };
        if !pid_str.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let path = entry.path();
        if entry.metadata().ok().map(|m| m.uid()) != Some(uid) {
            continue;
        }
        let Ok(comm) = std::fs::read_to_string(path.join("comm")) else {
            continue;
        };
        if comm.trim() != "gliff-server" {
            continue;
        }
        active = true;
        if let Ok(env_bytes) = std::fs::read(path.join("environ")) {
            for var in env_bytes.split(|&b| b == 0) {
                if let Some(rest) = var.strip_prefix(b"SSH_CONNECTION=") {
                    let text = String::from_utf8_lossy(rest);
                    if let Some(peer) = text.split_whitespace().next()
                        && !peer.is_empty()
                        && !peers.iter().any(|p| p == peer)
                    {
                        peers.push(peer.to_string());
                    }
                }
            }
        }
    }
    (active, peers)
}

fn probe_reminders() -> (u32, String) {
    let Some(raw) = output("omarchy-reminder", &["show", "--json"]) else {
        return (0, String::new());
    };
    let Ok(val) = serde_json::from_str::<serde_json::Value>(raw.trim()) else {
        return (0, String::new());
    };
    let count = val.get("count").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let tooltip = val
        .get("tooltip")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    (count, tooltip)
}

/// Runs a privileged helper outside the renderer's `NoNewPrivileges` sandbox
/// via a transient user service, returning its exit success.
fn run_outside_sandbox(program: &str, args: &[&str]) -> bool {
    let mut cmd = Command::new("systemd-run");
    cmd.args(["--user", "--wait", "--collect", "--quiet", "--", program])
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    match cmd.status() {
        Ok(status) => status.success(),
        Err(_) => Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success()),
    }
}

fn probe_passwordless_sudo() -> bool {
    run_outside_sandbox("/usr/bin/omarchy-sudo-passwordless", &["--active"])
}

/// Revokes passwordless sudo outside `NoNewPrivileges`, notifying on failure
/// just like Omarchy's `PasswordlessSudo.qml`.
pub fn disable_passwordless_sudo() -> bool {
    let ok = run_outside_sandbox("/usr/bin/omarchy-sudo-passwordless", &["--disable"]);
    if !ok {
        let _ = Command::new("omarchy-notification-send")
            .args([
                "Could not disable passwordless sudo",
                "Check the sudo configuration and try again.",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    ok
}

/// Read-only check for an active timed passwordless-sudo expiry timer (zero
/// sudo invocations, zero auth log entries).
fn has_nopasswd_expiry_timer() -> Option<bool> {
    let pattern = format!("omarchy-nopasswd-expire-{}-*.timer", current_uid());
    let out = output(
        "systemctl",
        &[
            "list-units",
            "--type=timer",
            "--no-legend",
            "--plain",
            &pattern,
        ],
    )?;
    Some(!out.trim().is_empty())
}

/// Keeps Omarchy's status indicators in sync without spamming `sudo` probes.
pub fn spawn_indicators(bus: Bus) -> mpsc::Sender<IndicatorRefresh> {
    let (tx, rx) = mpsc::channel::<IndicatorRefresh>();
    let poke = tx.clone();
    thread::Builder::new()
        .name("indicators".into())
        .spawn(move || {
            let mut passwordless_sudo = false;
            let mut last_timer: Option<bool> = None;
            let mut next_full = true;
            loop {
                let timer_now = has_nopasswd_expiry_timer();
                let timer_flipped = timer_now.is_some() && timer_now != last_timer;
                if next_full || timer_flipped {
                    passwordless_sudo = probe_passwordless_sudo();
                    last_timer = timer_now;
                }
                let state = paths::state_home().join("omarchy");
                let stay_awake = state.join("indicators/stay-awake").exists();
                let nightlight = output("omarchy-toggle-nightlight", &["--status"])
                    .is_some_and(|s| s.replace(' ', "").contains("\"enabled\":true"));
                let dnd = output("omarchy-shell", &["notifications", "isDnd"])
                    .is_some_and(|s| s.trim() == "on");
                let recording = Command::new("omarchy-capture-screenrecording")
                    .arg("--status")
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .is_ok_and(|s| s.success());
                let (remote_session, remote_peers) = probe_remote_session();
                let (reminder_count, reminder_tooltip) = probe_reminders();
                let sudo = passwordless_sudo;
                bus.patch(move |s| {
                    s.indicators = Indicators {
                        passwordless_sudo: sudo,
                        recording,
                        remote_session,
                        remote_peers,
                        reminder_count,
                        reminder_tooltip,
                        nightlight,
                        dnd,
                        stay_awake,
                    };
                });

                next_full = false;
                match rx.recv_timeout(Duration::from_secs(30)) {
                    Ok(first) => {
                        next_full = first == IndicatorRefresh::Full;
                        thread::sleep(Duration::from_millis(80));
                        while let Ok(req) = rx.try_recv() {
                            if req == IndicatorRefresh::Full {
                                next_full = true;
                            }
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => return,
                }
            }
        })
        .ok();
    poke
}

/// Parses `/run/t1bridge/touch-id-state.json` (cosmetic Touch ID prompt).
pub fn read_touch_id() -> Option<TouchId> {
    let text = std::fs::read_to_string("/run/t1bridge/touch-id-state.json").ok()?;
    let value: serde_json::Value = serde_json::from_str(text.trim()).ok()?;
    let pid = value.get("pid")?.as_u64()?;
    if pid == 0 || !Path::new(&format!("/proc/{pid}")).exists() {
        return None;
    }
    let stage = match value.get("state")?.as_str()? {
        "enrollment" => TouchIdStage::Enrollment,
        "authenticate" => TouchIdStage::Authenticate,
        "approve" => TouchIdStage::Approve,
        "retry" => TouchIdStage::Retry,
        "success" => TouchIdStage::Success,
        _ => return None,
    };
    let progress = value
        .get("progress")
        .and_then(|p| p.as_u64())
        .map(|p| p.min(100) as u8);
    Some(TouchId { stage, progress })
}

pub fn spawn_updates(bus: Bus) {
    thread::Builder::new()
        .name("updates".into())
        .spawn(move || {
            thread::sleep(Duration::from_secs(90));
            loop {
                let available = Command::new("omarchy-update-available")
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .is_ok_and(|s| s.success());
                bus.patch(move |s| s.update_available = available);
                thread::sleep(Duration::from_secs(3 * 3600));
            }
        })
        .ok();
}
