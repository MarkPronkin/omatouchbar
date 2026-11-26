//! Shared core of omatouchbar: theme, settings, fonts, the animated scene
//! and its software renderer. Used by the Touch Bar renderer daemon and by
//! the settings app's live preview.

pub mod agent_icons;
pub mod agents;
pub mod anim;
mod build;
pub mod canvas;
pub mod color;
pub mod config;
pub mod display;
pub mod menu;
pub mod node;
pub mod omarchy;
pub mod paths;
mod render;
pub mod scene;
pub mod speedtest;
pub mod state;
pub mod text;
pub mod theme;
mod touchfx;
pub mod weather;
mod wipe;
pub mod world_clock;

pub use canvas::Canvas;
pub use config::Config;
pub use scene::{Action, Command, KeyInput, Scene};
pub use speedtest::{SpeedPhase, SpeedTestEvent};
pub use state::DesktopState;
pub use text::Fonts;
pub use theme::{Palette, Theme};

/// Panel size of the T1 MacBook Pro Touch Bar.
pub const T1_WIDTH: u32 = 2170;
pub const T1_HEIGHT: u32 = 60;
