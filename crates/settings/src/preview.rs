//! A live, interactive Touch Bar preview running the real scene.
//!
//! Mouse presses become touches, so menus, panels, sliders and the
//! workspace scrubber all work; desktop actions are simulated on sample
//! data instead of being executed.

use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};

use adw::prelude::*;
use chrono::Utc;
use gtk::{gdk, glib};
use omatouchbar_core::{
    Canvas, Config, DesktopState, Fonts, KeyInput, Palette, Scene, SpeedTestEvent, T1_HEIGHT,
    T1_WIDTH,
    display::DisplayChange,
    menu::MenuTree,
    omarchy::BarLayout,
    scene::{Action, Command},
    state::{BtOp, DnsProvider, Indicator},
};

struct Inner {
    scene: Scene,
    world: omatouchbar_core::world_clock::Worker,
    world_frames: std::sync::mpsc::Receiver<omatouchbar_core::world_clock::Frame>,
    canvas: Canvas,
    pressed: bool,
    fn_held: bool,
    pointer: (f32, f32),
    dirty: bool,
    sim_speedtest: Option<Instant>,
    sim_stage: u8,
    sim_wifi: Option<(String, Instant)>,
    /// Bluetooth operations finishing at the instant.
    sim_bt: Vec<(String, BtOp, Instant)>,
}

#[derive(Clone)]
pub struct Preview {
    pub widget: gtk::Box,
    inner: Rc<RefCell<Inner>>,
    picture: gtk::Picture,
}

fn new_scene(config: Config, palette: Palette) -> Scene {
    let mut scene = Scene::new(
        T1_WIDTH,
        T1_HEIGHT,
        config,
        palette,
        Fonts::system(None),
        MenuTree::load(),
        BarLayout::from_omarchy(),
    );
    let model = std::fs::read_to_string("/sys/class/dmi/id/product_name").unwrap_or_default();
    scene.set_state(DesktopState::sample_for_model(&model));
    scene
}

impl Preview {
    pub fn new(config: &Config, toasts: &adw::ToastOverlay) -> Preview {
        let palette = Palette::load().unwrap_or_else(Palette::fallback);
        let (tx, world_frames) = std::sync::mpsc::channel();
        let world = omatouchbar_core::world_clock::Worker::new(true, move |frame| {
            let _ = tx.send(frame);
        });
        let inner = Rc::new(RefCell::new(Inner {
            world,
            world_frames,
            scene: new_scene(config.clone(), palette),
            canvas: Canvas::new(T1_WIDTH, T1_HEIGHT),
            pressed: false,
            fn_held: false,
            pointer: (0.0, 0.0),
            dirty: true,
            sim_speedtest: None,
            sim_stage: 0,
            sim_wifi: None,
            sim_bt: Vec::new(),
        }));

        let picture = gtk::Picture::new();
        picture.set_content_fit(gtk::ContentFit::Fill);
        picture.set_can_shrink(true);
        picture.set_cursor_from_name(Some("pointer"));
        let frame = gtk::AspectFrame::new(0.5, 0.5, T1_WIDTH as f32 / T1_HEIGHT as f32, false);
        frame.set_child(Some(&picture));
        frame.set_hexpand(true);

        let bezel = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        bezel.add_css_class("touchbar-bezel");
        bezel.append(&frame);

        // Pointer → touch.
        let drag = gtk::GestureDrag::new();
        {
            let inner = inner.clone();
            let picture = picture.clone();
            drag.connect_drag_begin(move |_, x, y| {
                let mut i = inner.borrow_mut();
                let p = map(&picture, x, y);
                i.pressed = true;
                i.pointer = p;
                let fn_held = i.fn_held;
                i.scene.input(fn_held, &[(0, p.0, p.1)], Instant::now());
                i.dirty = true;
            });
        }
        {
            let inner = inner.clone();
            let picture = picture.clone();
            drag.connect_drag_update(move |gesture, dx, dy| {
                let Some((sx, sy)) = gesture.start_point() else {
                    return;
                };
                let mut i = inner.borrow_mut();
                let p = map(&picture, sx + dx, sy + dy);
                i.pointer = p;
                let fn_held = i.fn_held;
                i.scene.input(fn_held, &[(0, p.0, p.1)], Instant::now());
                i.dirty = true;
            });
        }
        {
            let inner = inner.clone();
            drag.connect_drag_end(move |_, _, _| {
                let mut i = inner.borrow_mut();
                i.pressed = false;
                let fn_held = i.fn_held;
                i.scene.input(fn_held, &[], Instant::now());
                i.dirty = true;
            });
        }
        picture.add_controller(drag);

        // Render on the frame clock; upload only changed frames.
        {
            let inner = inner.clone();
            let toasts = toasts.clone();
            picture.add_tick_callback(move |picture, _| {
                let mut i = inner.borrow_mut();
                let now = Instant::now();
                while let Ok(frame) = i.world_frames.try_recv() {
                    i.scene.apply_world_clock(frame);
                }
                step_simulations(&mut i, now);
                let changed = i.scene.update(now, Utc::now());
                let actions = i.scene.take_actions();
                for action in actions {
                    simulate(&mut i, action, &toasts);
                }
                if changed || i.dirty {
                    i.dirty = false;
                    let Inner { scene, canvas, .. } = &mut *i;
                    scene.render(canvas);
                    let bytes = glib::Bytes::from(canvas.rgba());
                    let texture = gdk::MemoryTexture::new(
                        T1_WIDTH as i32,
                        T1_HEIGHT as i32,
                        gdk::MemoryFormat::R8g8b8a8Premultiplied,
                        &bytes,
                        (T1_WIDTH * 4) as usize,
                    );
                    picture.set_paintable(Some(&texture));
                }
                glib::ControlFlow::Continue
            });
        }

        let widget = gtk::Box::new(gtk::Orientation::Vertical, 0);
        widget.append(&bezel);
        Preview {
            widget,
            inner,
            picture,
        }
    }

