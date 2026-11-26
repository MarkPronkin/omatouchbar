//! Workspaces, focused monitor, active window and keyboard layouts from
//! Hyprland's IPC sockets, refreshed on Hyprland events.

use std::{
    collections::HashSet,
    io::{BufRead, BufReader, Read, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    sync::mpsc::{self, RecvTimeoutError},
    thread,
    time::Duration,
};

use omatouchbar_core::{
    paths,
    state::{Keyboard, Workspace, Workspaces},
};
use serde_json::Value;

use crate::bus::{Bus, Event};

fn instance_dir() -> Option<PathBuf> {
    let base = paths::runtime_dir().join("hypr");
    if let Ok(signature) = std::env::var("HYPRLAND_INSTANCE_SIGNATURE") {
        let dir = base.join(signature);
        if dir.join(".socket2.sock").exists() {
            return Some(dir);
        }
    }
    // Fall back to the newest live instance (Hyprland restarted).
    let mut candidates: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(&base)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join(".socket2.sock").exists())
        .filter_map(|p| Some((p.metadata().ok()?.modified().ok()?, p)))
        .collect();
    candidates.sort();
    candidates.pop().map(|(_, p)| p)
}

fn request(dir: &PathBuf, command: &str) -> Option<Value> {
    let mut stream = UnixStream::connect(dir.join(".socket.sock")).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    stream.write_all(command.as_bytes()).ok()?;
    let mut text = String::new();
    stream.read_to_string(&mut text).ok()?;
    serde_json::from_str(&text).ok()
}

/// Whether Hyprland animations are on (Omarchy plays theme intros and
/// reveals only then).
pub fn animations_enabled() -> Option<bool> {
    request(&instance_dir()?, "j/getoption animations:enabled")?
        .get("bool")?
        .as_bool()
}

fn monitor_blocks() -> Vec<(String, i32)> {
    std::fs::read_to_string(paths::state_home().join("omarchy/monitor-workspaces"))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let block = fields.next()?.parse().ok()?;
            let name = fields.next()?.to_string();
            (!name.is_empty()).then_some((name, block))
        })
        .collect()
}

fn refresh(dir: &PathBuf, bus: &Bus) -> String {
    let workspaces = request(dir, "j/workspaces");
    let monitors = request(dir, "j/monitors");
    let window = request(dir, "j/activewindow");
    let devices = request(dir, "j/devices");

    let mut ws = Workspaces::default();
    if let Some(Value::Array(list)) = &workspaces {
        ws.list = list
            .iter()
            .filter_map(|w| {
                Some(Workspace {
                    id: w.get("id")?.as_i64()? as i32,
                    name: w
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    windows: w.get("windows").and_then(Value::as_u64).unwrap_or(0) as u32,
                    monitor: w
                        .get("monitor")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                })
            })
            .collect();
    }
    // The Touch Bar follows the focused monitor.
    if let Some(Value::Array(list)) = &monitors {
        let focused = list
            .iter()
            .find(|m| m.get("focused").and_then(Value::as_bool) == Some(true))
            .or(list.first());
        if let Some(m) = focused {
            ws.monitor = m
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            ws.active = m
                .get("activeWorkspace")
                .and_then(|a| a.get("id"))
                .and_then(Value::as_i64)
                .unwrap_or(1) as i32;
        }
    }
    ws.block = monitor_blocks()
        .into_iter()
        .find(|(name, _)| *name == ws.monitor)
        .map(|(_, b)| b)
        .unwrap_or_else(|| (ws.active - 1).max(0) / 10);

    let (title, class) = match &window {
        Some(w) => (
            w.get("title")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            w.get("class")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        ),
        None => Default::default(),
    };
    let keyboard = devices
        .as_ref()
        .and_then(|d| d.get("keyboards"))
        .and_then(Value::as_array)
        .and_then(|list| {
            let main = list
                .iter()
                .find(|k| k.get("main").and_then(Value::as_bool) == Some(true))
                .or(list.first())?;
            let layouts = main.get("layout").and_then(Value::as_str).unwrap_or("");
            Some(Keyboard {
                layout: main
                    .get("active_keymap")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                count: layouts
                    .split(',')
                    .filter(|s| !s.trim().is_empty())
                    .count()
                    .max(1),
            })
        });

    let active_class = class.clone();
    bus.patch(move |s| {
        s.workspaces = ws;
        s.window_title = title;
        s.window_class = class;
        s.keyboard = keyboard;
    });
    active_class
}

