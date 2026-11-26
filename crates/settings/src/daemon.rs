//! Talks to the omatouchbar renderer: status, renderer selection, restart.

use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    process::Command,
    time::Duration,
};

use omatouchbar_core::paths;

#[derive(Clone, Debug, Default)]
pub struct Status {
    pub running: bool,
    pub connected: bool,
    pub width: u64,
    pub height: u64,
    pub fps: f64,
    pub present_ms: f64,
}

pub fn status() -> Status {
    let reply = (|| {
        let mut stream = UnixStream::connect(paths::control_socket()).ok()?;
        stream
            .set_read_timeout(Some(Duration::from_millis(800)))
            .ok()?;
        stream
            .set_write_timeout(Some(Duration::from_millis(800)))
            .ok()?;
        writeln!(stream, "status").ok()?;
        let mut line = String::new();
        BufReader::new(&stream).read_line(&mut line).ok()?;
        serde_json::from_str::<serde_json::Value>(&line).ok()
    })();
    let Some(v) = reply else {
        return Status::default();
    };
    Status {
        running: true,
        connected: v["connected"].as_bool().unwrap_or(false),
        width: v["width"].as_u64().unwrap_or(0),
        height: v["height"].as_u64().unwrap_or(0),
        fps: v["fps"].as_f64().unwrap_or(0.0),
        present_ms: v["presentMs"].as_f64().unwrap_or(0.0),
    }
}

/// The renderer binary: next to this executable, else on PATH.
pub fn renderer_binary() -> PathBuf {
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let candidate = dir.join("omatouchbar");
        if candidate.is_file() {
            return candidate;
        }
    }
    PathBuf::from("omatouchbar")
}

pub fn is_selected() -> bool {
    let link = paths::t1bridge_renderer_link();
    std::fs::canonicalize(link)
        .ok()
        .and_then(|target| {
            target
                .file_name()
                .map(|n| n.to_string_lossy().starts_with("omatouchbar"))
        })
        .unwrap_or(false)
}

/// Runs `omatouchbar enable|disable`; returns its message.
pub fn set_selected(on: bool) -> Result<String, String> {
    let output = Command::new(renderer_binary())
        .arg(if on { "enable" } else { "disable" })
        .output()
        .map_err(|e| format!("Could not run omatouchbar: {e}"))?;
    let text = String::from_utf8_lossy(&output.stdout)
        .trim()
        .lines()
        .last()
        .unwrap_or("")
        .to_string();
    if output.status.success() {
        Ok(if on {
            "omatouchbar is now driving the Touch Bar".into()
        } else {
            "Restored the previous Touch Bar renderer".into()
        })
    } else {
        Err(format!(
            "{text} {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

pub fn restart() -> bool {
    Command::new("systemctl")
        .args(["--user", "restart", "t1-touchbar.service"])
        .status()
        .is_ok_and(|s| s.success())
}