    pub fn set_config(&self, config: Config) {
        let mut i = self.inner.borrow_mut();
        i.scene.set_config(config);
        i.dirty = true;
    }

    /// The config the preview is showing (which the tour may vary).
    pub fn config(&self) -> Config {
        self.inner.borrow().scene.config().clone()
    }

    pub fn picture(&self) -> gtk::Picture {
        self.picture.clone()
    }

    /// Moves the preview into `slot`, wherever it was before.
    pub fn place(&self, slot: &gtk::Box) {
        if self.widget.parent().as_ref() == Some(slot.upcast_ref()) {
            return;
        }
        if let Some(parent) = self.widget.parent() {
            if let Some(parent) = parent.downcast_ref::<gtk::Box>() {
                parent.remove(&self.widget);
            } else {
                self.widget.unparent();
            }
        }
        slot.append(&self.widget);
    }

    pub fn set_palette(&self, palette: Palette) {
        self.inner.borrow_mut().scene.set_palette(palette);
    }

    pub fn set_fn(&self, held: bool) {
        let mut i = self.inner.borrow_mut();
        i.fn_held = held;
        let contacts: Vec<(u8, f32, f32)> = if i.pressed {
            vec![(0, i.pointer.0, i.pointer.1)]
        } else {
            vec![]
        };
        i.scene.input(held, &contacts, Instant::now());
    }

    pub fn command(&self, command: Command) {
        self.inner.borrow_mut().scene.command(command);
    }

    pub fn handle_key(&self, key: gdk::Key, state: gdk::ModifierType) -> bool {
        let mut i = self.inner.borrow_mut();
        if !i.scene.wants_keyboard() {
            return false;
        }
        let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
        let input = match key {
            gdk::Key::Up => Some(KeyInput::Select(-1)),
            gdk::Key::Down => Some(KeyInput::Select(1)),
            gdk::Key::Escape => Some(KeyInput::Escape),
            gdk::Key::Return | gdk::Key::KP_Enter => Some(KeyInput::Enter),
            gdk::Key::BackSpace => Some(if ctrl {
                KeyInput::Clear
            } else {
                KeyInput::Backspace
            }),
            _ if ctrl && matches!(key.to_unicode(), Some('u' | 'U')) => Some(KeyInput::Clear),
            _ if !ctrl => key
                .to_unicode()
                .filter(|c| !c.is_control() && *c != '\u{7f}')
                .map(|c| KeyInput::Text(c.to_string())),
            _ => None,
        };
        if let Some(input) = input {
            i.scene.key_input(input, Instant::now());
            i.dirty = true;
            true
        } else {
            false
        }
    }

