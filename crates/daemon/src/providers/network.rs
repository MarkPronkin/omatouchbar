//! Live network details (ping, packet loss, throughput, totals, IP, gateway),
//! Wi-Fi band selection, DNS provider switching and connectivity checks —
//! mirroring Omarchy's `shell/plugins/panels/network` panel.
//!
//! Polling runs ONLY while the Network panel is open (and pauses during active
//! speed tests or Wi-Fi password entry), keeping idle CPU and radio usage zero.

use std::{
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use omatouchbar_core::{
    Scene, paths,
    state::{Connectivity, DnsProvider, NetworkDetails},
};
use zbus::{blocking::Connection, proxy::CacheProperties};

use crate::bus::Bus;

const DETAILS_INTERVAL: Duration = Duration::from_millis(1500);
const BAND_DNS_INTERVAL: Duration = Duration::from_millis(4000);
const PING_HISTORY_WINDOW: usize = 24;
const PING_AVERAGE_WINDOW: usize = 5;

const NM_DEST: &str = "org.freedesktop.NetworkManager";
const NM_PATH: &str = "/org/freedesktop/NetworkManager";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BandStatus {
    pub band: String,
    pub selected: String,
    pub available: Vec<String>,
}

/// Decodes `\xNN` UTF-8 hex escapes emitted by `iw` for non-ASCII SSIDs
/// (mirrors Omarchy's `Model.js:decodeIwSsid`).
pub fn decode_iw_ssid(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\'
            && i + 3 < bytes.len()
            && bytes[i + 1] == b'x'
            && let Some(hi) = hex_val(bytes[i + 2])
            && let Some(lo) = hex_val(bytes[i + 3])
        {
            let byte = (hi << 4) | lo;
            if byte < 0x20 || byte == 0x7f {
                out.extend_from_slice(&bytes[i..i + 4]);
            } else {
                out.push(byte);
            }
            i += 4;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| raw.to_string())
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Parses tab-separated `key\tvalue` output from Omarchy's network scripts.
pub fn parse_key_value(raw: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in raw.lines() {
        if line.is_empty() {
            continue;
        }
        let Some((key, value)) = line.split_once('\t') else {
            continue;
        };
        let decoded = if key == "ssid" {
            decode_iw_ssid(value)
        } else {
            value.trim().to_string()
        };
        map.insert(key.to_string(), decoded);
    }
    map
}

/// Parses output of `omarchy-network-band`.
pub fn parse_band_status(raw: &str) -> BandStatus {
    let kv = parse_key_value(raw);
    let band = kv.get("band").cloned().unwrap_or_default();
    let selected = kv
        .get("selected")
        .filter(|s| !s.is_empty())
        .cloned()
        .unwrap_or_else(|| "auto".into());
    let available = kv
        .get("available")
        .map(|s| s.split_whitespace().map(str::to_string).collect())
        .unwrap_or_default();
    BandStatus {
        band,
        selected,
        available,
    }
}

/// Stateful throughput-delta and rolling ping/packet-loss calculator
/// (mirrors Omarchy's `Model.js:throughputState` and `pingLatencyState`).
#[derive(Debug, Default)]
pub struct TelemetryTracker {
    prev_iface: String,
    prev_rx: u64,
    prev_tx: u64,
    prev_time: Option<Instant>,
    download_rate: f64,
    upload_rate: f64,
    ping_iface: String,
    internet_samples: VecDeque<Option<f32>>,
}

impl TelemetryTracker {
    pub fn reset(&mut self) {
        self.prev_iface.clear();
        self.prev_rx = 0;
        self.prev_tx = 0;
        self.prev_time = None;
        self.download_rate = 0.0;
        self.upload_rate = 0.0;
        self.ping_iface.clear();
        self.internet_samples.clear();
    }

    pub fn ingest(&mut self, kv: &HashMap<String, String>, now: Instant) -> NetworkDetails {
        let iface = kv.get("iface").cloned().unwrap_or_default();
        let ip = kv.get("ip").cloned().unwrap_or_default();
        let prefix = kv.get("prefix").cloned().unwrap_or_default();
        let gateway = kv.get("gateway").cloned().unwrap_or_default();
        let speed_mbps = kv
            .get("speed")
            .and_then(|s| s.parse::<i64>().ok())
            .filter(|&v| v > 0)
            .map(|v| v as u32);
        let freq_mhz = kv
            .get("freq")
            .and_then(|s| s.parse::<f64>().ok())
            .filter(|&v| v.is_finite() && v > 0.0)
            .map(|v| v.round() as u32);
        let rx_bytes = kv.get("rx_bytes").and_then(|s| s.parse::<u64>().ok());
        let tx_bytes = kv.get("tx_bytes").and_then(|s| s.parse::<u64>().ok());

        // Throughput delta computation.
        let rx = rx_bytes.unwrap_or(0);
        let tx = tx_bytes.unwrap_or(0);
        if iface.is_empty() {
            self.prev_iface.clear();
            self.prev_rx = 0;
            self.prev_tx = 0;
            self.prev_time = None;
            self.download_rate = 0.0;
            self.upload_rate = 0.0;
        } else if iface != self.prev_iface || self.prev_time.is_none() {
            self.prev_iface = iface.clone();
            self.prev_rx = rx;
            self.prev_tx = tx;
            self.prev_time = Some(now);
            self.download_rate = 0.0;
            self.upload_rate = 0.0;
        } else if let Some(prev_t) = self.prev_time {
            let dt = now.saturating_duration_since(prev_t).as_secs_f64();
            if dt > 0.0 {
                self.download_rate = rx.saturating_sub(self.prev_rx) as f64 / dt;
                self.upload_rate = tx.saturating_sub(self.prev_tx) as f64 / dt;
            }
            self.prev_rx = rx;
            self.prev_tx = tx;
            self.prev_time = Some(now);
        }

        // Rolling internet ping & packet loss window.
        if iface.is_empty() || iface != self.ping_iface {
            self.ping_iface = iface.clone();
            self.internet_samples.clear();
        }
        if let Some(raw_ping) = kv.get("internet_ping_ms") {
            let sample = raw_ping
                .trim()
                .parse::<f32>()
                .ok()
                .filter(|v| v.is_finite() && *v >= 0.0);
            self.internet_samples.push_back(sample);
            while self.internet_samples.len() > PING_HISTORY_WINDOW {
                self.internet_samples.pop_front();
            }
        } else {
            self.internet_samples.clear();
        }

        let has_ping_samples = !self.internet_samples.is_empty();
        let ping_ms = average_ping(&self.internet_samples, PING_AVERAGE_WINDOW);
        let packet_loss = packet_loss_percent(&self.internet_samples);

        NetworkDetails {
            iface,
            ip,
            prefix,
            gateway,
            speed_mbps,
            freq_mhz,
            rx_bytes,
            tx_bytes,
            download_rate: self.download_rate,
            upload_rate: self.upload_rate,
            has_ping_samples,
            ping_ms,
            packet_loss,
            can_share_qr: false,
        }
    }
}

fn average_ping(samples: &VecDeque<Option<f32>>, window: usize) -> Option<f32> {
    let start = samples.len().saturating_sub(window.max(1));
    let mut sum = 0.0f64;
    let mut count = 0u32;
    for sample in samples.iter().skip(start) {
        if let Some(ms) = sample
            && ms.is_finite()
            && *ms >= 0.0
        {
            sum += f64::from(*ms);
            count += 1;
        }
    }
    (count > 0).then(|| (sum / f64::from(count)) as f32)
}

fn packet_loss_percent(samples: &VecDeque<Option<f32>>) -> u8 {
    if samples.is_empty() {
        return 0;
    }
    let lost = samples.iter().filter(|s| s.is_none()).count();
    ((lost as f64 / samples.len() as f64) * 100.0)
        .round()
        .clamp(0.0, 100.0) as u8
}

/// Reads NetworkManager connectivity state via system D-Bus (zero process spawn),
/// respecting `ConnectivityCheckEnabled` just like Omarchy's `Model.js`.
pub fn read_connectivity() -> Connectivity {
    if let Some(conn) = read_connectivity_dbus() {
        return conn;
    }
    let Some(out) = Command::new("nmcli")
        .args(["-t", "networking", "connectivity"])
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
    else {
        return Connectivity::Unknown;
    };
    match String::from_utf8_lossy(&out.stdout).trim() {
        "none" => Connectivity::None,
        "portal" => Connectivity::Portal,
        "limited" => Connectivity::Limited,
        "full" => Connectivity::Full,
        _ => Connectivity::Unknown,
    }
}

fn read_connectivity_dbus() -> Option<Connectivity> {
    let conn = Connection::system().ok()?;
    let proxy = zbus::blocking::proxy::Builder::<zbus::blocking::Proxy>::new(&conn)
        .destination(NM_DEST)
        .ok()?
        .path(NM_PATH)
        .ok()?
        .interface(NM_DEST)
        .ok()?
        .cache_properties(CacheProperties::No)
        .build()
        .ok()?;
    let enabled: bool = proxy.get_property("ConnectivityCheckEnabled").ok()?;
    if !enabled {
        return Some(Connectivity::Unknown);
    }
    let state: u32 = proxy.get_property("Connectivity").ok()?;
    Some(match state {
        1 => Connectivity::None,
        2 => Connectivity::Portal,
        3 => Connectivity::Limited,
        4 => Connectivity::Full,
        _ => Connectivity::Unknown,
    })
}

fn trigger_connectivity_check() {
    let Ok(conn) = Connection::system() else {
        return;
    };
    let Ok(proxy) = zbus::blocking::proxy::Builder::<zbus::blocking::Proxy>::new(&conn)
        .destination(NM_DEST)
        .and_then(|b| b.path(NM_PATH))
        .and_then(|b| b.interface(NM_DEST))
        .and_then(|b| b.cache_properties(CacheProperties::No).build())
    else {
        return;
    };
    if proxy
        .get_property::<bool>("ConnectivityCheckEnabled")
        .unwrap_or(false)
    {
        let _: Result<u32, _> = proxy.call("CheckConnectivity", &());
    }
}

fn omarchy_bin(name: &str) -> PathBuf {
    let root_bin = paths::omarchy_root().join("bin").join(name);
    if root_bin.exists() {
        return root_bin;
    }
    let usr_bin = Path::new("/usr/bin").join(name);
    if usr_bin.exists() {
        return usr_bin;
    }
    PathBuf::from(name)
}

fn omarchy_output(name: &str, args: &[&str]) -> Option<String> {
    let bin = omarchy_bin(name);
    let out = Command::new(&bin)
        .args(args)
        .env("LC_ALL", "C")
        .env("OMARCHY_PATH", paths::omarchy_root())
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).to_string())
}

