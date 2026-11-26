//! Reads the per-provider records produced by omarchy-agent-usage-update.

use omatouchbar_core::{agent_icons::AgentIconCache, agents, paths};
use std::{process::Command, sync::mpsc, thread};

use crate::bus::Bus;

pub enum Request {
    Read,
    Refresh(Vec<String>),
}

pub fn spawn(bus: Bus) -> mpsc::Sender<Request> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let path = paths::state_home().join("omarchy/agents");
        let mut icons = AgentIconCache::default();
        loop {
            let agents = agents::load(&path, &mut icons);
            bus.patch(move |s| s.agents = agents);
            let Ok(first) = rx.recv() else { break };
            // Burst writes come from the individual collectors finishing.
            if let Some(disabled) =
                std::iter::once(first)
                    .chain(rx.try_iter())
                    .find_map(|r| match r {
                        Request::Refresh(disabled) => Some(disabled),
                        Request::Read => None,
                    })
            {
                update(&disabled);
            }
        }
    });
    tx
}

fn update(disabled: &[String]) {
    let root = paths::omarchy_root();
    // The shell owns collector settings and shares its refresh with the on-screen widget.
    if Command::new("omarchy-shell")
        .args(["omarchy.agents", "refresh"])
        .env("OMARCHY_PATH", &root)
        .output()
        .is_ok_and(|out| out.status.success())
    {
        return;
    }
    // A custom Touch Bar layout can include Agents without loading the shell plugin.
    let mut command = Command::new("omarchy-agent-usage-update");
    for id in disabled {
        command.args(["--except", id]);
    }
    if !command
        .env("OMARCHY_PATH", root)
        .status()
        .is_ok_and(|s| s.success())
    {
        log::warn!("Omarchy could not refresh agent usage");
    }
}