    /// Rebuilds the scene so entrance animations play again.
    pub fn replay(&self) {
        let mut i = self.inner.borrow_mut();
        let config = i.scene.config().clone();
        let palette = Palette::load().unwrap_or_else(Palette::fallback);
        i.scene = new_scene(config, palette);
        i.dirty = true;
    }
}

fn map(picture: &gtk::Picture, x: f64, y: f64) -> (f32, f32) {
    let w = picture.width().max(1) as f64;
    let h = picture.height().max(1) as f64;
    (
        (x / w * T1_WIDTH as f64) as f32,
        (y / h * T1_HEIGHT as f64) as f32,
    )
}

fn step_simulations(inner: &mut Inner, now: Instant) {
    let (due, waiting): (Vec<_>, Vec<_>) = std::mem::take(&mut inner.sim_bt)
        .into_iter()
        .partition(|(_, _, at)| now >= *at);
    inner.sim_bt = waiting;
    for (address, op, _) in due {
        let bt = &mut inner.scene.state_mut().bluetooth;
        match op {
            BtOp::Pair => {
                if let Some(i) = bt.nearby.iter().position(|d| d.address == address) {
                    let mut device = bt.nearby.remove(i);
                    device.paired = true;
                    device.connected = true;
                    bt.devices.push(device);
                }
            }
            BtOp::Connect | BtOp::Disconnect => {
                if let Some(d) = bt.devices.iter_mut().find(|d| d.address == address) {
                    d.connected = op == BtOp::Connect;
                }
            }
            BtOp::Forget => bt.devices.retain(|d| d.address != address),
        }
        bt.devices.sort_by(|a, b| {
            b.connected
                .cmp(&a.connected)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        bt.connected = bt
            .devices
            .iter()
            .filter(|d| d.connected)
            .map(|d| d.name.clone())
            .collect();
        inner.scene.apply_bluetooth_done(&address, op, true);
        inner.dirty = true;
    }
    if let Some((ssid, at)) = inner.sim_wifi.clone()
        && now >= at
    {
        inner.sim_wifi = None;
        inner.scene.apply_wifi_connect_result(&ssid, Ok(()), false);
        let s = inner.scene.state_mut();
        for net in &mut s.network.networks {
            net.active = net.ssid == ssid;
            if net.active {
                net.known = true;
            }
        }
        s.network.name = ssid;
        inner.dirty = true;
    }
    if let Some(t0) = inner.sim_speedtest {
        let sec = now.saturating_duration_since(t0).as_secs_f32();
        if inner.sim_stage == 0 && sec >= 0.45 {
            inner.sim_stage = 1;
            inner
                .scene
                .apply_speedtest_event(SpeedTestEvent::Ping(11.2));
        } else if inner.sim_stage == 1 && sec >= 1.0 && sec < 4.5 {
            let u = (sec - 1.0) / 3.5;
            let mbps = 460.0 * (1.0 - (-u * 3.8).exp()) + (sec * 5.2).sin() * 24.0;
            inner
                .scene
                .apply_speedtest_event(SpeedTestEvent::DownloadSample(mbps.max(12.0)));
        } else if inner.sim_stage == 1 && sec >= 4.5 {
            inner.sim_stage = 2;
            inner
                .scene
                .apply_speedtest_event(SpeedTestEvent::DownloadDone(468.4));
            inner
                .scene
                .apply_speedtest_event(SpeedTestEvent::UploadStart);
        } else if inner.sim_stage == 2 && sec >= 4.7 && sec < 8.0 {
            let u = (sec - 4.7) / 3.3;
            let mbps = 215.0 * (1.0 - (-u * 3.6).exp()) + (sec * 6.1).cos() * 14.0;
            inner
                .scene
                .apply_speedtest_event(SpeedTestEvent::UploadSample(mbps.max(8.0)));
        } else if inner.sim_stage == 2 && sec >= 8.0 {
            inner.sim_stage = 3;
            inner
                .scene
                .apply_speedtest_event(SpeedTestEvent::UploadDone(218.7));
            inner.sim_speedtest = None;
        }
    }
}

/// Places the preview's location search finds.
fn sample_places(query: &str) -> Vec<omatouchbar_core::weather::Location> {
    const PLACES: [(&str, &str, f64, f64, &str); 10] = [
        ("Berlin", "Land Berlin, Germany", 52.52, 13.41, "DE"),
        ("Copenhagen", "Capital Region, Denmark", 55.68, 12.57, "DK"),
        (
            "Kaliningrad",
            "Kaliningrad Oblast, Russia",
            54.71,
            20.51,
            "RU",
        ),
        ("Lisbon", "Lisbon, Portugal", 38.72, -9.14, "PT"),
        ("London", "England, United Kingdom", 51.51, -0.13, "GB"),
        (
            "Los Angeles",
            "California, United States",
            34.05,
            -118.24,
            "US",
        ),
        ("New York", "New York, United States", 40.71, -74.01, "US"),
        ("Paris", "Île-de-France, France", 48.85, 2.35, "FR"),
        (
            "San Francisco",
            "California, United States",
            37.77,
            -122.42,
            "US",
        ),
        ("Tokyo", "Tokyo, Japan", 35.69, 139.69, "JP"),
    ];
    let query = query.trim().to_lowercase();
    PLACES
        .iter()
        .filter(|(name, ..)| {
            name.to_lowercase()
                .split(' ')
                .any(|word| word.starts_with(&query))
                || name.to_lowercase().starts_with(&query)
        })
        .map(
            |&(name, region, lat, lon, country)| omatouchbar_core::weather::Location {
                name: name.into(),
                region: region.into(),
                coords: Some((lat, lon)),
                country: Some(country.into()),
            },
        )
        .collect()
}

/// Applies an action to the sample state so the preview reacts.
fn simulate(inner: &mut Inner, action: Action, toasts: &adw::ToastOverlay) {
    match action {
        Action::WorldClock(request) => {
            inner.world.send(request);
            return;
        }
        Action::ConnectWifiPassword { ssid, .. } => {
            inner.sim_wifi = Some((ssid, Instant::now() + Duration::from_millis(1200)));
            return;
        }
        Action::CancelWifiConnect => {
            inner.sim_wifi = None;
            return;
        }
        Action::SearchWeatherLocation(query) => {
            let found = sample_places(&query);
            inner.scene.apply_weather_suggestions(&query, Some(found));
            return;
        }
        Action::StartSpeedTest => {
            inner.sim_speedtest = Some(Instant::now());
            inner.sim_stage = 0;
            return;
        }
        Action::StopSpeedTest => {
            inner.sim_speedtest = None;
            inner.sim_stage = 0;
            return;
        }
        Action::BluetoothDevice { address, op } => {
            let wait = if op == BtOp::Pair { 2200 } else { 1200 };
            inner
                .sim_bt
                .push((address, op, Instant::now() + Duration::from_millis(wait)));
            return;
        }
        _ => {}
    }
    let scene = &mut inner.scene;
    let s = scene.state_mut();
    match action {
        Action::SetVolume(v) => {
            if let Some(a) = s.audio.as_mut() {
                a.volume = v;
                a.muted = false;
            }
        }
        Action::StepVolume(up) => {
            if let Some(a) = s.audio.as_mut() {
                a.volume = if up {
                    (a.volume + 5).min(100)
                } else {
                    a.volume.saturating_sub(5)
                };
            }
        }
        Action::ConfigureDisplay(change) => {
            let Some(panel) = &mut s.display_panel else {
                return;
            };
            match change {
                DisplayChange::TextSize(size) => panel.text_size = size,
                DisplayChange::Scale { monitor, preset } => {
                    if let Some((_, effective)) = panel
                        .focused()
                        .filter(|m| m.name == monitor)
                        .and_then(|m| m.scales().into_iter().find(|(p, _)| *p == preset))
                    {
                        panel.scale = Some(f64::from(effective) / 100.0);
                    }
                }
                DisplayChange::Brightness { monitor, percent } => {
                    if panel.focused().is_some_and(|m| m.name == monitor) {
                        panel.brightness = Some(percent);
                        if monitor == "eDP-1" {
                            s.display_brightness = Some(percent);
                        }
                    }
                }
                DisplayChange::Enabled { monitor, enabled } => {
                    if panel.can_set_enabled(&monitor, enabled) {
                        if let Some(m) = panel.monitors.iter_mut().find(|m| m.name == monitor) {
                            m.enabled = enabled;
                            m.focused &= enabled;
                        }
                        if panel.focused().is_none()
                            && let Some(next) = panel.monitors.iter_mut().find(|m| m.enabled)
                        {
                            next.focused = true;
                            panel.brightness = if next.name == "eDP-1" {
                                s.display_brightness
                            } else {
                                None
                            };
                            panel.scale = Some(if next.name == "eDP-1" { 2.0 } else { 1.0 });
                        }
                    }
                }
            }
        }
        Action::SetDisplayBrightness(v) => {
            s.display_brightness = Some(v);
            if let Some(p) = &mut s.display_panel
                && p.focused().is_some_and(|m| m.name == "eDP-1")
            {
                p.brightness = Some(v);
            }
        }
        Action::SetKeyboardBacklight(v) => s.keyboard_backlight = Some(v),
        Action::StepDisplayBrightness(up) => {
            let v = s.display_brightness.unwrap_or(50);
            s.display_brightness = Some(if up {
                (v + 6).min(100)
            } else {
                v.saturating_sub(6)
            });
            if let Some(p) = &mut s.display_panel
                && p.focused().is_some_and(|m| m.name == "eDP-1")
            {
                p.brightness = s.display_brightness;
            }
        }
        Action::StepKeyboardBacklight(up) => {
            let v = s.keyboard_backlight.unwrap_or(50);
            s.keyboard_backlight = Some(if up {
                (v + 6).min(100)
            } else {
                v.saturating_sub(6)
            });
        }
        Action::FocusWorkspace { id, .. } => {
            s.workspaces.active = id;
            if !s.workspaces.list.iter().any(|w| w.id == id) {
                s.workspaces.list.push(omatouchbar_core::state::Workspace {
                    id,
                    name: id.to_string(),
                    windows: 0,
                    monitor: s.workspaces.monitor.clone(),
                });
            }
        }
        Action::MediaNext | Action::MediaPrevious => {
            if let Some(m) = s.media.as_mut() {
                let tracks = [
                    ("Midnight City", "M83"),
                    ("Intro", "The xx"),
                    ("Teardrop", "Massive Attack"),
                    ("Strobe", "deadmau5"),
                ];
                let index = tracks.iter().position(|t| t.0 == m.title).unwrap_or(0);
                let next = if matches!(action, Action::MediaNext) {
                    (index + 1) % tracks.len()
                } else {
                    (index + tracks.len() - 1) % tracks.len()
                };
                m.title = tracks[next].0.into();
                m.artist = tracks[next].1.into();
            }
        }
        Action::Key(code) => toasts.add_toast(
            adw::Toast::builder()
                .title(key_name(code))
                .timeout(1)
                .build(),
        ),
        Action::Shell(command) => {
            let toast = adw::Toast::builder()
                .title(format!("Would run: {}", truncate(&command, 70)))
                .timeout(2)
                .build();
            toasts.add_toast(toast);
        }
        Action::LoadTrayMenu(menu) => scene.apply_tray_menu(
            &menu,
            vec![omatouchbar_core::state::TrayMenuEntry {
                id: 1,
                label: "Open player".into(),
                enabled: true,
                submenu: false,
                checked: false,
            }],
        ),
        Action::LoadProvider { menu, provider } => {
            let rows = omatouchbar_core::menu::sample_provider_rows(&provider);
            scene.apply_provider(&menu, &provider, rows);
        }
        Action::ActivateTray { .. } | Action::ClickTrayMenu { .. } => toasts.add_toast(
            adw::Toast::builder()
                .title("Would activate the tray item")
                .timeout(1)
                .build(),
        ),
        Action::SetWeatherLocation(location) => {
            if let Some(w) = s.weather.as_mut() {
                w.place = omatouchbar_core::weather::Place {
                    name: location
                        .as_ref()
                        .map(|l| l.name.clone())
                        .or_else(|| w.detected.clone())
                        .unwrap_or_default(),
                    auto: location.is_none(),
                };
                w.loading = false;
            }
            let title = match &location {
                Some(l) => format!("Would set the weather location to {}", l.name),
                None => "Would detect the weather location automatically".into(),
            };
            toasts.add_toast(adw::Toast::builder().title(title).timeout(2).build());
        }
        Action::RefreshWeather => toasts.add_toast(
            adw::Toast::builder()
                .title("Would refresh Omarchy weather")
                .timeout(1)
                .build(),
        ),
        Action::RefreshAgents { .. } => toasts.add_toast(
            adw::Toast::builder()
                .title("Would refresh agent usage")
                .timeout(1)
                .build(),
        ),
        Action::CycleKeyboardLayout => toasts.add_toast(
            adw::Toast::builder()
                .title("Next keyboard layout")
                .timeout(1)
                .build(),
        ),
        Action::TriggerIndicator {
            indicator,
            currently_active,
        } => match indicator {
            Indicator::PasswordlessSudo => {
                s.indicators.passwordless_sudo = !currently_active;
                let msg = if currently_active {
                    "Passwordless sudo disabled (preview)"
                } else {
                    "Would open passwordless sudo confirmation (enabled in preview)"
                };
                toasts.add_toast(adw::Toast::builder().title(msg).timeout(2).build());
            }
            Indicator::ScreenRecording => {
                let msg = if currently_active {
                    "Stopped screen recording (preview)"
                } else {
                    "Would open screen recording menu"
                };
                toasts.add_toast(adw::Toast::builder().title(msg).timeout(2).build());
            }
            Indicator::Reminder => toasts.add_toast(
                adw::Toast::builder()
                    .title(if currently_active {
                        "Would show upcoming reminders"
                    } else {
                        "Would open reminder prompt"
                    })
                    .timeout(2)
                    .build(),
            ),
            Indicator::NightLight | Indicator::Dnd | Indicator::StayAwake => {}
            Indicator::RemoteSession => {}
        },
        Action::SetReminder => {
            s.indicators.reminder_count += 1;
            toasts.add_toast(
                adw::Toast::builder()
                    .title("Would open Omarchy reminder prompt (added 1 in preview)")
                    .timeout(2)
                    .build(),
            );
        }
        Action::ClearReminders => toasts.add_toast(
            adw::Toast::builder()
                .title("Cleared reminders (preview)")
                .timeout(1)
                .build(),
        ),
        Action::SetDns(DnsProvider::Custom) => toasts.add_toast(
            adw::Toast::builder()
                .title("Would open custom DNS prompt")
                .timeout(2)
                .build(),
        ),
        Action::SetDns(provider) => {
            s.network.dns = provider;
            s.network.pending_dns = None;
        }
        Action::SetWifiBand(band) => {
            if band != "auto" {
                s.network.band_current = band.clone();
            }
            s.network.band_selected = band;
            s.network.pending_band = None;
        }
        Action::CopyToClipboard(value) => toasts.add_toast(
            adw::Toast::builder()
                .title(format!("Copied {value} (preview)"))
                .timeout(1)
                .build(),
        ),
        Action::ShareWifiQr { ssid, .. } => toasts.add_toast(
            adw::Toast::builder()
                .title(format!("Would show Wi-Fi QR code for {ssid}"))
                .timeout(2)
                .build(),
        ),
        Action::OpenCaptivePortal => toasts.add_toast(
            adw::Toast::builder()
                .title("Would open captive portal sign-in")
                .timeout(2)
                .build(),
        ),
        // Toggles were already reflected optimistically by the scene.
        _ => {}
    }
}

fn key_name(code: u16) -> String {
    match code {
        1 => "Esc pressed".into(),
        59..=68 => format!("F{} pressed", code - 58),
        87 => "F11 pressed".into(),
        88 => "F12 pressed".into(),
        _ => format!("Key {code}"),
    }
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        text.chars().take(max - 1).collect::<String>() + "…"
    }
}
