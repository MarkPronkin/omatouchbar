//! Background threads that keep the desktop state current.

pub mod agents;
pub mod audio;
pub mod dbus;
pub mod display;
pub mod hyprland;
pub mod network;
pub mod speedtest;
pub mod system;
pub mod tray;
pub mod weather;
pub mod wifi;

use crate::{bus::Bus, exec::Refresh};
use std::sync::{Arc, Mutex};

/// Starts every provider and returns the handles used to nudge them.
pub fn start(
    bus: &Bus,
    sink: Arc<Mutex<Option<String>>>,
) -> (Refresh, std::sync::mpsc::Sender<()>) {
    let hyprland = hyprland::spawn(bus.clone());
    let audio = audio::spawn(bus.clone(), sink);
    let network = system::spawn_network(bus.clone());
    let brightness = system::spawn_brightness(bus.clone());
    let bluetooth = dbus::spawn_bluetooth(bus.clone());
    dbus::spawn_power(bus.clone());
    dbus::spawn_media(bus.clone());
    system::spawn_updates(bus.clone());
    let indicators = system::spawn_indicators(bus.clone());
    (
        Refresh {
            network: Some(network),
            bluetooth,
            tray: tray::spawn(bus.clone()),
            agents: Some(agents::spawn(bus.clone())),
            brightness: Some(brightness),
            audio: Some(audio),
            indicators: Some(indicators),
        },
        hyprland,
    )
}
