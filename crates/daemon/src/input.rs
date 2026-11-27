//! Scripted contacts share the physical strip's hit testing and gesture timers.
use std::{
    io::{self, BufRead, BufReader, Read, Write},
    os::unix::net::UnixStream,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use omatouchbar_core::{KeyInput, Scene, paths};
use serde::Deserialize;
use serde_json::json;

use crate::{
    bus::{Bus, Event},
    haptics::Player,
};

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Frame {
    contacts: Vec<(u8, f32, f32)>,
    #[serde(rename = "fn")]
    function: bool,
    key: Option<KeyInput>,
}

pub enum Request {
    Begin,
    Frame(Frame),
    End,
}

#[derive(Default)]
pub struct Input {
    session: Option<u64>,
    physical_active: bool,
}

impl Input {
    pub fn physical(
        &mut self,
        scene: &mut Scene,
        player: &Player,
        function: bool,
        contacts: &[(u8, f32, f32)],
        now: Instant,
    ) {
        self.physical_active = function || !contacts.is_empty();
        if self.session.is_some() {
            if !self.physical_active {
                return;
            }
            self.session = None;
            scene.cancel_input(now);
        }
        Self::apply(scene, Some(player), function, contacts, now);
    }

    pub fn disconnected(&mut self, scene: &mut Scene, now: Instant) {
        if self.physical_active {
            self.physical_active = false;
            scene.cancel_input(now);
        }
    }

    fn apply(
        scene: &mut Scene,
        player: Option<&Player>,
        function: bool,
        contacts: &[(u8, f32, f32)],
        now: Instant,
    ) {
        if let Some(position) = scene.input_press_position(function, contacts, now)
            && let Some(player) = player
        {
            player.press(position);
        }
    }

    pub fn request(
        &mut self,
        session: u64,
        request: Request,
        scene: &mut Scene,
        player: Option<&Player>,
        size: (u32, u32),
    ) -> Result<(), &'static str> {
        let now = Instant::now();
        match request {
            Request::Begin => {
                if self.physical_active || self.session.is_some() {
                    return Err("Touch Bar input is busy");
                }
                self.session = Some(session);
            }
            Request::End => {
                if self.session == Some(session) {
                    self.session = None;
                    scene.cancel_input(now);
                }
            }
            Request::Frame(frame) => {
                if self.session != Some(session) {
                    return Err("input session interrupted");
                }
                let mut ids = [false; 256];
                if frame.contacts.len() > t1hw::wire::MAX_CONTACTS
                    || frame.contacts.iter().any(|&(id, x, y)| {
                        std::mem::replace(&mut ids[id as usize], true)
                            || !(0.0..size.0 as f32).contains(&x)
                            || !(0.0..size.1 as f32).contains(&y)
                    })
                {
                    return Err("invalid contact ID, count or coordinates");
                }
                if frame.key.is_some() && !scene.wants_keyboard() {
                    return Err("no active text prompt");
                }
                Self::apply(scene, player, frame.function, &frame.contacts, now);
                if let Some(key) = frame.key {
                    scene.key_input(key, now);
                }
            }
        }
        Ok(())
    }
}

fn ask(bus: &Bus, id: u64, request: Request) -> String {
    let (tx, rx) = mpsc::channel();
    bus.send(Event::Input(id, request, tx));
    rx.recv_timeout(Duration::from_secs(2))
        .unwrap_or_else(|_| "error: renderer busy".into())
}

/// A session owns its contacts until an explicit release, a disconnect, or physical input.
pub fn serve(mut stream: BufReader<UnixStream>, bus: Bus) {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let _ = stream
        .get_ref()
        .set_read_timeout(Some(Duration::from_secs(30)));
    let mut response = ask(&bus, id, Request::Begin);
    while writeln!(stream.get_mut(), "{response}").is_ok() && response == "ok" {
        let mut line = String::new();
        // Bound both memory and the time an abandoned contact can repeat.
        if (&mut stream).take(8193).read_line(&mut line).is_err() || !line.ends_with('\n') {
            break;
        }
        if line.trim_end() == "end" {
            let _ = writeln!(stream.get_mut(), "{}", ask(&bus, id, Request::End));
            return;
        }
        response = match serde_json::from_str(&line) {
            Ok(frame) => ask(&bus, id, Request::Frame(frame)),
            Err(_) => "error: invalid input frame".into(),
        };
    }
    ask(&bus, id, Request::End);
}

fn exchange(stream: &mut BufReader<UnixStream>, message: &str) -> io::Result<()> {
    writeln!(stream.get_mut(), "{message}")?;
    let mut reply = String::new();
    stream.read_line(&mut reply)?;
    match reply.trim_end() {
        "ok" => Ok(()),
        "" => Err(io::Error::other("renderer disconnected")),
        error => Err(io::Error::other(error)),
    }
}

#[derive(Debug)]
pub enum Gesture {
    Stream,
    Stroke {
        start: [u32; 2],
        end: [u32; 2],
        duration: Duration,
    },
}

