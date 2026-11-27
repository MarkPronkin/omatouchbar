//! Executes scene actions.
//!
//! T1Bridge starts renderers with `NoNewPrivileges`, which every child
//! would inherit — so commands the user launches (menu entries, updates,
//! settings) run as transient units of the user manager via
//! `systemd-run --user`, exactly as if started from the desktop. Quick
//! status helpers (pactl, hyprctl, nmcli) are spawned directly.

use std::{
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex, mpsc::Sender},
    thread,
};

use omatouchbar_core::{
    Config, menu,
    scene::{Action, Guard, Panel},
    state::{DnsProvider, Indicator},
};
use t1hw::Features;

use crate::{
    bus::{Bus, Event},
    hw::Hw,
    providers::{agents, dbus, display, network, speedtest, system, tray, weather, wifi},
};

/// Nudges a provider to refresh now.
#[derive(Clone, Default)]
pub struct Refresh {
    pub network: Option<Sender<()>>,
    pub bluetooth: Option<Sender<()>>,
    pub brightness: Option<Sender<()>>,
    pub audio: Option<Sender<()>>,
    pub agents: Option<Sender<agents::Request>>,
    pub tray: Option<Sender<()>>,
    pub indicators: Option<Sender<system::IndicatorRefresh>>,
}

pub struct Exec {
    pub haptics: crate::haptics::Player,
    children: Vec<Child>,
    world: omatouchbar_core::world_clock::Worker,
    pub weather: weather::Provider,
    pub display: display::Provider,
    pub network_details: network::DetailsProvider,
    pub wifi: wifi::WifiConnector,
    pub speedtest: speedtest::SpeedTestRunner,
    pub bluetooth_scan: dbus::BluetoothScan,
    /// Omarchy's screensaver on the strip.
    pub screensaver: crate::screensaver::Mirror,
    bus: Bus,
    /// The PipeWire sink Omarchy's volume commands control.
    pub sink: Arc<Mutex<Option<String>>>,
    pub refresh: Refresh,
}

impl Exec {
    pub fn new(bus: Bus, size: (u32, u32), haptics_enabled: bool) -> Exec {
        Exec {
            haptics: crate::haptics::Player::new(haptics_enabled),
            screensaver: crate::screensaver::Mirror::new(bus.clone(), size),
            children: Vec::new(),
            world: {
                let bus = bus.clone();
                omatouchbar_core::world_clock::Worker::new(false, move |frame| {
                    bus.send(Event::WorldClock(frame))
                })
            },
            weather: weather::Provider::start(bus.clone()),
            display: display::Provider::start(bus.clone()),
            network_details: network::DetailsProvider::start(bus.clone()),
            wifi: wifi::WifiConnector::new(),
            speedtest: speedtest::SpeedTestRunner::new(),
            bluetooth_scan: dbus::BluetoothScan::start(),
            bus,
            sink: Arc::new(Mutex::new(None)),
            refresh: Refresh::default(),
        }
    }

