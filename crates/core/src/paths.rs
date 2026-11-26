//! XDG and Omarchy locations.

use std::{env, path::PathBuf};

fn home() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

fn xdg(var: &str, fallback: &str) -> PathBuf {
    env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home().join(fallback))
}

pub fn config_home() -> PathBuf {
    xdg("XDG_CONFIG_HOME", ".config")
}

pub fn state_home() -> PathBuf {
    xdg("XDG_STATE_HOME", ".local/state")
}

pub fn runtime_dir() -> PathBuf {
    env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("/run/user/{}", rustix_uid())))
}

fn rustix_uid() -> u32 {
    // Avoid a libc dependency in core: read our own uid from /proc.
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("Uid:"))
                .and_then(|l| l.split_whitespace().nth(1).and_then(|v| v.parse().ok()))
        })
        .unwrap_or(1000)
}

/// `$OMARCHY_PATH`, normally `/usr/share/omarchy`.
pub fn omarchy_root() -> PathBuf {
    env::var_os("OMARCHY_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/usr/share/omarchy"))
}

/// Omarchy's weather location (`omarchy-weather-location` owns the format).
pub fn weather_location_file() -> PathBuf {
    home().join(".local/state/omarchy/settings/weather.json")
}

/// The renderer's control socket.
pub fn control_socket() -> PathBuf {
    runtime_dir().join("omatouchbar.sock")
}

/// T1Bridge's renderer selection link.
pub fn t1bridge_renderer_link() -> PathBuf {
    config_home().join("t1bridge/renderer")
}
