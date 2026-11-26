//! Native NetworkManager D-Bus Wi-Fi connection manager.
//!
//! Secrets pass directly over the private system D-Bus socket — never in
//! process arguments or temporary files — and activation failures report
//! accurate reasons ("Wrong password", "Network not found", etc.) while
//! cleaning up failed newly-created profiles.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc::Sender,
    },
    thread,
    time::{Duration, Instant},
};

use zbus::{
    blocking::{Connection, Proxy},
    proxy::CacheProperties,
    zvariant::{OwnedObjectPath, OwnedValue, Value},
};

use crate::bus::{Bus, Event};

const NM_DEST: &str = "org.freedesktop.NetworkManager";
const NM_PATH: &str = "/org/freedesktop/NetworkManager";
const NM_SETTINGS_PATH: &str = "/org/freedesktop/NetworkManager/Settings";

const NM_DEVICE_TYPE_WIFI: u32 = 2;

const NM_AP_FLAGS_PRIVACY: u32 = 0x1;
const NM_AP_SEC_KEY_MGMT_PSK: u32 = 0x100;
const NM_AP_SEC_KEY_MGMT_SAE: u32 = 0x400;

const NM_AC_ACTIVATED: u32 = 2;
const NM_AC_DEACTIVATING: u32 = 3;
const NM_AC_DEACTIVATED: u32 = 4;

const NM_AC_REASON_USER_DISCONNECTED: u32 = 2;
const NM_AC_REASON_NO_SECRETS: u32 = 9;
const NM_AC_REASON_LOGIN_FAILED: u32 = 10;

const NM_DEV_REASON_NO_SECRETS: u32 = 7;
const NM_DEV_REASON_SUPPLICANT_DISCONNECT: u32 = 8;
const NM_DEV_REASON_SUPPLICANT_CONFIG_FAILED: u32 = 9;
const NM_DEV_REASON_SUPPLICANT_FAILED: u32 = 10;
const NM_DEV_REASON_SUPPLICANT_TIMEOUT: u32 = 11;
const NM_DEV_REASON_SSID_NOT_FOUND: u32 = 53;

#[derive(Clone, Default)]
pub struct WifiConnector {
    generation: Arc<AtomicU64>,
    active_conn: Arc<Mutex<Option<OwnedObjectPath>>>,
}

impl WifiConnector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        let active = self.active_conn.lock().ok().and_then(|mut g| g.take());
        if let Some(path) = active {
            thread::spawn(move || {
                if let Ok(conn) = Connection::system() {
                    let _ = deactivate(&conn, &path);
                }
            });
        }
    }

    /// Connects to a secured Wi-Fi network using a password entered on the Touch Bar.
    pub fn connect_with_password(
        &self,
        ssid: String,
        password: String,
        bus: Bus,
        refresh: Option<Sender<()>>,
    ) {
        let my_gen = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let generation = self.generation.clone();
        let active_slot = self.active_conn.clone();
        thread::Builder::new()
            .name("wifi-connect".into())
            .spawn(move || {
                let outcome =
                    run_connect(&ssid, Some(&password), my_gen, &generation, &active_slot);
                if let Ok(mut g) = active_slot.lock() {
                    *g = None;
                }
                if let Some(tx) = refresh {
                    let _ = tx.send(());
                }
                if generation.load(Ordering::SeqCst) != my_gen {
                    return;
                }
                match outcome {
                    Ok(()) => bus.send(Event::WifiConnectResult {
                        ssid,
                        result: Ok(()),
                        auth_failed: false,
                    }),
                    Err((msg, auth_failed)) => bus.send(Event::WifiConnectResult {
                        ssid,
                        result: Err(msg),
                        auth_failed,
                    }),
                }
            })
            .ok();
    }

    /// Connects to a known or open Wi-Fi network, reporting auth failures so the
    /// Touch Bar can prompt for a new password if stored credentials are rejected.
    pub fn connect_known_or_open(&self, ssid: String, bus: Bus, refresh: Option<Sender<()>>) {
        let my_gen = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let generation = self.generation.clone();
        let active_slot = self.active_conn.clone();
        thread::Builder::new()
            .name("wifi-connect".into())
            .spawn(move || {
                let outcome = run_connect(&ssid, None, my_gen, &generation, &active_slot);
                if let Ok(mut g) = active_slot.lock() {
                    *g = None;
                }
                if let Some(tx) = refresh {
                    let _ = tx.send(());
                }
                if generation.load(Ordering::SeqCst) != my_gen {
                    return;
                }
                if let Err((msg, auth_failed)) = outcome
                    && auth_failed
                {
                    bus.send(Event::WifiConnectResult {
                        ssid,
                        result: Err(msg),
                        auth_failed: true,
                    });
                }
            })
            .ok();
    }
}

