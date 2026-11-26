//! D-Bus providers: battery (UPower), power profiles, Bluetooth (BlueZ)
//! and media (MPRIS change signals → `omarchy-shell media status`).

use std::{
    collections::{HashMap, HashSet},
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};

use omatouchbar_core::{
    Scene,
    state::{Battery, Bluetooth, BtDevice, BtKind, BtOp, Charge, Media, is_human_bt_name},
};
use zbus::{
    MatchRule,
    blocking::{Connection, MessageIterator},
    message::Type,
    proxy::CacheProperties,
    zvariant::{OwnedObjectPath, OwnedValue},
};

use crate::bus::{Bus, Event};

/// Runs `refresh` now and then after every burst of matching signals.
pub(super) fn watch_signals(
    name: &str,
    connection: Connection,
    rules: Vec<MatchRule<'static>>,
    extra: Option<mpsc::Receiver<()>>,
    refresh: impl Fn(&Connection) + Send + 'static,
) -> mpsc::Sender<()> {
    let (tx, rx) = mpsc::channel::<()>();
    for rule in rules {
        let tx = tx.clone();
        let connection = connection.clone();
        thread::Builder::new()
            .name(format!("{name}-signals"))
            .spawn(move || {
                let Ok(iterator) = MessageIterator::for_match_rule(rule, &connection, Some(256))
                else {
                    return;
                };
                for message in iterator {
                    if message.is_err() || tx.send(()).is_err() {
                        break;
                    }
                }
            })
            .ok();
    }
    if let Some(extra) = extra {
        let tx = tx.clone();
        thread::spawn(move || {
            while extra.recv().is_ok() {
                if tx.send(()).is_err() {
                    break;
                }
            }
        });
    }
    let poke = tx.clone();
    thread::Builder::new()
        .name(name.to_string())
        .spawn(move || {
            loop {
                refresh(&connection);
                match rx.recv_timeout(Duration::from_secs(60)) {
                    Ok(()) => {
                        // Debounce: wait for the burst to settle.
                        thread::sleep(Duration::from_millis(150));
                        while rx.try_recv().is_ok() {}
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => return,
                }
            }
        })
        .ok();
    poke
}

fn properties_rule(sender: &'static str, path: Option<&'static str>) -> Option<MatchRule<'static>> {
    let mut builder = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender(sender)
        .ok()?
        .interface("org.freedesktop.DBus.Properties")
        .ok()?
        .member("PropertiesChanged")
        .ok()?;
    if let Some(path) = path {
        builder = builder.path(path).ok()?;
    }
    Some(builder.build())
}

fn property<T>(
    connection: &Connection,
    dest: &str,
    path: &str,
    iface: &str,
    name: &str,
) -> Option<T>
where
    T: TryFrom<OwnedValue>,
    T::Error: Into<zbus::Error>,
{
    let proxy = zbus::blocking::proxy::Builder::<zbus::blocking::Proxy>::new(connection)
        .destination(dest.to_string())
        .ok()?
        .path(path.to_string())
        .ok()?
        .interface(iface.to_string())
        .ok()?
        .cache_properties(CacheProperties::No)
        .build()
        .ok()?;
    proxy.get_property::<T>(name).ok()
}

// ------------------------------------------------------------------ UPower

const UPOWER: &str = "org.freedesktop.UPower";
const DISPLAY_DEVICE: &str = "/org/freedesktop/UPower/devices/DisplayDevice";

