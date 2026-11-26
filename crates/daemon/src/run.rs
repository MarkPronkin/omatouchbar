//! The renderer's event loop.

use std::{
    io::Read,
    os::{fd::AsFd, unix::net::UnixStream},
    sync::mpsc::Receiver,
    time::{Duration, Instant},
};

use chrono::Utc;
use omatouchbar_core::{
    Canvas, Config, DesktopState, Fonts, Palette, Scene, T1_HEIGHT, T1_WIDTH,
    config::Mode,
    menu::MenuTree,
    omarchy::BarLayout,
    scene::{Page, Panel},
};
use rustix::event::{PollFd, PollFlags};

use crate::{
    bus::{Bus, Event, Watched},
    control,
    exec::Exec,
    hw::Hw,
    providers::{self, system},
    sysbar,
    themesync::ThemeSync,
};

fn apply_bar_policy(config: &Config) {
    if config.mode == Mode::Bar && config.hide_system_bar {
        sysbar::hide();
    } else {
        sysbar::restore_if_ours();
    }
}

fn status(scene: &Scene, hw: &Hw) -> String {
    let (w, h) = hw.size().unwrap_or((0, 0));
    let page = match scene.page() {
        Page::Bar => "bar".to_string(),
        Page::Keyboard => "keyboard".into(),
        Page::Controls => "controls".into(),
        Page::FnKeys => "function".into(),
        Page::Menu(id) => format!("menu:{id}"),
        // Keep the existing control-socket page ID for scripts.
        Page::Panel(Panel::Brightness) => "panel:brightness".into(),
        Page::Panel(p) => format!("panel:{}", p.title().to_lowercase()),
    };
    serde_json::json!({
        "running": true,
        "pid": std::process::id(),
        "version": env!("CARGO_PKG_VERSION"),
        "connected": hw.connected(),
        "width": w,
        "height": h,
        "fps": (hw.fps * 10.0).round() / 10.0,
        "presentMs": (hw.latency_ms * 10.0).round() / 10.0,
        "features": format!("{:#x}", hw.features()),
        "mode": format!("{:?}", scene.config().mode).to_lowercase(),
        "page": page,
        "systemBarHidden": sysbar::is_hidden(),
        "brightness": scene.brightness(),
    })
    .to_string()
}

fn timespec(d: Duration) -> rustix::time::Timespec {
    rustix::time::Timespec {
        tv_sec: d.as_secs() as _,
        tv_nsec: d.subsec_nanos() as _,
    }
}