fn proxy<'a>(
    conn: &'a Connection,
    path: &'a str,
    iface: &'a str,
) -> Result<Proxy<'a>, zbus::Error> {
    zbus::blocking::proxy::Builder::<Proxy>::new(conn)
        .destination(NM_DEST)?
        .path(path)?
        .interface(iface)?
        .cache_properties(CacheProperties::No)
        .build()
}

fn deactivate(conn: &Connection, active_path: &OwnedObjectPath) -> Result<(), zbus::Error> {
    let nm = proxy(conn, NM_PATH, NM_DEST)?;
    let _: () = nm.call("DeactivateConnection", &(active_path.as_ref(),))?;
    Ok(())
}

fn delete_profile(conn: &Connection, profile_path: &OwnedObjectPath) {
    if let Ok(p) = proxy(
        conn,
        profile_path.as_str(),
        "org.freedesktop.NetworkManager.Settings.Connection",
    ) {
        let _: Result<(), _> = p.call("Delete", &());
    }
}

fn find_wifi_device(conn: &Connection) -> Result<OwnedObjectPath, (String, bool)> {
    let nm =
        proxy(conn, NM_PATH, NM_DEST).map_err(|_| ("NetworkManager unavailable".into(), false))?;
    let devices: Vec<OwnedObjectPath> = nm
        .call("GetDevices", &())
        .map_err(|_| ("No Wi-Fi adapter".into(), false))?;
    for dev in devices {
        let is_wifi = proxy(conn, dev.as_str(), "org.freedesktop.NetworkManager.Device")
            .ok()
            .and_then(|p| p.get_property::<u32>("DeviceType").ok())
            == Some(NM_DEVICE_TYPE_WIFI);
        if is_wifi {
            return Ok(dev);
        }
    }
    Err(("No Wi-Fi adapter".into(), false))
}

struct ApInfo {
    path: OwnedObjectPath,
    flags: u32,
    wpa_flags: u32,
    rsn_flags: u32,
    strength: u8,
}

fn find_best_ap(conn: &Connection, dev: &OwnedObjectPath, ssid: &str) -> Option<ApInfo> {
    let wifi = proxy(
        conn,
        dev.as_str(),
        "org.freedesktop.NetworkManager.Device.Wireless",
    )
    .ok()?;
    let aps: Vec<OwnedObjectPath> = wifi.call("GetAllAccessPoints", &()).ok()?;
    let target = ssid.as_bytes();
    let mut best: Option<ApInfo> = None;
    for ap_path in aps {
        let info = {
            let Ok(ap) = proxy(
                conn,
                ap_path.as_str(),
                "org.freedesktop.NetworkManager.AccessPoint",
            ) else {
                continue;
            };
            let Ok(bytes) = ap.get_property::<Vec<u8>>("Ssid") else {
                continue;
            };
            if bytes != target {
                continue;
            }
            (
                ap.get_property::<u8>("Strength").unwrap_or(0),
                ap.get_property::<u32>("Flags").unwrap_or(0),
                ap.get_property::<u32>("WpaFlags").unwrap_or(0),
                ap.get_property::<u32>("RsnFlags").unwrap_or(0),
            )
        };
        let (strength, flags, wpa_flags, rsn_flags) = info;
        if best.as_ref().is_none_or(|b| strength > b.strength) {
            best = Some(ApInfo {
                path: ap_path,
                flags,
                wpa_flags,
                rsn_flags,
                strength,
            });
        }
    }
    best
}