/// Runs an Omarchy helper outside T1Bridge's `NoNewPrivileges` sandbox via
/// `systemd-run --user --wait`, allowing `sudo` / `pkexec` to succeed.
fn run_outside_sandbox(program: &Path, args: &[&str]) -> bool {
    let prog_str = program.to_string_lossy();
    let mut cmd = Command::new("systemd-run");
    cmd.args(["--user", "--wait", "--collect", "--quiet", "--"])
        .arg(prog_str.as_ref())
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    match cmd.status() {
        Ok(status) => status.success(),
        Err(_) => Command::new(program)
            .args(args)
            .env("OMARCHY_PATH", paths::omarchy_root())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success()),
    }
}

fn read_dns() -> Option<DnsProvider> {
    let raw = omarchy_output("omarchy-dns", &[])?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        Some(DnsProvider::Dhcp)
    } else {
        DnsProvider::parse(trimmed)
    }
}

enum Request {
    SetActive(bool),
    RefreshNow,
}

pub struct DetailsProvider {
    tx: mpsc::Sender<Request>,
    active: bool,
    bus: Bus,
    dns_in_flight: Arc<AtomicUsize>,
    band_in_flight: Arc<AtomicUsize>,
}

impl DetailsProvider {
    pub fn start(bus: Bus) -> Self {
        let (tx, rx) = mpsc::channel::<Request>();
        let dns_in_flight = Arc::new(AtomicUsize::new(0));
        let band_in_flight = Arc::new(AtomicUsize::new(0));
        let worker_bus = bus.clone();
        let worker_dns = dns_in_flight.clone();
        let worker_band = band_in_flight.clone();

        thread::Builder::new()
            .name("net-details".into())
            .spawn(move || {
                let mut active = false;
                let mut tracker = TelemetryTracker::default();
                let mut last_slow_poll: Option<Instant> = None;

                loop {
                    let msg = if active {
                        match rx.recv_timeout(DETAILS_INTERVAL) {
                            Ok(r) => Some(r),
                            Err(mpsc::RecvTimeoutError::Timeout) => None,
                            Err(mpsc::RecvTimeoutError::Disconnected) => return,
                        }
                    } else {
                        match rx.recv() {
                            Ok(r) => Some(r),
                            Err(_) => return,
                        }
                    };

                    let mut force_slow = false;
                    let was_active = active;
                    for req in msg.into_iter().chain(rx.try_iter()) {
                        match req {
                            Request::SetActive(next) => active = next,
                            Request::RefreshNow => force_slow = true,
                        }
                    }

                    if !active {
                        if was_active {
                            tracker.reset();
                            last_slow_poll = None;
                            // Reset volatile throughput & ping history on close so the next
                            // open never shows stale readings from minutes ago.
                            worker_bus.patch(|s| {
                                s.network.details.download_rate = 0.0;
                                s.network.details.upload_rate = 0.0;
                                s.network.details.has_ping_samples = false;
                                s.network.details.ping_ms = None;
                                s.network.details.packet_loss = 0;
                            });
                        }
                        continue;
                    }

                    if !was_active {
                        tracker.reset();
                        last_slow_poll = None;
                        force_slow = true;
                        thread::spawn(trigger_connectivity_check);
                    }

                    // 1. Refresh DNS and Wi-Fi band on open, on explicit refresh, or every ~4s.
                    let need_slow = force_slow
                        || last_slow_poll.is_none_or(|t| t.elapsed() >= BAND_DNS_INTERVAL);
                    if need_slow {
                        last_slow_poll = Some(Instant::now());
                        if worker_dns.load(Ordering::SeqCst) == 0
                            && let Some(dns) = read_dns()
                        {
                            worker_bus.patch(move |s| {
                                if s.network.pending_dns.is_none() {
                                    s.network.dns = dns;
                                }
                            });
                        }
                        if let Some(raw_band) = omarchy_output("omarchy-network-band", &[]) {
                            let status = parse_band_status(&raw_band);
                            let busy = worker_band.load(Ordering::SeqCst) > 0;
                            if !(busy && status.available.is_empty()) {
                                worker_bus.patch(move |s| {
                                    s.network.band_current = status.band;
                                    if s.network.pending_band.is_none() {
                                        s.network.band_selected = status.selected;
                                    }
                                    s.network.band_available = status.available;
                                });
                            }
                        }
                    }

                    // 2. Refresh live connection details (`omarchy-network-status --verbose`).
                    let connectivity = read_connectivity();
                    if let Some(raw_details) =
                        omarchy_output("omarchy-network-status", &["--verbose"])
                    {
                        let kv = parse_key_value(&raw_details);
                        let busy = worker_band.load(Ordering::SeqCst) > 0;
                        let empty_iface = kv.get("iface").is_none_or(String::is_empty);
                        if !(busy && empty_iface) {
                            let details = tracker.ingest(&kv, Instant::now());
                            worker_bus.patch(move |s| {
                                let can_share_qr = s.network.details.can_share_qr;
                                s.network.details = NetworkDetails {
                                    can_share_qr,
                                    ..details
                                };
                                s.network.connectivity = connectivity;
                            });
                        }
                    }
                }
            })
            .ok();

        Self {
            tx,
            active: false,
            bus,
            dns_in_flight,
            band_in_flight,
        }
    }

