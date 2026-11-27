//! Plays Omarchy's screensaver on the Touch Bar alongside the desktop's.
//!
//! The desktop screensaver is a terminal (window class
//! `org.omarchy.screensaver`) running `omarchy-screensaver`, a shell loop
//! that starts `ttfx --random-effect` on the branding text again each time
//! the last effect finishes. The Hyprland provider reports the window; while
//! it is open this follows the loop's `ttfx` children, and every time a new
//! one starts it starts a fresh random effect of its own, sized for the
//! strip, so both screens begin each effect together. An effect that settles
//! first holds its logo until the desktop moves on, as the desktop does.
//! Touching the strip ends the desktop screensaver the way a key press
//! does: its shell's signal trap restores the cursor and closes it.
//!
//! `ttfx` paces itself at Omarchy's 120 fps (effects advance per frame, so
//! that is also their speed). A reader thread turns its output back into
//! frames and leaves the newest for the render loop, which draws at its own
//! frame rate.

use std::{
    io::{Read, Write},
    os::unix::process::CommandExt,
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use omatouchbar_core::screensaver::{self, Geometry, Grid, Parser};

use crate::bus::{Bus, Event};

/// What the render loop hears from the screensaver.
pub enum SaverEvent {
    /// Show the screensaver, on cells laid out like this.
    Start(Geometry),
    /// A new frame is waiting in [`Mirror::take_frame`].
    Frame,
    Stop,
}

/// How often the desktop's effect loop is checked for a new effect.
const POLL: Duration = Duration::from_millis(100);
/// How long to look for the desktop's loop before playing on our own (a
/// custom screensaver in the same window class).
const FIND_LOOP: Duration = Duration::from_secs(4);
/// Pause between effects when playing on our own (the desktop's loop polls
/// once a second).
const OWN_HOLD: Duration = Duration::from_millis(1500);

#[derive(Default)]
struct Slot {
    frame: Option<Arc<Grid>>,
    /// A wake-up is on its way to the render loop.
    signalled: bool,
}

struct Shared {
    bus: Bus,
    slot: Mutex<Slot>,
    /// The effect whose frames are current; older readers drop theirs.
    generation: AtomicU64,
    effect: Mutex<Option<String>>,
}

impl Shared {
    fn publish(&self, generation: u64, grid: &Grid) {
        if self.generation.load(Ordering::Relaxed) != generation {
            return;
        }
        let wake = {
            let Ok(mut slot) = self.slot.lock() else {
                return;
            };
            slot.frame = Some(Arc::new(grid.clone()));
            !std::mem::replace(&mut slot.signalled, true)
        };
        if wake {
            self.bus.send(Event::Screensaver(SaverEvent::Frame));
        }
    }
}

/// What the worker plays to.
#[derive(Clone, Debug, PartialEq)]
enum Lead {
    /// Follow the desktop screensaver's effect loop.
    Desktop,
    /// `omatouchbar ctl screensaver`: effects back to back, on the strip
    /// only; a named effect repeats.
    Demo(Option<String>),
}

struct Worker {
    lead: Lead,
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

impl Worker {
    fn halt(self) {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.thread().unpark();
        let _ = self.thread.join();
    }
}

/// The screensaver's state on the render thread.
pub struct Mirror {
    shared: Arc<Shared>,
    size: (u32, u32),
    enabled: bool,
    /// The desktop screensaver's window is open.
    desktop: bool,
    /// The strip was touched: stay out until the desktop's window closes.
    dismissed: bool,
    demo: Option<Option<String>>,
    worker: Option<Worker>,
}

impl Mirror {
    pub fn new(bus: Bus, size: (u32, u32)) -> Mirror {
        Mirror {
            shared: Arc::new(Shared {
                bus,
                slot: Mutex::new(Slot::default()),
                generation: AtomicU64::new(0),
                effect: Mutex::new(None),
            }),
            size,
            enabled: true,
            desktop: false,
            dismissed: false,
            demo: None,
            worker: None,
        }
    }

    /// Follows the setting (`screensaver` in config.toml).
    pub fn configure(&mut self, enabled: bool) {
        if enabled != self.enabled {
            self.enabled = enabled;
            self.sync();
        }
    }

    /// The desktop screensaver's window opened or closed.
    pub fn set_desktop(&mut self, open: bool) {
        if open != self.desktop {
            log::info!(
                "desktop screensaver {}",
                if open { "started" } else { "ended" }
            );
            self.desktop = open;
            if !open {
                self.dismissed = false;
                self.demo = None;
            }
            self.sync();
        }
    }

    /// `ctl screensaver [effect]` plays on the strip alone; `off` stops it.
    pub fn demo(&mut self, effect: Option<String>) -> Result<(), String> {
        match effect.as_deref() {
            Some("off" | "stop") => self.demo = None,
            Some(name) if !effects().iter().any(|e| e == name) => {
                return Err(format!("unknown effect '{name}'"));
            }
            _ => {
                self.dismissed = false;
                self.demo = Some(effect);
                // Restart even if one is already playing.
                if let Some(worker) = self.worker.take() {
                    worker.halt();
                }
            }
        }
        self.sync();
        Ok(())
    }

    /// The strip was touched during the screensaver: end it everywhere.
    pub fn dismiss(&mut self) {
        self.demo = None;
        if self.desktop {
            self.dismissed = true;
            let shells = find_shells();
            log::info!("touch ends the desktop screensaver ({shells:?})");
            for pid in shells {
                if let Some(pid) = rustix::process::Pid::from_raw(pid as i32) {
                    let _ = rustix::process::kill_process(pid, rustix::process::Signal::TERM);
                }
            }
        }
        self.sync();
    }

    /// The newest frame, if one came since the last call.
    pub fn take_frame(&self) -> Option<Arc<Grid>> {
        let mut slot = self.shared.slot.lock().ok()?;
        slot.signalled = false;
        slot.frame.take()
    }

    /// The effect playing now (for `omatouchbar status`).
    pub fn effect(&self) -> Option<String> {
        self.worker.as_ref()?;
        self.shared.effect.lock().ok()?.clone()
    }

    pub fn stop(&mut self) {
        self.demo = None;
        self.desktop = false;
        self.sync();
    }

    fn sync(&mut self) {
        let lead = if let Some(effect) = &self.demo {
            Some(Lead::Demo(effect.clone()))
        } else if self.desktop && self.enabled && !self.dismissed {
            Some(Lead::Desktop)
        } else {
            None
        };
        if self.worker.as_ref().map(|w| &w.lead) == lead.as_ref() {
            return;
        }
        if let Some(worker) = self.worker.take() {
            worker.halt();
        }
        let Some(lead) = lead else {
            self.shared.generation.fetch_add(1, Ordering::Relaxed);
            if let Ok(mut slot) = self.shared.slot.lock() {
                slot.frame = None;
            }
            self.shared.bus.send(Event::Screensaver(SaverEvent::Stop));
            return;
        };
        let (text, geometry) = screensaver::branding(self.size.0, self.size.1);
        self.shared
            .bus
            .send(Event::Screensaver(SaverEvent::Start(geometry)));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let (shared, stop, lead) = (self.shared.clone(), stop.clone(), lead.clone());
            thread::Builder::new()
                .name("screensaver".into())
                .spawn(move || play(&shared, &stop, &lead, &text, geometry))
        };
        match thread {
            Ok(thread) => {
                self.worker = Some(Worker { lead, stop, thread });
            }
            Err(e) => {
                log::warn!("could not start screensaver worker: {e}");
                self.shared.bus.send(Event::Screensaver(SaverEvent::Stop));
            }
        }
    }
}

impl Drop for Mirror {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() {
            worker.halt();
        }
    }
}