type SettingsMap = HashMap<String, HashMap<String, OwnedValue>>;

fn find_existing_profile(conn: &Connection, ssid: &str) -> Option<(OwnedObjectPath, SettingsMap)> {
    let settings = proxy(
        conn,
        NM_SETTINGS_PATH,
        "org.freedesktop.NetworkManager.Settings",
    )
    .ok()?;
    let list: Vec<OwnedObjectPath> = settings.call("ListConnections", &()).ok()?;
    let target = ssid.as_bytes();
    for path in list {
        let map = {
            let Ok(c) = proxy(
                conn,
                path.as_str(),
                "org.freedesktop.NetworkManager.Settings.Connection",
            ) else {
                continue;
            };
            let Ok(map) = c.call::<_, _, SettingsMap>("GetSettings", &()) else {
                continue;
            };
            map
        };
        let is_wifi = map
            .get("connection")
            .and_then(|s| s.get("type"))
            .and_then(|v| String::try_from(v.try_clone().ok()?).ok())
            .is_some_and(|t| t == "802-11-wireless");
        if !is_wifi {
            continue;
        }
        let matches = map
            .get("802-11-wireless")
            .and_then(|w| w.get("ssid"))
            .and_then(|v| Vec::<u8>::try_from(v.try_clone().ok()?).ok())
            .is_some_and(|b| b == target);
        if matches {
            return Some((path, map));
        }
    }
    None
}

fn ap_key_mgmt(ap: Option<&ApInfo>) -> &'static str {
    let Some(ap) = ap else {
        return "wpa-psk";
    };
    let has_sae = (ap.rsn_flags & NM_AP_SEC_KEY_MGMT_SAE) != 0;
    let has_psk = (ap.rsn_flags & NM_AP_SEC_KEY_MGMT_PSK) != 0
        || (ap.wpa_flags & NM_AP_SEC_KEY_MGMT_PSK) != 0;
    if has_sae && !has_psk {
        "sae"
    } else if ap.wpa_flags == 0 && ap.rsn_flags == 0 && (ap.flags & NM_AP_FLAGS_PRIVACY) != 0 {
        "none"
    } else {
        "wpa-psk"
    }
}

fn friendly_dbus_error(err: &zbus::Error, password: Option<&str>) -> (String, bool) {
    let text = err.to_string().to_lowercase();
    if text.contains("psk") || text.contains("wep-key") || text.contains("key-mgmt") {
        if password.is_some_and(|p| p.chars().count() < 8) {
            return ("Min 8 characters".into(), true);
        }
        return ("Invalid password".into(), true);
    }
    if text.contains("no secrets") || text.contains("secret") {
        return ("Password required".into(), true);
    }
    if text.contains("permission") || text.contains("not authorized") {
        return ("Not authorized".into(), false);
    }
    log::warn!("wifi dbus error: {err}");
    ("Connection failed".into(), false)
}

