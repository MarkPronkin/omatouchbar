//! Replays rapid input and theme switches against the scene and reports
//! slow frames and whether the scene settles. `cargo run --release -p omatouchbar-core --example stress`

use std::time::{Duration, Instant};

use omatouchbar_core::{
    Canvas, Config, DesktopState, Fonts, Palette, Scene, T1_HEIGHT, T1_WIDTH, menu::MenuTree,
    omarchy::BarLayout,
};

fn main() {
    let palette = Palette::load().unwrap_or_else(Palette::fallback);
    let palettes = [Palette::fallback(), palette.clone()];
    let mut scene = Scene::new(
        T1_WIDTH,
        T1_HEIGHT,
        Config {
            // The script taps the bar's menu and widgets.
            mode: omatouchbar_core::config::Mode::Bar,
            ..Config::default()
        },
        palette,
        Fonts::system(None),
        MenuTree::load(),
        BarLayout::from_omarchy(),
    );
    scene.set_state(DesktopState::sample());
    let mut canvas = Canvas::new(T1_WIDTH, T1_HEIGHT);
    let start = Instant::now();
    let mut now = start;
    let utc = chrono::Utc::now();
    let mut worst = (Duration::ZERO, 0u32);
    let mut frames = 0u32;
    let mut total = Duration::ZERO;
    // Frames drawn while a theme switch reveal runs (two themes each).
    let (mut wipe_frames, mut wipe_total, mut wipe_worst) = (0u32, Duration::ZERO, Duration::ZERO);
    // (time in ms, fn held, touch)
    let script: Vec<(u64, bool, Option<(f32, f32)>)> = vec![
        (1000, true, None),
        (1080, false, None),
        (1140, true, None),
        (1200, false, None),
        (1230, true, None),
        (1260, false, None),
        (1500, false, Some((180.0, 30.0))), // tap the logo (menu)
        (1560, false, None),
        (1600, false, Some((400.0, 30.0))), // tap quickly again
        (1640, false, None),
        (1700, false, Some((180.0, 30.0))),
        (1730, false, None),
    ];
    let mut step = 0;
    let panels = [
        omatouchbar_core::scene::Panel::Power,
        omatouchbar_core::scene::Panel::Audio,
        omatouchbar_core::scene::Panel::Network,
        omatouchbar_core::scene::Panel::Clock,
        omatouchbar_core::scene::Panel::Brightness,
    ];
    for tick in 0..600u32 {
        if tick >= 300 && tick % 40 == 0 {
            let panel = panels[((tick - 300) / 40) as usize % panels.len()];
            if (tick / 40) % 2 == 0 {
                scene.command(omatouchbar_core::Command::OpenPanel(panel));
            } else {
                scene.command(omatouchbar_core::Command::Home);
            }
        }
        // Theme switches, the second while panels open and close.
        if tick == 150 || tick == 470 {
            scene.set_palette(palettes[(tick == 150) as usize ^ 1].clone());
        }
        let wiping = (150..210).contains(&tick) || (470..530).contains(&tick);
        let t = tick as u64 * 16;
        now = start + Duration::from_millis(t);
        while step < script.len() && script[step].0 <= t {
            let (_, fn_held, touch) = script[step];
            let contacts: Vec<(u8, f32, f32)> =
                touch.map(|(x, y)| vec![(0, x, y)]).unwrap_or_default();
            scene.input(fn_held, &contacts, now);
            step += 1;
        }
        let t0 = Instant::now();
        let changed = scene.update(now, utc);
        if changed {
            scene.render(&mut canvas);
            frames += 1;
        }
        let spent = t0.elapsed();
        total += spent;
        if spent > worst.0 {
            worst = (spent, tick);
        }
        if wiping && changed {
            wipe_frames += 1;
            wipe_total += spent;
            wipe_worst = wipe_worst.max(spent);
        }
        let _ = scene.take_actions();
    }
    println!(
        "frames rendered: {frames}, avg {:.2} ms, worst {:.2} ms at tick {}",
        total.as_secs_f64() * 1000.0 / frames.max(1) as f64,
        worst.0.as_secs_f64() * 1000.0,
        worst.1
    );
    println!(
        "theme switch frames: {wipe_frames}, avg {:.2} ms, worst {:.2} ms",
        wipe_total.as_secs_f64() * 1000.0 / wipe_frames.max(1) as f64,
        wipe_worst.as_secs_f64() * 1000.0
    );
    println!("still animating after script: {}", scene.animating());
    println!("glyphs cached: {}", scene.cached_glyphs());
    println!("page: {:?}", scene.page());
}
