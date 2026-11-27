//! Live desktop status shown on the Touch Bar.

use chrono::Utc;

pub use crate::agents::{AgentLimit, AgentUsage, format_reset_duration};
pub use crate::weather::Weather;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Workspace {
    pub id: i32,
    pub name: String,
    pub windows: u32,
    pub monitor: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Workspaces {
    pub list: Vec<Workspace>,
    pub active: i32,
    pub monitor: String,
    /// Omarchy's per-monitor workspace block (10 slots each).
    pub block: i32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Charge {
    #[default]
    Unknown,
    Charging,
    Discharging,
    Full,
    NotCharging,
    /// On AC, held below full by a charge limit (Omarchy's "Threshold").
    Holding,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Battery {
    pub percent: f32,
    pub charge: Charge,
    pub seconds_to_empty: u64,
    pub seconds_to_full: u64,
    /// Charge/discharge rate in watts.
    pub rate_watts: f32,
    /// Formatted details as Omarchy's battery panel shows them
    /// (`omarchy-battery-status --shell`).
    pub size: String,
    pub cycles: String,
    pub time: String,
    pub threshold: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Audio {
    pub volume: u8,
    pub muted: bool,
    pub sink: String,
}

/// What an audio device is, for its icon (as Omarchy's audio panel tells them apart).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AudioDeviceKind {
    #[default]
    Speaker,
    Headphones,
    Bluetooth,
    Display,
    Microphone,
    Camera,
}

/// An output (sink) or input (source) the user can make the default.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AudioDevice {
    /// PipeWire node id (the pactl index).
    pub id: u32,
    /// Node name, e.g. `alsa_output.pci-0000_00_1f.3.analog-stereo`.
    pub name: String,
    pub label: String,
    pub kind: AudioDeviceKind,
    pub default: bool,
}

/// An application playing audio (a PipeWire playback stream).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AudioStream {
    /// Sink-input index.
    pub id: u32,
    pub label: String,
    pub volume: u8,
    pub muted: bool,
}

/// Output and input devices and per-app streams, as Omarchy's audio panel lists them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mixer {
    pub outputs: Vec<AudioDevice>,
    pub inputs: Vec<AudioDevice>,
    pub streams: Vec<AudioStream>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NetKind {
    #[default]
    Unknown,
    Disconnected,
    Wifi,
    Ethernet,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Connectivity {
    #[default]
    Unknown,
    None,
    Portal,
    Limited,
    Full,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum DnsProvider {
    #[default]
    Dhcp,
    Cloudflare,
    Google,
    Custom,
}

impl DnsProvider {
    pub const ALL: [DnsProvider; 4] = [
        DnsProvider::Dhcp,
        DnsProvider::Cloudflare,
        DnsProvider::Google,
        DnsProvider::Custom,
    ];

    pub fn label(self) -> &'static str {
        match self {
            DnsProvider::Dhcp => "DHCP",
            DnsProvider::Cloudflare => "Cloudflare",
            DnsProvider::Google => "Google",
            DnsProvider::Custom => "Custom",
        }
    }

    pub fn cli_arg(self) -> &'static str {
        self.label()
    }

    pub fn parse(raw: &str) -> Option<DnsProvider> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "dhcp" => Some(DnsProvider::Dhcp),
            "cloudflare" => Some(DnsProvider::Cloudflare),
            "google" => Some(DnsProvider::Google),
            "custom" => Some(DnsProvider::Custom),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WifiNetwork {
    pub ssid: String,
    pub signal: u8,
    pub secure: bool,
    /// A saved NetworkManager profile exists.
    pub known: bool,
    pub active: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct NetworkDetails {
    /// Active default-route interface name (e.g. "wlan0"), empty when offline.
    pub iface: String,
    pub ip: String,
    pub prefix: String,
    pub gateway: String,
    /// Negotiated Ethernet link speed in Mbps, if available.
    pub speed_mbps: Option<u32>,
    /// Active Wi-Fi frequency in MHz, if available.
    pub freq_mhz: Option<u32>,
    /// Interface cumulative received bytes.
    pub rx_bytes: Option<u64>,
    /// Interface cumulative transmitted bytes.
    pub tx_bytes: Option<u64>,
    /// Live receiving throughput in bytes/sec.
    pub download_rate: f64,
    /// Live sending throughput in bytes/sec.
    pub upload_rate: f64,
    /// Whether at least one ping probe has returned since opening the panel.
    pub has_ping_samples: bool,
    /// Rolling average internet ping latency in ms (`None` if probes timed out).
    pub ping_ms: Option<f32>,
    /// Packet loss percentage (0–100) over the rolling 24-sample window.
    pub packet_loss: u8,
    /// Whether the active Wi-Fi connection can be shared as a QR code.
    pub can_share_qr: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Network {
    pub kind: NetKind,
    pub name: String,
    pub signal: u8,
    pub wifi_enabled: bool,
    /// Whether a Wi-Fi radio adapter is present.
    pub wifi_available: bool,
    pub connectivity: Connectivity,
    /// Active system DNS provider from `omarchy-dns`.
    pub dns: DnsProvider,
    /// Optimistically shown while a DNS change is applying.
    pub pending_dns: Option<DnsProvider>,
    /// Active Wi-Fi band ("2.4", "5", "6", or empty).
    pub band_current: String,
    /// Pinned Wi-Fi band ("auto", "2.4", "5", "6").
    pub band_selected: String,
    /// Available bands on the active SSID (e.g. ["2.4", "5"]).
    pub band_available: Vec<String>,
    /// Optimistically shown while a band change is reassociating.
    pub pending_band: Option<String>,
    /// Live connection metrics polled while the Network panel is open.
    pub details: NetworkDetails,
    /// Nearby networks, best first.
    pub networks: Vec<WifiNetwork>,
}

impl Network {
    pub fn has_captive_portal(&self) -> bool {
        self.connectivity == Connectivity::Portal
    }

    pub fn restricted(&self) -> bool {
        matches!(
            self.connectivity,
            Connectivity::Portal | Connectivity::Limited
        )
    }

    pub fn has_details(&self) -> bool {
        !self.details.iface.is_empty() || matches!(self.kind, NetKind::Wifi | NetKind::Ethernet)
    }

    pub fn has_transfer_stats(&self) -> bool {
        self.details.rx_bytes.is_some() || self.details.tx_bytes.is_some()
    }

    /// Ping and packet loss only mean something when the internet is reachable
    /// and at least one probe has returned (matches Omarchy's `internetPingReadable`).
    pub fn internet_ping_readable(&self) -> bool {
        if matches!(
            self.connectivity,
            Connectivity::Portal | Connectivity::Limited | Connectivity::None
        ) {
            return false;
        }
        self.details.has_ping_samples
    }

    pub fn effective_dns(&self) -> DnsProvider {
        self.pending_dns.unwrap_or(self.dns)
    }

    pub fn effective_band(&self) -> &str {
        if let Some(pending) = self.pending_band.as_deref()
            && !pending.is_empty()
        {
            return pending;
        }
        if self.band_selected.is_empty() {
            "auto"
        } else {
            &self.band_selected
        }
    }

    /// Mirrors Omarchy's `canSelectBand`: only on Wi-Fi (or mid-reconnect),
    /// when there is more than one band to pick or a pin is currently active.
    pub fn can_select_band(&self) -> bool {
        let pinned = self.effective_band() != "auto";
        (self.kind == NetKind::Wifi || self.pending_band.is_some())
            && (self.band_available.len() > 1 || pinned)
    }
}

/// Formats a byte count identically to Omarchy's network panel (`Model.js:formatBytes`).
pub fn format_bytes(bytes: u64) -> String {
    let n = bytes as f64;
    if n < 1024.0 {
        format!("{bytes} B")
    } else if n < 1024.0 * 1024.0 {
        format!("{:.1} KB", n / 1024.0)
    } else if n < 1024.0 * 1024.0 * 1024.0 {
        format!("{:.1} MB", n / (1024.0 * 1024.0))
    } else {
        format!("{:.2} GB", n / (1024.0 * 1024.0 * 1024.0))
    }
}

/// Formats a bytes/sec throughput rate identically to Omarchy's network panel.
pub fn format_rate(bytes_per_sec: f64) -> String {
    let n = if bytes_per_sec.is_finite() && bytes_per_sec >= 0.0 {
        bytes_per_sec
    } else {
        0.0
    };
    if n < 1024.0 {
        format!("{} B/s", n.round() as u64)
    } else if n < 1024.0 * 1024.0 {
        format!("{:.1} KB/s", n / 1024.0)
    } else if n < 1024.0 * 1024.0 * 1024.0 {
        format!("{:.1} MB/s", n / (1024.0 * 1024.0))
    } else {
        format!("{:.2} GB/s", n / (1024.0 * 1024.0 * 1024.0))
    }
}

/// Formats ping latency identically to Omarchy's `Model.js:formatPingLatency`.
pub fn format_ping_latency(ping_ms: Option<f32>, readable: bool) -> String {
    if !readable {
        return "--".into();
    }
    match ping_ms {
        Some(ms) if ms.is_finite() && ms >= 0.0 => {
            if ms > 0.0 && ms < 10.0 {
                format!("{ms:.1} ms")
            } else {
                format!("{ms:.0} ms")
            }
        }
        _ => "Timeout".into(),
    }
}

/// Formats packet loss percentage identically to Omarchy's `Model.js:formatPacketLoss`.
pub fn format_packet_loss(percent: u8, readable: bool) -> String {
    if !readable {
        "--".into()
    } else {
        format!("{percent}%")
    }
}

/// Formats Ethernet negotiated link speed (`Model.js:formatHeaderSpeed`).
pub fn format_eth_speed(mbps: u32) -> String {
    if mbps == 0 {
        return String::new();
    }
    if mbps >= 1000 {
        if mbps.is_multiple_of(1000) {
            format!("{}gbit", mbps / 1000)
        } else {
            format!("{:.1}gbit", mbps as f64 / 1000.0)
        }
    } else {
        format!("{mbps}mbit")
    }
}

/// Formats a Wi-Fi band token ("auto", "2.4", "5", "6") for display.
pub fn format_wifi_band(band: &str) -> String {
    match band {
        "" | "auto" => "Auto".into(),
        other => format!("{other}GHz"),
    }
}

/// What a Bluetooth device is, from BlueZ's `Icon` property.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BtKind {
    #[default]
    Other,
    Headphones,
    Speaker,
    Keyboard,
    Mouse,
    Gamepad,
    Phone,
    Computer,
    Display,
}

impl BtKind {
    /// Maps a freedesktop icon name (`audio-headset`, `input-mouse`, …).
    pub fn from_icon(icon: &str) -> BtKind {
        match icon {
            "audio-headset" | "audio-headphones" => BtKind::Headphones,
            "audio-card" | "multimedia-player" => BtKind::Speaker,
            "input-keyboard" => BtKind::Keyboard,
            "input-mouse" | "input-tablet" => BtKind::Mouse,
            "input-gaming" => BtKind::Gamepad,
            "phone" => BtKind::Phone,
            "computer" => BtKind::Computer,
            "video-display" => BtKind::Display,
            _ => BtKind::Other,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BtDevice {
    pub name: String,
    pub address: String,
    pub connected: bool,
    /// Remembered (paired, bonded or trusted), not just found by a scan.
    pub paired: bool,
    pub kind: BtKind,
    /// Charge the device reports (BlueZ `Battery1`).
    pub battery: Option<u8>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bluetooth {
    pub available: bool,
    pub powered: bool,
    /// An adapter is scanning for new devices.
    pub discovering: bool,
    pub connected: Vec<String>,
    /// Remembered devices, connected first, then by name.
    pub devices: Vec<BtDevice>,
    /// Unpaired devices a scan found, by name.
    pub nearby: Vec<BtDevice>,
}

impl Bluetooth {
    /// A remembered or nearby device.
    pub fn device(&self, address: &str) -> Option<&BtDevice> {
        self.devices
            .iter()
            .chain(&self.nearby)
            .find(|d| d.address == address)
    }
}

/// Something done to a Bluetooth device (`omarchy-bluetooth-device <op>`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BtOp {
    Pair,
    Connect,
    Disconnect,
    Forget,
}

impl BtOp {
    pub fn arg(self) -> &'static str {
        match self {
            BtOp::Pair => "pair",
            BtOp::Connect => "connect",
            BtOp::Disconnect => "disconnect",
            BtOp::Forget => "forget",
        }
    }

    /// How an operation that has returned went: the operation that
    /// failed (pairing can succeed and the connection after it fail) and
    /// whether the device ended up as asked.
    pub fn outcome(self, bt: &Bluetooth, address: &str) -> (BtOp, bool) {
        match self {
            BtOp::Pair => match bt.devices.iter().find(|d| d.address == address) {
                Some(d) if d.connected => (BtOp::Pair, true),
                Some(_) => (BtOp::Connect, false),
                None => (BtOp::Pair, false),
            },
            op => (op, op.done(bt, address)),
        }
    }

    /// Whether `bt` shows this operation finished for `address`.
    pub fn done(self, bt: &Bluetooth, address: &str) -> bool {
        let remembered = bt.devices.iter().find(|d| d.address == address);
        match self {
            BtOp::Pair | BtOp::Connect => remembered.is_some_and(|d| d.connected),
            BtOp::Disconnect => remembered.is_none_or(|d| !d.connected),
            BtOp::Forget => remembered.is_none(),
        }
    }
}

/// Whether a Bluetooth name is a real one rather than the address or a
/// UUID BlueZ falls back to (as Omarchy's panel filters them).
pub fn is_human_bt_name(name: &str) -> bool {
    let name = name.trim();
    if name.is_empty() {
        return false;
    }
    let hex = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_hexdigit());
    let address = name.len() == 17
        && name
            .split([':', '-'])
            .all(|part| part.len() == 2 && hex(part))
        && name.split([':', '-']).count() == 6;
    let uuid = {
        let parts: Vec<&str> = name.split('-').collect();
        let lens: Vec<usize> = parts.iter().map(|p| p.len()).collect();
        (lens == [8, 4, 4, 4, 12] && parts.iter().all(|p| hex(p)))
            || (name.len() == 32 && hex(name))
            || name
                .strip_prefix("0x")
                .or_else(|| name.strip_prefix("0X"))
                .is_some_and(|h| (4..=32).contains(&h.len()) && hex(h))
    };
    !address && !uuid
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Media {
    pub title: String,
    pub artist: String,
    pub player: String,
    pub playing: bool,
    pub can_next: bool,
    pub can_previous: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Keyboard {
    /// Active layout's full name, e.g. "English (US)".
    pub layout: String,
    pub count: usize,
}

/// Omarchy's manual status and quick-toggle indicators (`omarchy.indicators`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Indicator {
    PasswordlessSudo,
    ScreenRecording,
    RemoteSession,
    Reminder,
    NightLight,
    Dnd,
    StayAwake,
}

impl Indicator {
    /// Default order matching Omarchy's `Indicators.qml`.
    pub const ALL: [Indicator; 7] = [
        Indicator::PasswordlessSudo,
        Indicator::ScreenRecording,
        Indicator::RemoteSession,
        Indicator::Reminder,
        Indicator::NightLight,
        Indicator::Dnd,
        Indicator::StayAwake,
    ];

    /// Parses an Omarchy indicator ID from `shell.json` (`"PasswordlessSudo"`, etc.).
    pub fn from_omarchy_id(id: &str) -> Option<Indicator> {
        match id {
            "PasswordlessSudo" | "passwordless-sudo" => Some(Indicator::PasswordlessSudo),
            "ScreenRecording" | "screen-recording" => Some(Indicator::ScreenRecording),
            "RemoteSession" | "remote-session" => Some(Indicator::RemoteSession),
            "Reminder" | "reminder" => Some(Indicator::Reminder),
            "NightLight" | "night-light" | "nightlight" => Some(Indicator::NightLight),
            "Dnd" | "dnd" => Some(Indicator::Dnd),
            "StayAwake" | "stay-awake" => Some(Indicator::StayAwake),
            _ => None,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Indicator::PasswordlessSudo => "sudo",
            Indicator::ScreenRecording => "rec",
            Indicator::RemoteSession => "remote",
            Indicator::Reminder => "reminder",
            Indicator::NightLight => "night",
            Indicator::Dnd => "dnd",
            Indicator::StayAwake => "awake",
        }
    }

    /// Whether the indicator can be triggered while inactive. `RemoteSession`
    /// mirrors Omarchy's `revealWhenInactive: false` (warning only).
    pub fn reveal_when_inactive(self) -> bool {
        !matches!(self, Indicator::RemoteSession)
    }

    /// Whether an active indicator represents a security or capture warning.
    pub fn urgent(self) -> bool {
        matches!(
            self,
            Indicator::PasswordlessSudo | Indicator::ScreenRecording | Indicator::RemoteSession
        )
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Indicators {
    pub passwordless_sudo: bool,
    pub recording: bool,
    pub remote_session: bool,
    pub remote_peers: Vec<String>,
    pub reminder_count: u32,
    pub reminder_tooltip: String,
    pub nightlight: bool,
    pub dnd: bool,
    pub stay_awake: bool,
}

impl Indicators {
    pub fn is_active(&self, indicator: Indicator) -> bool {
        match indicator {
            Indicator::PasswordlessSudo => self.passwordless_sudo,
            Indicator::ScreenRecording => self.recording,
            Indicator::RemoteSession => self.remote_session,
            Indicator::Reminder => self.reminder_count > 0,
            Indicator::NightLight => self.nightlight,
            Indicator::Dnd => self.dnd,
            Indicator::StayAwake => self.stay_awake,
        }
    }

    pub fn any_active(&self, items: &[Indicator]) -> bool {
        items.iter().any(|&i| self.is_active(i))
    }

    pub fn any_urgent(&self, items: &[Indicator]) -> bool {
        items.iter().any(|&i| i.urgent() && self.is_active(i))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TouchIdStage {
    Enrollment,
    Authenticate,
    Approve,
    Retry,
    Success,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TouchId {
    pub stage: TouchIdStage,
    pub progress: Option<u8>,
}

/// A decoded icon shared by desktop state, scene nodes and animation frames.
#[derive(Clone, Debug, PartialEq)]
pub struct IconImage(pub(crate) std::sync::Arc<tiny_skia::Pixmap>);

impl IconImage {
    /// Converts a StatusNotifier network-order ARGB pixmap to premultiplied RGBA.
    pub fn from_argb(width: i32, height: i32, mut pixels: Vec<u8>) -> Option<Self> {
        if !(1..=512).contains(&width)
            || !(1..=512).contains(&height)
            || pixels.len() != width as usize * height as usize * 4
        {
            return None;
        }
        for px in pixels.chunks_exact_mut(4) {
            let alpha = px[0];
            px.rotate_left(1);
            for c in &mut px[..3] {
                *c = ((u16::from(*c) * u16::from(alpha) + 127) / 255) as u8;
            }
        }
        let size = tiny_skia::IntSize::from_wh(width as u32, height as u32)?;
        Some(Self(tiny_skia::Pixmap::from_vec(pixels, size)?.into()))
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrayItem {
    /// Watcher registration (bus name and object path), unique even when app IDs coincide.
    pub address: String,
    pub id: String,
    pub title: String,
    pub icon: Option<IconImage>,
    pub menu_path: Option<String>,
    pub menu_only: bool,
    pub attention: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TrayMenuEntry {
    pub id: i32,
    pub label: String,
    pub enabled: bool,
    pub submenu: bool,
    pub checked: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DesktopState {
    pub workspaces: Workspaces,
    pub window_title: String,
    pub window_class: String,
    pub keyboard: Option<Keyboard>,
    pub battery: Option<Battery>,
    pub power_profile: Option<String>,
    pub power_profiles: Vec<String>,
    pub audio: Option<Audio>,
    pub mic_muted: Option<bool>,
    /// Default input (microphone) volume.
    pub mic_volume: Option<u8>,
    pub mixer: Mixer,
    pub display_brightness: Option<u8>,
    pub display_panel: Option<crate::display::DisplayPanel>,
    pub keyboard_backlight: Option<u8>,
    pub network: Network,
    pub bluetooth: Bluetooth,
    pub media: Option<Media>,
    pub tray: Vec<TrayItem>,
    pub weather: Option<Weather>,
    pub agents: Vec<AgentUsage>,
    pub indicators: Indicators,
    pub update_available: bool,
    pub touch_id: Option<TouchId>,
}

impl DesktopState {
    /// Plausible data for previews and snapshots.
    pub fn sample() -> DesktopState {
        Self::sample_for_model("MacBookPro14,3")
    }

    /// The demo Display panel uses the built-in Retina screen of the host.
    /// Both chassis have the same Touch Bar pixel dimensions.
    pub fn sample_for_model(model: &str) -> DesktopState {
        let (width, height) = match model.trim() {
            "MacBookPro13,2" | "MacBookPro14,2" => (2560, 1600),
            _ => (2880, 1800),
        };
        let ws = |id: i32, windows: u32| Workspace {
            id,
            name: id.to_string(),
            windows,
            monitor: "eDP-1".into(),
        };
        DesktopState {
            workspaces: Workspaces {
                list: vec![ws(1, 2), ws(2, 1), ws(3, 3), ws(5, 1)],
                active: 3,
                monitor: "eDP-1".into(),
                block: 0,
            },
            window_title: "nvim ~/Projects/omatouchbar".into(),
            window_class: "foot".into(),
            keyboard: Some(Keyboard {
                layout: "English (US)".into(),
                count: 2,
            }),
            battery: Some(Battery {
                percent: 82.0,
                charge: Charge::Discharging,
                seconds_to_empty: 3 * 3600 + 25 * 60,
                seconds_to_full: 0,
                rate_watts: 11.4,
                size: "55Wh".into(),
                cycles: "408".into(),
                time: "3h 25m".into(),
                threshold: "75-80%".into(),
            }),
            power_profile: Some("balanced".into()),
            power_profiles: vec![
                "power-saver".into(),
                "balanced".into(),
                "performance".into(),
            ],
            audio: Some(Audio {
                volume: 46,
                muted: false,
                sink: "Built-in Speakers".into(),
            }),
            mic_muted: Some(false),
            mic_volume: Some(72),
            mixer: Mixer {
                outputs: vec![
                    AudioDevice {
                        id: 85,
                        name: "omarchy_speaker_tuning".into(),
                        label: "Built-in Speakers".into(),
                        kind: AudioDeviceKind::Speaker,
                        default: true,
                    },
                    AudioDevice {
                        id: 91,
                        name: "bluez_output.AC_80_0A_00_00_01.1".into(),
                        label: "AirPods Pro".into(),
                        kind: AudioDeviceKind::Headphones,
                        default: false,
                    },
                    AudioDevice {
                        id: 70,
                        name: "alsa_output.pci-0000_01_00.1.hdmi-stereo".into(),
                        label: "LG UltraFine".into(),
                        kind: AudioDeviceKind::Display,
                        default: false,
                    },
                ],
                inputs: vec![
                    AudioDevice {
                        id: 66,
                        name: "alsa_input.pci-0000_00_1f.3.analog-stereo".into(),
                        label: "Internal Microphone".into(),
                        kind: AudioDeviceKind::Microphone,
                        default: true,
                    },
                    AudioDevice {
                        id: 92,
                        name: "bluez_input.AC_80_0A_00_00_01.0".into(),
                        label: "AirPods Pro".into(),
                        kind: AudioDeviceKind::Headphones,
                        default: false,
                    },
                ],
                streams: vec![
                    AudioStream {
                        id: 120,
                        label: "Spotify".into(),
                        volume: 80,
                        muted: false,
                    },
                    AudioStream {
                        id: 133,
                        label: "Firefox".into(),
                        volume: 55,
                        muted: false,
                    },
                    AudioStream {
                        id: 141,
                        label: "Discord".into(),
                        volume: 100,
                        muted: true,
                    },
                ],
            },
            display_brightness: Some(70),
            display_panel: Some(crate::display::DisplayPanel {
                monitors: vec![
                    crate::display::Monitor {
                        name: "eDP-1".into(),
                        enabled: true,
                        focused: true,
                        width,
                        height,
                        make: "Apple Computer Inc".into(),
                        model: "Color LCD".into(),
                    },
                    crate::display::Monitor {
                        name: "DP-1".into(),
                        enabled: false,
                        focused: false,
                        width: 3840,
                        height: 2160,
                        make: "LG Electronics".into(),
                        model: "LG ULTRAFINE".into(),
                    },
                ],
                brightness: Some(70),
                scale: Some(2.0),
                text_size: 12,
            }),
            keyboard_backlight: Some(40),
            network: Network {
                kind: NetKind::Wifi,
                name: "Studio 5G".into(),
                signal: 78,
                wifi_enabled: true,
                wifi_available: true,
                connectivity: Connectivity::Full,
                dns: DnsProvider::Cloudflare,
                pending_dns: None,
                band_current: "5".into(),
                band_selected: "auto".into(),
                band_available: vec!["2.4".into(), "5".into()],
                pending_band: None,
                details: NetworkDetails {
                    iface: "wlan0".into(),
                    ip: "192.168.1.42".into(),
                    prefix: "24".into(),
                    gateway: "192.168.1.1".into(),
                    speed_mbps: None,
                    freq_mhz: Some(5180),
                    rx_bytes: Some(1_524_713_390),
                    tx_bytes: Some(193_462_272),
                    download_rate: 1_468_006.0,
                    upload_rate: 246_272.0,
                    has_ping_samples: true,
                    ping_ms: Some(12.0),
                    packet_loss: 0,
                    can_share_qr: true,
                },
                networks: vec![
                    WifiNetwork {
                        ssid: "Studio 5G".into(),
                        signal: 78,
                        secure: true,
                        known: true,
                        active: true,
                    },
                    WifiNetwork {
                        ssid: "Studio".into(),
                        signal: 64,
                        secure: true,
                        known: true,
                        active: false,
                    },
                    WifiNetwork {
                        ssid: "Café Libre".into(),
                        signal: 52,
                        secure: false,
                        known: false,
                        active: false,
                    },
                    WifiNetwork {
                        ssid: "NETGEAR-4F2A".into(),
                        signal: 31,
                        secure: true,
                        known: false,
                        active: false,
                    },
                    WifiNetwork {
                        ssid: "Guest Wi-Fi".into(),
                        signal: 28,
                        secure: false,
                        known: false,
                        active: false,
                    },
                    WifiNetwork {
                        ssid: "TP-Link_5G_9A".into(),
                        signal: 22,
                        secure: true,
                        known: false,
                        active: false,
                    },
                    WifiNetwork {
                        ssid: "NeighborNet".into(),
                        signal: 18,
                        secure: true,
                        known: false,
                        active: false,
                    },
                ],
            },
            bluetooth: Bluetooth {
                available: true,
                powered: true,
                discovering: true,
                connected: vec!["AirPods Pro".into()],
                devices: vec![
                    BtDevice {
                        name: "AirPods Pro".into(),
                        // Matches the sample's `bluez_output.AC_80_0A_00_00_01.1`.
                        address: "AC:80:0A:00:00:01".into(),
                        connected: true,
                        paired: true,
                        kind: BtKind::Headphones,
                        battery: Some(80),
                    },
                    BtDevice {
                        name: "MX Master 3S".into(),
                        address: "AA:BB:CC:DD:EE:02".into(),
                        connected: false,
                        paired: true,
                        kind: BtKind::Mouse,
                        battery: None,
                    },
                    BtDevice {
                        name: "Keychron K2".into(),
                        address: "AA:BB:CC:DD:EE:03".into(),
                        connected: false,
                        paired: true,
                        kind: BtKind::Keyboard,
                        battery: None,
                    },
                ],
                nearby: vec![
                    BtDevice {
                        name: "JBL Flip 6".into(),
                        address: "AA:BB:CC:DD:EE:04".into(),
                        kind: BtKind::Speaker,
                        ..BtDevice::default()
                    },
                    BtDevice {
                        name: "Pixel 8".into(),
                        address: "AA:BB:CC:DD:EE:05".into(),
                        kind: BtKind::Phone,
                        ..BtDevice::default()
                    },
                    BtDevice {
                        name: "WH-1000XM5".into(),
                        address: "AA:BB:CC:DD:EE:06".into(),
                        kind: BtKind::Headphones,
                        ..BtDevice::default()
                    },
                ],
            },
            media: Some(Media {
                title: "Midnight City".into(),
                artist: "M83".into(),
                player: "Spotify".into(),
                playing: true,
                can_next: true,
                can_previous: true,
            }),
            tray: vec![TrayItem {
                address: "org.example.Music/StatusNotifierItem".into(),
                id: "music".into(),
                title: "Music".into(),
                menu_path: Some("/Menu".into()),
                ..TrayItem::default()
            }],
            weather: Some(Weather {
                icon: crate::weather::icon(2, false).into(),
                place: crate::weather::Place {
                    name: "Copenhagen".into(),
                    auto: true,
                },
                detected: Some("Copenhagen".into()),
                report: Some(crate::weather::Report::sample(
                    chrono::Local::now().naive_local(),
                )),
                ..Weather::default()
            }),
            agents: crate::agents::sample(Utc::now()),
            indicators: Indicators {
                passwordless_sudo: false,
                recording: false,
                remote_session: false,
                remote_peers: Vec::new(),
                reminder_count: 0,
                reminder_tooltip: String::new(),
                dnd: false,
                nightlight: true,
                stay_awake: false,
            },
            update_available: true,
            touch_id: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bluetooth_names_skip_addresses_and_uuids() {
        assert!(is_human_bt_name("AirPods Pro"));
        assert!(is_human_bt_name("K2"));
        assert!(!is_human_bt_name(""));
        assert!(!is_human_bt_name("  "));
        assert!(!is_human_bt_name("AA-BB-CC-DD-EE-01"));
        assert!(!is_human_bt_name("aa:bb:cc:dd:ee:01"));
        assert!(!is_human_bt_name("0000110b-0000-1000-8000-00805f9b34fb"));
        assert!(!is_human_bt_name("0x180F"));
        assert!(!is_human_bt_name("0123456789abcdef0123456789ABCDEF"));
        // Hex-looking words that aren't addresses stay.
        assert!(is_human_bt_name("Cafe"));
        assert!(is_human_bt_name("BEEF-CAFE"));
    }

    #[test]
    fn bluetooth_ops_know_when_they_are_done() {
        let bt = DesktopState::sample().bluetooth;
        let airpods = "AC:80:0A:00:00:01";
        let mouse = "AA:BB:CC:DD:EE:02";
        let speaker = "AA:BB:CC:DD:EE:04";
        assert!(BtOp::Connect.done(&bt, airpods));
        assert!(!BtOp::Connect.done(&bt, mouse));
        assert!(!BtOp::Disconnect.done(&bt, airpods));
        assert!(BtOp::Disconnect.done(&bt, mouse));
        // A nearby device is paired once it shows up remembered and connected.
        assert!(!BtOp::Pair.done(&bt, speaker));
        assert!(!BtOp::Forget.done(&bt, mouse));
        assert!(BtOp::Forget.done(&bt, speaker));
        assert_eq!(bt.device(speaker).map(|d| d.kind), Some(BtKind::Speaker));
        // Paired but not connected: the connection is what failed.
        assert_eq!(BtOp::Pair.outcome(&bt, mouse), (BtOp::Connect, false));
        assert_eq!(BtOp::Pair.outcome(&bt, airpods), (BtOp::Pair, true));
        assert_eq!(BtOp::Pair.outcome(&bt, speaker), (BtOp::Pair, false));
        assert_eq!(BtOp::Forget.outcome(&bt, mouse), (BtOp::Forget, false));
        assert_eq!(BtKind::from_icon("input-keyboard"), BtKind::Keyboard);
        assert_eq!(BtKind::from_icon("something-else"), BtKind::Other);
    }

    #[test]
    fn formats_network_metrics_like_omarchy() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(193_462_272), "184.5 MB");
        assert_eq!(format_bytes(1_524_713_390), "1.42 GB");

        assert_eq!(format_rate(0.0), "0 B/s");
        assert_eq!(format_rate(f64::NAN), "0 B/s");
        assert_eq!(format_rate(246_272.0), "240.5 KB/s");
        assert_eq!(format_rate(1_468_006.0), "1.4 MB/s");

        assert_eq!(format_ping_latency(Some(12.4), false), "--");
        assert_eq!(format_ping_latency(None, true), "Timeout");
        assert_eq!(format_ping_latency(Some(4.25), true), "4.2 ms");
        assert_eq!(format_ping_latency(Some(12.6), true), "13 ms");

        assert_eq!(format_packet_loss(0, false), "--");
        assert_eq!(format_packet_loss(0, true), "0%");
        assert_eq!(format_packet_loss(17, true), "17%");

        assert_eq!(format_eth_speed(0), "");
        assert_eq!(format_eth_speed(100), "100mbit");
        assert_eq!(format_eth_speed(1000), "1gbit");
        assert_eq!(format_eth_speed(2500), "2.5gbit");

        assert_eq!(format_wifi_band("auto"), "Auto");
        assert_eq!(format_wifi_band("2.4"), "2.4GHz");
        assert_eq!(format_wifi_band("5"), "5GHz");

        assert_eq!(
            DnsProvider::parse("Cloudflare"),
            Some(DnsProvider::Cloudflare)
        );
        assert_eq!(DnsProvider::parse(" dhcp "), Some(DnsProvider::Dhcp));
        assert_eq!(DnsProvider::parse("google"), Some(DnsProvider::Google));
        assert_eq!(DnsProvider::parse("Custom"), Some(DnsProvider::Custom));
        assert_eq!(DnsProvider::parse("invalid"), None);

        let mut net = DesktopState::sample().network;
        assert!(net.internet_ping_readable());
        assert!(net.can_select_band());
        net.connectivity = Connectivity::Portal;
        assert!(!net.internet_ping_readable());
        assert!(net.has_captive_portal());
        assert!(net.restricted());
    }
}