    fn spawn(&mut self, program: &str, args: &[&str]) {
        match Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => self.children.push(child),
            Err(e) => log::warn!("{program}: {e}"),
        }
    }

    /// Runs a user command outside the renderer's sandbox.
    ///
    /// Uses `ExitType=cgroup` so commands that fork and exit their initial
    /// process (such as Omarchy launchers calling `exec setsid uwsm-app ...`,
    /// or `wl-copy`) are not immediately killed by systemd before their child
    /// finishes or moves into its own scope.
    fn launch(&mut self, command: &str) {
        log::info!("run: {command}");
        match Command::new("systemd-run")
            .args(systemd_run_launch_args(command))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => self.children.push(child),
            Err(e) => {
                log::warn!("systemd-run unavailable ({e}); running directly");
                self.spawn("bash", &["-lc", command]);
            }
        }
    }

    /// Reaps finished children.
    pub fn reap(&mut self) {
        self.children
            .retain_mut(|child| matches!(child.try_wait(), Ok(None)));
    }

    fn sink(&self) -> String {
        self.sink
            .lock()
            .ok()
            .and_then(|s| s.clone())
            .unwrap_or_else(|| "@DEFAULT_SINK@".into())
    }

    pub fn run(&mut self, action: Action, hw: &mut Hw, config: &Config) {
        log::debug!("action {action:?}");
        match action {
            Action::Key(code) => {
                if let Some(client) = hw.client()
                    && let Err(e) = client.tap_keys(&[code])
                {
                    log::warn!("tap key {code}: {e}");
                }
            }
            Action::Shell(command) => self.launch(&command),
            Action::FocusWorkspace {
                id,
                slot,
                monitor,
                same_block,
            } => {
                let expression = if same_block && !monitor.is_empty() {
                    format!(
                        "o.workspaces.focus({slot}, \"{}\")",
                        monitor.replace('"', "")
                    )
                } else {
                    format!("hl.dsp.focus({{ workspace = \"{id}\" }})")
                };
                self.spawn("hyprctl", &["dispatch", &expression]);
            }
            Action::SetVolume(value) => {
                let sink = self.sink();
                let volume = format!("{value}%");
                self.spawn(
                    "sh",
                    &[
                        "-c",
                        "pactl set-sink-mute \"$1\" 0; pactl set-sink-volume \"$1\" \"$2\"",
                        "sh",
                        &sink,
                        &volume,
                    ],
                );
                if config.osd {
                    self.spawn(
                        "omarchy-osd",
                        &["-i", "volume-high", "-p", &value.to_string()],
                    );
                }
            }
            Action::ToggleMute => {
                if config.osd {
                    self.spawn("omarchy-audio-output-volume", &["mute-toggle"]);
                } else {
                    let sink = self.sink();
                    self.spawn("pactl", &["set-sink-mute", &sink, "toggle"]);
                }
            }
            Action::StepVolume(up) => self.spawn(
                "omarchy-audio-output-volume",
                &[if up { "raise" } else { "lower" }],
            ),
            Action::ToggleMicMute => {
                self.spawn("pactl", &["set-source-mute", "@DEFAULT_SOURCE@", "toggle"])
            }
            Action::SetMicVolume(value) => self.spawn(
                "pactl",
                &[
                    "set-source-volume",
                    "@DEFAULT_SOURCE@",
                    &format!("{value}%"),
                ],
            ),
            // Omarchy's helpers also move playing (recording) streams to the
            // new device, as choosing one in its audio panel does.
            Action::SetDefaultOutput { id, name } => self.spawn(
                "sh",
                &[
                    "-c",
                    "if command -v omarchy-audio-output-set-default >/dev/null; then \
                     exec omarchy-audio-output-set-default \"$1\" \"$2\"; \
                     else exec pactl set-default-sink \"$2\"; fi",
                    "sh",
                    &id.to_string(),
                    &name,
                ],
            ),
            Action::SetDefaultInput { id, name } => self.spawn(
                "sh",
                &[
                    "-c",
                    "if command -v omarchy-audio-input-set-default >/dev/null; then \
                     exec omarchy-audio-input-set-default \"$1\" \"$2\"; \
                     else exec pactl set-default-source \"$2\"; fi",
                    "sh",
                    &id.to_string(),
                    &name,
                ],
            ),
            Action::SetStreamVolume { id, volume } => self.spawn(
                "pactl",
                &[
                    "set-sink-input-volume",
                    &id.to_string(),
                    &format!("{volume}%"),
                ],
            ),
            Action::ToggleStreamMute(id) => {
                self.spawn("pactl", &["set-sink-input-mute", &id.to_string(), "toggle"])
            }
            Action::MediaPlayPause => self.spawn("omarchy-shell", &["-q", "media", "playPause"]),
            Action::MediaNext => self.spawn("omarchy-shell", &["-q", "media", "next"]),
            Action::MediaPrevious => self.spawn("omarchy-shell", &["-q", "media", "previous"]),
            Action::SetDisplayBrightness(value) => {
                let via_hw = hw.has(Features::DISPLAY_BRIGHTNESS)
                    && hw
                        .client()
                        .map(|c| c.set_display_brightness(value).is_ok())
                        .unwrap_or(false);
                if !via_hw {
                    self.spawn("brightnessctl", &["-q", "set", &format!("{value}%")]);
                }
                if config.osd {
                    self.spawn(
                        "omarchy-osd",
                        &["-i", "brightness", "-p", &value.to_string()],
                    );
                }
                self.nudge_brightness();
            }
            Action::ConfigureDisplay(change) => self.display.change(change, config.osd),
            Action::SetKeyboardBacklight(value) => {
                let via_hw = hw.has(Features::KEYBOARD_BACKLIGHT)
                    && hw
                        .client()
                        .map(|c| c.set_keyboard_backlight(value).is_ok())
                        .unwrap_or(false);
                if !via_hw {
                    self.spawn(
                        "brightnessctl",
                        &["-q", "-d", "*kbd_backlight", "set", &format!("{value}%")],
                    );
                }
                if config.osd {
                    self.spawn("omarchy-osd", &["-i", "keyboard", "-p", &value.to_string()]);
                }
                self.nudge_brightness();
            }
            Action::StepDisplayBrightness(up) => {
                let via_hw = hw.has(Features::DISPLAY_BRIGHTNESS)
                    && hw
                        .client()
                        .map(|c| c.step_display_brightness(up).is_ok())
                        .unwrap_or(false);
                if !via_hw {
                    self.spawn(
                        "brightnessctl",
                        &["-q", "set", if up { "+6%" } else { "6%-" }],
                    );
                }
                self.nudge_brightness();
            }
            Action::StepKeyboardBacklight(up) => {
                let via_hw = hw.has(Features::KEYBOARD_BACKLIGHT)
                    && hw
                        .client()
                        .map(|c| c.step_keyboard_backlight(up).is_ok())
                        .unwrap_or(false);
                if !via_hw {
                    self.spawn(
                        "brightnessctl",
                        &[
                            "-q",
                            "-d",
                            "*kbd_backlight",
                            "set",
                            if up { "+6%" } else { "6%-" },
                        ],
                    );
                }
                self.nudge_brightness();
            }
            Action::SetPowerProfile(profile) => {
                self.spawn("omarchy-powerprofiles-set", &["autodetect", &profile])
            }
            Action::SetWifi(on) => {
                self.spawn("nmcli", &["radio", "wifi", if on { "on" } else { "off" }]);
                if let Some(tx) = &self.refresh.network {
                    let _ = tx.send(());
                }
            }
            Action::SetBluetooth(on) => {
                self.spawn("omarchy-bluetooth-power", &[if on { "on" } else { "off" }])
            }
            Action::ConnectWifi {
                ssid,
                known: _,
                secure: _,
            } => {
                self.wifi.connect_known_or_open(
                    ssid,
                    self.bus.clone(),
                    self.refresh.network.clone(),
                );
            }
            Action::ConnectWifiPassword { ssid, password } => {
                self.wifi.connect_with_password(
                    ssid,
                    password,
                    self.bus.clone(),
                    self.refresh.network.clone(),
                );
            }
            Action::CancelWifiConnect => self.wifi.cancel(),
            Action::StartSpeedTest => self.speedtest.start(self.bus.clone()),
            Action::StopSpeedTest => self.speedtest.stop(),
            Action::SetDns(DnsProvider::Custom) => {
                self.launch(
                    "omarchy-launch-floating-terminal-with-presentation 'omarchy-dns Custom'",
                );
            }
            Action::SetDns(provider) => {
                self.network_details.set_dns(provider);
            }
            Action::SetWifiBand(band) => {
                self.network_details
                    .set_band(band, self.refresh.network.clone());
            }
            Action::CopyToClipboard(value) => {
                self.launch(&format!(
                    "printf %s {} | wl-copy",
                    menu::shell_quote(&value)
                ));
            }
            Action::ShareWifiQr { iface, ssid } => {
                let mut payload = serde_json::Map::new();
                if !iface.is_empty() {
                    payload.insert("iface".into(), serde_json::Value::String(iface));
                }
                if !ssid.is_empty() {
                    payload.insert("ssid".into(), serde_json::Value::String(ssid));
                }
                let json = serde_json::Value::Object(payload).to_string();
                self.launch(&format!(
                    "omarchy-shell shell summon omarchy.wifiqr {}",
                    menu::shell_quote(&json)
                ));
            }
            Action::OpenCaptivePortal => {
                self.launch("omarchy-network-captive-portal http://ping.omacom.io/nm-check.txt");
            }
            Action::BluetoothDevice { address, op } => {
                dbus::run_bluetooth_op(self.bus.clone(), address, op)
            }
            Action::ActivateTray {
                address,
                context_menu,
            } => tray::activate(address, context_menu),
            Action::LoadTrayMenu(menu) => tray::request_menu(self.bus.clone(), menu),
            Action::ClickTrayMenu { menu, id } => tray::click(menu, id),
            Action::WorldClock(request) => self.world.send(request),
            Action::RefreshWeather => self.weather.refresh(),
            Action::SearchWeatherLocation(query) => self.weather.search(query),
            Action::SetWeatherLocation(location) => self.weather.set_location(location),
            Action::RefreshAgents { disabled } => {
                if let Some(tx) = &self.refresh.agents {
                    let _ = tx.send(agents::Request::Refresh(disabled));
                }
            }
            Action::CycleKeyboardLayout => {
                self.spawn("omarchy-shell", &["-q", "shell", "cycleInput", "next"])
            }
            Action::TriggerIndicator {
                indicator,
                currently_active,
            } => self.trigger_indicator(indicator, currently_active),
            Action::SetReminder => self.launch("omarchy-reminder -i"),
            Action::ClearReminders => {
                self.run_and_refresh_indicators("omarchy-reminder", &["clear"], false);
            }
            Action::CancelTouchId => {
                if let Some(client) = hw.client() {
                    let _ = client.cancel_touch_id();
                }
            }
            Action::DismissScreensaver => self.screensaver.dismiss(),
            Action::EvaluateGuards { menu, guards } => self.evaluate_guards(menu, guards),
            Action::LoadProvider { menu, provider } => self.load_provider(menu, provider),
            Action::PanelOpened(panel) => {
                let target = match panel {
                    Panel::Network => &self.refresh.network,
                    Panel::Bluetooth => &self.refresh.bluetooth,
                    Panel::Brightness => &self.refresh.brightness,
                    Panel::Audio => &self.refresh.audio,
                    Panel::Agents => {
                        if let Some(tx) = &self.refresh.agents {
                            let _ = tx.send(agents::Request::Read);
                        }
                        return;
                    }
                    Panel::Indicators => {
                        if let Some(tx) = &self.refresh.indicators {
                            let _ = tx.send(system::IndicatorRefresh::Full);
                        }
                        return;
                    }
                    Panel::Tray => &self.refresh.tray,
                    Panel::Weather => {
                        self.weather.opened();
                        return;
                    }
                    Panel::Power | Panel::Clock => &None,
                };
                if let Some(tx) = target {
                    let _ = tx.send(());
                }
            }
        }
    }

    fn nudge_brightness(&self) {
        if let Some(tx) = &self.refresh.brightness {
            let _ = tx.send(());
        }
    }

    fn run_and_refresh_indicators(
        &self,
        program: &'static str,
        args: &'static [&'static str],
        full: bool,
    ) {
        let tx = self.refresh.indicators.clone();
        thread::spawn(move || {
            let _ = Command::new(program)
                .args(args)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            if let Some(tx) = tx {
                let _ = tx.send(if full {
                    system::IndicatorRefresh::Full
                } else {
                    system::IndicatorRefresh::Fast
                });
            }
        });
    }

    fn trigger_indicator(&mut self, indicator: Indicator, currently_active: bool) {
        match indicator {
            Indicator::PasswordlessSudo => {
                if currently_active {
                    let tx = self.refresh.indicators.clone();
                    thread::spawn(move || {
                        system::disable_passwordless_sudo();
                        if let Some(tx) = tx {
                            let _ = tx.send(system::IndicatorRefresh::Full);
                        }
                    });
                } else {
                    self.launch(
                        "omarchy-launch-floating-terminal-with-presentation omarchy-sudo-passwordless",
                    );
                }
            }
            Indicator::ScreenRecording => {
                if currently_active {
                    self.run_and_refresh_indicators(
                        "omarchy-capture-screenrecording",
                        &["--stop-recording"],
                        false,
                    );
                } else {
                    self.launch("omarchy-menu toggle trigger.capture.screenrecord");
                }
            }
            Indicator::RemoteSession => {}
            Indicator::Reminder => {
                if currently_active {
                    self.launch("omarchy-reminder show");
                } else {
                    self.launch("omarchy-reminder -i");
                }
            }
            Indicator::NightLight => {
                self.run_and_refresh_indicators("omarchy-toggle-nightlight", &[], false);
            }
            Indicator::Dnd => {
                self.run_and_refresh_indicators(
                    "omarchy-toggle-notification-silencing",
                    &[],
                    false,
                );
            }
            Indicator::StayAwake => {
                let args: &'static [&'static str] = if currently_active {
                    &["allow-idle"]
                } else {
                    &["stay-awake"]
                };
                self.run_and_refresh_indicators("omarchy-toggle-idle", args, false);
            }
        }
    }

    fn evaluate_guards(&self, menu: String, guards: Vec<Guard>) {
        let bus = self.bus.clone();
        thread::spawn(move || {
            let mut script = String::new();
            for (i, guard) in guards.iter().enumerate() {
                script.push_str(&format!("if ( {} ) >/dev/null 2>&1 </dev/null; then echo \"{i} 1\"; else echo \"{i} 0\"; fi\n", guard.expr));
            }
            let output = match Command::new("bash")
                .args(["-lc", &script])
                .stdin(Stdio::null())
                .stderr(Stdio::null())
                .output()
            {
                Ok(output) => output,
                Err(e) => {
                    log::warn!("menu guards: {e}");
                    return;
                }
            };
            let text = String::from_utf8_lossy(&output.stdout);
            let results = text
                .lines()
                .filter_map(|line| {
                    let (index, value) = line.split_once(' ')?;
                    let guard = guards.get(index.parse::<usize>().ok()?)?;
                    Some((guard.id.clone(), guard.kind, value.trim() == "1"))
                })
                .collect();
            bus.send(Event::Guards { menu, results });
        });
    }

    fn load_provider(&self, menu: String, provider: String) {
        let Some((script, _)) = menu::provider_script(&provider) else {
            return;
        };
        let bus = self.bus.clone();
        thread::spawn(move || {
            let Ok(output) = Command::new("bash")
                .args(["-lc", script])
                .stdin(Stdio::null())
                .stderr(Stdio::null())
                .output()
            else {
                return;
            };
            let text = String::from_utf8_lossy(&output.stdout);
            let rows = text
                .lines()
                .filter_map(|line| {
                    let mut fields = line.split('\t');
                    let label = fields.next()?.to_string();
                    let value = fields.next()?.to_string();
                    let current = fields.next().unwrap_or("");
                    (!label.is_empty()).then(|| (label, value.clone(), value == current))
                })
                .take(200)
                .collect();
            bus.send(Event::ProviderRows {
                menu,
                provider,
                rows,
            });
        });
    }
}

fn systemd_run_launch_args(command: &str) -> [&str; 9] {
    [
        "--user",
        "--collect",
        "--quiet",
        "--no-block",
        "--property=ExitType=cgroup",
        "--",
        "bash",
        "-lc",
        command,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_uses_cgroup_exit_type_for_forking_launchers() {
        let args = systemd_run_launch_args(
            "omarchy-launch-floating-terminal-with-presentation omarchy-sudo-passwordless",
        );
        assert!(args.contains(&"--property=ExitType=cgroup"));
        assert_eq!(
            args.last(),
            Some(&"omarchy-launch-floating-terminal-with-presentation omarchy-sudo-passwordless")
        );
    }
}