pub fn spawn(bus: Bus) -> mpsc::Sender<()> {
    let (tx, rx) = mpsc::channel::<()>();
    let poke = tx.clone();
    thread::Builder::new()
        .name("hyprland".into())
        .spawn(move || {
            loop {
                let Some(dir) = instance_dir() else {
                    thread::sleep(Duration::from_secs(2));
                    continue;
                };
                let mut last_class = refresh(&dir, &bus);
                let mut omarchy_terminals: HashSet<String> = HashSet::new();
                let mut saw_untracked_omarchy_terminal = last_class == "org.omarchy.terminal";
                let Ok(stream) = UnixStream::connect(dir.join(".socket2.sock")) else {
                    thread::sleep(Duration::from_secs(2));
                    continue;
                };
                // Read events on a helper thread; coalesce bursts here.
                let (events_tx, events_rx) = mpsc::channel::<String>();
                let reader = thread::spawn(move || {
                    for line in BufReader::new(stream).lines() {
                        let Ok(line) = line else { break };
                        if events_tx.send(line).is_err() {
                            break;
                        }
                    }
                });
                let mut dirty = false;
                loop {
                    match events_rx.recv_timeout(Duration::from_millis(if dirty {
                        12
                    } else {
                        500
                    })) {
                        Ok(line) => {
                            let (name, data) = line.split_once(">>").unwrap_or((line.as_str(), ""));
                            if name == "screencast" {
                                bus.send(Event::RefreshIndicators(false));
                            } else if name == "openwindow" {
                                let mut fields = data.splitn(4, ',');
                                if let (Some(addr), Some(_), Some(class)) =
                                    (fields.next(), fields.next(), fields.next())
                                    && class == "org.omarchy.terminal"
                                {
                                    omarchy_terminals.insert(addr.trim().to_string());
                                }
                            } else if name == "activewindow"
                                && data.starts_with("org.omarchy.terminal,")
                                && omarchy_terminals.is_empty()
                            {
                                saw_untracked_omarchy_terminal = true;
                            } else if name == "closewindow" {
                                let closed_tracked = omarchy_terminals.remove(data.trim());
                                let closed_untracked = omarchy_terminals.is_empty()
                                    && (saw_untracked_omarchy_terminal
                                        || last_class == "org.omarchy.terminal");
                                if closed_tracked || closed_untracked {
                                    // Presentation terminals (e.g. Passwordless Sudo)
                                    // run as org.omarchy.terminal; refresh once closed.
                                    saw_untracked_omarchy_terminal = false;
                                    bus.send(Event::RefreshIndicators(true));
                                }
                            }
                            if matches!(
                                name,
                                "workspace"
                                    | "workspacev2"
                                    | "focusedmon"
                                    | "focusedmonv2"
                                    | "activewindow"
                                    | "activewindowv2"
                                    | "createworkspace"
                                    | "createworkspacev2"
                                    | "destroyworkspace"
                                    | "destroyworkspacev2"
                                    | "moveworkspace"
                                    | "moveworkspacev2"
                                    | "openwindow"
                                    | "closewindow"
                                    | "movewindow"
                                    | "movewindowv2"
                                    | "activelayout"
                                    | "monitoradded"
                                    | "monitorremoved"
                                    | "renameworkspace"
                                    | "changeworkspaceid"
                                    | "windowtitle"
                                    | "windowtitlev2"
                                    | "configreloaded"
                            ) {
                                dirty = true;
                            }
                            if matches!(
                                name,
                                "workspace" | "workspacev2" | "activewindow" | "focusedmon"
                            ) {
                                bus.send(Event::Activity);
                            }
                        }
                        Err(RecvTimeoutError::Timeout) => {
                            while rx.try_recv().is_ok() {
                                dirty = true;
                            }
                            if dirty {
                                dirty = false;
                                last_class = refresh(&dir, &bus);
                            }
                        }
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                }
                let _ = reader.join();
                thread::sleep(Duration::from_millis(500));
            }
        })
        .ok();
    poke
}
