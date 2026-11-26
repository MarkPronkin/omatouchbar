//! Renders the Touch ID effect's scripted demo as a contact sheet.
//!
//! `cargo run -p omatouchbar-core --example touchfx -- <unlock|enroll> <out.png> <ms,ms,…> [x0 x1] [--still]`
//!
//! One row per timestamp (milliseconds into the demo), optionally cropped
//! to columns `x0..x1` of the strip. `--still` renders with motion off.

use std::time::{Duration, Instant};

use omatouchbar_core::{
    Canvas, Config, DesktopState, Fonts, Palette, Scene, T1_HEIGHT, T1_WIDTH,
    menu::MenuTree,
    omarchy::BarLayout,
    scene::{Command, TouchIdDemo},
};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let still = args.iter().any(|a| a == "--still");
    let args: Vec<&String> = args.iter().filter(|a| *a != "--still").collect();
    let demo = match args.first().map(|s| s.as_str()) {
        Some("enroll") => TouchIdDemo::Enroll,
        _ => TouchIdDemo::Unlock,
    };
    let out = args.get(1).map(|s| s.as_str()).unwrap_or("touchfx.png");
    let times: Vec<u64> = args
        .get(2)
        .map(|s| s.split(',').filter_map(|t| t.parse().ok()).collect())
        .unwrap_or_else(|| vec![200, 500, 900, 1500, 2500]);
    let x0: u32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0);
    let x1: u32 = args
        .get(4)
        .and_then(|s| s.parse().ok())
        .unwrap_or(T1_WIDTH)
        .min(T1_WIDTH);

    let mut config = Config::default();
    config.motion.enabled = !still;
    let mut scene = Scene::new(
        T1_WIDTH,
        T1_HEIGHT,
        config,
        Palette::load().unwrap_or_else(Palette::fallback),
        Fonts::system(None),
        MenuTree::load(),
        BarLayout::from_omarchy(),
    );
    scene.set_state(DesktopState::sample());
    let utc = chrono::Utc::now();
    let mut now = Instant::now();
    for _ in 0..90 {
        now += Duration::from_millis(16);
        scene.update(now, utc);
    }
    scene.command(Command::TouchIdDemo(demo));
    let start = now + Duration::from_millis(16);

    let w = x1 - x0;
    let gap = 6;
    let mut sheet = tiny_skia::Pixmap::new(
        w,
        T1_HEIGHT * times.len() as u32 + gap * (times.len() as u32 - 1),
    )
    .unwrap();
    sheet.fill(tiny_skia::Color::from_rgba8(70, 70, 70, 255));
    let mut canvas = Canvas::new(T1_WIDTH, T1_HEIGHT);
    let mut render_ms = Vec::new();
    for (row, &ms) in times.iter().enumerate() {
        let target = start + Duration::from_millis(ms);
        while now < target {
            now = (now + Duration::from_millis(16)).min(target);
            scene.update(now, utc);
        }
        let t0 = Instant::now();
        scene.render(&mut canvas);
        render_ms.push(t0.elapsed().as_secs_f32() * 1000.0);
        let src = canvas.pixmap();
        let y = row as u32 * (T1_HEIGHT + gap);
        let data = sheet.data_mut();
        for py in 0..T1_HEIGHT {
            let from = ((py * T1_WIDTH + x0) * 4) as usize;
            let to = (((y + py) * w) * 4) as usize;
            data[to..to + (w * 4) as usize]
                .copy_from_slice(&src.data()[from..from + (w * 4) as usize]);
        }
    }
    sheet.save_png(out).expect("write png");
    let avg = render_ms.iter().sum::<f32>() / render_ms.len() as f32;
    let max = render_ms.iter().cloned().fold(0.0, f32::max);
    println!(
        "wrote {out} ({} frames; render avg {avg:.2} ms, max {max:.2} ms)",
        times.len()
    );
}
