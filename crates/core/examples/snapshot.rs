//! Renders the scene to a PNG for design review.
//!
//! `cargo run -p omatouchbar-core --example snapshot -- <page> <out.png> [frames]`
//!
//! Pages: bar, menu, menu:<id>, sound, sound-input, sound-sources, display (also -end, -one, -fixed, -cold), brightness, battery, network,
//! bluetooth (also -busy, -failed, -scan, -off, -end), clock (also -city, -add, -people, -globe, -calendar), tray, weather, weather-end, weather-search,
//! weather-searching, weather-loading, agents, agents-live (your own records),
//! agents-edge (alarms, accounts, balance, sign-in), indicators, fkeys, controls,
//! touchid, dots, flat, theme-wipe (-keyboard): a switch to SNAPSHOT_THEME (default gruvbox).
//! With `frames`, renders a contact sheet of the transition into the page
//! (one row every 33 ms). The output stacks the left and right halves of
//! the strip so details stay legible.

use std::time::{Duration, Instant};

use omatouchbar_core::{
    Canvas, Config, DesktopState, Fonts, KeyInput, Palette, Scene, SpeedTestEvent, T1_HEIGHT,
    T1_WIDTH,
    config::{ButtonStyle, Mode, WorkspaceStyle},
    menu::MenuTree,
    node::Key,
    omarchy::BarLayout,
    scene::{Command, Panel, SoundTab},
    state::{BtOp, Connectivity, NetKind, NetworkDetails, TouchId, TouchIdStage},
};

