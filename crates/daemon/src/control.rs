//! Line-based control socket at `$XDG_RUNTIME_DIR/omatouchbar.sock`.

use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::{UnixListener, UnixStream},
    sync::mpsc,
    thread,
    time::Duration,
};

use omatouchbar_core::{
    Command,
    config::Mode,
    paths,
    scene::{Panel, SoundTab, TouchIdDemo},
};

use crate::bus::{Bus, Event};

pub fn parse(words: &[&str]) -> Result<Option<Command>, String> {
    Ok(Some(match words {
        ["menu"] => Command::OpenMenu(None),
        ["menu", id] => Command::OpenMenu(Some(id.to_string())),
        ["panel", "mic" | "microphone" | "input"] => Command::OpenSound(SoundTab::Input),
        ["panel", "sources" | "apps" | "mixer"] => Command::OpenSound(SoundTab::Sources),
        ["panel", name] => Command::OpenPanel(match *name {
            "sound" | "audio" | "volume" => Panel::Audio,
            "brightness" | "display" => Panel::Brightness,
            "battery" | "power" => Panel::Power,
            "network" | "wifi" => Panel::Network,
            "bluetooth" => Panel::Bluetooth,
            "tray" => Panel::Tray,
            "weather" => Panel::Weather,
            "agents" => Panel::Agents,
            "indicators" | "toggles" | "ind" => Panel::Indicators,
            "clock" | "time" | "elsewhen" | "world-clock" => Panel::Clock,
            other => return Err(format!("unknown panel '{other}'")),
        }),
        ["speedtest"] => Command::OpenSpeedTest,
        ["touchid"] | ["touchid", "unlock" | "demo"] => Command::TouchIdDemo(TouchIdDemo::Unlock),
        ["touchid", "enroll"] => Command::TouchIdDemo(TouchIdDemo::Enroll),
        ["home"] => Command::Home,
        ["wake"] => Command::Wake,
        ["mode", name] => Command::SetMode(match *name {
            "bar" => Mode::Bar,
            "keyboard" | "keys" => Mode::Keyboard,
            "controls" => Mode::Controls,
            "function" | "fn" => Mode::Function,
            other => return Err(format!("unknown mode '{other}'")),
        }),
        ["status"] | ["controls"] | ["reload"] | ["snapshot", _] | ["debug", "on" | "off"] => {
            return Ok(None);
        }
        ["screensaver"] | ["screensaver", _] => return Ok(None),
        ["haptics", "on" | "off"] => return Ok(None),
        _ => return Err(format!("unknown command '{}'", words.join(" "))),
    }))
}

fn handle(stream: UnixStream, bus: &Bus) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    let words: Vec<&str> = line.split_whitespace().collect();
    if words == ["input"] {
        let bus = bus.clone();
        thread::Builder::new()
            .name("control-input".into())
            .spawn(move || crate::input::serve(reader, bus))
            .ok();
        return;
    }
    let (tx, rx) = mpsc::channel();
    let reply = match words.as_slice() {
        ["haptics", state @ ("on" | "off")] => {
            bus.send(Event::Haptics(*state == "on", tx));
            rx.recv_timeout(Duration::from_secs(2))
                .unwrap_or_else(|_| "error: renderer busy".into())
        }
        ["haptics", _] => "error: use haptics on or off".into(),
        [command @ ("status" | "controls")] => {
            bus.send(if *command == "status" {
                Event::Status(crate::haptics::available(), tx)
            } else {
                Event::Controls(tx)
            });
            rx.recv_timeout(Duration::from_secs(2))
                .unwrap_or_else(|_| "error: renderer busy".into())
        }
        ["reload"] => {
            bus.send(Event::Reload);
            "ok".into()
        }
        ["debug", state] => {
            bus.send(Event::Debug(*state == "on"));
            "ok".into()
        }
        ["screensaver", rest @ ..] if rest.len() <= 1 => {
            bus.send(Event::ScreensaverDemo(
                rest.first().map(|s| s.to_string()),
                tx,
            ));
            rx.recv_timeout(Duration::from_secs(2))
                .unwrap_or_else(|_| "error: renderer busy".into())
        }
        ["snapshot", path] => {
            bus.send(Event::Snapshot(path.to_string(), tx));
            rx.recv_timeout(Duration::from_secs(2))
                .unwrap_or_else(|_| "error: renderer busy".into())
        }
        _ => match parse(&words) {
            Ok(Some(command)) => {
                bus.send(Event::Control(command, tx));
                rx.recv_timeout(Duration::from_secs(2))
                    .unwrap_or_else(|_| "error: renderer busy".into())
            }
            Ok(None) => "ok".into(),
            Err(e) => format!("error: {e}"),
        },
    };
    let _ = writeln!(reader.get_mut(), "{reply}");
}

pub fn serve(bus: Bus) {
    let path = paths::control_socket();
    // A live socket means another renderer is running; a stale one is removed.
    if UnixStream::connect(&path).is_ok() {
        log::warn!("control socket {} is in use; not serving", path.display());
        return;
    }
    let _ = std::fs::remove_file(&path);
    let listener = match UnixListener::bind(&path) {
        Ok(listener) => listener,
        Err(e) => {
            log::warn!("control socket {}: {e}", path.display());
            return;
        }
    };
    thread::Builder::new()
        .name("control".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                handle(stream, &bus);
            }
        })
        .ok();
}

pub fn remove_socket() {
    let _ = std::fs::remove_file(paths::control_socket());
}

/// Sends one command to the running renderer and prints the reply.
pub fn client(args: &[String]) -> i32 {
    if args.is_empty() {
        eprintln!("usage: omatouchbar ctl <command>");
        return 2;
    }
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    match crate::input::Gesture::parse(&words) {
        Ok(Some(gesture)) => {
            return match gesture.run() {
                Ok(()) => {
                    println!("ok");
                    0
                }
                Err(e) => {
                    eprintln!("omatouchbar: {e}");
                    1
                }
            };
        }
        Err(e) => {
            eprintln!("omatouchbar: {e}");
            return 2;
        }
        Ok(None) => {}
    }
    if let Err(e) = parse(&words) {
        eprintln!("omatouchbar: {e}");
        return 2;
    }
    let mut stream = match UnixStream::connect(paths::control_socket()) {
        Ok(s) => s,
        Err(_) => {
            eprintln!(
                "omatouchbar is not running (is it the selected T1Bridge renderer? try `omatouchbar enable`)"
            );
            return 1;
        }
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
    if writeln!(stream, "{}", words.join(" ")).is_err() {
        return 1;
    }
    let mut reply = String::new();
    if BufReader::new(&stream).read_line(&mut reply).unwrap_or(0) == 0 {
        eprintln!("omatouchbar: no renderer reply");
        return 1;
    }
    let reply = reply.trim();
    println!("{reply}");
    if reply.starts_with("error") { 1 } else { 0 }
}