fn read_battery(c: &Connection) -> Option<Battery> {
    let iface = "org.freedesktop.UPower.Device";
    let present: bool = property(c, UPOWER, DISPLAY_DEVICE, iface, "IsPresent")?;
    if !present {
        return None;
    }
    let percent: f64 = property(c, UPOWER, DISPLAY_DEVICE, iface, "Percentage")?;
    let state: u32 = property(c, UPOWER, DISPLAY_DEVICE, iface, "State").unwrap_or(0);
    let to_empty: i64 = property(c, UPOWER, DISPLAY_DEVICE, iface, "TimeToEmpty").unwrap_or(0);
    let to_full: i64 = property(c, UPOWER, DISPLAY_DEVICE, iface, "TimeToFull").unwrap_or(0);
    let rate: f64 = property(c, UPOWER, DISPLAY_DEVICE, iface, "EnergyRate").unwrap_or(0.0);
    let on_battery: bool =
        property(c, UPOWER, "/org/freedesktop/UPower", UPOWER, "OnBattery").unwrap_or(false);
    // Omarchy's rule: on AC and pending charge, or "full" below 99%, means a
    // charge limit is holding the battery.
    let holding = !on_battery && (state == 5 || (state == 4 && percent < 99.0));
    let charge = match state {
        _ if holding => Charge::Holding,
        1 => Charge::Charging,
        2 | 3 | 6 => Charge::Discharging,
        4 => Charge::Full,
        5 => Charge::NotCharging,
        _ if percent > 0.0 => Charge::NotCharging,
        _ => Charge::Unknown,
    };
    let mut battery = Battery {
        percent: percent as f32,
        charge,
        seconds_to_empty: to_empty.max(0) as u64,
        seconds_to_full: to_full.max(0) as u64,
        rate_watts: rate.abs() as f32,
        ..Battery::default()
    };
    // Size, cycles, time and charge limit exactly as Omarchy formats them.
    if let Ok(out) = Command::new("omarchy-battery-status")
        .arg("--shell")
        .stderr(Stdio::null())
        .output()
    {
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            let Some((key, value)) = line.split_once('\t') else {
                continue;
            };
            let value = value.trim().to_string();
            match key {
                "size" => battery.size = value,
                "cycles" => battery.cycles = value,
                "time" => battery.time = value,
                "threshold" => battery.threshold = value,
                _ => {}
            }
        }
    }
    Some(battery)
}

const PPD: &str = "net.hadess.PowerProfiles";
const PPD_PATH: &str = "/net/hadess/PowerProfiles";

fn read_profiles(c: &Connection) -> (Option<String>, Vec<String>) {
    let active: Option<String> = property(c, PPD, PPD_PATH, PPD, "ActiveProfile");
    let list: Vec<HashMap<String, OwnedValue>> =
        property(c, PPD, PPD_PATH, PPD, "Profiles").unwrap_or_default();
    let mut profiles: Vec<String> = list
        .iter()
        .filter_map(|p| {
            p.get("Profile")
                .and_then(|v| String::try_from(v.try_clone().ok()?).ok())
        })
        .collect();
    let order = |p: &String| match p.as_str() {
        "power-saver" => 0,
        "balanced" => 1,
        "performance" => 2,
        _ => 3,
    };
    profiles.sort_by_key(order);
    (active, profiles)
}

pub fn spawn_power(bus: Bus) {
    let Ok(connection) = Connection::system() else {
        log::warn!("system D-Bus unavailable; no battery status");
        return;
    };
    let rules: Vec<MatchRule<'static>> = [
        properties_rule(UPOWER, Some(DISPLAY_DEVICE)),
        properties_rule(PPD, Some(PPD_PATH)),
    ]
    .into_iter()
    .flatten()
    .collect();
    watch_signals("power", connection, rules, None, move |c| {
        let battery = read_battery(c);
        let (profile, profiles) = read_profiles(c);
        bus.patch(move |s| {
            s.battery = battery;
            s.power_profile = profile;
            s.power_profiles = profiles;
        });
    });
}

// ------------------------------------------------------------------- BlueZ

type ManagedObjects = HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>;
type Props = HashMap<String, OwnedValue>;

fn bluez_objects(c: &Connection) -> Option<ManagedObjects> {
    let proxy = zbus::blocking::fdo::ObjectManagerProxy::builder(c)
        .destination("org.bluez")
        .ok()?
        .path("/")
        .ok()?
        .build()
        .ok()?;
    let objects = proxy.get_managed_objects().ok()?;
    Some(
        objects
            .into_iter()
            .map(|(path, ifaces)| {
                (
                    path,
                    ifaces
                        .into_iter()
                        .map(|(name, props)| (name.to_string(), props))
                        .collect(),
                )
            })
            .collect(),
    )
}

fn flag(props: &Props, key: &str) -> bool {
    props
        .get(key)
        .and_then(|v| bool::try_from(v).ok())
        .unwrap_or(false)
}

fn text(props: &Props, key: &str) -> String {
    props
        .get(key)
        .and_then(|v| v.try_clone().ok())
        .and_then(|v| String::try_from(v).ok())
        .unwrap_or_default()
}

