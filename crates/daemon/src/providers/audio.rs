//! Audio from PipeWire (via `pactl`), refreshed on `pactl subscribe` events:
//! output volume and mute on the sink Omarchy's volume commands control
//! (`omarchy-audio-output-sink`), the default input's volume and mute, the
//! output and input devices, and per-app playback streams — what Omarchy's
//! audio panel lists.

use std::{
    io::{BufRead, BufReader},
    process::{Command, Stdio},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::Duration,
};

use omatouchbar_core::state::{Audio, AudioDevice, AudioDeviceKind, AudioStream, Mixer};
use serde_json::Value;

use crate::bus::Bus;

fn output(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program)
        .args(args)
        .env("LC_ALL", "C")
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn pactl_json(args: &[&str]) -> Value {
    let mut all = vec!["-f", "json"];
    all.extend_from_slice(args);
    output("pactl", &all)
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(Value::Null)
}

fn list(value: &Value) -> &[Value] {
    value.as_array().map_or(&[], Vec::as_slice)
}

fn sink_name() -> Option<String> {
    output("omarchy-audio-output-sink", &[])
        .filter(|s| !s.is_empty())
        .or_else(|| output("pactl", &["get-default-sink"]))
}

fn str_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

fn prop<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get("properties")
        .and_then(|p| p.get(key))
        .and_then(Value::as_str)
        .unwrap_or("")
}

/// First channel's volume in percent (`"value_percent": "46%"`).
fn volume_percent(v: &Value) -> u8 {
    v.get("volume")
        .and_then(Value::as_object)
        .and_then(|channels| channels.values().next())
        .and_then(|c| c.get("value_percent"))
        .and_then(Value::as_str)
        .and_then(|p| p.trim_end_matches('%').trim().parse::<u32>().ok())
        .unwrap_or(0)
        .min(100) as u8
}

fn muted(v: &Value) -> bool {
    v.get("mute").and_then(Value::as_bool).unwrap_or(false)
}

/// The active port's description, if the device has ports.
fn active_port(v: &Value) -> Option<&str> {
    let active = str_of(v, "active_port");
    list(v.get("ports").unwrap_or(&Value::Null))
        .iter()
        .find(|p| str_of(p, "name") == active)
        .map(|p| str_of(p, "description"))
        .filter(|d| !d.is_empty())
}

/// Something is plugged into it: no ports, or a port that is not
/// "not available" (as `omarchy-audio-sink-availability` decides).
fn available(v: &Value) -> bool {
    let ports = list(v.get("ports").unwrap_or(&Value::Null));
    ports.is_empty()
        || ports
            .iter()
            .any(|p| str_of(p, "availability") != "not available")
}

/// Omarchy's `friendlyDeviceLabel`, plus dropping the machine's model name
/// a speaker tuning's description starts with ("MacBookPro14,3 Open
/// Matched Speakers").
fn friendly(label: &str, product: &str) -> String {
    let mut label = label.trim();
    if !product.is_empty()
        && let Some(rest) = label.strip_prefix(product)
        && !rest.trim().is_empty()
    {
        label = rest.trim();
    }
    for prefix in [
        "sof-soundwire ",
        "Built-in Audio ",
        "Built-In Audio ",
        "Builtin Audio ",
    ] {
        if let Some(rest) = label.strip_prefix(prefix) {
            label = rest.trim();
        }
    }
    for suffix in [" Output", " Input"] {
        if let Some(rest) = label.strip_suffix(suffix) {
            label = rest.trim();
        }
    }
    label.replace("Microphones", "Microphone")
}

fn device_label(v: &Value, product: &str) -> String {
    let name = str_of(v, "name");
    let description = str_of(v, "description");
    if name.starts_with("bluez_") && !description.is_empty() {
        return friendly(description, product);
    }
    // A sound card's port names what is in use ("Speakers", "Headphones",
    // "Internal Microphone") better than the codec does; HDMI ports all
    // read "HDMI / DisplayPort", so those keep the device's name.
    if name.starts_with("alsa_")
        && let Some(port) = active_port(v)
        && !port.contains("HDMI")
        && !port.contains("DisplayPort")
    {
        return friendly(port, product);
    }
    let nick = [prop(v, "node.nick"), prop(v, "device.profile.description")]
        .into_iter()
        .find(|s| !s.is_empty() && !name.starts_with("alsa_"));
    let label = friendly(nick.unwrap_or(description), product);
    if label.is_empty() {
        name.to_string()
    } else {
        label
    }
}

