//! Styles the app the way Omarchy's own apps look, from the live theme.
//!
//! Colors come from the active Omarchy palette, corners from Hyprland's
//! `decoration:rounding` (sharp on the default), type from the system
//! monospace font at the shell's `[font] base-size`, and the fill and border
//! strengths from the shell's state tokens (normal 0.04/0.4, hover
//! 0.08/0.25, selected 0.18). Everything re-applies when the theme changes.

use std::{cell::Cell, cell::RefCell, process::Command, rc::Rc};

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use omatouchbar_core::{Config, Palette, Theme, color::Rgba, config::BackgroundMode, paths};

/// The resolved look, for widgets that draw themselves.
#[derive(Clone, Copy, Debug)]
pub struct Tokens {
    pub bg: Rgba,
    pub fg: Rgba,
    pub accent: Rgba,
    pub on_accent: Rgba,
    pub secondary: Rgba,
    pub red: Rgba,
    pub radius: f64,
    /// Body text size in px.
    pub text: f64,
}

thread_local! {
    static TOKENS: Cell<Option<Tokens>> = const { Cell::new(None) };
}

pub fn tokens() -> Tokens {
    TOKENS
        .with(|t| t.get())
        .unwrap_or_else(|| resolve(&Palette::load().unwrap_or_else(Palette::fallback)))
}

/// Hyprland's window rounding, which Omarchy apps use for every corner.
fn rounding() -> f64 {
    Command::new("hyprctl")
        .args(["getoption", "decoration:rounding"])
        .output()
        .ok()
        .and_then(|out| {
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .find_map(|line| {
                    line.trim()
                        .strip_prefix("int:")
                        .map(str::trim)
                        .map(String::from)
                })
        })
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.0)
        .clamp(0.0, 24.0)
}

/// The shell's `[font] base-size` (user file over the theme's), 12 by default.
fn base_size() -> f64 {
    let read = |path: std::path::PathBuf| -> Option<f64> {
        let text = std::fs::read_to_string(path).ok()?;
        let value: toml::Value = toml::from_str(&text).ok()?;
        value.get("font")?.get("base-size")?.as_float().or_else(|| {
            value
                .get("font")?
                .get("base-size")?
                .as_integer()
                .map(|v| v as f64)
        })
    };
    read(paths::config_home().join("omarchy/shell.toml"))
        .or_else(|| read(paths::state_home().join("omarchy/current/theme/shell.toml")))
        .unwrap_or(12.0)
        .clamp(8.0, 24.0)
}

fn resolve(palette: &Palette) -> Tokens {
    let mut config = Config::default();
    config.appearance.background = BackgroundMode::Theme;
    let theme = Theme::from_palette(palette, &config);
    let bg = palette
        .colors
        .get("background")
        .copied()
        .unwrap_or(Rgba::rgb(0.086, 0.086, 0.102));
    let fg = theme.fg;
    // Text on the accent: the theme's own dark or light ink, whichever
    // stands out (the strip's rule assumes a black bar, not a light window).
    let (dark, light) = if bg.luminance() < fg.luminance() {
        (bg, fg)
    } else {
        (fg, bg)
    };
    let on_accent = if theme.accent.luminance() > 0.45 {
        dark.darken(0.1)
    } else {
        light
    };
    Tokens {
        bg,
        fg,
        accent: theme.accent,
        on_accent,
        secondary: bg.mix(fg, 0.68),
        red: theme.red,
        radius: rounding(),
        // Folio's 13 px design size, scaled like the shell's text.
        text: (13.0 * base_size() / 12.0).round(),
    }
}

fn hex(c: Rgba) -> String {
    c.to_hex()
}