fn read_bluetooth(c: &Connection) -> Bluetooth {
    let Some(objects) = bluez_objects(c) else {
        return Bluetooth::default();
    };
    let mut state = Bluetooth::default();
    let mut devices: Vec<BtDevice> = Vec::new();
    let mut nearby: Vec<BtDevice> = Vec::new();
    for ifaces in objects.values() {
        if let Some(adapter) = ifaces.get("org.bluez.Adapter1") {
            state.available = true;
            state.powered |= flag(adapter, "Powered");
            state.discovering |= flag(adapter, "Discovering");
        }
        let Some(device) = ifaces.get("org.bluez.Device1") else {
            continue;
        };
        let address = text(device, "Address");
        let connected = flag(device, "Connected");
        let paired = flag(device, "Paired") || flag(device, "Bonded") || flag(device, "Trusted");
        let remembered = connected || paired;
        // As Omarchy's panel: only devices with a real name. A remembered
        // one without falls back to its address so it can still be forgotten.
        let name = [text(device, "Alias"), text(device, "Name")]
            .into_iter()
            .find(|n| is_human_bt_name(n))
            .map(|n| n.trim().to_string())
            .or_else(|| remembered.then(|| address.clone()));
        let Some(name) = name else {
            continue;
        };
        let battery = ifaces
            .get("org.bluez.Battery1")
            .and_then(|b| b.get("Percentage"))
            .and_then(|v| u8::try_from(v).ok());
        let entry = BtDevice {
            name,
            address,
            connected,
            paired,
            kind: BtKind::from_icon(&text(device, "Icon")),
            battery,
        };
        if remembered {
            devices.push(entry);
        } else {
            nearby.push(entry);
        }
    }
    devices.sort_by(|a, b| {
        b.connected
            .cmp(&a.connected)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    nearby.sort_by_key(|d| d.name.to_lowercase());
    state.connected = devices
        .iter()
        .filter(|d| d.connected)
        .map(|d| d.name.clone())
        .collect();
    state.devices = devices;
    state.nearby = nearby;
    state
}

/// Runs `omarchy-bluetooth-device <op> <address>`, then reports how it went
/// (`Event::BluetoothDone`) along with the fresh device list.
pub fn run_bluetooth_op(bus: Bus, address: String, op: BtOp) {
    let spawned = thread::Builder::new().name("bt-op".into()).spawn(move || {
        let status = Command::new("omarchy-bluetooth-device")
            .args([op.arg(), address.as_str()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if let Err(e) = &status {
            log::warn!("omarchy-bluetooth-device: {e}");
        }
        let state = Connection::system().ok().map(|c| read_bluetooth(&c));
        let (op, ok) = match (&status, &state) {
            (Ok(_), Some(state)) => op.outcome(state, &address),
            _ => (op, false),
        };
        if !ok {
            log::info!("bluetooth: could not {} {address}", op.arg());
        }
        if let Some(state) = state {
            bus.patch(move |s| s.bluetooth = state);
        }
        bus.send(Event::BluetoothDone { address, op, ok });
    });
    if let Err(e) = spawned {
        log::warn!("bluetooth: {e}");
    }
}

/// Keeps BlueZ discovery running while the Bluetooth panel is open, so new
/// devices show up to pair, and stops it when the panel closes — a scan
/// left running starves Bluetooth audio on the same radio. The discovery
/// session belongs to this thread's bus connection, so BlueZ also ends it
/// if the renderer exits.
pub struct BluetoothScan {
    tx: Option<mpsc::Sender<bool>>,
    active: bool,
}

impl BluetoothScan {
    pub fn start() -> BluetoothScan {
        let (tx, rx) = mpsc::channel();
        let tx = thread::Builder::new()
            .name("bt-scan".into())
            .spawn(move || scan_loop(rx))
            .ok()
            .map(|_| tx);
        BluetoothScan { tx, active: false }
    }

    pub fn configure(&mut self, scene: &Scene) {
        let want = scene.wants_bluetooth_scan();
        if want != self.active {
            self.active = want;
            if let Some(tx) = &self.tx {
                let _ = tx.send(want);
            }
        }
    }
}

fn scan_loop(rx: mpsc::Receiver<bool>) {
    let mut want = false;
    let mut connection: Option<Connection> = None;
    // Adapters this connection holds a discovery session on.
    let mut ours: HashSet<OwnedObjectPath> = HashSet::new();
    loop {
        let message = if want {
            // BlueZ refuses to scan while an adapter powers up, and scans can
            // end on their own: keep nudging while the panel wants one.
            rx.recv_timeout(Duration::from_secs(1))
        } else {
            rx.recv().map_err(|_| mpsc::RecvTimeoutError::Disconnected)
        };
        match message {
            Ok(w) => want = w,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
        while let Ok(w) = rx.try_recv() {
            want = w;
        }
        if connection.is_none() {
            connection = Connection::system().ok();
        }
        let Some(c) = connection.as_ref() else {
            continue;
        };
        if !want {
            for path in ours.drain() {
                let _ = adapter_call(c, &path, "StopDiscovery");
            }
            continue;
        }
        let Some(objects) = bluez_objects(c) else {
            continue;
        };
        for (path, ifaces) in objects {
            let Some(adapter) = ifaces.get("org.bluez.Adapter1") else {
                continue;
            };
            if !flag(adapter, "Powered") {
                // Powering down ends every session.
                ours.remove(&path);
                continue;
            }
            if ours.contains(&path) && flag(adapter, "Discovering") {
                continue;
            }
            match adapter_call(c, &path, "StartDiscovery") {
                Ok(()) => {
                    log::debug!("bluetooth: scanning on {}", path.as_str());
                    ours.insert(path);
                }
                // This connection already has a session there.
                Err(zbus::Error::MethodError(name, ..))
                    if name.as_str() == "org.bluez.Error.InProgress" =>
                {
                    ours.insert(path);
                }
                Err(e) => log::debug!("bluetooth: StartDiscovery: {e}"),
            }
        }
    }
}

fn adapter_call(c: &Connection, path: &OwnedObjectPath, method: &str) -> zbus::Result<()> {
    c.call_method(
        Some("org.bluez"),
        path.as_str(),
        Some("org.bluez.Adapter1"),
        method,
        &(),
    )
    .map(|_| ())
}

pub fn spawn_bluetooth(bus: Bus) -> Option<mpsc::Sender<()>> {
    let connection = Connection::system().ok()?;
    let mut rules = Vec::new();
    if let Some(rule) = properties_rule("org.bluez", None) {
        rules.push(rule);
    }
    for member in ["InterfacesAdded", "InterfacesRemoved"] {
        if let Ok(builder) = MatchRule::builder()
            .msg_type(Type::Signal)
            .sender("org.bluez")
            && let Ok(builder) = builder.member(member)
        {
            rules.push(builder.build());
        }
    }
    Some(watch_signals(
        "bluetooth",
        connection,
        rules,
        None,
        move |c| {
            let state = read_bluetooth(c);
            bus.patch(move |s| s.bluetooth = state);
        },
    ))
}

// ------------------------------------------------------------------- Media

fn read_media() -> Option<Media> {
    let out = Command::new("omarchy-shell")
        .args(["media", "status"])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    let has = value
        .get("hasMedia")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !has {
        return None;
    }
    let text = |key: &str| {
        value
            .get(key)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    let flag = |key: &str| value.get(key).and_then(|v| v.as_bool()).unwrap_or(false);
    Some(Media {
        title: text("title"),
        artist: text("artist"),
        player: text("identity"),
        playing: flag("playing"),
        can_next: flag("canGoNext"),
        can_previous: flag("canGoPrevious"),
    })
}

pub fn spawn_media(bus: Bus) {
    let Ok(connection) = Connection::session() else {
        return;
    };
    let mut rules = Vec::new();
    if let Ok(b) = MatchRule::builder()
        .msg_type(Type::Signal)
        .path("/org/mpris/MediaPlayer2")
        && let Ok(b) = b.interface("org.freedesktop.DBus.Properties")
    {
        rules.push(b.build());
    }
    if let Ok(b) = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender("org.freedesktop.DBus")
        && let Ok(b) = b
            .member("NameOwnerChanged")
            .and_then(|b| b.arg0ns("org.mpris.MediaPlayer2"))
    {
        rules.push(b.build());
    }
    watch_signals("media", connection, rules, None, move |_| {
        let media = read_media();
        bus.patch(move |s| s.media = media);
    });
}