/// One `ttfx` run and the thread reading its frames.
struct Effect {
    child: Child,
    reader: Option<JoinHandle<()>>,
    started: Instant,
}

impl Effect {
    fn start(shared: &Arc<Shared>, name: &str, text: &str, g: Geometry) -> Option<Effect> {
        let generation = shared.generation.fetch_add(1, Ordering::Relaxed) + 1;
        let mut command = Command::new("ttfx");
        command
            .args(["--frame-rate", &screensaver::FRAME_RATE.to_string()])
            .args(["--canvas-width", &g.cols.to_string()])
            .args(["--canvas-height", &g.rows.to_string()])
            .args(["--ignore-terminal-dimensions"])
            .args(["--anchor-canvas", "c", "--anchor-text", "c"])
            .args(["--no-eol", "--no-restore-cursor"])
            .arg(name)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        // Never outlive the renderer, even if it is killed.
        unsafe {
            command.pre_exec(|| {
                rustix::process::set_parent_process_death_signal(Some(
                    rustix::process::Signal::KILL,
                ))
                .map_err(std::io::Error::from)
            });
        }
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(e) => {
                log::warn!("screensaver: ttfx: {e}");
                return None;
            }
        };
        // The art is a few kilobytes: it fits the pipe, so this can't block.
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
        let mut stdout = child.stdout.take()?;
        let reader = {
            let shared = shared.clone();
            let (cols, rows) = (g.cols as usize, g.rows as usize);
            thread::Builder::new()
                .name("screensaver-frames".into())
                .spawn(move || {
                    let mut parser = Parser::new(cols, rows);
                    let mut publish = |grid: &Grid| shared.publish(generation, grid);
                    let mut buffer = vec![0u8; 1 << 16];
                    loop {
                        match stdout.read(&mut buffer) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => parser.feed(&buffer[..n], &mut publish),
                        }
                    }
                    parser.finish(&mut publish);
                })
                .ok()
        };
        if let Ok(mut effect) = shared.effect.lock() {
            *effect = Some(name.to_string());
        }
        log::info!("screensaver: {name}");
        Some(Effect {
            child,
            reader,
            started: Instant::now(),
        })
    }

    /// Whether `ttfx` finished on its own.
    fn finished(&mut self) -> bool {
        !matches!(self.child.try_wait(), Ok(None))
    }

    fn end(mut self, shared: &Shared) {
        // Whatever it prints while dying is no longer wanted.
        shared.generation.fetch_add(1, Ordering::Relaxed);
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

/// The worker: starts an effect each time the lead does, until stopped.
fn play(shared: &Arc<Shared>, stop: &AtomicBool, lead: &Lead, text: &str, g: Geometry) {
    let began = Instant::now();
    let mut rng = Rng::seeded();
    let mut last: Option<String> = None;
    let mut effect: Option<Effect> = None;
    let mut finished_at: Option<Instant> = None;
    let mut shells: Vec<u32> = Vec::new();
    let mut scanned: Option<Instant> = None;
    let mut desktop_ttfx: Option<u32> = None;
    let mut own = matches!(lead, Lead::Demo(_));
    let next = |rng: &mut Rng, last: &mut Option<String>| -> String {
        let name = match lead {
            Lead::Demo(Some(name)) => name.clone(),
            _ => pick(effects(), rng, last.as_deref()),
        };
        *last = Some(name.clone());
        name
    };
    while !stop.load(Ordering::Relaxed) {
        let now = Instant::now();
        let mut restart = false;
        if !own {
            if shells.iter().all(|&pid| !alive(pid))
                && scanned.is_none_or(|t| now.duration_since(t) > Duration::from_secs(1))
            {
                shells = find_shells();
                scanned = Some(now);
            }
            match shells.iter().find_map(|&pid| ttfx_child(pid)) {
                Some(pid) if Some(pid) != desktop_ttfx => {
                    desktop_ttfx = Some(pid);
                    restart = true;
                }
                _ => {}
            }
            if desktop_ttfx.is_none() && now.duration_since(began) > FIND_LOOP {
                log::info!("screensaver: no omarchy-screensaver loop found; playing on our own");
                own = true;
            }
        }
        if let Some(e) = effect.as_mut()
            && finished_at.is_none()
            && e.finished()
        {
            finished_at = Some(now);
            log::debug!(
                "screensaver: effect settled after {:.1} s",
                e.started.elapsed().as_secs_f32()
            );
        }
        if own
            && (effect.is_none() || finished_at.is_some_and(|t| now.duration_since(t) > OWN_HOLD))
        {
            restart = true;
        }
        if restart {
            if let Some(e) = effect.take() {
                e.end(shared);
            }
            let name = next(&mut rng, &mut last);
            effect = Effect::start(shared, &name, text, g);
            finished_at = None;
            if effect.is_none() {
                shared.bus.send(Event::Screensaver(SaverEvent::Stop));
                break;
            }
        }
        thread::park_timeout(POLL);
    }
    if let Some(e) = effect.take() {
        e.end(shared);
    }
    if let Ok(mut effect) = shared.effect.lock() {
        *effect = None;
    }
}

// ------------------------------------------------------------ processes

/// The `omarchy-screensaver` shells (one per monitor), oldest first.
fn find_shells() -> Vec<u32> {
    let mut pids: Vec<u32> = std::fs::read_dir("/proc")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.parse().ok())
        .filter(|&pid: &u32| {
            std::fs::read(format!("/proc/{pid}/cmdline")).is_ok_and(|cmdline| {
                let mut args = cmdline.split(|&b| b == 0);
                let first = args.next().unwrap_or_default();
                // `bash …/omarchy-screensaver` (or the script itself).
                std::iter::once(first)
                    .chain(args.take(1))
                    .any(|arg| arg.rsplit(|&b| b == b'/').next() == Some(b"omarchy-screensaver"))
            })
        })
        .collect();
    pids.sort_unstable();
    pids
}