    pub fn configure(&mut self, scene: &Scene) {
        let want = scene.wants_network_details();
        if want != self.active {
            self.active = want;
            let _ = self.tx.send(Request::SetActive(want));
        }
    }

    pub fn refresh(&self) {
        if self.active {
            let _ = self.tx.send(Request::RefreshNow);
        }
    }

    pub fn set_dns(&self, provider: DnsProvider) {
        self.bus.patch(move |s| {
            s.network.pending_dns = Some(provider);
        });
        self.dns_in_flight.fetch_add(1, Ordering::SeqCst);
        let bus = self.bus.clone();
        let in_flight = self.dns_in_flight.clone();
        let tx = self.tx.clone();
        thread::Builder::new()
            .name("net-dns".into())
            .spawn(move || {
                let packaged = Path::new("/usr/bin/omarchy-dns");
                let program = if packaged.exists() {
                    packaged.to_path_buf()
                } else {
                    omarchy_bin("omarchy-dns")
                };
                let ok = run_outside_sandbox(&program, &[provider.cli_arg()]);
                let remaining = in_flight.fetch_sub(1, Ordering::SeqCst).saturating_sub(1);
                let confirmed = read_dns();
                bus.patch(move |s| {
                    if ok {
                        s.network.dns = confirmed.unwrap_or(provider);
                    } else if let Some(actual) = confirmed {
                        s.network.dns = actual;
                    }
                    if remaining == 0 {
                        s.network.pending_dns = None;
                    }
                });
                let _ = tx.send(Request::RefreshNow);
            })
            .ok();
    }

