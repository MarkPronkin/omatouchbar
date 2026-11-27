//! Optional Trackpad Haptics integration. The renderer reports UI events;
//! the separate player owns the actuator and pulse patterns.
use std::{io, os::unix::net::UnixDatagram, path::PathBuf, time::Duration};

use omatouchbar_core::{
    paths,
    scene::LevelFeedback,
    state::{TouchId, TouchIdStage},
};

fn socket_path() -> PathBuf {
    paths::runtime_dir().join("trackpad-haptics.sock")
}

/// Runs on the control socket thread so querying an optional player cannot delay a gesture.
pub fn available() -> bool {
    fn capabilities() -> io::Result<serde_json::Value> {
        let socket = UnixDatagram::bind("")?;
        socket.set_write_timeout(Some(Duration::from_millis(100)))?;
        socket.set_read_timeout(Some(Duration::from_millis(300)))?;
        socket.connect(socket_path())?;
        socket.send(b"v1 capabilities")?;
        let mut buf = [0; 4096];
        let count = socket.recv(&mut buf)?;
        Ok(serde_json::from_slice(&buf[..count])?)
    }
    capabilities().is_ok_and(|r| {
        r["version"] == 1
            && r["result"]["playback"] == true
            && r["result"]["gestures"]
                .as_array()
                .is_some_and(|g| g.iter().any(|name| name == "press-center"))
    })
}

#[derive(Default)]
struct Fingerprint {
    stage: Option<TouchIdStage>,
    enrollment_progress: u8,
}

impl Fingerprint {
    fn update(&mut self, prompt: Option<&TouchId>) -> Option<&'static str> {
        use TouchIdStage::*;
        let Some(prompt) = prompt else {
            *self = Self::default();
            return None;
        };
        let previous = self.stage.replace(prompt.stage);
        let started = previous.is_none() || (previous == Some(Success) && prompt.stage != Success);
        if started {
            self.enrollment_progress = 0;
        }
        let progress = prompt.progress.unwrap_or(0);
        match prompt.stage {
            Success if previous != Some(Success) => Some("fp-success"),
            Retry if previous != Some(Retry) => Some("fp-retry"),
            Enrollment if progress > self.enrollment_progress => {
                self.enrollment_progress = progress;
                Some("fp-step")
            }
            Authenticate | Approve | Enrollment if started => Some("fp-ready"),
            _ => None,
        }
    }
}

pub struct Player {
    path: PathBuf,
    events: Option<UnixDatagram>,
    enabled: bool,
    fingerprint: Fingerprint,
}

impl Player {
    pub fn new(enabled: bool) -> Self {
        let events = UnixDatagram::unbound()
            .and_then(|s| {
                s.set_nonblocking(true)?;
                Ok(s)
            })
            .ok();
        Self {
            path: socket_path(),
            events,
            enabled,
            fingerprint: Fingerprint::default(),
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    fn event(&self, name: &str) {
        if self.enabled
            && let Some(socket) = &self.events
        {
            // A missing or busy optional player must never stall touch input.
            let _ = socket.send_to(format!("v1 gesture {name}").as_bytes(), &self.path);
        }
    }

    pub fn press(&self, position: f32) {
        self.event(if position < 1.0 / 3.0 {
            "press-left"
        } else if position < 2.0 / 3.0 {
            "press-center"
        } else {
            "press-right"
        });
    }

    pub fn adjust(&self, effect: LevelFeedback) {
        self.event(match effect {
            LevelFeedback::SliderOpened => "open",
            LevelFeedback::Tick => "tick",
            LevelFeedback::Minimum => "minimum",
            LevelFeedback::Maximum => "maximum",
        });
    }

    pub fn sync_touch_id(&mut self, prompt: Option<&TouchId>, connected: bool) {
        if let Some(event) = self.fingerprint.update(prompt)
            && connected
        {
            self.event(event);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fingerprint_transitions_follow_progress_once_and_rearm_on_new_prompts() {
        use TouchIdStage::*;
        let mut tracker = Fingerprint::default();
        for (stage, progress, expected) in [
            (Authenticate, None, Some("fp-ready")),
            (Authenticate, None, None),
            (Retry, None, Some("fp-retry")),
            (Retry, None, None),
            (Authenticate, None, None),
            (Success, None, Some("fp-success")),
            (Enrollment, Some(0), Some("fp-ready")),
            (Enrollment, Some(25), Some("fp-step")),
            (Retry, None, Some("fp-retry")),
            (Enrollment, Some(25), None),
            (Enrollment, Some(100), Some("fp-step")),
            (Success, None, Some("fp-success")),
            (Success, None, None),
        ] {
            assert_eq!(tracker.update(Some(&TouchId { stage, progress })), expected);
        }
        assert_eq!(tracker.update(None), None);
        assert_eq!(
            tracker.update(Some(&TouchId {
                stage: Approve,
                progress: None
            })),
            Some("fp-ready")
        );
    }
}
