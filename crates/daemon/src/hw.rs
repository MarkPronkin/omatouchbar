//! The Touch Bar hardware session: connection lifecycle, damage tracking
//! and frame submission.

use std::{
    os::fd::BorrowedFd,
    path::Path,
    time::{Duration, Instant},
};

use omatouchbar_core::Canvas;
use t1hw::{Client, Event, Features, Rect};

const BLOCK: usize = 16;

pub struct Hw {
    client: Option<Client>,
    prev: Vec<u32>,
    cur: Vec<u32>,
    next_attempt: Instant,
    backoff: Duration,
    full_next: bool,
    warned: bool,
    frames: u32,
    window_start: Instant,
    pub fps: f32,
    /// Smoothed submit-to-release time.
    pub latency_ms: f32,
}

impl Hw {
    pub fn new() -> Hw {
        Hw {
            client: None,
            prev: Vec::new(),
            cur: Vec::new(),
            next_attempt: Instant::now(),
            backoff: Duration::from_millis(250),
            full_next: true,
            warned: false,
            frames: 0,
            window_start: Instant::now(),
            fps: 0.0,
            latency_ms: 0.0,
        }
    }

    pub fn connected(&self) -> bool {
        self.client.is_some()
    }

    pub fn size(&self) -> Option<(u32, u32)> {
        self.client.as_ref().map(|c| (c.width(), c.height()))
    }

    pub fn features(&self) -> u64 {
        self.client.as_ref().map(|c| c.features.0).unwrap_or(0)
    }

    pub fn fd(&self) -> Option<BorrowedFd<'_>> {
        self.client.as_ref().map(|c| c.fd())
    }

    pub fn retry_in(&self, now: Instant) -> Duration {
        self.next_attempt.saturating_duration_since(now)
    }

    /// Attempts to connect when due. Returns true on a new connection.
    pub fn try_connect(&mut self, now: Instant) -> bool {
        if self.client.is_some() || now < self.next_attempt {
            return false;
        }
        match Client::connect(Path::new(t1hw::SOCKET_PATH), Features::ALL) {
            Ok(client) => {
                log::info!(
                    "connected to Touch Bar {}x{} (protocol 1.{}, features {:#x})",
                    client.width(),
                    client.height(),
                    client.info.minor,
                    client.features.0
                );
                let pixels = (client.width() * client.height()) as usize;
                self.prev = vec![0; pixels];
                self.cur = vec![0; pixels];
                self.client = Some(client);
                self.full_next = true;
                self.backoff = Duration::from_millis(250);
                self.warned = false;
                true
            }
            Err(e) => {
                if !self.warned {
                    log::warn!("Touch Bar hardware unavailable: {e}");
                    self.warned = true;
                }
                self.next_attempt = now + self.backoff;
                self.backoff = (self.backoff * 2).min(Duration::from_secs(5));
                false
            }
        }
    }

    pub fn disconnect(&mut self, reason: &str) {
        if self.client.take().is_some() {
            log::warn!("Touch Bar connection lost: {reason}");
            self.next_attempt = Instant::now() + Duration::from_millis(300);
        }
    }

    pub fn read(&mut self, out: &mut Vec<Event>) {
        let result = match self.client.as_mut() {
            Some(client) => client.read_events(out),
            None => return,
        };
        if let Err(e) = result {
            self.disconnect(&e.to_string());
        }
        for event in out.iter() {
            match event {
                Event::ActionFailed { action, code } => log::warn!("{action} rejected: {code}"),
                Event::BufferFree {
                    latency: Some(latency),
                } => {
                    let ms = latency.as_secs_f32() * 1000.0;
                    self.latency_ms = if self.latency_ms == 0.0 {
                        ms
                    } else {
                        self.latency_ms * 0.85 + ms * 0.15
                    };
                    if ms > 60.0 {
                        log::warn!("frame took {ms:.0}ms to present");
                    } else {
                        log::debug!("frame presented in {ms:.1}ms");
                    }
                }
                _ => {}
            }
        }
    }

    pub fn can_submit(&self) -> bool {
        self.client.as_ref().and_then(|c| c.free_buffer()).is_some()
    }

    /// Converts and submits a frame if anything changed. Returns whether a
    /// frame was sent.
    pub fn present(&mut self, canvas: &Canvas, brightness: f32, now: Instant) -> bool {
        let Some(client) = self.client.as_mut() else {
            return false;
        };
        let (w, h) = (client.width() as usize, client.height() as usize);
        if canvas.width() as usize != w || canvas.height() as usize != h {
            return false;
        }
        let Some(index) = client.free_buffer() else {
            return false;
        };
        canvas.write_xrgb(&mut self.cur, brightness);
        let damage = if self.full_next {
            Some(Vec::new())
        } else {
            damage(&self.prev, &self.cur, w, h)
        };
        let Some(damage) = damage else { return false };
        // One bounding rectangle: each rectangle is a separate transfer to
        // the panel, so a single span is cheaper than several small ones.
        let damage: Vec<Rect> = match (damage.first(), damage.last()) {
            (Some(first), Some(last)) => vec![Rect {
                x: first.x,
                y: 0,
                width: last.x + last.width - first.x,
                height: h as u32,
            }],
            _ => Vec::new(),
        };
        log::debug!(
            "present buffer {index} x={} w={}",
            damage.first().map(|r| r.x).unwrap_or(0),
            damage.first().map(|r| r.width).unwrap_or(w as u32)
        );
        client.buffer_pixels(index).copy_from_slice(&self.cur);
        if let Err(e) = client.submit(index, &damage) {
            self.disconnect(&e.to_string());
            return false;
        }
        self.full_next = false;
        std::mem::swap(&mut self.prev, &mut self.cur);
        self.frames += 1;
        let elapsed = now.saturating_duration_since(self.window_start);
        if elapsed >= Duration::from_secs(1) {
            self.fps = self.frames as f32 / elapsed.as_secs_f32();
            self.frames = 0;
            self.window_start = now;
        }
        true
    }

    pub fn idle_tick(&mut self, now: Instant) {
        if now.saturating_duration_since(self.window_start) >= Duration::from_secs(2) {
            self.fps = self.frames as f32
                / now
                    .saturating_duration_since(self.window_start)
                    .as_secs_f32();
            self.frames = 0;
            self.window_start = now;
        }
    }

    pub fn client(&mut self) -> Option<&mut Client> {
        self.client.as_mut()
    }

    pub fn has(&self, feature: Features) -> bool {
        self.client.as_ref().is_some_and(|c| c.has(feature))
    }
}

