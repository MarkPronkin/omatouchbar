//! Elsewhen runs its date, place and globe models in JavaScript. A lazy worker
//! uses the installed models for both the hardware renderer and its GTK preview.
use std::{
    io::{BufRead, BufReader, Write},
    process::{Command, Stdio},
    sync::mpsc::{self, Sender},
    thread,
};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Request {
    pub session: u64,
    #[serde(flatten)]
    pub command: Input,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "op", content = "value", rename_all = "camelCase")]
pub enum Input {
    Open,
    Close,
    Reload,
    Back,
    Tap(String),
    Text(String),
    Backspace,
    Clear,
    Enter,
    Select(i8),
    Drag {
        id: String,
        fraction: f32,
        released: bool,
    },
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Frame {
    pub session: u64,
    pub view: String,
    pub nav: String,
    pub editing: bool,
    pub items: Vec<Item>,
    pub error: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Item {
    pub id: String,
    pub title: String,
    pub sub: String,
    pub icon: String,
    pub action: bool,
    pub active: bool,
    pub drag: bool,
    pub fraction: Option<f32>,
    pub daylight: Vec<[f32; 2]>,
    pub land: Vec<Vec<[f32; 2]>>,
    pub markers: Vec<[f32; 2]>,
    pub light: Option<[f32; 3]>,
    #[serde(deserialize_with = "avatars")]
    pub avatars: Vec<Option<crate::state::IconImage>>,
    pub initials: Vec<String>,
    pub week: Vec<Day>,
}

fn avatars<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<Option<crate::state::IconImage>>, D::Error> {
    let faces = Vec::<String>::deserialize(deserializer)?;
    Ok(faces
        .into_iter()
        .take(4)
        .map(|hex| {
            if !hex.is_ascii() || hex.len() != 24 * 24 * 8 {
                return None;
            }
            let mut pixels = Vec::with_capacity(24 * 24 * 4);
            for i in (0..hex.len()).step_by(2) {
                pixels.push(u8::from_str_radix(&hex[i..i + 2], 16).ok()?);
            }
            for rgba in pixels.chunks_exact_mut(4) {
                rgba.rotate_right(1);
            }
            crate::state::IconImage::from_argb(24, 24, pixels)
        })
        .collect())
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Day {
    pub label: String,
    pub day: String,
    pub today: bool,
    pub dim: bool,
    pub weekend: bool,
}

/// Keeps process startup and pipe I/O off the render thread. Closing stdin
/// also ends the child when its hosting daemon or preview exits.
pub struct Worker {
    tx: Sender<Request>,
}

impl Worker {
    pub fn new(preview: bool, deliver: impl Fn(Frame) + Send + Sync + 'static) -> Self {
        let (tx, rx) = mpsc::channel::<Request>();
        let deliver = std::sync::Arc::new(deliver);
        thread::spawn(move || {
            let mut process: Option<std::process::Child> = None;
            for request in rx {
                if let Some(child) = &mut process
                    && child.try_wait().ok().flatten().is_some()
                {
                    process = None;
                }
                if process.is_none() {
                    if !matches!(request.command, Input::Open) {
                        continue;
                    }
                    let mut child = match Command::new("node")
                        .arg("-e")
                        .arg(include_str!("world_clock.js"))
                        .arg(if preview { "preview" } else { "live" })
                        .stdin(Stdio::piped())
                        .stdout(Stdio::piped())
                        .spawn()
                    {
                        Ok(child) => child,
                        Err(err) => {
                            deliver(Frame {
                                session: request.session,
                                error: format!("World clock: {err}"),
                                ..Default::default()
                            });
                            continue;
                        }
                    };
                    let out = child.stdout.take().expect("piped stdout");
                    let forward = deliver.clone();
                    thread::spawn(move || {
                        for line in BufReader::new(out).lines().map_while(Result::ok) {
                            match serde_json::from_str(&line) {
                                Ok(frame) => forward(frame),
                                Err(err) => log::warn!("world clock response: {err}"),
                            }
                        }
                    });
                    process = Some(child);
                }
                let child = process.as_mut().unwrap();
                let input = child.stdin.as_mut().expect("piped stdin");
                let sent = serde_json::to_vec(&request).expect("serializable world clock request");
                if input
                    .write_all(&sent)
                    .and_then(|()| input.write_all(b"\n"))
                    .is_err()
                {
                    let _ = child.wait();
                    process = None;
                    deliver(Frame {
                        session: request.session,
                        error: "World clock stopped. Reopen to retry.".into(),
                        ..Default::default()
                    });
                }
            }
            if let Some(mut child) = process {
                drop(child.stdin.take());
                let _ = child.wait();
            }
        });
        Self { tx }
    }

    pub fn send(&self, request: Request) {
        let _ = self.tx.send(request);
    }
}