impl Gesture {
    pub fn parse(words: &[&str]) -> Result<Option<Self>, &'static str> {
        fn point(x: &str, y: &str) -> Result<[u32; 2], &'static str> {
            Ok([
                x.parse().map_err(|_| "invalid x coordinate")?,
                y.parse().map_err(|_| "invalid y coordinate")?,
            ])
        }
        fn duration(ms: &str) -> Result<Duration, &'static str> {
            ms.parse::<u64>()
                .ok()
                .filter(|n| (1..=60000).contains(n))
                .map(Duration::from_millis)
                .ok_or("duration must be 1–60000 ms")
        }
        let (start, end, duration) = match words {
            ["input"] => return Ok(Some(Self::Stream)),
            ["tap", x, y] => {
                let p = point(x, y)?;
                (p, p, Duration::ZERO)
            }
            ["hold", x, y, ms] => {
                let p = point(x, y)?;
                (p, p, duration(ms)?)
            }
            ["drag", x, y, end_x, end_y, ms] => (point(x, y)?, point(end_x, end_y)?, duration(ms)?),
            _ => return Ok(None),
        };
        Ok(Some(Self::Stroke {
            start,
            end,
            duration,
        }))
    }

    pub fn run(self) -> io::Result<()> {
        let socket = UnixStream::connect(paths::control_socket())?;
        socket.set_read_timeout(Some(Duration::from_secs(3)))?;
        socket.set_write_timeout(Some(Duration::from_secs(3)))?;
        let mut stream = BufReader::new(socket);
        exchange(&mut stream, "input")?;
        match self {
            Self::Stream => {
                for line in io::stdin().lock().lines() {
                    let line = line?;
                    if line.trim_end() == "end" {
                        break;
                    }
                    exchange(&mut stream, &line)?;
                    println!("ok");
                }
            }
            Self::Stroke {
                start,
                end,
                duration,
            } => {
                let frame = |x: f64, y: f64| json!({"contacts": [[0, x, y]]}).to_string();
                exchange(&mut stream, &frame(start[0].into(), start[1].into()))?;
                if !duration.is_zero() {
                    let since = Instant::now();
                    let interval = if start == end {
                        Duration::from_secs(10)
                    } else {
                        Duration::from_millis(16)
                    };
                    loop {
                        thread::sleep(interval.min(duration.saturating_sub(since.elapsed())));
                        let elapsed = since.elapsed();
                        if elapsed >= duration {
                            exchange(&mut stream, &frame(end[0].into(), end[1].into()))?;
                            break;
                        }
                        let fraction = elapsed.as_secs_f64() / duration.as_secs_f64();
                        let position = |axis: usize| {
                            start[axis] as f64 + (end[axis] as f64 - start[axis] as f64) * fraction
                        };
                        exchange(&mut stream, &frame(position(0), position(1)))?;
                    }
                }
                exchange(&mut stream, "{}")?;
            }
        }
        exchange(&mut stream, "end")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omatouchbar_core::{
        Config, DesktopState, Fonts, Palette, T1_HEIGHT, T1_WIDTH,
        config::Mode,
        menu::MenuTree,
        node::Key,
        scene::{Page, Panel},
    };

    #[test]
    fn scripted_contacts_release_only_on_explicit_up_and_yield_to_hardware() {
        let size = (T1_WIDTH, T1_HEIGHT);
        let mut config = Config {
            mode: Mode::Bar,
            ..Default::default()
        };
        config.motion.enabled = false;
        let mut scene = Scene::new(
            size.0,
            size.1,
            config,
            Palette::fallback(),
            Fonts::from_files(None, None, None, None, None),
            MenuTree::default(),
            None,
        );
        scene.set_state(DesktopState::sample());
        scene.update(Instant::now(), chrono::Utc::now());
        let rect = scene.node_rect(Key::new("w/audio")).unwrap();
        let down = || {
            Request::Frame(Frame {
                contacts: vec![(0, rect.cx(), rect.cy())],
                ..Frame::default()
            })
        };
        let mut input = Input::default();
        let player = Player::new(false);
        input
            .request(1, Request::Begin, &mut scene, None, size)
            .unwrap();
        input.request(1, down(), &mut scene, None, size).unwrap();
        assert!(
            input
                .request(2, Request::Begin, &mut scene, None, size)
                .is_err()
        );
        input.physical(&mut scene, &player, false, &[], Instant::now());
        assert_eq!(input.session, Some(1));
        // Losing the client in mid-tap must not activate the Sound panel.
        input
            .request(1, Request::End, &mut scene, None, size)
            .unwrap();
        assert_eq!(scene.page(), &Page::Bar);

        input
            .request(2, Request::Begin, &mut scene, None, size)
            .unwrap();
        input.request(2, down(), &mut scene, None, size).unwrap();
        input.physical(
            &mut scene,
            &player,
            false,
            &[(0, rect.cx(), rect.cy())],
            Instant::now(),
        );
        assert!(
            input
                .request(2, Request::Frame(Frame::default()), &mut scene, None, size)
                .is_err()
        );
        input
            .request(2, Request::End, &mut scene, None, size)
            .unwrap();
        assert_eq!(scene.page(), &Page::Bar);
        input.physical(&mut scene, &player, false, &[], Instant::now());
        assert_eq!(scene.page(), &Page::Panel(Panel::Audio));
    }
}
