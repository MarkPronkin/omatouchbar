//! Renders Omarchy's screensaver as the Touch Bar plays it, for review.
//!
//! `cargo run -p omatouchbar-core --example screensaver -- <effect> <out.png> [frames] [crop]`
//!
//! Runs `ttfx <effect>` on your screensaver branding at the strip's size
//! (seeded, so runs repeat) and stacks `frames` (default 12) evenly spaced
//! frames, the last one the settled logo. With `crop`, each row shows the
//! middle 720 px at 2× instead of the whole strip.

use std::{
    io::{Read, Write},
    process::{Command, Stdio},
    sync::Arc,
    time::{Duration, Instant},
};

use chrono::Utc;
use omatouchbar_core::{
    Canvas, Config, DesktopState, Fonts, Palette, Scene, T1_HEIGHT, T1_WIDTH,
    menu::MenuTree,
    omarchy::BarLayout,
    screensaver::{self, Geometry, Grid, Parser},
};

/// Effects gated on wall-clock time run paced (unpaced they never end).
const TIMED: [&str; 2] = ["matrix", "thunderstorm"];

fn ttfx(effect: &str, text: &str, g: &Geometry) -> Vec<u8> {
    let paced = TIMED.contains(&effect);
    let rate = if paced {
        screensaver::FRAME_RATE.to_string()
    } else {
        "0".into()
    };
    let mut child = Command::new("ttfx")
        .args(["--seed", "7", "--frame-rate", &rate])
        .args(["--canvas-width", &g.cols.to_string()])
        .args(["--canvas-height", &g.rows.to_string()])
        .args(["--ignore-terminal-dimensions", "--anchor-canvas", "c"])
        .args(["--anchor-text", "c", "--no-eol", "--no-restore-cursor"])
        .arg(effect)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("ttfx");
    let mut stdin = child.stdin.take().unwrap();
    let text = text.to_string();
    std::thread::spawn(move || stdin.write_all(text.as_bytes()));
    let mut out = Vec::new();
    child.stdout.take().unwrap().read_to_end(&mut out).unwrap();
    let _ = child.wait();
    out
}

fn main() {
    let mut args = std::env::args().skip(1);
    let effect = args.next().unwrap_or_else(|| "beams".into());
    let out = args.next().unwrap_or_else(|| "screensaver.png".into());
    let count: usize = args.next().and_then(|f| f.parse().ok()).unwrap_or(12);
    let crop = args.next().as_deref() == Some("crop");

    let (text, geometry) = screensaver::branding(T1_WIDTH, T1_HEIGHT);
    let bytes = ttfx(&effect, &text, &geometry);
    let mut frames: Vec<Grid> = Vec::new();
    let mut parser = Parser::new(geometry.cols as usize, geometry.rows as usize);
    // Keep every frame's text cheaply by sampling after the fact: frames
    // are small next to the stream, but a long effect has thousands.
    let mut all = 0usize;
    parser.feed(&bytes, &mut |_| all += 1);
    parser.finish(&mut |_| all += 1);
    let wanted: Vec<usize> = (0..count)
        .map(|i| ((i as f32 + 1.0) / count as f32 * (all - 1) as f32).round() as usize)
        .collect();
    let mut parser = Parser::new(geometry.cols as usize, geometry.rows as usize);
    let mut index = 0usize;
    let mut take = |g: &Grid| {
        if wanted.contains(&index) {
            frames.push(g.clone());
        }
        index += 1;
    };
    parser.feed(&bytes, &mut take);
    parser.finish(&mut take);
    println!(
        "{effect}: {all} frames ({:.1} s at {} fps), {}×{} cells of {}×{} px",
        all as f32 / screensaver::FRAME_RATE as f32,
        screensaver::FRAME_RATE,
        geometry.cols,
        geometry.rows,
        geometry.cell_w,
        geometry.cell_h
    );

    let fonts = Fonts::system(None);
    let mut scene = Scene::new(
        T1_WIDTH,
        T1_HEIGHT,
        Config::default(),
        Palette::load().unwrap_or_else(Palette::fallback),
        fonts,
        MenuTree::load(),
        BarLayout::from_omarchy(),
    );
    scene.set_state(DesktopState::sample());
    scene.screensaver_start(geometry);
    let mut now = Instant::now();
    // Let the strip fade over to the screensaver first.
    for _ in 0..60 {
        now += Duration::from_millis(16);
        scene.update(now, Utc::now());
    }

    let (row_w, row_h, scale, src_x) = if crop {
        (1440u32, T1_HEIGHT * 2, 2.0, (T1_WIDTH - 720) as i32 / 2)
    } else {
        (T1_WIDTH, T1_HEIGHT, 1.0, 0)
    };
    let gap = 4;
    let mut sheet =
        tiny_skia::Pixmap::new(row_w, (row_h + gap) * frames.len().max(1) as u32).unwrap();
    sheet.fill(tiny_skia::Color::from_rgba8(60, 60, 60, 255));
    let mut canvas = Canvas::new(T1_WIDTH, T1_HEIGHT);
    for (i, frame) in frames.into_iter().enumerate() {
        scene.screensaver_frame(Arc::new(frame));
        now += Duration::from_millis(16);
        scene.update(now, Utc::now());
        scene.render(&mut canvas);
        let paint = tiny_skia::PixmapPaint {
            quality: tiny_skia::FilterQuality::Nearest,
            ..Default::default()
        };
        sheet.draw_pixmap(
            0,
            0,
            canvas.pixmap().as_ref(),
            &paint,
            tiny_skia::Transform::from_scale(scale, scale)
                .post_translate(-src_x as f32 * scale, (i as u32 * (row_h + gap)) as f32),
            None,
        );
    }
    sheet.save_png(&out).expect("write png");
    println!("saved {out}");
}