fn alive(pid: u32) -> bool {
    std::path::Path::new(&format!("/proc/{pid}")).exists()
}

/// The `ttfx` a screensaver shell is running, if any.
fn ttfx_child(shell: u32) -> Option<u32> {
    let children = std::fs::read_to_string(format!("/proc/{shell}/task/{shell}/children")).ok()?;
    children
        .split_whitespace()
        .filter_map(|pid| pid.parse().ok())
        .find(|pid: &u32| {
            std::fs::read_to_string(format!("/proc/{pid}/comm")).is_ok_and(|c| c.trim() == "ttfx")
        })
}

// -------------------------------------------------------------- effects

/// The effects this `ttfx` has, from its help (the same pool its
/// `--random-effect` draws from).
fn effects() -> &'static [String] {
    static EFFECTS: OnceLock<Vec<String>> = OnceLock::new();
    EFFECTS.get_or_init(|| {
        let help = Command::new("ttfx")
            .arg("--help")
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default();
        let list = parse_effects(&help);
        if list.is_empty() {
            log::warn!("screensaver: couldn't list ttfx effects");
            return vec!["decrypt".into()];
        }
        list
    })
}

fn parse_effects(help: &str) -> Vec<String> {
    help.lines()
        .skip_while(|l| l.trim() != "Commands:")
        .skip(1)
        .take_while(|l| !l.trim().is_empty() && l.starts_with(' '))
        .filter_map(|l| l.split_whitespace().next())
        .filter(|name| *name != "help")
        .map(str::to_string)
        .collect()
}