pub fn css(t: &Tokens) -> String {
    let r = t.radius;
    // Small parts (buttons, rows, fields) cap the rounding like Folio does.
    let rs = r.min(4.0);
    // A rounded desktop gets pill switches; a sharp one square switches.
    let pill = if r > 0.0 { 999.0 } else { 0.0 };
    let text = t.text;
    let small = (text * 0.86).round();
    let tiny = (text * 0.8).round();
    let title = (text * 1.08).round();
    let heading = (text * 1.25).round();
    let display = (text * 1.85).round();
    let hero = (text * 2.6).round();
    format!(
        r#"
@define-color accent_bg_color {accent};
@define-color accent_fg_color {on_accent};
@define-color accent_color {accent};
@define-color destructive_bg_color {red};
@define-color window_bg_color {bg};
@define-color window_fg_color {fg};
@define-color view_bg_color {bg};
@define-color view_fg_color {fg};
@define-color headerbar_bg_color {bg};
@define-color headerbar_fg_color {fg};
@define-color card_bg_color {bg};
@define-color card_fg_color {fg};
@define-color popover_bg_color {bg};
@define-color popover_fg_color {fg};
@define-color dialog_bg_color {bg};
@define-color dialog_fg_color {fg};
:root {{
  --accent-bg-color: {accent};
  --accent-fg-color: {on_accent};
  --accent-color: {accent};
  --window-bg-color: {bg};
  --window-fg-color: {fg};
  --view-bg-color: {bg};
  --view-fg-color: {fg};
  --popover-bg-color: {bg};
  --popover-fg-color: {fg};
  --dialog-bg-color: {bg};
  --dialog-fg-color: {fg};
  --card-bg-color: {bg};
  --card-fg-color: {fg};
}}

window.omt {{ background: transparent; color: {fg}; }}
window.omt.windowed {{ background: {bg}; }}
.omt, .omt * {{ font-family: monospace; }}
.omt {{ font-size: {text}px; }}

/* The launcher's scrim and card. */
.omt-scrim {{ background: alpha({bg}, 0.5); }}
.omt-card {{
  background: {bg};
  color: {fg};
  border: 2px solid {fg};
  border-radius: {r}px;
}}
.windowed .omt-card {{ border: none; border-radius: 0; }}

.omt-hair {{ background: alpha({fg}, 0.12); min-height: 1px; min-width: 1px; }}
.omt-dim {{ color: {secondary}; }}
.omt-faint {{ color: alpha({fg}, 0.35); }}
.omt-accent {{ color: {accent}; }}
.omt-small {{ font-size: {small}px; }}
.omt-tiny {{ font-size: {tiny}px; }}
.omt-title {{ font-size: {title}px; font-weight: 600; }}
.omt-heading {{ font-size: {heading}px; font-weight: 600; }}
.omt-display {{ font-size: {display}px; font-weight: 700; }}
.omt-hero {{ font-size: {hero}px; font-weight: 800; letter-spacing: -1px; }}
.omt-logo {{ font-family: omarchy; color: {accent}; }}
.omt-logo-xl {{ font-size: {logo}px; }}
.omt-glyph {{ min-width: 22px; font-size: {heading}px; }}
.omt-section {{
  color: {secondary};
  font-size: {tiny}px;
  font-weight: 600;
  letter-spacing: 1.3px;
}}
.omt-kbd {{
  color: {secondary};
  font-size: {tiny}px;
  padding: 0 5px;
  min-width: 10px;
  border: 1px solid alpha({fg}, 0.28);
  border-bottom-width: 2px;
  border-radius: {rs}px;
}}

/* Header, footer. */
.omt-header {{ min-height: 52px; padding: 0 10px 0 20px; }}
.omt-footer {{ min-height: 32px; padding: 0 18px; color: {secondary}; font-size: {tiny}px; }}
.omt-status {{
  padding: 3px 10px;
  border-radius: {rs}px;
  color: {secondary};
  font-size: {small}px;
}}
.omt-status:hover {{ background: alpha({fg}, 0.06); color: {fg}; }}
.omt-dot {{ min-width: 8px; min-height: 8px; border-radius: 99px; background: alpha({fg}, 0.3); }}
.omt-dot.live {{ background: {accent}; box-shadow: 0 0 8px alpha({accent}, 0.7); }}
.omt-dot.big {{ min-width: 12px; min-height: 12px; }}

/* Folio's sidebar. */
.omt-sidebar {{ background: transparent; padding: 10px 12px; }}
.omt-sidebar > row {{
  min-height: 34px;
  padding: 0 12px;
  border-radius: {rs}px;
  background: transparent;
  color: {fg};
  transition: background-color 120ms ease-out, color 120ms ease-out;
}}
.omt-sidebar > row:hover {{ background: alpha({fg}, 0.06); }}
.omt-sidebar > row:selected {{ background: alpha({accent}, 0.16); color: {accent}; }}
.omt-sidebar > row:focus-visible {{ outline: 1px solid {accent}; outline-offset: -1px; }}
.omt-sidebar > row.omt-side-label,
.omt-sidebar > row.omt-side-label:hover {{ background: transparent; min-height: 0; padding: 14px 14px 6px 14px; }}

/* Settings pages. */
.omt-page {{ padding: 6px 30px 30px 30px; }}
.omt-page-title {{ font-size: {display}px; font-weight: 700; }}
.omt-row {{
  padding: 9px 14px;
  min-height: 34px;
  border-radius: {rs}px;
  transition: background-color 120ms ease-out;
}}
.omt-row:hover {{ background: alpha({fg}, 0.05); }}
.omt-row.compact {{ padding: 2px 6px 2px 14px; min-height: 0; }}
.omt-row.disabled {{ opacity: 0.45; }}
.omt-row-caption {{ color: {secondary}; font-size: {small}px; }}

/* Controls, after the shell's Button, ButtonGroup and ToggleSwitch. */
.omt button.omt-btn {{
  background: alpha({fg}, 0.04);
  color: {fg};
  border: 1px solid alpha({fg}, 0.4);
  border-radius: {rs}px;
  padding: 5px 12px;
  min-height: 20px;
  box-shadow: none;
  outline: none;
  transition: background-color 120ms ease-out, border-color 120ms ease-out, color 120ms ease-out;
}}
.omt button.omt-btn:hover {{ background: alpha({fg}, 0.08); border-color: alpha({fg}, 0.25); }}
.omt button.omt-btn:active {{ background: alpha({fg}, 0.22); }}
.omt button.omt-btn:focus-visible {{ border-color: {accent}; }}
.omt button.omt-btn.flat {{ border-color: transparent; background: transparent; }}
.omt button.omt-btn.flat:hover {{ background: alpha({fg}, 0.08); }}
.omt button.omt-btn.icon {{ padding: 4px; min-width: 22px; }}
.omt button.omt-btn.primary {{ background: {accent}; color: {on_accent}; border-color: {accent}; font-weight: 600; }}
.omt button.omt-btn.primary:hover {{ background: mix({accent}, {fg}, 0.14); border-color: mix({accent}, {fg}, 0.14); }}
.omt button.omt-btn.primary label {{ color: {on_accent}; }}
.omt button.omt-btn.danger {{ color: {red}; border-color: alpha({red}, 0.6); }}
.omt button.omt-btn.danger:hover {{ background: alpha({red}, 0.12); }}
.omt button.omt-btn.armed {{ background: {red}; color: {bg}; border-color: {red}; }}
.omt button.omt-btn:disabled {{ opacity: 0.4; }}

.omt menubutton.omt-menu > button {{
  background: transparent;
  color: {secondary};
  border: 1px solid transparent;
  border-radius: {rs}px;
  padding: 3px 10px;
  min-height: 18px;
  box-shadow: none;
  font-size: {small}px;
}}
.omt menubutton.omt-menu > button:hover {{ background: alpha({fg}, 0.08); color: {fg}; }}
.omt menubutton.omt-menu > button:checked {{ color: {accent}; }}

.omt-seg > button.omt-btn {{ border-radius: 0; margin-left: -1px; }}
.omt-seg > button.omt-btn:first-child {{ border-radius: {rs}px 0 0 {rs}px; margin-left: 0; }}
.omt-seg > button.omt-btn:last-child {{ border-radius: 0 {rs}px {rs}px 0; }}
.omt-seg > button.omt-btn:checked {{
  background: alpha({accent}, 0.18);
  border-color: {accent};
  color: {accent};
}}

.omt switch {{
  min-width: 40px;
  min-height: 20px;
  padding: 0;
  border: 1px solid alpha({fg}, 0.4);
  border-radius: {pill}px;
  background: alpha({fg}, 0.04);
  box-shadow: none;
  outline: none;
  transition: background-color 120ms ease-out, border-color 120ms ease-out;
}}
.omt switch:hover {{ background: alpha({fg}, 0.08); }}
.omt switch:checked {{ background: alpha({accent}, 0.22); border-color: {accent}; }}
.omt switch:focus-visible {{ border-color: {accent}; }}
.omt switch > slider {{
  min-width: 14px;
  min-height: 14px;
  margin: 2px;
  border: none;
  border-radius: {pill}px;
  background: mix({fg}, {bg}, 0.3);
  box-shadow: none;
  transition: all 140ms cubic-bezier(0.23, 1, 0.32, 1);
}}
.omt switch:checked > slider {{ background: {accent}; }}
.omt switch > image {{ color: transparent; }}

.omt scale {{ padding: 8px 0; min-height: 10px; }}
.omt scale > trough {{
  min-height: 4px;
  border: none;
  border-radius: 2px;
  background: alpha({fg}, 0.18);
  outline: none;
}}
.omt scale > trough > highlight {{ background: {accent}; border-radius: 2px; border: none; min-height: 4px; }}
.omt scale > trough > slider {{
  min-width: 10px;
  min-height: 10px;
  margin: -6px;
  border-radius: 99px;
  border: 2px solid {bg};
  background: {fg};
  box-shadow: none;
  transition: background-color 120ms ease-out;
}}
.omt scale:hover > trough > slider {{ background: {accent}; }}
.omt-value {{ min-width: 52px; color: {secondary}; font-size: {small}px; }}

.omt entry, .omt spinbutton {{
  background: alpha({fg}, 0.04);
  color: {fg};
  border: 1px solid alpha({fg}, 0.4);
  border-radius: {rs}px;
  min-height: 28px;
  box-shadow: none;
  outline: none;
}}
.omt entry {{ padding: 0 10px; }}
.omt entry:focus-within, .omt spinbutton:focus-within {{ border-color: {accent}; }}
.omt spinbutton > text {{ padding: 0 8px; min-width: 44px; background: none; border: none; box-shadow: none; outline: none; }}
.omt spinbutton > button {{
  border: none;
  border-left: 1px solid alpha({fg}, 0.18);
  border-radius: 0;
  background: transparent;
  min-width: 26px;
  padding: 0;
  box-shadow: none;
  color: {fg};
}}
.omt spinbutton > button:hover {{ background: alpha({fg}, 0.08); }}

.omt scrollbar {{ background: transparent; border: none; }}
.omt scrollbar slider {{ min-width: 4px; min-height: 24px; margin: 2px; border-radius: 0; background: alpha({fg}, 0.22); border: none; }}
.omt scrollbar.horizontal slider {{ min-width: 24px; min-height: 4px; }}
.omt scrollbar slider:hover {{ background: alpha({fg}, 0.4); }}

.omt popover > contents {{
  background: {bg};
  color: {fg};
  border: 1px solid alpha({fg}, 0.4);
  border-radius: {r}px;
  box-shadow: 0 8px 24px alpha(black, 0.35);
  padding: 4px;
}}
.omt popover > arrow {{ background: transparent; border: none; }}
.omt popover listbox {{ background: transparent; }}
.omt popover listbox > row {{ padding: 6px 12px; border-radius: {rs}px; min-height: 22px; }}
.omt popover listbox > row:hover {{ background: alpha({fg}, 0.08); color: {accent}; }}

toast {{
  background: {bg};
  color: {fg};
  border: 1px solid alpha({fg}, 0.4);
  border-radius: {r}px;
  box-shadow: 0 8px 24px alpha(black, 0.35);
  font-family: monospace;
  margin-bottom: 44px;
}}

/* Mode cards. */
.omt button.omt-mode {{
  padding: 14px 16px;
  border: 1px solid alpha({fg}, 0.16);
  border-radius: {rs}px;
  background: alpha({fg}, 0.03);
  color: {fg};
  box-shadow: none;
  outline: none;
  font-weight: normal;
  transition: background-color 140ms ease-out, border-color 140ms ease-out;
}}
.omt button.omt-mode:hover {{ background: alpha({fg}, 0.06); border-color: alpha({fg}, 0.3); }}
.omt button.omt-mode:checked {{ border-color: {accent}; background: alpha({accent}, 0.09); }}
.omt button.omt-mode:checked .omt-mode-name,
.omt button.omt-mode:checked .omt-glyph {{ color: {accent}; }}
.omt button.omt-mode:focus-visible {{ border-color: {accent}; }}
.omt-mode-name {{ font-size: {title}px; font-weight: 600; }}

.touchbar-bezel {{
  background-color: #000000;
  border-radius: {bezel}px;
  padding: 8px 12px;
  box-shadow: inset 0 1px alpha(white, 0.07), 0 14px 34px alpha(black, 0.42);
}}

/* The tour. */
.omt-steps > box {{ min-width: 22px; min-height: 3px; background: alpha({fg}, 0.16); transition: all 260ms cubic-bezier(0.23, 1, 0.32, 1); }}
.omt-steps > box.done {{ background: alpha({fg}, 0.5); }}
.omt-steps > box.current {{ min-width: 44px; background: {accent}; }}
.omt-tile-progress {{ min-height: 2px; }}
.omt-fact-glyph {{ color: {accent}; min-width: 26px; font-size: {heading}px; }}
.omt-callout-hint {{ color: {secondary}; font-size: {tiny}px; }}
.omt-feature {{ padding: 16px 18px; border: 1px solid alpha({fg}, 0.12); border-radius: {rs}px; background: alpha({fg}, 0.025); }}
.omt-cursor {{ color: {accent}; }}
.omt-callout-box {{ padding: 0 12px; }}
"#,
        accent = hex(t.accent),
        on_accent = hex(t.on_accent),
        red = hex(t.red),
        bg = hex(t.bg),
        fg = hex(t.fg),
        secondary = hex(t.secondary),
        bezel = r.clamp(10.0, 18.0),
        logo = (text * 5.4).round(),
    )
}

