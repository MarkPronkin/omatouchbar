//! Hardware smoke test: negotiate, draw a moving gradient, log touches.
//!
//! Stop the T1Bridge renderer first (`systemctl --user stop t1-touchbar`),
//! run `cargo run -p t1hw --example probe -- 10`, then start it again.

use std::{
    path::Path,
    time::{Duration, Instant},
};

use t1hw::{Client, Event, Features};

fn main() {
    let seconds: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(8);
    let mut client = match Client::connect(Path::new(t1hw::SOCKET_PATH), Features::ALL) {
        Ok(client) => client,
        Err(e) => {
            eprintln!("connect failed: {e}");
            std::process::exit(1);
        }
    };
    println!(
        "connected: {}x{} minor={} max_buffers={} features={:#x}",
        client.width(),
        client.height(),
        client.info.minor,
        client.info.max_buffers,
        client.features.0
    );
    let (w, h) = (client.width() as usize, client.height() as usize);
    let start = Instant::now();
    let mut frames = 0u32;
    let mut events = Vec::new();
    let mut touch_x: Option<u32> = None;
    while start.elapsed() < Duration::from_secs(seconds) {
        events.clear();
        if let Err(e) = client.read_events(&mut events) {
            eprintln!("connection lost: {e}");
            break;
        }
        for event in &events {
            match event {
                Event::Input(frame) => {
                    touch_x = frame.contacts.iter().find(|c| c.tip).map(|c| c.x);
                    if !frame.contacts.is_empty() || frame.fn_pressed {
                        println!("input fn={} {:?}", frame.fn_pressed, frame.contacts);
                    }
                }
                Event::ActionFailed { action, code } => println!("{action} failed: {code}"),
                Event::BufferFree { .. } => {}
            }
        }
        if let Some(index) = client.free_buffer() {
            let t = start.elapsed().as_secs_f32();
            let pixels = client.buffer_pixels(index);
            for y in 0..h {
                for x in 0..w {
                    let phase = (x as f32 / w as f32 + t * 0.25).fract();
                    let r = (phase * 255.0) as u32;
                    let g = ((1.0 - phase) * 180.0) as u32;
                    let b = (y as u32 * 255 / h as u32) / 2;
                    let mut pixel = (r << 16) | (g << 8) | b;
                    if let Some(tx) = touch_x {
                        if (x as i64 - tx as i64).abs() < 30 {
                            pixel = 0x00ff_ffff;
                        }
                    }
                    pixels[y * w + x] = pixel;
                }
            }
            if let Err(e) = client.submit(index, &[]) {
                eprintln!("submit failed: {e}");
                break;
            }
            frames += 1;
        } else {
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    let secs = start.elapsed().as_secs_f32();
    println!(
        "submitted {frames} full frames in {secs:.1}s = {:.1} fps",
        frames as f32 / secs
    );
}