/// Omarchy's `sinkGlyph` / `sourceGlyph` rules.
fn device_kind(v: &Value, input: bool) -> AudioDeviceKind {
    let blob = [
        str_of(v, "name"),
        str_of(v, "description"),
        str_of(v, "active_port"),
        prop(v, "node.nick"),
        prop(v, "device.icon-name"),
        prop(v, "device.product.name"),
        prop(v, "device.form-factor"),
    ]
    .join(" ")
    .to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| blob.contains(w));
    if input {
        if has(&["headset"]) {
            AudioDeviceKind::Headphones
        } else if has(&["bluetooth", "bluez"]) {
            AudioDeviceKind::Bluetooth
        } else if has(&["webcam", "camera"]) {
            AudioDeviceKind::Camera
        } else {
            AudioDeviceKind::Microphone
        }
    } else if has(&["headphone", "headset", "earbud", "earphone", "airpod"]) {
        AudioDeviceKind::Headphones
    } else if has(&["bluetooth", "bluez"]) {
        AudioDeviceKind::Bluetooth
    } else if has(&["hdmi", "displayport", "display"]) {
        AudioDeviceKind::Display
    } else {
        AudioDeviceKind::Speaker
    }
}

fn is_monitor(v: &Value) -> bool {
    prop(v, "device.class") == "monitor" || str_of(v, "name").ends_with(".monitor")
}

fn devices(
    nodes: &Value,
    default: &str,
    input: bool,
    hidden: Option<&str>,
    product: &str,
) -> Vec<AudioDevice> {
    list(nodes)
        .iter()
        .filter(|v| {
            let name = str_of(v, "name");
            if name.is_empty() {
                return false;
            }
            if name == default {
                return true;
            }
            // A speaker tuning fronts the physical speakers: choosing them
            // would only bypass it (`omarchy-audio-sink-availability`).
            Some(name) != hidden && !(input && is_monitor(v)) && available(v)
        })
        .filter_map(|v| {
            let id = v.get("index").and_then(Value::as_u64)? as u32;
            let name = str_of(v, "name");
            Some(AudioDevice {
                id,
                name: name.to_string(),
                label: device_label(v, product),
                kind: device_kind(v, input),
                default: name == default,
            })
        })
        .collect()
}

/// Omarchy's stream naming: the app's name, with a few made friendlier.
fn stream_label(v: &Value) -> String {
    let app = prop(v, "application.name").trim();
    let generic = app.is_empty() || app.eq_ignore_ascii_case("audio-src");
    let label = if !generic {
        app.to_string()
    } else {
        [
            prop(v, "application.process.binary"),
            prop(v, "media.name"),
            prop(v, "node.name"),
        ]
        .into_iter()
        .map(str::trim)
        .find(|s| !s.is_empty() && !s.eq_ignore_ascii_case("audio-src"))
        .unwrap_or("Stream")
        .to_string()
    };
    // "spotify" → "Spotify"
    if label.chars().any(char::is_uppercase) {
        label
    } else {
        let mut chars = label.chars();
        chars
            .next()
            .map(|c| c.to_uppercase().chain(chars).collect())
            .unwrap_or(label)
    }
}

fn streams(inputs: &Value) -> Vec<AudioStream> {
    list(inputs)
        .iter()
        .filter(|v| {
            // A filter chain's own playback (a speaker tuning) is the
            // processing, not an app.
            let node = prop(v, "node.name");
            !node.starts_with("omarchy_speaker_tuning")
                && !(prop(v, "application.name").is_empty()
                    && !prop(v, "node.link-group").is_empty())
        })
        .filter_map(|v| {
            Some(AudioStream {
                id: v.get("index").and_then(Value::as_u64)? as u32,
                label: stream_label(v),
                volume: volume_percent(v),
                muted: muted(v),
            })
        })
        .collect()
}

struct Snapshot {
    mixer: Mixer,
    mic: Option<(u8, bool)>,
}

fn snapshot(
    info: &Value,
    sinks: &Value,
    sources: &Value,
    sink_inputs: &Value,
    fronted: Option<&str>,
    product: &str,
) -> Snapshot {
    let default_sink = str_of(info, "default_sink_name");
    let default_source = str_of(info, "default_source_name");
    let mic = list(sources)
        .iter()
        .find(|v| !default_source.is_empty() && str_of(v, "name") == default_source)
        .map(|v| (volume_percent(v), muted(v)));
    Snapshot {
        mixer: Mixer {
            outputs: devices(sinks, default_sink, false, fronted, product),
            inputs: devices(sources, default_source, true, None, product),
            streams: streams(sink_inputs),
        },
        mic,
    }
}