pub fn run() -> i32 {
    let (bus, rx) = Bus::new();

    // Signals arrive through a socketpair so poll can wake on them.
    let (mut signal_read, signal_write) = UnixStream::pair().expect("socketpair");
    signal_read.set_nonblocking(true).ok();
    for signal in [
        signal_hook::consts::SIGTERM,
        signal_hook::consts::SIGINT,
        signal_hook::consts::SIGHUP,
    ] {
        if let Ok(writer) = signal_write.try_clone() {
            let _ = signal_hook::low_level::pipe::register(signal, writer);
        }
    }
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        sysbar::restore_if_ours();
        control::remove_socket();
        default_hook(info);
    }));

    let config = Config::load().unwrap_or_else(|e| {
        log::warn!("config: {e}; using defaults");
        Config::default()
    });
    let palette = Palette::load().unwrap_or_else(Palette::fallback);
    let fonts = Fonts::system(None);
    if fonts.is_empty() {
        log::error!("no usable fonts found (is fontconfig installed?)");
    }
    let mut hw = Hw::new();
    hw.try_connect(Instant::now());
    let (width, height) = hw.size().unwrap_or((T1_WIDTH, T1_HEIGHT));
    let mut scene = Scene::new(
        width,
        height,
        config,
        palette,
        fonts,
        MenuTree::load(),
        BarLayout::from_omarchy(),
    );
    let mut canvas = Canvas::new(width, height);
    let mut state = DesktopState {
        touch_id: system::read_touch_id(),
        ..DesktopState::default()
    };
    scene.set_state(state.clone());

    let mut exec = Exec::new(bus.clone());
    let mut kbd = crate::kbd::KeyboardGrab::new();
    let (refresh, hyprland) = providers::start(&bus, exec.sink.clone());
    exec.refresh = refresh;
    let mut theme_sync = ThemeSync::new(bus.clone());
    crate::watch::spawn(bus.clone());
    control::serve(bus.clone());
    apply_bar_policy(scene.config());
    log::info!("omatouchbar {} running", env!("CARGO_PKG_VERSION"));

    let mut events = Vec::new();
    let mut last_frame = Instant::now() - Duration::from_secs(1);
    let mut need_frame = true;
    let mut contacts: Vec<(u8, f32, f32)> = Vec::new();
    let mut starved_since: Option<Instant> = None;
    let mut starved_warned = false;

    loop {
        let now = Instant::now();
        exec.weather.configure(&scene);
        exec.display.set_visible(scene.wants_display_panel());
        exec.network_details.configure(&scene);
        exec.bluetooth_scan.configure(&scene);
        let interval = Duration::from_secs_f32(1.0 / scene.config().motion.fps.max(1) as f32);
        let mut timeout = if hw.connected() && need_frame && !hw.can_submit() {
            // All buffers are with the service; its FrameReleased wakes us.
            Duration::from_millis(50)
        } else if hw.connected() && (need_frame || scene.animating()) {
            interval.saturating_sub(now.saturating_duration_since(last_frame))
        } else {
            scene.next_deadline(now)
        };
        if !hw.connected() {
            timeout = timeout.min(hw.retry_in(now).max(Duration::from_millis(50)));
        }
        timeout = timeout.min(Duration::from_secs(5));

        let poll_start = Instant::now();
        let ready = {
            let bus_fd = bus.wake_fd();
            let signal_fd = signal_read.as_fd();
            let mut fds = vec![
                PollFd::new(&bus_fd, PollFlags::IN),
                PollFd::new(&signal_fd, PollFlags::IN),
            ];
            let hw_fd = hw.fd();
            if let Some(fd) = hw_fd.as_ref() {
                fds.push(PollFd::new(fd, PollFlags::IN));
            }
            let _ = rustix::event::poll(&mut fds, Some(&timespec(timeout)));
            fds.iter()
                .map(|f| !f.revents().is_empty())
                .collect::<Vec<_>>()
        };
        if ready.get(1).copied().unwrap_or(false) {
            let mut buffer = [0u8; 16];
            let _ = signal_read.read(&mut buffer);
            log::info!("shutting down");
            break;
        }
        let woke = Instant::now();
        let slept = woke.saturating_duration_since(poll_start);
        if ready[0] {
            bus.drain();
            handle_events(
                &rx,
                &mut scene,
                &mut state,
                &hw,
                &hyprland,
                &canvas,
                &exec,
                &mut theme_sync,
            );
        }
        let t_events = woke.elapsed();

        // Touch Bar input.
        events.clear();
        hw.read(&mut events);
        let now = Instant::now();
        for event in events.drain(..) {
            if let t1hw::Event::Input(frame) = event {
                contacts.clear();
                contacts.extend(
                    frame
                        .contacts
                        .iter()
                        .filter(|c| c.tip && c.in_range)
                        .map(|c| (c.id, c.x as f32, c.y as f32)),
                );
                log::debug!(
                    "input t={}ms fn={} contacts={:?} (slept {}ms)",
                    frame.monotonic_ns / 1_000_000,
                    frame.fn_pressed,
                    contacts
                        .iter()
                        .map(|c| (c.0, c.1 as i32))
                        .collect::<Vec<_>>(),
                    slept.as_millis()
                );
                scene.input(frame.fn_pressed, &contacts, now);
                need_frame = true;
            }
        }
        if !hw.connected() && hw.try_connect(now) {
            if let Some((w, h)) = hw.size()
                && (w != canvas.width() || h != canvas.height())
            {
                log::warn!("panel is {w}x{h}; restart the renderer to relayout");
            }
            need_frame = true;
        }

        if state.touch_id.is_some() && system::read_touch_id().is_none() {
            state.touch_id = None;
            scene.set_state(state.clone());
        }

        let t_input = woke.elapsed();
        let now = Instant::now();
        if scene.update(now, Utc::now()) {
            need_frame = true;
        }
        let t_update = woke.elapsed();
        kbd.set_active(scene.wants_keyboard(), &bus);
        for action in scene.take_actions() {
            exec.run(action, &mut hw, scene.config());
        }
        exec.reap();

        let t_actions = woke.elapsed();
        let mut t_render = Duration::ZERO;
        if need_frame
            && hw.connected()
            && now.saturating_duration_since(last_frame) + Duration::from_millis(2) >= interval
        {
            if hw.can_submit() {
                starved_since = None;
                let r0 = Instant::now();
                scene.render(&mut canvas);
                t_render = r0.elapsed();
                if hw.present(&canvas, scene.brightness(), now) {
                    last_frame = now;
                }
                need_frame = false;
            } else {
                let since = *starved_since.get_or_insert(now);
                if now.saturating_duration_since(since) > Duration::from_millis(250)
                    && !starved_warned
                {
                    starved_warned = true;
                    log::warn!(
                        "no free frame buffer for {}ms (service has not released frames)",
                        now.saturating_duration_since(since).as_millis()
                    );
                }
            }
        } else if !hw.connected() {
            need_frame = true;
        }
        if starved_since.is_none() {
            starved_warned = false;
        }
        let total = woke.elapsed();
        if total > Duration::from_millis(40) {
            log::warn!(
                "slow loop {}ms: events {}ms, input {}ms, update {}ms, actions {}ms, render {}ms",
                total.as_millis(),
                t_events.as_millis(),
                (t_input - t_events).as_millis(),
                (t_update - t_input).as_millis(),
                (t_actions - t_update).as_millis(),
                t_render.as_millis()
            );
        }
        hw.idle_tick(now);
    }

    kbd.stop();
    exec.speedtest.stop();
    exec.wifi.cancel();
    sysbar::restore_if_ours();
    control::remove_socket();
    0
}

