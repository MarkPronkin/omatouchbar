//! Live internet speed test runner driven by `omarchy-network-speedtest`.
//!
//! Measures quick ping latency during the Touch Bar's ignition sweep, then
//! streams live download and upload Mbps readings straight to the scene.
//! Stopping or leaving the panel immediately terminates the active worker.

use std::{
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    thread,
};

use omatouchbar_core::SpeedTestEvent;

use crate::bus::{Bus, Event};

#[derive(Clone, Default)]
pub struct SpeedTestRunner {
    generation: Arc<AtomicU64>,
    child: Arc<Mutex<Option<Child>>>,
}

impl SpeedTestRunner {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn stop(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        if let Ok(mut guard) = self.child.lock()
            && let Some(mut c) = guard.take()
        {
            let _ = c.kill();
            let _ = c.wait();
        }
    }

    pub fn start(&self, bus: Bus) {
        self.stop();
        let my_gen = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let generation = self.generation.clone();
        let child_slot = self.child.clone();

        thread::Builder::new()
            .name("speedtest".into())
            .spawn(move || {
                // 1. Quick 2-packet ping probe (~0.25s) while ignition sweep plays.
                if let Some(ms) = probe_ping() {
                    if generation.load(Ordering::SeqCst) != my_gen {
                        return;
                    }
                    bus.send(Event::SpeedTest(SpeedTestEvent::Ping(ms)));
                }
                if generation.load(Ordering::SeqCst) != my_gen {
                    return;
                }

                // 2. Download phase via `omarchy-network-speedtest down`.
                match run_phase("down", my_gen, &generation, &child_slot, &bus) {
                    Ok(last_down) => {
                        if generation.load(Ordering::SeqCst) != my_gen {
                            return;
                        }
                        bus.send(Event::SpeedTest(SpeedTestEvent::DownloadDone(last_down)));
                    }
                    Err(None) => return, // cancelled
                    Err(Some(err)) => {
                        if generation.load(Ordering::SeqCst) == my_gen {
                            bus.send(Event::SpeedTest(SpeedTestEvent::Error(err)));
                        }
                        return;
                    }
                }

                if generation.load(Ordering::SeqCst) != my_gen {
                    return;
                }
                bus.send(Event::SpeedTest(SpeedTestEvent::UploadStart));

                // 3. Upload phase via `omarchy-network-speedtest up`.
                match run_phase("up", my_gen, &generation, &child_slot, &bus) {
                    Ok(last_up) => {
                        if generation.load(Ordering::SeqCst) == my_gen {
                            bus.send(Event::SpeedTest(SpeedTestEvent::UploadDone(last_up)));
                        }
                    }
                    Err(None) => {}
                    Err(Some(err)) => {
                        if generation.load(Ordering::SeqCst) == my_gen {
                            bus.send(Event::SpeedTest(SpeedTestEvent::Error(err)));
                        }
                    }
                }
            })
            .ok();
    }
}

impl Drop for SpeedTestRunner {
    fn drop(&mut self) {
        self.stop();
    }
}

fn probe_ping() -> Option<f32> {
    let out = Command::new("ping")
        .args(["-n", "-c", "2", "-i", "0.2", "-W", "1", "1.1.1.1"])
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut sum = 0.0f32;
    let mut count = 0u32;
    for line in text.lines() {
        if let Some((_, rest)) = line.split_once("time=")
            && let Some(num) = rest.split_whitespace().next()
            && let Ok(ms) = num.parse::<f32>()
            && ms.is_finite()
            && ms >= 0.0
        {
            sum += ms;
            count += 1;
        }
    }
    (count > 0).then(|| sum / count as f32)
}

/// Runs one direction (`"down"` or `"up"`), streaming samples to the bus.
/// Returns `Err(None)` if cancelled, `Err(Some(msg))` on failure.
fn run_phase(
    direction: &str,
    my_gen: u64,
    generation: &AtomicU64,
    child_slot: &Mutex<Option<Child>>,
    bus: &Bus,
) -> Result<f32, Option<String>> {
    let mut child = Command::new("omarchy-network-speedtest")
        .arg(direction)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Some(format!("speedtest unavailable: {e}")))?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    if let Ok(mut guard) = child_slot.lock() {
        if generation.load(Ordering::SeqCst) != my_gen {
            let _ = child.kill();
            let _ = child.wait();
            return Err(None);
        }
        *guard = Some(child);
    }

    // Drain stderr on a helper thread so the pipe never stalls.
    let stderr_handle = stderr.map(|err| {
        thread::spawn(move || {
            let mut text = String::new();
            let _ = std::io::Read::read_to_string(&mut BufReader::new(err), &mut text);
            text.trim().to_string()
        })
    });

    let mut last_mbps = 0.0f32;
    if let Some(out) = stdout {
        for line in BufReader::new(out).lines() {
            if generation.load(Ordering::SeqCst) != my_gen {
                return Err(None);
            }
            let Ok(line) = line else { break };
            if let Ok(val) = line.trim().parse::<f32>()
                && val.is_finite()
                && val >= 0.0
            {
                last_mbps = val;
                let event = if direction == "down" {
                    SpeedTestEvent::DownloadSample(val)
                } else {
                    SpeedTestEvent::UploadSample(val)
                };
                bus.send(Event::SpeedTest(event));
            }
        }
    }

    let status = if let Ok(mut guard) = child_slot.lock() {
        guard.take().and_then(|mut c| c.wait().ok())
    } else {
        None
    };
    let err_text = stderr_handle
        .and_then(|h| h.join().ok())
        .unwrap_or_default();

    if generation.load(Ordering::SeqCst) != my_gen {
        return Err(None);
    }

    if status.is_some_and(|s| s.success()) {
        Ok(last_mbps)
    } else {
        let msg = if err_text.is_empty() {
            "Speed test failed".into()
        } else {
            err_text
                .lines()
                .next()
                .unwrap_or("Speed test failed")
                .to_string()
        };
        Err(Some(msg))
    }
}
