//! Selecting omatouchbar as the T1Bridge renderer.
//!
//! T1Bridge runs the executable (or symlink target) at
//! `~/.config/t1bridge/renderer`, falling back to its built-in renderer if
//! it is missing or exits. We keep any previous selection in
//! `renderer.previous` and restore it on `disable`.

use std::{
    fs,
    os::unix::fs::symlink,
    path::{Path, PathBuf},
    process::Command,
};

use omatouchbar_core::paths;

fn previous_path(link: &Path) -> PathBuf {
    link.with_file_name("renderer.previous")
}

fn restart_renderer() {
    match Command::new("systemctl")
        .args(["--user", "restart", "t1-touchbar.service"])
        .status()
    {
        Ok(s) if s.success() => println!("Restarted t1-touchbar.service."),
        Ok(_) | Err(_) => eprintln!("Could not restart t1-touchbar.service; restart it manually."),
    }
}

fn own_executable() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    // Prefer the stable packaged/installed path when we run from it.
    Some(fs::canonicalize(&exe).unwrap_or(exe))
}

pub fn is_selected() -> bool {
    let link = paths::t1bridge_renderer_link();
    match (fs::canonicalize(&link), own_executable()) {
        (Ok(target), Some(me)) => target == me || target.file_name() == me.file_name(),
        _ => false,
    }
}

pub fn enable() -> i32 {
    let link = paths::t1bridge_renderer_link();
    let Some(exe) = own_executable() else {
        eprintln!("Cannot determine omatouchbar's own path.");
        return 1;
    };
    if is_selected() {
        println!("omatouchbar is already the Touch Bar renderer.");
        restart_renderer();
        return 0;
    }
    if let Some(dir) = link.parent()
        && let Err(e) = fs::create_dir_all(dir)
    {
        eprintln!("Cannot create {}: {e}", dir.display());
        return 1;
    }
    if fs::symlink_metadata(&link).is_ok() {
        let previous = previous_path(&link);
        if let Err(e) = fs::rename(&link, &previous) {
            eprintln!("Cannot keep the existing renderer selection: {e}");
            return 1;
        }
        println!(
            "Saved the previous renderer selection to {}.",
            previous.display()
        );
    }
    if let Err(e) = symlink(&exe, &link) {
        eprintln!("Cannot create {}: {e}", link.display());
        return 1;
    }
    println!("Selected {} as the Touch Bar renderer.", exe.display());
    restart_renderer();
    0
}

pub fn disable() -> i32 {
    let link = paths::t1bridge_renderer_link();
    if !is_selected() {
        println!("omatouchbar is not the selected Touch Bar renderer.");
        return 0;
    }
    if let Err(e) = fs::remove_file(&link) {
        eprintln!("Cannot remove {}: {e}", link.display());
        return 1;
    }
    let previous = previous_path(&link);
    if fs::symlink_metadata(&previous).is_ok() {
        if fs::rename(&previous, &link).is_ok() {
            println!("Restored the previous renderer selection.");
        }
    } else {
        println!("T1Bridge will use its built-in renderer.");
    }
    restart_renderer();
    0
}