#[derive(Default)]
struct Cache {
    /// Sink names the fronted sink was looked up for, and the answer.
    sinks: Vec<String>,
    fronted: Option<String>,
    product: String,
}

fn refresh(bus: &Bus, shared: &Arc<Mutex<Option<String>>>, cache: &mut Cache) {
    let info = pactl_json(&["info"]);
    let sinks = pactl_json(&["list", "sinks"]);
    let sources = pactl_json(&["list", "sources"]);
    let sink_inputs = pactl_json(&["list", "sink-inputs"]);

    let names: Vec<String> = list(&sinks)
        .iter()
        .map(|v| str_of(v, "name").to_string())
        .collect();
    if names != cache.sinks {
        cache.fronted = output("omarchy-audio-tuning", &["fronted-sink"]).filter(|s| !s.is_empty());
        cache.sinks = names;
    }
    let snap = snapshot(
        &info,
        &sinks,
        &sources,
        &sink_inputs,
        cache.fronted.as_deref(),
        &cache.product,
    );

    let audio = sink_name().map(|sink| {
        let node = list(&sinks).iter().find(|v| str_of(v, "name") == sink);
        let volume = node.map(volume_percent).unwrap_or_else(|| {
            output("pactl", &["get-sink-volume", &sink])
                .and_then(|text| {
                    text.split_whitespace()
                        .find(|w| w.ends_with('%'))
                        .and_then(|w| w.trim_end_matches('%').parse::<u32>().ok())
                })
                .unwrap_or(0)
                .min(100) as u8
        });
        let is_muted = node.map(muted).unwrap_or_else(|| {
            output("pactl", &["get-sink-mute", &sink]).is_some_and(|t| t.contains("yes"))
        });
        let label = snap
            .mixer
            .outputs
            .iter()
            .find(|d| d.default)
            .map(|d| d.label.clone())
            .or_else(|| node.map(|v| str_of(v, "description").to_string()))
            .unwrap_or_else(|| sink.clone());
        if let Ok(mut s) = shared.lock() {
            *s = Some(sink);
        }
        Audio {
            volume,
            muted: is_muted,
            sink: label,
        }
    });
    bus.patch(move |s| {
        s.audio = audio;
        s.mic_muted = snap.mic.map(|(_, m)| m);
        s.mic_volume = snap.mic.map(|(v, _)| v);
        s.mixer = snap.mixer;
    });
}

