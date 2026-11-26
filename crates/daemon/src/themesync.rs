//! Starts the Touch Bar's theme reveal when the desktop starts its own.
//!
//! `omarchy-theme-set` swaps the new theme directory in (which the watcher
//! sees), records the switch in `theme.generation`, hands the shell its
//! transition and only then repoints `current/background`. The shell starts
//! its wallpaper wipe as soon as it gets that call, so the link changing
//! marks the wipe's start to within a few milliseconds.
//!
//! A wallpaper with an intro video (when intros and Hyprland animations are
//! on) works differently: the desktop keeps the old picture up until OWE is
//! playing the video, then changes. The shell polls OWE's render status for
//! that moment, and so does this.

use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime},
};

use omatouchbar_core::{Palette, paths};
use serde_json::Value;

use crate::{
    bus::{Bus, Event},
    providers::hyprland,
};

/// How long the background link may take to follow the swap. An intro
/// first waits for OWE to finish preparing the clip.
const LINK_WAIT: Duration = Duration::from_millis(1500);
/// How long an intro may take to start playing.
const INTRO_WAIT: Duration = Duration::from_secs(4);
/// An intro OWE hasn't started by then isn't coming.
const INTRO_START: Duration = Duration::from_millis(1500);
/// A switch older than this means colors.toml was edited in place.
const SWITCH_AGE: Duration = Duration::from_secs(3);
const POLL: Duration = Duration::from_millis(8);

pub struct ThemeSync {
    bus: Bus,
    /// Bumped by every change, so an older wait gives way to a newer one.
    generation: Arc<AtomicU64>,
    /// The palette waiting for the desktop.
    pending: Option<Palette>,
}

impl ThemeSync {
    pub fn new(bus: Bus) -> ThemeSync {
        ThemeSync {
            bus,
            generation: Arc::new(AtomicU64::new(0)),
            pending: None,
        }
    }

    /// A new theme palette was written; `shown` is the one on the Touch Bar.
    /// [`Event::ThemeReady`] follows when the desktop starts showing it.
    pub fn changed(&mut self, palette: Palette, shown: &Palette) {
        if self.pending.as_ref() == Some(&palette) || (self.pending.is_none() && palette == *shown)
        {
            return;
        }
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.pending = Some(palette);
        let (bus, latest) = (self.bus.clone(), self.generation.clone());
        let spawned = thread::Builder::new()
            .name("theme-sync".into())
            .spawn(move || {
                let started = Instant::now();
                let superseded = || latest.load(Ordering::SeqCst) != generation;
                if let Some(cue) = wait_for_desktop(&superseded) {
                    bus.send(Event::ThemeReady {
                        generation,
                        cue,
                        waited: started.elapsed(),
                    });
                }
            });
        if spawned.is_err() {
            self.bus.send(Event::ThemeReady {
                generation,
                cue: "now",
                waited: Duration::ZERO,
            });
        }
    }

    /// The palette to show for `generation`, unless a newer change replaced it.
    pub fn ready(&mut self, generation: u64) -> Option<Palette> {
        (generation == self.generation.load(Ordering::SeqCst))
            .then(|| self.pending.take())
            .flatten()
    }
}

/// Waits until the desktop shows the new theme and returns what it waited
/// for, or `None` once a newer change takes over.
fn wait_for_desktop(superseded: &dyn Fn() -> bool) -> Option<&'static str> {
    let state = paths::state_home().join("omarchy");
    let current = state.join("current");
    let modified = |name: &str| std::fs::metadata(current.join(name)).ok()?.modified().ok();
    let Some(switched) = modified("theme.generation").or_else(|| modified("theme.name")) else {
        return Some("no theme switch");
    };
    let age = SystemTime::now()
        .duration_since(switched)
        .unwrap_or_default();
    if age > SWITCH_AGE {
        return Some("edited in place");
    }

    // The background link follows the shell's transition call.
    let link = current.join("background");
    let deadline = Instant::now() + LINK_WAIT.saturating_sub(age);
    loop {
        if superseded() {
            return None;
        }
        let relinked = std::fs::symlink_metadata(&link)
            .and_then(|m| m.modified())
            .is_ok_and(|t| t >= switched);
        if relinked {
            break;
        }
        if Instant::now() >= deadline {
            return Some("no new background");
        }
        thread::sleep(POLL);
    }
    if !intro_expected(&state, &link) {
        return Some("desktop wipe");
    }

    // Wait for the intro to play, as the shell does before it changes.
    let start = Instant::now();
    let mut started = false;
    loop {
        if superseded() {
            return None;
        }
        let Some(status) = owe("render-status") else {
            return Some("OWE unavailable");
        };
        let field = |key: &str| status.get(key);
        let playing = field("kind").and_then(Value::as_str) == Some("video")
            && field("ready").and_then(Value::as_bool) == Some(true)
            && field("has_transition").and_then(Value::as_bool) == Some(false)
            && field("time_pos").and_then(Value::as_f64).unwrap_or(0.0) > 0.0;
        if playing {
            return Some("intro playing");
        }
        match owe("intro-status").and_then(|s| s.get("running")?.as_bool()) {
            Some(true) => started = true,
            Some(false) if started => return Some("intro ended"),
            _ => {}
        }
        let waited = start.elapsed();
        if waited > INTRO_WAIT || (!started && waited > INTRO_START) {
            return Some("intro never played");
        }
        thread::sleep(POLL * 2);
    }
}

/// Whether the switch plays the new wallpaper's intro video, by the same
/// rules as `omarchy-theme-bg-boot-intro`.
fn intro_expected(state: &Path, link: &Path) -> bool {
    if state.join("toggles/background-intros-off").exists()
        || hyprland::animations_enabled() == Some(false)
    {
        return false;
    }
    let Ok(background) = std::fs::canonicalize(link) else {
        return false;
    };
    let still = background
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| {
            matches!(
                e.to_ascii_lowercase().as_str(),
                "jpg" | "jpeg" | "png" | "bmp" | "webp"
            )
        });
    let (Some(dir), Some(stem)) = (background.parent(), background.file_stem()) else {
        return false;
    };
    let stem = stem.to_string_lossy();
    still
        && ["mp4", "m4v", "mov", "webm", "mkv", "avi"]
            .iter()
            .any(|ext| dir.join("intros").join(format!("{stem}.{ext}")).is_file())
}

/// One request to the OWE wallpaper daemon.
fn owe(cmd: &str) -> Option<Value> {
    let mut stream = UnixStream::connect(paths::runtime_dir().join("owe/owed.sock")).ok()?;
    stream
        .set_read_timeout(Some(Duration::from_millis(250)))
        .ok()?;
    stream
        .set_write_timeout(Some(Duration::from_millis(250)))
        .ok()?;
    writeln!(stream, "{}", serde_json::json!({ "cmd": cmd })).ok()?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).ok()?;
    serde_json::from_str(&line).ok()
}