    pub fn set_band(&self, band: String, nm_refresh: Option<mpsc::Sender<()>>) {
        let pending = band.clone();
        self.bus.patch(move |s| {
            s.network.pending_band = Some(pending);
        });
        self.band_in_flight.fetch_add(1, Ordering::SeqCst);
        let bus = self.bus.clone();
        let in_flight = self.band_in_flight.clone();
        let tx = self.tx.clone();
        thread::Builder::new()
            .name("net-band".into())
            .spawn(move || {
                let program = omarchy_bin("omarchy-network-band");
                let ok = run_outside_sandbox(&program, &[&band]);
                let remaining = in_flight.fetch_sub(1, Ordering::SeqCst).saturating_sub(1);
                let fresh =
                    omarchy_output("omarchy-network-band", &[]).map(|raw| parse_band_status(&raw));
                bus.patch(move |s| {
                    if let Some(status) = fresh {
                        if !status.band.is_empty() {
                            s.network.band_current = status.band;
                        }
                        s.network.band_selected =
                            if ok && status.selected == "auto" && band != "auto" {
                                band
                            } else {
                                status.selected
                            };
                        if !status.available.is_empty() {
                            s.network.band_available = status.available;
                        }
                    } else if ok {
                        s.network.band_selected = band;
                    }
                    if remaining == 0 {
                        s.network.pending_band = None;
                    }
                });
                if let Some(nm) = nm_refresh {
                    let _ = nm.send(());
                }
                let _ = tx.send(Request::RefreshNow);
            })
            .ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_iw_ssid_and_parses_verbose_status() {
        let raw = concat!(
            "iface\twlan0\n",
            "ip\t192.168.1.42\n",
            "prefix\t24\n",
            "gateway\t192.168.1.1\n",
            "rx_bytes\t1048576\n",
            "tx_bytes\t524288\n",
            "type\twifi\n",
            "ssid\tCaf\\xc3\\xa9 Libre\n",
            "freq\t5180\n",
            "internet_ping_ms\t12.4\n",
        );
        let kv = parse_key_value(raw);
        assert_eq!(kv.get("iface").map(String::as_str), Some("wlan0"));
        assert_eq!(kv.get("ip").map(String::as_str), Some("192.168.1.42"));
        assert_eq!(kv.get("gateway").map(String::as_str), Some("192.168.1.1"));
        assert_eq!(kv.get("ssid").map(String::as_str), Some("Café Libre"));
        assert_eq!(decode_iw_ssid("bad\\x07ctrl"), "bad\\x07ctrl");
    }

    #[test]
    fn tracks_throughput_deltas_and_rolling_ping_and_packet_loss() {
        let mut tracker = TelemetryTracker::default();
        let t0 = Instant::now();

        let sample = |iface: &str, rx: u64, tx: u64, ping: &str| {
            let mut kv = HashMap::new();
            kv.insert("iface".into(), iface.into());
            kv.insert("ip".into(), "10.0.0.5".into());
            kv.insert("gateway".into(), "10.0.0.1".into());
            kv.insert("rx_bytes".into(), rx.to_string());
            kv.insert("tx_bytes".into(), tx.to_string());
            kv.insert("internet_ping_ms".into(), ping.into());
            kv
        };

        // First sample establishes baseline with 0 rate (no spike).
        let d1 = tracker.ingest(&sample("wlan0", 1_000_000, 500_000, "10.0"), t0);
        assert_eq!(d1.download_rate, 0.0);
        assert_eq!(d1.upload_rate, 0.0);
        assert!(d1.has_ping_samples);
        assert_eq!(d1.ping_ms, Some(10.0));
        assert_eq!(d1.packet_loss, 0);

        // Second sample 2 seconds later computes exact bytes/sec and averages ping.
        let d2 = tracker.ingest(
            &sample("wlan0", 1_200_000, 550_000, "20.0"),
            t0 + Duration::from_secs(2),
        );
        assert!((d2.download_rate - 100_000.0).abs() < 1.0);
        assert!((d2.upload_rate - 25_000.0).abs() < 1.0);
        assert_eq!(d2.ping_ms, Some(15.0));
        assert_eq!(d2.packet_loss, 0);

        // Third sample times out -> 1/3 = 33% packet loss, ping still averages valid recent samples.
        let d3 = tracker.ingest(
            &sample("wlan0", 1_200_000, 550_000, ""),
            t0 + Duration::from_secs(4),
        );
        assert_eq!(d3.packet_loss, 33);
        assert_eq!(d3.ping_ms, Some(15.0));

        // Interface switch resets both throughput baseline and ping history.
        let d4 = tracker.ingest(
            &sample("enp0s1", 9_000_000, 9_000_000, "4.2"),
            t0 + Duration::from_secs(6),
        );
        assert_eq!(d4.download_rate, 0.0);
        assert_eq!(d4.upload_rate, 0.0);
        assert_eq!(d4.packet_loss, 0);
        assert!((d4.ping_ms.unwrap() - 4.2).abs() < 0.01);
    }

    #[test]
    fn parses_band_status() {
        let parsed = parse_band_status("band\t5\navailable\t2.4 5 6\nselected\tauto\n");
        assert_eq!(
            parsed,
            BandStatus {
                band: "5".into(),
                selected: "auto".into(),
                available: vec!["2.4".into(), "5".into(), "6".into()],
            }
        );
        let empty = parse_band_status("");
        assert_eq!(empty.selected, "auto");
        assert!(empty.available.is_empty());
    }
}