pub fn spawn(bus: Bus, shared: Arc<Mutex<Option<String>>>) -> mpsc::Sender<()> {
    let (tx, rx) = mpsc::channel::<()>();
    let poke = tx.clone();
    thread::Builder::new()
        .name("audio".into())
        .spawn(move || {
            let mut cache = Cache {
                product: std::fs::read_to_string("/sys/class/dmi/id/product_name")
                    .map(|s| s.trim().to_string())
                    .unwrap_or_default(),
                ..Cache::default()
            };
            loop {
                refresh(&bus, &shared, &mut cache);
                let child = Command::new("pactl")
                    .arg("subscribe")
                    .env("LC_ALL", "C")
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .spawn();
                let Ok(mut child) = child else {
                    thread::sleep(Duration::from_secs(3));
                    continue;
                };
                let stdout = child.stdout.take().unwrap();
                let tx = tx.clone();
                let reader = thread::spawn(move || {
                    for line in BufReader::new(stdout).lines() {
                        let Ok(line) = line else { break };
                        if [
                            " on sink #",
                            " on sink-input #",
                            " on server",
                            " on source #",
                            " on card #",
                        ]
                        .iter()
                        .any(|p| line.contains(p))
                            && tx.send(()).is_err()
                        {
                            break;
                        }
                    }
                });
                // Coalesce event bursts into one refresh.
                loop {
                    match rx.recv_timeout(Duration::from_secs(5)) {
                        Ok(()) => {
                            thread::sleep(Duration::from_millis(25));
                            while rx.try_recv().is_ok() {}
                            refresh(&bus, &shared, &mut cache);
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            if reader.is_finished() {
                                break;
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }
                let _ = child.kill();
                let _ = child.wait();
                thread::sleep(Duration::from_secs(2));
            }
        })
        .ok();
    poke
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn port(name: &str, description: &str, availability: &str) -> Value {
        json!({"name": name, "description": description, "availability": availability})
    }

    #[test]
    fn lists_devices_like_omarchy() {
        let info = json!({
            "default_sink_name": "omarchy_speaker_tuning",
            "default_source_name": "alsa_input.pci-0000_00_1f.3.analog-stereo"
        });
        let sinks = json!([
            {"index": 65, "name": "alsa_output.pci-0000_00_1f.3.analog-surround-40",
             "description": "Built-in Audio Analog Surround 4.0", "mute": false,
             "volume": {"front-left": {"value_percent": "62%"}},
             "active_port": "analog-output-speaker",
             "ports": [port("analog-output-speaker", "Speakers", "availability unknown")],
             "properties": {"node.nick": "CS8409/CS42L83 Analog"}},
            {"index": 85, "name": "omarchy_speaker_tuning",
             "description": "MacBookPro14,3 Open Matched Speakers", "mute": false,
             "volume": {"front-left": {"value_percent": "100%"}}, "ports": [], "properties": {}},
            {"index": 90, "name": "bluez_output.AC_80.1", "description": "AirPods Pro",
             "mute": false, "volume": {"mono": {"value_percent": "40%"}}, "ports": [],
             "properties": {"device.icon-name": "audio-headphones-bluetooth"}},
            {"index": 70, "name": "alsa_output.pci-0000_00_1f.3.hdmi-stereo",
             "description": "Built-in Audio Digital Stereo (HDMI)", "mute": false,
             "volume": {}, "active_port": "hdmi-output-0",
             "ports": [port("hdmi-output-0", "HDMI / DisplayPort", "not available")],
             "properties": {}}
        ]);
        let sources = json!([
            {"index": 65, "name": "alsa_output.pci-0000_00_1f.3.analog-surround-40.monitor",
             "description": "Monitor of Built-in Audio", "mute": false, "volume": {},
             "properties": {"device.class": "monitor"}},
            {"index": 66, "name": "alsa_input.pci-0000_00_1f.3.analog-stereo",
             "description": "Built-in Audio Analog Stereo", "mute": true,
             "volume": {"front-left": {"value_percent": "90%"}},
             "active_port": "analog-input-internal-mic",
             "ports": [port("analog-input-internal-mic", "Internal Microphone", "availability unknown"),
                       port("analog-input-mic", "Microphone", "not available")],
             "properties": {"device.class": "sound"}},
            {"index": 91, "name": "bluez_input.AC_80.0", "description": "AirPods Pro",
             "mute": false, "volume": {"mono": {"value_percent": "100%"}}, "ports": [],
             "properties": {"device.form-factor": "headset"}}
        ]);
        let inputs = json!([
            {"index": 86, "mute": false, "volume": {"front-left": {"value_percent": "100%"}},
             "properties": {"node.name": "omarchy_speaker_tuning.playback",
                            "node.link-group": "filter-chain-1127-8"}},
            {"index": 120, "mute": false, "volume": {"front-left": {"value_percent": "80%"}},
             "properties": {"application.name": "spotify"}},
            {"index": 133, "mute": true, "volume": {"front-left": {"value_percent": "55%"}},
             "properties": {"application.name": "audio-src",
                            "application.process.binary": "chromium"}}
        ]);
        let fronted = "alsa_output.pci-0000_00_1f.3.analog-surround-40";
        let snap = snapshot(
            &info,
            &sinks,
            &sources,
            &inputs,
            Some(fronted),
            "MacBookPro14,3",
        );

        let outputs: Vec<_> = snap
            .mixer
            .outputs
            .iter()
            .map(|d| (d.id, d.label.as_str(), d.kind, d.default))
            .collect();
        assert_eq!(
            outputs,
            [
                (85, "Open Matched Speakers", AudioDeviceKind::Speaker, true),
                (90, "AirPods Pro", AudioDeviceKind::Headphones, false),
            ]
        );
        let inputs: Vec<_> = snap
            .mixer
            .inputs
            .iter()
            .map(|d| (d.id, d.label.as_str(), d.kind, d.default))
            .collect();
        assert_eq!(
            inputs,
            [
                (66, "Internal Microphone", AudioDeviceKind::Microphone, true),
                (91, "AirPods Pro", AudioDeviceKind::Headphones, false),
            ]
        );
        assert_eq!(snap.mic, Some((90, true)));
        let streams: Vec<_> = snap
            .mixer
            .streams
            .iter()
            .map(|s| (s.id, s.label.as_str(), s.volume, s.muted))
            .collect();
        assert_eq!(
            streams,
            [(120, "Spotify", 80, false), (133, "Chromium", 55, true)]
        );
    }
}