#[allow(clippy::too_many_arguments)]
fn handle_events(
    rx: &Receiver<Event>,
    scene: &mut Scene,
    state: &mut DesktopState,
    hw: &Hw,
    hyprland: &std::sync::mpsc::Sender<()>,
    canvas: &Canvas,
    exec: &Exec,
    theme_sync: &mut ThemeSync,
) {
    let mut state_changed = false;
    for event in rx.try_iter() {
        match event {
            Event::Patch(patch) => {
                patch(state);
                state_changed = true;
            }
            Event::Files(kind) => match kind {
                Watched::Config => match Config::load() {
                    Ok(config) => {
                        log::info!("settings changed");
                        apply_bar_policy(&config);
                        scene.set_config(config);
                    }
                    Err(e) => log::warn!("config.toml: {e}"),
                },
                // Revealed when the desktop starts its own switch.
                Watched::Theme => {
                    if let Some(palette) = Palette::load() {
                        theme_sync.changed(palette, scene.palette());
                    }
                }
                Watched::ShellJson => scene.set_omarchy_layout(BarLayout::from_omarchy()),
                Watched::WeatherLocation => exec.weather.location_changed(),
                Watched::AgentUsage => {
                    if let Some(tx) = &exec.refresh.agents {
                        let _ = tx.send(providers::agents::Request::Read);
                    }
                }
                Watched::Menu => scene.set_menu(MenuTree::load()),
                Watched::TouchId => {
                    state.touch_id = system::read_touch_id();
                    state_changed = true;
                }
                Watched::Indicators => {
                    if let Some(tx) = &exec.refresh.indicators {
                        let _ = tx.send(system::IndicatorRefresh::Fast);
                    }
                }
                Watched::MonitorBlocks => {
                    let _ = hyprland.send(());
                }
            },
            Event::Control(command, reply) => {
                let mode_change = matches!(command, omatouchbar_core::Command::SetMode(_));
                scene.command(command);
                if mode_change {
                    apply_bar_policy(scene.config());
                }
                let _ = reply.send("ok".into());
            }
            Event::Status(reply) => {
                let _ = reply.send(status(scene, hw));
            }
            Event::Snapshot(path, reply) => {
                let result = match canvas.save_png(std::path::Path::new(&path)) {
                    Ok(()) => format!("saved {path}"),
                    Err(e) => format!("error: {e}"),
                };
                let _ = reply.send(result);
            }
            Event::Reload => {
                if let Ok(config) = Config::load() {
                    apply_bar_policy(&config);
                    scene.set_config(config);
                }
                if let Some(palette) = Palette::load() {
                    scene.set_palette(palette);
                }
                scene.set_omarchy_layout(BarLayout::from_omarchy());
                scene.set_menu(MenuTree::load());
                scene.set_fonts(Fonts::system(None));
                if let Some(tx) = &exec.refresh.indicators {
                    let _ = tx.send(system::IndicatorRefresh::Full);
                }
            }
            Event::Debug(on) => {
                log::set_max_level(if on {
                    log::LevelFilter::Debug
                } else {
                    log::LevelFilter::Info
                });
                log::info!("debug logging {}", if on { "on" } else { "off" });
            }
            Event::Guards { menu, results } => scene.apply_guards(&menu, results),
            Event::ProviderRows {
                menu,
                provider,
                rows,
            } => scene.apply_provider(&menu, &provider, rows),
            Event::TrayMenu { menu, entries } => scene.apply_tray_menu(&menu, entries),
            Event::Activity => scene.wake(),
            Event::KeyInput(input) => scene.key_input(input, Instant::now()),
            Event::WifiConnectResult {
                ssid,
                result,
                auth_failed,
            } => {
                if result.is_ok() {
                    exec.network_details.refresh();
                }
                scene.apply_wifi_connect_result(&ssid, result, auth_failed);
            }
            Event::SpeedTest(ev) => scene.apply_speedtest_event(ev),
            Event::BluetoothDone { address, op, ok } => {
                scene.apply_bluetooth_done(&address, op, ok)
            }
            Event::WorldClock(frame) => scene.apply_world_clock(frame),
            Event::WeatherSuggestions { query, results } => {
                scene.apply_weather_suggestions(&query, results)
            }
            Event::ThemeReady {
                generation,
                cue,
                waited,
            } => {
                if let Some(palette) = theme_sync.ready(generation) {
                    log::info!("theme changed ({cue} after {} ms)", waited.as_millis());
                    scene.set_palette(palette);
                }
            }
            Event::RefreshIndicators(full) => {
                if let Some(tx) = &exec.refresh.indicators {
                    let _ = tx.send(if full {
                        system::IndicatorRefresh::Full
                    } else {
                        system::IndicatorRefresh::Fast
                    });
                }
            }
        }
    }
    if state_changed {
        scene.set_state(state.clone());
    }
}
