//! Optional Trackpad Haptics integration. The renderer reports UI events;
//! the separate player owns the actuator, pulse patterns and preferences.
use std::{
    io,
    os::linux::net::SocketAddrExt,
    os::unix::net::{SocketAddr, UnixDatagram},
    path::PathBuf,
    sync::atomic::{AtomicU32, Ordering},
    time::Duration,
};

use omatouchbar_core::{
    paths,
    scene::LevelFeedback,
    state::{TouchId, TouchIdStage},
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Haptics {
    Off,
    Uniform,
    Medium,
    Zoned,
}

impl Haptics {
    pub fn from_command(word: &str) -> Option<Self> {
        match word {
            "off" => Some(Self::Off),
            "on" => Some(Self::Uniform),
            "medium" => Some(Self::Medium),
            "zoned" => Some(Self::Zoned),
            _ => None,
        }
    }

    pub fn command(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Uniform => "on",
            Self::Medium => "medium",
            Self::Zoned => "zoned",
        }
    }
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
    fingerprint: Fingerprint,
}

impl Player {
    pub fn new() -> Self {
        let events = UnixDatagram::unbound()
            .and_then(|s| {
                s.set_nonblocking(true)?;
                Ok(s)
            })
            .ok();
        Self {
            path: paths::runtime_dir().join("trackpad-haptics.sock"),
            events,
            fingerprint: Fingerprint::default(),
        }
    }

    fn event(&self, name: &str) {
        if let Some(socket) = &self.events {
            // A missing or busy optional player must never stall touch input.
            let _ = socket.send_to(name.as_bytes(), &self.path);
        }
    }

    fn request(&self, message: &str) -> io::Result<String> {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let name = format!(
            "omatouchbar-haptics-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let addr = SocketAddr::from_abstract_name(name.as_bytes())?;
        let client = UnixDatagram::bind_addr(&addr)?;
        client.set_read_timeout(Some(Duration::from_millis(500)))?;
        client.set_write_timeout(Some(Duration::from_millis(100)))?;
        client.connect(&self.path)?;
        client.send(message.as_bytes())?;
        let mut buf = [0; 512];
        let n = client.recv(&mut buf)?;
        Ok(String::from_utf8_lossy(&buf[..n]).into_owned())
    }

    pub fn mode(&self) -> Haptics {
        self.request("get-mode")
            .ok()
            .and_then(|s| Haptics::from_command(&s))
            .unwrap_or(Haptics::Off)
    }

    pub fn set_mode(&self, mode: Haptics) -> io::Result<()> {
        let reply = self.request(&format!("mode {}", mode.command()))?;
        if reply == "ok" {
            Ok(())
        } else {
            Err(io::Error::other(reply))
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
