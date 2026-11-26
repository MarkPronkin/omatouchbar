//! Events from worker threads to the render loop, with an eventfd wakeup.

use std::{
    os::fd::{AsFd, BorrowedFd, OwnedFd},
    sync::{
        Arc,
        mpsc::{self, Receiver, Sender},
    },
};

use omatouchbar_core::{Command, DesktopState, KeyInput, SpeedTestEvent, scene::GuardKind};
use rustix::event::EventfdFlags;

pub type Patch = Box<dyn FnOnce(&mut DesktopState) + Send>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Watched {
    Config,
    Theme,
    ShellJson,
    Menu,
    TouchId,
    Indicators,
    MonitorBlocks,
    AgentUsage,
    WeatherLocation,
}

pub enum Event {
    /// Update part of the desktop state.
    Patch(Patch),
    Files(Watched),
    TrayMenu {
        menu: omatouchbar_core::scene::TrayMenu,
        entries: Vec<omatouchbar_core::state::TrayMenuEntry>,
    },
    Control(Command, mpsc::Sender<String>),
    Status(mpsc::Sender<String>),
    /// Save the current frame as a PNG (debugging and the settings app).
    Snapshot(String, mpsc::Sender<String>),
    Reload,
    Debug(bool),
    Guards {
        menu: String,
        results: Vec<(String, GuardKind, bool)>,
    },
    ProviderRows {
        menu: String,
        provider: String,
        rows: Vec<(String, String, bool)>,
    },
    /// Desktop activity that should keep the strip awake.
    Activity,
    WorldClock(omatouchbar_core::world_clock::Frame),
    /// Physical keystroke captured while the Touch Bar password prompt is active.
    KeyInput(KeyInput),
    /// Result of an async Wi-Fi connection attempt.
    WifiConnectResult {
        ssid: String,
        result: Result<(), String>,
        auth_failed: bool,
    },
    /// Live telemetry sample or phase change from the speed test runner.
    SpeedTest(SpeedTestEvent),
    /// Refresh Omarchy indicators (`true` also probes passwordless sudo).
    RefreshIndicators(bool),
    /// A Bluetooth device operation returned (see `BtOp::outcome`).
    BluetoothDone {
        address: String,
        op: omatouchbar_core::state::BtOp,
        ok: bool,
    },
    /// The desktop started showing a new theme (see `themesync`).
    ThemeReady {
        generation: u64,
        /// What the wait ended on, for the log.
        cue: &'static str,
        waited: std::time::Duration,
    },
    /// Places matching the weather location search (`None`: lookup failed).
    WeatherSuggestions {
        query: String,
        results: Option<Vec<omatouchbar_core::weather::Location>>,
    },
}

#[derive(Clone)]
pub struct Bus {
    tx: Sender<Event>,
    wake: Arc<OwnedFd>,
}

impl Bus {
    pub fn new() -> (Bus, Receiver<Event>) {
        let (tx, rx) = mpsc::channel();
        let wake = rustix::event::eventfd(0, EventfdFlags::CLOEXEC | EventfdFlags::NONBLOCK)
            .expect("eventfd");
        (
            Bus {
                tx,
                wake: Arc::new(wake),
            },
            rx,
        )
    }

    pub fn send(&self, event: Event) {
        if self.tx.send(event).is_ok() {
            let _ = rustix::io::write(&*self.wake, &1u64.to_ne_bytes());
        }
    }

    pub fn patch(&self, f: impl FnOnce(&mut DesktopState) + Send + 'static) {
        self.send(Event::Patch(Box::new(f)));
    }

    pub fn wake_fd(&self) -> BorrowedFd<'_> {
        self.wake.as_fd()
    }

    pub fn drain(&self) {
        let mut buffer = [0u8; 8];
        let _ = rustix::io::read(&*self.wake, &mut buffer);
    }
}