fn main() {
    let mut args = std::env::args().skip(1);
    let page = args.next().unwrap_or_else(|| "bar".into());
    let out = args.next().unwrap_or_else(|| "snapshot.png".into());
    let frames: usize = args.next().and_then(|f| f.parse().ok()).unwrap_or(0);

    let mut config = Config {
        mode: Mode::Bar,
        ..Config::default()
    };
    match page.as_str() {
        "fkeys" => config.mode = Mode::Function,
        "controls" => config.mode = Mode::Controls,
        "keyboard"
        | "keyboard-idle"
        | "keyboard-meter"
        | "keyboard-meter-out"
        | "keyboard-sound"
        | "keyboard-fn"
        | "keyboard-fn-up"
        | "keyboard-press"
        | "theme-wipe-keyboard" => config.mode = Mode::Keyboard,
        "dots" => config.appearance.workspace_style = WorkspaceStyle::Dots,
        "flat" | "quick-audio-flat" => config.appearance.button_style = ButtonStyle::Flat,
        "quick-audio-icon" => config.appearance.volume_percentage = false,
        "bar-indicators-inline" => config.appearance.always_show_indicators = true,
        _ => {}
    }
    let palette = Palette::load().unwrap_or_else(Palette::fallback);
    let fonts = Fonts::system(None);
    let mut scene = Scene::new(
        T1_WIDTH,
        T1_HEIGHT,
        config,
        palette,
        fonts,
        MenuTree::load(),
        BarLayout::from_omarchy(),
    );
    let model = std::env::var("SNAPSHOT_MODEL").unwrap_or_default();
    let mut state = DesktopState::sample_for_model(&model);
    if page == "touchid" {
        state.touch_id = Some(TouchId {
            stage: TouchIdStage::Enrollment,
            progress: Some(60),
        });
    }
    if page == "cjk" || page == "cjk-sound" {
        if let Some(media) = state.media.as_mut() {
            media.title = "夜に駆ける 🎧".into();
            media.artist = "YOASOBI · 周杰倫 · 아이유".into();
        }
    }
    if page == "keyboard-idle" {
        state.media = None;
    }
    // The daemon reads monitors in the background (~70 ms through
    // systemd-run): without a cached read, they land mid-animation.
    let mut cold_panel = None;
    if page == "display-cold" {
        if let Some(panel) = state.display_panel.as_mut() {
            panel.monitors.truncate(1);
        }
        cold_panel = state.display_panel.take();
    }
    if page == "display-one" || page == "display-fixed" {
        // A laptop on its own, as most Touch Bar Macs run.
        if let Some(panel) = state.display_panel.as_mut() {
            panel.monitors.truncate(1);
            if page == "display-fixed" {
                panel.brightness = None;
            }
        }
    }
    if page == "sound-two" {
        state.mixer.outputs.truncate(2);
    }
    if page == "bar-indicators-empty" {
        state.indicators.nightlight = false;
    }
    if page == "bar-indicators-urgent" || page == "indicators-active" {
        state.indicators.passwordless_sudo = true;
        state.indicators.stay_awake = true;
        state.indicators.reminder_count = 2;
        state.indicators.dnd = true;
    }
    if page == "bar-indicators-inline" {
        state.indicators.passwordless_sudo = true;
        state.indicators.stay_awake = true;
    }
    if page == "network-disconnected" {
        state.network.kind = NetKind::Disconnected;
        state.network.name.clear();
        state.network.connectivity = Connectivity::None;
        state.network.band_current.clear();
        state.network.band_available.clear();
        state.network.details = NetworkDetails::default();
        for net in &mut state.network.networks {
            net.active = false;
        }
    }
    if page == "weather-loading"
        && let Some(w) = state.weather.as_mut()
    {
        w.loading = true;
        w.place.name = "Kaliningrad".into();
    }
    if page == "network-portal" {
        state.network.connectivity = Connectivity::Portal;
    }
    if page.starts_with("agents-live") {
        state.agents = omatouchbar_core::agents::load(
            &omatouchbar_core::paths::state_home().join("omarchy/agents"),
            &mut Default::default(),
        );
    }
    if page.starts_with("agents-edge") {
        state.agents = agents_edge();
    }
    if page == "bluetooth-off" {
        state.bluetooth.powered = false;
        state.bluetooth.discovering = false;
        state.bluetooth.nearby.clear();
        for device in &mut state.bluetooth.devices {
            device.connected = false;
        }
        state.bluetooth.connected.clear();
    }
    if page == "bluetooth-scan" {
        state.bluetooth.nearby.clear();
    }
    if page == "network-loss" {
        state.network.details.ping_ms = Some(148.0);
        state.network.details.packet_loss = 17;
    }
    scene.set_state(state);

    let start = Instant::now();
    let mut now = start;
    let utc = chrono::Utc::now();
    let tick = |scene: &mut Scene, now: &mut Instant, ms: u64| {
        let steps = (ms / 16).max(1);
        for _ in 0..steps {
            *now += Duration::from_millis(16);
            scene.update(*now, utc);
        }
    };
    tick(&mut scene, &mut now, 1500);

    let command = match page.as_str() {
        "menu" => Some(Command::OpenMenu(None)),
        p if p.starts_with("menu:") => Some(Command::OpenMenu(Some(p[5..].to_string()))),
        "sound" | "sound-end" | "sound-two" | "cjk-sound" => Some(Command::OpenPanel(Panel::Audio)),
        "sound-input" => Some(Command::OpenSound(SoundTab::Input)),
        "sound-sources" => Some(Command::OpenSound(SoundTab::Sources)),
        "display" | "display-end" | "display-one" | "display-fixed" | "display-cold"
        | "brightness" => Some(Command::OpenPanel(Panel::Brightness)),
        "battery" => Some(Command::OpenPanel(Panel::Power)),
        "network"
        | "network-expanded"
        | "network-scrolled"
        | "network-end"
        | "network-disconnected"
        | "network-portal"
        | "network-loss" => Some(Command::OpenPanel(Panel::Network)),
        p if p.starts_with("bluetooth") => Some(Command::OpenPanel(Panel::Bluetooth)),
        "tray" => Some(Command::OpenPanel(Panel::Tray)),
        "weather" | "weather-end" | "weather-loading" => Some(Command::OpenPanel(Panel::Weather)),
        p if p.starts_with("agents") => Some(Command::OpenPanel(Panel::Agents)),
        "indicators" | "indicators-active" => Some(Command::OpenPanel(Panel::Indicators)),
        p if p == "clock" || p.starts_with("clock-") => Some(Command::OpenPanel(Panel::Clock)),
        _ => None,
    };
    let mut canvas = Canvas::new(T1_WIDTH, T1_HEIGHT);
    let half = T1_WIDTH / 2 + 1;
    let rows = if frames > 0 { frames } else { 1 };
    let mut sheet =
        tiny_skia::Pixmap::new(half, T1_HEIGHT * 2 * rows as u32 + (rows as u32 - 1) * 6).unwrap();
    sheet.fill(tiny_skia::Color::from_rgba8(60, 60, 60, 255));
    if let Some(c) = command {
        scene.command(c);
    }
    if matches!(scene.page(), omatouchbar_core::scene::Page::Menu(_)) {
        for action in scene.take_actions() {
            if let omatouchbar_core::Action::LoadProvider { menu, provider } = action {
                let rows = omatouchbar_core::menu::sample_provider_rows(&provider);
                scene.apply_provider(&menu, &provider, rows);
            }
        }
    }
    if page == "clock" || page.starts_with("clock-") {
        use omatouchbar_core::{
            Action,
            world_clock::{Input, Request, Worker},
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let worker = Worker::new(true, move |frame| {
            let _ = tx.send(frame);
        });
        let open = scene
            .take_actions()
            .into_iter()
            .find_map(|a| match a {
                Action::WorldClock(req) => Some(req),
                _ => None,
            })
            .expect("clock panel open");
        let session = open.session;
        worker.send(open);
        let mut frame = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("world clock preview");
        let target = match page.as_str() {
            "clock-city" => frame
                .items
                .iter()
                .find(|i| i.id.starts_with("city/"))
                .map(|i| i.id.as_str()),
            "clock-people" => Some("choosePeople"),
            "clock-add" => Some("add"),
            "clock-calendar" => Some("calendar"),
            "clock-globe" => Some("globe"),
            _ => None,
        };
        if let Some(id) = target {
            worker.send(Request {
                session,
                command: Input::Tap(id.into()),
            });
            frame = rx
                .recv_timeout(Duration::from_secs(5))
                .expect("world clock view");
        }
        assert!(frame.error.is_empty(), "{}", frame.error);
        scene.apply_world_clock(frame);
    }
    match page.as_str() {
        "wifi-pass" => {
            scene.open_wifi_prompt("Szv".into(), 74, false);
            scene.key_input(KeyInput::Text("SuperSecret42!".into()), now);
        }
        "wifi-pass-show" => {
            scene.open_wifi_prompt("Szv".into(), 74, true);
            scene.key_input(KeyInput::Text("SuperSecret42!".into()), now);
        }
        "wifi-pass-err" => {
            scene.open_wifi_prompt("Szv".into(), 74, false);
            scene.key_input(KeyInput::Text("wrongpass".into()), now);
            scene.apply_wifi_connect_result("Szv", Err("Wrong password".into()), true);
        }
        "wifi-pass-conn" => {
            scene.open_wifi_prompt("Szv".into(), 74, false);
            scene.key_input(KeyInput::Text("SuperSecret42!".into()), now);
            scene.key_input(KeyInput::Enter, now);
        }
        "weather-search" | "weather-searching" => {
            scene.open_weather_search();
            scene.key_input(KeyInput::Text("Kalin".into()), now);
            if page == "weather-search" {
                let found = omatouchbar_core::weather::parse_geocoding(
                    r#"{"results":[
                    {"name":"Kaliningrad","latitude":54.706,"longitude":20.511,"country_code":"RU","country":"Russia","admin1":"Kaliningrad Oblast"},
                    {"name":"Kalinin","latitude":38.69,"longitude":67.42,"country_code":"TJ","country":"Tajikistan","admin1":"Sughd"},
                    {"name":"Kalininsk","latitude":51.5,"longitude":44.47,"country_code":"RU","country":"Russia","admin1":"Saratov Oblast"},
                    {"name":"Kalinino","latitude":45.2,"longitude":39.9,"country_code":"RU","country":"Russia","admin1":"Krasnodar Krai"}]}"#,
                );
                scene.apply_weather_suggestions("Kalin", found);
            }
        }
        // Volume and brightness keys pressed: their groups fill as meters.
        // With frames, -meter shows them light up and -meter-out fade.
        "keyboard-meter" | "keyboard-meter-out" => {
            let mut state = scene.state().clone();
            if let Some(audio) = state.audio.as_mut() {
                audio.volume = 62;
            }
            state.display_brightness = state.display_brightness.map(|b| b.saturating_sub(10));
            scene.set_state(state);
            let wait = match (page.as_str(), frames) {
                ("keyboard-meter", 0) => 500,
                ("keyboard-meter", _) => 0,
                _ => 1700,
            };
            tick(&mut scene, &mut now, wait);
        }
        // Hold Fn (and let go): render with a frame count to see the swap.
        "keyboard-fn" => {
            scene.input(true, &[], now);
        }
        "keyboard-fn-up" => {
            scene.input(true, &[], now);
            tick(&mut scene, &mut now, 900);
            scene.input(false, &[], now);
        }
        // Press and hold the player: with frames, see it sink under the finger.
        "keyboard-press" => {
            let (x, y) = (T1_WIDTH as f32 - 120.0, T1_HEIGHT as f32 / 2.0);
            scene.input(false, &[(0, x, y)], now);
        }
        "keyboard-sound" => {
            // Tap the player at the right end: the Sound panel grows out of it.
            let (x, y) = (T1_WIDTH as f32 - 120.0, T1_HEIGHT as f32 / 2.0);
            scene.input(false, &[(0, x, y)], now);
            now += Duration::from_millis(40);
            scene.update(now, utc);
            scene.input(false, &[], now);
            tick(&mut scene, &mut now, 900);
        }
        "bluetooth-busy" | "bluetooth-failed" => {
            tick(&mut scene, &mut now, 900);
            let tap = |scene: &mut Scene, now: &mut Instant, key: Key| {
                if let Some(r) = scene.node_rect(key) {
                    scene.input(false, &[(0, r.cx(), r.cy())], *now);
                    *now += Duration::from_millis(40);
                    scene.update(*now, utc);
                    scene.input(false, &[], *now);
                    *now += Duration::from_millis(16);
                    scene.update(*now, utc);
                }
            };
            let dev = |a: &str| Key::new("p/bt/dev").with(a);
            tap(&mut scene, &mut now, dev("AA:BB:CC:DD:EE:04"));
            tap(&mut scene, &mut now, dev("AA:BB:CC:DD:EE:03"));
            tap(
                &mut scene,
                &mut now,
                Key::new("p/bt/forget").with("AA:BB:CC:DD:EE:02"),
            );
            if page == "bluetooth-failed" {
                scene.apply_bluetooth_done("AA:BB:CC:DD:EE:04", BtOp::Pair, false);
                scene.apply_bluetooth_done("AA:BB:CC:DD:EE:03", BtOp::Connect, false);
            }
            tick(&mut scene, &mut now, 700);
        }
        "speedtest-ign" => {
            scene.command(Command::OpenSpeedTest);
            tick(&mut scene, &mut now, 420);
        }
        "speedtest-wait" => {
            scene.command(Command::OpenSpeedTest);
            tick(&mut scene, &mut now, 1400);
            scene.apply_speedtest_event(SpeedTestEvent::Ping(11.4));
            tick(&mut scene, &mut now, 300);
        }
        "speedtest-err" => {
            scene.command(Command::OpenSpeedTest);
            tick(&mut scene, &mut now, 1000);
            scene.apply_speedtest_event(SpeedTestEvent::DownloadSample(42.0));
            tick(&mut scene, &mut now, 300);
            scene.apply_speedtest_event(SpeedTestEvent::Error(
                "Could not reach speed.omarchy.org".into(),
            ));
            tick(&mut scene, &mut now, 600);
        }
        "speedtest-down" => {
            scene.command(Command::OpenSpeedTest);
            tick(&mut scene, &mut now, 1000);
            scene.apply_speedtest_event(SpeedTestEvent::Ping(11.4));
            for v in [
                68.0, 145.0, 260.0, 342.0, 410.0, 458.0, 489.0, 472.0, 512.4, 495.2, 534.8,
            ] {
                scene.apply_speedtest_event(SpeedTestEvent::DownloadSample(v));
                tick(&mut scene, &mut now, 140);
            }
        }
        "speedtest-up" => {
            scene.command(Command::OpenSpeedTest);
            tick(&mut scene, &mut now, 1000);
            scene.apply_speedtest_event(SpeedTestEvent::Ping(9.8));
            for v in [85.0, 210.0, 360.0, 448.0, 492.0, 518.6, 504.2] {
                scene.apply_speedtest_event(SpeedTestEvent::DownloadSample(v));
                tick(&mut scene, &mut now, 100);
            }
            scene.apply_speedtest_event(SpeedTestEvent::DownloadDone(504.2));
            scene.apply_speedtest_event(SpeedTestEvent::UploadStart);
            for v in [42.0, 118.0, 195.0, 248.0, 286.4, 312.7, 298.1] {
                scene.apply_speedtest_event(SpeedTestEvent::UploadSample(v));
                tick(&mut scene, &mut now, 130);
            }
        }
        "speedtest-done" => {
            scene.command(Command::OpenSpeedTest);
            tick(&mut scene, &mut now, 1000);
            scene.apply_speedtest_event(SpeedTestEvent::Ping(10.2));
            for v in [90.0, 240.0, 410.0, 580.0, 690.0, 742.5, 728.0, 756.3] {
                scene.apply_speedtest_event(SpeedTestEvent::DownloadSample(v));
                tick(&mut scene, &mut now, 90);
            }
            scene.apply_speedtest_event(SpeedTestEvent::DownloadDone(756.3));
            scene.apply_speedtest_event(SpeedTestEvent::UploadStart);
            for v in [55.0, 150.0, 240.0, 310.0, 345.0, 338.8, 352.1] {
                scene.apply_speedtest_event(SpeedTestEvent::UploadSample(v));
                tick(&mut scene, &mut now, 90);
            }
            scene.apply_speedtest_event(SpeedTestEvent::UploadDone(352.1));
            tick(&mut scene, &mut now, 350);
        }
        "quick-audio" | "quick-audio-flat" | "quick-audio-icon" | "quick-brightness" => {
            let key = if page == "quick-brightness" {
                omatouchbar_core::node::Key::new("w/brightness")
            } else {
                omatouchbar_core::node::Key::new("w/audio")
            };
            if let Some(r) = scene.node_rect(key) {
                scene.input(false, &[(0, r.cx(), r.cy())], now);
                now += Duration::from_millis(245);
                scene.update(now, utc);
            }
        }
        p if p.starts_with("theme-wipe") => {
            let name = std::env::var("SNAPSHOT_THEME").unwrap_or_else(|_| "gruvbox".into());
            let root =
                std::env::var("OMARCHY_PATH").unwrap_or_else(|_| "/usr/share/omarchy".into());
            let path = format!("{root}/themes/{name}/colors.toml");
            let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
            scene.set_palette(Palette::parse(&text).expect("colors.toml"));
        }
        "quick-audio-close" => {
            if let Some(r) = scene.node_rect(omatouchbar_core::node::Key::new("w/audio")) {
                scene.input(false, &[(0, r.cx(), r.cy())], now);
                tick(&mut scene, &mut now, 400);
                scene.input(false, &[(0, r.cx() + 90.0, r.cy())], now);
                tick(&mut scene, &mut now, 120);
                scene.input(false, &[], now);
            }
        }
        _ => {}
    }
    if frames == 0
        && !page.starts_with("speedtest-")
        && !page.starts_with("wifi-pass")
        && !matches!(
            page.as_str(),
            "bluetooth-busy" | "bluetooth-failed" | "keyboard-meter" | "keyboard-meter-out"
        )
    {
        tick(&mut scene, &mut now, 2000);
    } else if frames == 0 && page.starts_with("wifi-pass") {
        tick(&mut scene, &mut now, 650);
    }
    if page == "network-expanded" {
        scene.set_wifi_networks_expanded(true);
        tick(&mut scene, &mut now, 650);
        scene.set_scroll_offset(420.0);
        tick(&mut scene, &mut now, 32);
    } else if page == "network-scrolled" {
        scene.set_scroll_offset(1350.0);
        tick(&mut scene, &mut now, 32);
    } else if page.ends_with("-end") {
        scene.set_scroll_offset(9999.0);
        tick(&mut scene, &mut now, 32);
    }
    // SNAPSHOT_SCROLL=<px> scrolls a row-shaped panel before rendering.
    if let Some(offset) = std::env::var("SNAPSHOT_SCROLL")
        .ok()
        .and_then(|v| v.parse::<f32>().ok())
    {
        scene.set_scroll_offset(offset);
        tick(&mut scene, &mut now, 32);
    }
    for row in 0..rows {
        if frames > 0 {
            tick(&mut scene, &mut now, 33);
        }
        if row == 2
            && let Some(panel) = cold_panel.take()
        {
            let mut state = scene.state().clone();
            state.display_panel = Some(panel);
            scene.set_state(state);
        }
        canvas = {
            let mut c = Canvas::new(T1_WIDTH, T1_HEIGHT);
            scene.render(&mut c);
            c
        };
        let y = row as i32 * (T1_HEIGHT as i32 * 2 + 6);
        let paint = tiny_skia::PixmapPaint::default();
        sheet.draw_pixmap(
            0,
            y,
            canvas.pixmap().as_ref(),
            &paint,
            tiny_skia::Transform::identity(),
            None,
        );
        sheet.draw_pixmap(
            -(half as i32) + 1,
            y + T1_HEIGHT as i32,
            canvas.pixmap().as_ref(),
            &paint,
            tiny_skia::Transform::identity(),
            None,
        );
    }
    let _ = canvas;
    sheet.save_png(&out).expect("write png");
    println!("wrote {out}");
}