/// A random effect other than the last one.
fn pick(all: &[String], rng: &mut Rng, last: Option<&str>) -> String {
    let pool: Vec<&String> = all
        .iter()
        .filter(|e| Some(e.as_str()) != last || all.len() == 1)
        .collect();
    pool[rng.below(pool.len())].clone()
}

/// xorshift64*, seeded from the clock: enough to shuffle effects.
struct Rng(u64);

impl Rng {
    fn seeded() -> Rng {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x5eed);
        Rng(nanos | 1)
    }

    fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 33) as usize % n.max(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_effects_from_help() {
        let help = "Terminal text effects\n\nUsage: ttfx [OPTIONS] [COMMAND]\n\nCommands:\n  beams            Create beams\n  binarypath       Binary\n  help             Print this message\n\nOptions:\n  -v, --version\n";
        assert_eq!(parse_effects(help), vec!["beams", "binarypath"]);
    }

    #[test]
    fn picks_a_different_effect_each_time() {
        let mut rng = Rng(42);
        let mut last = None;
        let pool = ["beams".into(), "decrypt".into(), "matrix".into()];
        for _ in 0..50 {
            let next = pick(&pool, &mut rng, last.as_deref());
            assert_ne!(Some(&next), last.as_ref());
            last = Some(next);
        }
        assert_eq!(pick(&pool[..1], &mut rng, Some("beams")), "beams");
    }
}