fn run_connect(
    ssid: &str,
    password: Option<&str>,
    my_gen: u64,
    generation: &AtomicU64,
    active_slot: &Mutex<Option<OwnedObjectPath>>,
) -> Result<(), (String, bool)> {
    let conn = Connection::system().map_err(|_| ("D-Bus unavailable".into(), false))?;
    let dev = find_wifi_device(&conn)?;
    let ap = find_best_ap(&conn, &dev, ssid);
    let ap_path_str = ap
        .as_ref()
        .map(|a| a.path.as_str().to_string())
        .unwrap_or_else(|| "/".to_string());
    let ap_path =
        OwnedObjectPath::try_from(ap_path_str).map_err(|_| ("Invalid AP path".into(), false))?;
    let nm =
        proxy(&conn, NM_PATH, NM_DEST).map_err(|_| ("NetworkManager unavailable".into(), false))?;

    let mut created_profile: Option<OwnedObjectPath> = None;

    let active_path: OwnedObjectPath = if let Some(pw) = password {
        let key_mgmt = ap_key_mgmt(ap.as_ref());
        if let Some((existing_path, mut existing_map)) = find_existing_profile(&conn, ssid) {
            // Update the existing saved profile's password in place, then activate it.
            let sec = existing_map
                .entry("802-11-wireless-security".to_string())
                .or_default();
            sec.insert(
                "key-mgmt".to_string(),
                OwnedValue::from(zbus::zvariant::Str::from(key_mgmt)),
            );
            if key_mgmt == "none" {
                sec.insert(
                    "wep-key0".to_string(),
                    OwnedValue::from(zbus::zvariant::Str::from(pw)),
                );
            } else {
                sec.insert(
                    "psk".to_string(),
                    OwnedValue::from(zbus::zvariant::Str::from(pw)),
                );
                sec.remove("psk-flags");
            }
            let cproxy = proxy(
                &conn,
                existing_path.as_str(),
                "org.freedesktop.NetworkManager.Settings.Connection",
            )
            .map_err(|e| friendly_dbus_error(&e, Some(pw)))?;
            let _: () = cproxy
                .call("Update", &(existing_map,))
                .map_err(|e| friendly_dbus_error(&e, Some(pw)))?;
            nm.call(
                "ActivateConnection",
                &(existing_path.as_ref(), dev.as_ref(), ap_path.as_ref()),
            )
            .map_err(|e| friendly_dbus_error(&e, Some(pw)))?
        } else {
            // Build a brand-new connection settings dictionary.
            let mut settings: HashMap<&str, HashMap<&str, Value<'_>>> = HashMap::new();
            let mut conn_sec: HashMap<&str, Value<'_>> = HashMap::new();
            conn_sec.insert("id", Value::from(ssid));
            conn_sec.insert("type", Value::from("802-11-wireless"));
            conn_sec.insert("autoconnect", Value::from(true));
            settings.insert("connection", conn_sec);

            let mut wifi_sec: HashMap<&str, Value<'_>> = HashMap::new();
            wifi_sec.insert("ssid", Value::from(ssid.as_bytes().to_vec()));
            wifi_sec.insert("mode", Value::from("infrastructure"));
            settings.insert("802-11-wireless", wifi_sec);

            let mut wsec: HashMap<&str, Value<'_>> = HashMap::new();
            wsec.insert("key-mgmt", Value::from(key_mgmt));
            if key_mgmt == "none" {
                wsec.insert("wep-key0", Value::from(pw));
            } else {
                wsec.insert("psk", Value::from(pw));
            }
            settings.insert("802-11-wireless-security", wsec);

            let (profile_path, active_path): (OwnedObjectPath, OwnedObjectPath) = nm
                .call(
                    "AddAndActivateConnection",
                    &(settings, dev.as_ref(), ap_path.as_ref()),
                )
                .map_err(|e| friendly_dbus_error(&e, Some(pw)))?;
            created_profile = Some(profile_path);
            active_path
        }
    } else if let Some((existing_path, _)) = find_existing_profile(&conn, ssid) {
        nm.call(
            "ActivateConnection",
            &(existing_path.as_ref(), dev.as_ref(), ap_path.as_ref()),
        )
        .map_err(|e| friendly_dbus_error(&e, None))?
    } else {
        // Open / OWE network without a saved profile.
        let mut settings: HashMap<&str, HashMap<&str, Value<'_>>> = HashMap::new();
        let mut conn_sec: HashMap<&str, Value<'_>> = HashMap::new();
        conn_sec.insert("id", Value::from(ssid));
        conn_sec.insert("type", Value::from("802-11-wireless"));
        conn_sec.insert("autoconnect", Value::from(true));
        settings.insert("connection", conn_sec);

        let mut wifi_sec: HashMap<&str, Value<'_>> = HashMap::new();
        wifi_sec.insert("ssid", Value::from(ssid.as_bytes().to_vec()));
        wifi_sec.insert("mode", Value::from("infrastructure"));
        settings.insert("802-11-wireless", wifi_sec);

        let (profile_path, active_path): (OwnedObjectPath, OwnedObjectPath) = nm
            .call(
                "AddAndActivateConnection",
                &(settings, dev.as_ref(), ap_path.as_ref()),
            )
            .map_err(|e| friendly_dbus_error(&e, None))?;
        created_profile = Some(profile_path);
        active_path
    };

    if let Ok(mut g) = active_slot.lock() {
        *g = Some(active_path.clone());
    }

    // Monitor the ActiveConnection and Device state until activated, failed, cancelled, or timed out.
    let ac_proxy = proxy(
        &conn,
        active_path.as_str(),
        "org.freedesktop.NetworkManager.Connection.Active",
    )
    .map_err(|_| ("Activation lost".into(), false))?;
    let dev_proxy = proxy(&conn, dev.as_str(), "org.freedesktop.NetworkManager.Device")
        .map_err(|_| ("Wi-Fi device lost".into(), false))?;

    let deadline = Instant::now() + Duration::from_secs(28);
    let mut last_dev_reason = 0u32;
    loop {
        if generation.load(Ordering::SeqCst) != my_gen {
            let _ = deactivate(&conn, &active_path);
            if let Some(profile) = created_profile.as_ref() {
                delete_profile(&conn, profile);
            }
            return Err(("Cancelled".into(), false));
        }

        if let Ok((_, r)) = dev_proxy.get_property::<(u32, u32)>("StateReason")
            && r != 0
        {
            last_dev_reason = r;
        }

        match ac_proxy.get_property::<u32>("State") {
            Ok(NM_AC_ACTIVATED) => {
                log::info!("wifi connected to {ssid}");
                return Ok(());
            }
            Ok(NM_AC_DEACTIVATING | NM_AC_DEACTIVATED) | Err(_) => {
                let ac_reason = ac_proxy.get_property::<u32>("StateReason").unwrap_or(0);
                if let Ok((_, r)) = dev_proxy.get_property::<(u32, u32)>("StateReason")
                    && r != 0
                {
                    last_dev_reason = r;
                }
                if let Some(profile) = created_profile.as_ref() {
                    delete_profile(&conn, profile);
                }
                if ac_reason == NM_AC_REASON_USER_DISCONNECTED {
                    return Err(("Cancelled".into(), false));
                }
                let auth_failed = matches!(
                    ac_reason,
                    NM_AC_REASON_NO_SECRETS | NM_AC_REASON_LOGIN_FAILED
                ) || matches!(
                    last_dev_reason,
                    NM_DEV_REASON_NO_SECRETS
                        | NM_DEV_REASON_SUPPLICANT_DISCONNECT
                        | NM_DEV_REASON_SUPPLICANT_CONFIG_FAILED
                        | NM_DEV_REASON_SUPPLICANT_FAILED
                        | NM_DEV_REASON_SUPPLICANT_TIMEOUT
                );
                let msg = if auth_failed {
                    "Wrong password".into()
                } else if last_dev_reason == NM_DEV_REASON_SSID_NOT_FOUND {
                    "Network not found".into()
                } else {
                    "Connection failed".into()
                };
                log::info!(
                    "wifi connect to {ssid} failed: {msg} (ac_reason={ac_reason}, dev_reason={last_dev_reason})"
                );
                return Err((msg, auth_failed));
            }
            Ok(_) => {}
        }

        if Instant::now() >= deadline {
            let _ = deactivate(&conn, &active_path);
            if let Some(profile) = created_profile.as_ref() {
                delete_profile(&conn, profile);
            }
            return Err(("Timed out — check password".into(), true));
        }

        thread::sleep(Duration::from_millis(120));
    }
}
