//! Hides and restores the on-screen Omarchy bar.
//!
//! Omarchy hides the bar while `~/.local/state/omarchy/toggles/bar-off`
//! exists (`omarchy-toggle-bar on|off`). We record when *we* hid it, so we
//! never show a bar the user hid themselves.

use std::{path::PathBuf, process::Command};

use omatouchbar_core::paths;

fn flag() -> PathBuf {
    paths::state_home().join("omarchy/toggles/bar-off")
}

fn marker() -> PathBuf {
    paths::runtime_dir().join("omatouchbar/hid-bar")
}

pub fn is_hidden() -> bool {
    flag().exists()
}

fn toggle(state: &str) -> bool {
    match Command::new("omarchy-toggle-bar").arg(state).status() {
        Ok(status) => status.success(),
        Err(e) => {
            log::warn!("omarchy-toggle-bar {state}: {e}");
            false
        }
    }
}

/// Hides the on-screen bar unless it is already hidden.
pub fn hide() {
    if is_hidden() {
        return;
    }
    if toggle("on") {
        let path = marker();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(path, b"1\n");
        log::info!("hid the on-screen Omarchy bar");
    }
}

/// Shows the bar again, but only if omatouchbar hid it.
pub fn restore_if_ours() {
    let path = marker();
    if !path.exists() {
        return;
    }
    let _ = std::fs::remove_file(&path);
    if is_hidden() && toggle("off") {
        log::info!("restored the on-screen Omarchy bar");
    }
}