/// Applies the theme now and whenever Omarchy switches themes. `changed`
/// receives each new palette (for the preview and hand-drawn widgets).
pub fn install(changed: impl Fn(Palette) + 'static) -> Styler {
    let provider = gtk::CssProvider::new();
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
    let apply = {
        let provider = provider.clone();
        move |palette: &Palette| {
            let tokens = resolve(palette);
            TOKENS.with(|t| t.set(Some(tokens)));
            provider.load_from_string(&css(&tokens));
            let scheme = if palette.dark {
                adw::ColorScheme::ForceDark
            } else {
                adw::ColorScheme::ForceLight
            };
            adw::StyleManager::default().set_color_scheme(scheme);
        }
    };
    apply(&Palette::load().unwrap_or_else(Palette::fallback));

    // Omarchy rewrites ~/.local/state/omarchy/current/ on `theme set`.
    let dir = Palette::path()
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf());
    let monitor = dir.and_then(|dir| {
        gio::File::for_path(dir)
            .monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE)
            .ok()
    });
    if let Some(monitor) = &monitor {
        let pending = Rc::new(Cell::new(false));
        let changed = Rc::new(changed);
        monitor.connect_changed(move |_, _, _, _| {
            if pending.replace(true) {
                return;
            }
            let (pending, apply, changed) = (pending.clone(), apply.clone(), changed.clone());
            glib::timeout_add_local_once(std::time::Duration::from_millis(300), move || {
                pending.set(false);
                if let Some(palette) = Palette::load() {
                    apply(&palette);
                    changed(palette);
                }
            });
        });
    }
    Styler {
        _provider: provider,
        _monitor: RefCell::new(monitor),
    }
}

pub struct Styler {
    _provider: gtk::CssProvider,
    _monitor: RefCell<Option<gio::FileMonitor>>,
}
