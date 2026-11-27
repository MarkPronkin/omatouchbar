//! Watches settings, theme, bar layout, menu and state files with inotify.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

use inotify::{Inotify, WatchDescriptor, WatchMask};
use omatouchbar_core::{Config, paths};

use crate::bus::{Bus, Event, Watched};

struct Entry {
    dir: PathBuf,
    /// File names in `dir` mapped to what they affect (empty: any file).
    files: Vec<(&'static str, Watched)>,
    any: Option<Watched>,
}

fn entries() -> Vec<Entry> {
    let state = paths::state_home().join("omarchy");
    let config_dir = Config::path()
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    vec![
        Entry {
            dir: config_dir,
            files: vec![("config.toml", Watched::Config)],
            any: None,
        },
        Entry {
            dir: state.join("current"),
            files: vec![
                ("theme.name", Watched::Theme),
                ("theme.generation", Watched::Theme),
                ("theme", Watched::Theme),
            ],
            any: None,
        },
        Entry {
            dir: state.join("current/theme"),
            files: vec![("colors.toml", Watched::Theme)],
            any: None,
        },
        Entry {
            dir: paths::config_home().join("omarchy"),
            files: vec![("shell.json", Watched::ShellJson)],
            any: None,
        },
        Entry {
            dir: paths::config_home().join("omarchy/extensions"),
            files: vec![("omarchy-menu.jsonc", Watched::Menu)],
            any: None,
        },
        Entry {
            dir: PathBuf::from("/run/t1bridge"),
            files: vec![("touch-id-state.json", Watched::TouchId)],
            any: None,
        },
        Entry {
            dir: state.join("indicators"),
            files: vec![],
            any: Some(Watched::Indicators),
        },
        Entry {
            dir: state.join("toggles"),
            files: vec![],
            any: Some(Watched::Indicators),
        },
        Entry {
            dir: paths::runtime_dir(),
            files: vec![("omarchy-screenrecord-pid", Watched::Indicators)],
            any: None,
        },
        Entry {
            dir: paths::runtime_dir().join("omarchy-reminders"),
            files: vec![],
            any: Some(Watched::Indicators),
        },
        Entry {
            dir: state.join("agents/usage"),
            files: vec![],
            any: Some(Watched::AgentUsage),
        },
        Entry {
            dir: state.join("agents"),
            files: vec![("order.json", Watched::AgentUsage)],
            any: None,
        },
        Entry {
            dir: state.join("settings"),
            files: vec![("weather.json", Watched::WeatherLocation)],
            any: None,
        },
        Entry {
            dir: state.clone(),
            files: vec![("monitor-workspaces", Watched::MonitorBlocks)],
            any: None,
        },
    ]
}

/// How long a file must stay quiet before its change is reported.
fn debounce(kind: Watched) -> Duration {
    match kind {
        // T1Bridge atomically publishes a complete record for each scan result.
        Watched::TouchId => Duration::ZERO,
        // A theme switch swaps the whole directory in at once, and the Touch
        // Bar's reveal has to start with the desktop's (see `themesync`).
        Watched::Theme => Duration::from_millis(15),
        _ => Duration::from_millis(120),
    }
}

pub fn spawn(bus: Bus) {
    thread::Builder::new()
        .name("watch".into())
        .spawn(move || {
            if let Err(e) = run(bus) {
                log::warn!("file watching stopped: {e}");
            }
        })
        .ok();
}

fn run(bus: Bus) -> std::io::Result<()> {
    let mut inotify = Inotify::init()?;
    let entries = entries();
    let mask = WatchMask::CLOSE_WRITE
        | WatchMask::MOVED_TO
        | WatchMask::CREATE
        | WatchMask::DELETE
        | WatchMask::MOVED_FROM;
    let mut watches: HashMap<WatchDescriptor, usize> = HashMap::new();
    let mut missing: Vec<usize> = Vec::new();
    for (i, entry) in entries.iter().enumerate() {
        match inotify.watches().add(&entry.dir, mask) {
            Ok(wd) => {
                watches.insert(wd, i);
            }
            Err(_) => missing.push(i),
        }
    }
    let mut buffer = [0u8; 8192];
    let mut pending: HashMap<Watched, Instant> = HashMap::new();
    let mut last_rescan = Instant::now();
    loop {
        // Sleep until something happens, waking only to debounce or to
        // retry watches for directories that did not exist yet.
        let now = Instant::now();
        let timeout = if let Some(due) = pending
            .iter()
            .map(|(kind, t)| (*t + debounce(*kind)).saturating_duration_since(now))
            .min()
        {
            Some(due.max(Duration::from_millis(2)))
        } else if !missing.is_empty() {
            Some(Duration::from_secs(3))
        } else {
            None
        };
        {
            use std::os::fd::AsFd;
            let fd = inotify.as_fd();
            let mut fds = [rustix::event::PollFd::new(
                &fd,
                rustix::event::PollFlags::IN,
            )];
            let ts = timeout.map(|t| rustix::time::Timespec {
                tv_sec: t.as_secs() as _,
                tv_nsec: t.subsec_nanos() as _,
            });
            let _ = rustix::event::poll(&mut fds, ts.as_ref());
        }
        match inotify.read_events(&mut buffer) {
            Ok(events) => {
                for event in events {
                    let Some(&index) = watches.get(&event.wd) else {
                        continue;
                    };
                    let entry = &entries[index];
                    if event.mask.contains(inotify::EventMask::IGNORED) {
                        watches.remove(&event.wd);
                        missing.push(index);
                        if let Some(kind) = entry.any {
                            pending.insert(kind, Instant::now());
                        }
                        continue;
                    }
                    let name = event.name.and_then(|n| n.to_str()).unwrap_or("");
                    if let Some(kind) = entry.any {
                        pending.insert(kind, Instant::now());
                    }
                    for (file, kind) in &entry.files {
                        if *file == name {
                            pending.insert(*kind, Instant::now());
                        }
                    }
                    // A replaced theme directory needs a fresh watch.
                    if name == "theme" {
                        missing.push(2);
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(e),
        }
        if !missing.is_empty() && last_rescan.elapsed() > Duration::from_millis(500) {
            last_rescan = Instant::now();
            missing.sort_unstable();
            missing.dedup();
            missing.retain(|&i| match inotify.watches().add(&entries[i].dir, mask) {
                Ok(wd) => {
                    watches.insert(wd, i);
                    // Files may already have arrived before this directory existed at startup.
                    if let Some(kind) = entries[i].any {
                        pending.insert(kind, Instant::now());
                    }
                    for (_, kind) in &entries[i].files {
                        if matches!(
                            kind,
                            Watched::Config | Watched::WeatherLocation | Watched::TouchId
                        ) {
                            pending.insert(*kind, Instant::now());
                        }
                    }
                    false
                }
                Err(_) => true,
            });
        }
        let now = Instant::now();
        let due: Vec<Watched> = pending
            .iter()
            .filter(|(kind, t)| now.duration_since(**t) >= debounce(**kind))
            .map(|(k, _)| *k)
            .collect();
        for kind in due {
            pending.remove(&kind);
            bus.send(Event::Files(kind));
        }
    }
}