/// Agents in every state the panel draws: near a limit, several accounts,
/// a prepaid balance, a lapsed sign-in and limits that couldn't refresh.
fn agents_edge() -> Vec<omatouchbar_core::state::AgentUsage> {
    use omatouchbar_core::agents::{AgentAccount, AgentBalance, AgentLimit, AgentUsage};
    let now = chrono::Utc::now();
    let mut icons = omatouchbar_core::agent_icons::AgentIconCache::default();
    let limit = |window: &str, used: u8, minutes: i64| AgentLimit {
        window: window.into(),
        used,
        resets_at: Some(now + chrono::Duration::minutes(minutes)),
    };
    vec![
        AgentUsage {
            icons: icons.load("claude"),
            id: "claude".into(),
            name: "Claude Code".into(),
            plan: "Max 20x".into(),
            limits: vec![limit("Session", 96, 47), limit("Weekly", 58, 3 * 24 * 60)],
            accounts: vec![
                AgentAccount {
                    id: "personal".into(),
                    label: "Personal".into(),
                    plan: "Max 20x".into(),
                    primary: true,
                    active: true,
                    limits: vec![limit("Session", 96, 47), limit("Weekly", 58, 3 * 24 * 60)],
                    ..AgentAccount::default()
                },
                AgentAccount {
                    id: "work".into(),
                    label: "Work".into(),
                    plan: "Pro".into(),
                    limits: vec![
                        limit("Session", 12, 4 * 60),
                        limit("Weekly", 30, 5 * 24 * 60),
                    ],
                    ..AgentAccount::default()
                },
            ],
            ..AgentUsage::default()
        },
        AgentUsage {
            icons: icons.load("codex"),
            id: "codex".into(),
            name: "Codex".into(),
            plan: "Pro".into(),
            needs_sign_in: true,
            limits: vec![limit("Weekly", 40, 2 * 24 * 60)],
            limits_stale: true,
            ..AgentUsage::default()
        },
        AgentUsage {
            icons: icons.load("grok"),
            id: "grok".into(),
            name: "Grok".into(),
            plan: "SuperGrok".into(),
            limits: vec![limit("Monthly", 22, 12 * 24 * 60)],
            limits_stale: true,
            note: Some("Limits paused".into()),
            ..AgentUsage::default()
        },
        AgentUsage {
            icons: icons.load("openrouter"),
            id: "openrouter".into(),
            name: "OpenRouter".into(),
            plan: "Prepaid".into(),
            balance: Some(AgentBalance {
                remaining: 3.2,
                funded: 40.0,
                spent: 36.8,
                currency: "USD".into(),
                estimated: false,
            }),
            ..AgentUsage::default()
        },
    ]
}