/// Column-band damage between two frames; `None` when nothing changed.
fn damage(prev: &[u32], cur: &[u32], w: usize, h: usize) -> Option<Vec<Rect>> {
    let blocks = w.div_ceil(BLOCK);
    let mut dirty = vec![false; blocks];
    for y in 0..h {
        let row = y * w;
        let a = &prev[row..row + w];
        let b = &cur[row..row + w];
        for (block, flag) in dirty.iter_mut().enumerate() {
            if *flag {
                continue;
            }
            let x0 = block * BLOCK;
            let x1 = (x0 + BLOCK).min(w);
            if a[x0..x1] != b[x0..x1] {
                *flag = true;
            }
        }
    }
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for (i, &d) in dirty.iter().enumerate() {
        if !d {
            continue;
        }
        match spans.last_mut() {
            Some((_, end)) if *end == i => *end = i + 1,
            _ => spans.push((i, i + 1)),
        }
    }
    if spans.is_empty() {
        return None;
    }
    // Merge the closest spans until within the protocol's limit.
    while spans.len() > t1hw::wire::MAX_DAMAGE {
        let (index, _) = spans
            .windows(2)
            .enumerate()
            .map(|(i, p)| (i, p[1].0 - p[0].1))
            .min_by_key(|(_, gap)| *gap)
            .unwrap();
        spans[index].1 = spans[index + 1].1;
        spans.remove(index + 1);
    }
    Some(
        spans
            .into_iter()
            .map(|(a, b)| {
                let x = (a * BLOCK) as u32;
                let x1 = ((b * BLOCK).min(w)) as u32;
                Rect {
                    x,
                    y: 0,
                    width: x1 - x,
                    height: h as u32,
                }
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn damage_finds_changed_columns() {
        let (w, h) = (100, 4);
        let prev = vec![0u32; w * h];
        let mut cur = prev.clone();
        assert!(damage(&prev, &cur, w, h).is_none());
        cur[2 * w + 40] = 1;
        cur[w + 95] = 1;
        let rects = damage(&prev, &cur, w, h).unwrap();
        assert_eq!(
            rects,
            vec![
                Rect {
                    x: 32,
                    y: 0,
                    width: 16,
                    height: 4
                },
                Rect {
                    x: 80,
                    y: 0,
                    width: 16,
                    height: 4
                }
            ]
        );
    }
}
