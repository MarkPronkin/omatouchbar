//! omatouchbar — the Omarchy bar on the T1 MacBook Pro Touch Bar.
//!
//! Runs as a T1Bridge renderer (`~/.config/t1bridge/renderer`), speaking
//! T1Bridge's hardware IPC to draw frames and receive touches.

mod bus;
mod control;
mod exec;
mod hw;
mod kbd;
mod logger;
mod providers;
mod run;
mod selection;
mod sysbar;
mod themesync;
mod watch;

const HELP: &str = "\
omatouchbar — the Omarchy bar on the T1 Touch Bar

Usage:
  omatouchbar [run]          Run the Touch Bar renderer (T1Bridge starts this)
  omatouchbar enable         Select omatouchbar as the T1Bridge renderer and restart it
  omatouchbar disable        Restore the previous renderer and restart it
  omatouchbar status         Show renderer status
  omatouchbar ctl <command>  Send a command to the running renderer:
                               menu [id]   open the Omarchy menu (or a submenu)
                               panel <sound|mic|sources|display|brightness|battery|network|bluetooth|clock|tray|weather|agents|indicators>
                               speedtest   launch the Touch Bar network speed test
                               touchid [unlock|enroll]  play a demo of the Touch ID animation
                               home        return to the bar
                               mode <keyboard|bar|controls|function>
                               reload      reload config, theme, layout and menu
                               snapshot <file.png>  save the current frame
                               debug <on|off>      verbose logging to the journal
  omatouchbar restore-bar    Show the on-screen Omarchy bar if omatouchbar hid it

Settings live in ~/.config/omatouchbar/config.toml (edit with omatouchbar-settings).
";

fn main() {
    logger::init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        None | Some("run") => run::run(),
        Some("ctl") => control::client(&args[1..]),
        Some("status") => control::client(&["status".to_string()]),
        Some("enable") => selection::enable(),
        Some("disable") => selection::disable(),
        Some("restore-bar") => {
            sysbar::restore_if_ours();
            0
        }
        Some("--version" | "-V") => {
            println!("omatouchbar {}", env!("CARGO_PKG_VERSION"));
            0
        }
        Some("help" | "-h" | "--help") => {
            print!("{HELP}");
            0
        }
        Some(other) => {
            eprintln!("omatouchbar: unknown command '{other}'\n\n{HELP}");
            2
        }
    };
    std::process::exit(code);
}
