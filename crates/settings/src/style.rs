//! Styles the settings app from the active Omarchy theme, live.

use std::{cell::Cell, rc::Rc};

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use omatouchbar_core::{Config, Palette, Theme, color::Rgba, config::BackgroundMode};

fn hex(c: Rgba) -> String {
    c.to_hex()
}

pub fn css(palette: &Palette) -> String {
    // The settings window uses the desktop colors. Its preview uses the strip colors.
    let mut config = Config::default();
    config.appearance.background = BackgroundMode::Theme;
    let theme = Theme::from_palette(palette, &config);
    let get = |names: &[&str], fallback: Rgba| {
        names
            .iter()
            .find_map(|n| palette.colors.get(*n).copied())
            .unwrap_or(fallback)
    };
    let bg = get(&["background"], Rgba::rgb(0.1, 0.1, 0.12));
    let darker = get(&["dark_background", "darker_background"], bg.darken(0.15));
    let lighter = get(&["lighter_background"], bg.mix(theme.fg, 0.08));
    let card = lighter.mix(bg, 0.35);
    let fg = theme.fg;
    let accent = theme.accent;
    let on_accent = theme.on_accent;
    let destructive = theme.red;
    format!(
        r#"
@define-color accent_bg_color {accent};
@define-color accent_fg_color {on_accent};
@define-color accent_color {accent};
@define-color destructive_bg_color {destructive};
@define-color window_bg_color {bg};
@define-color window_fg_color {fg};
@define-color view_bg_color {darker};
@define-color view_fg_color {fg};
@define-color headerbar_bg_color {bg};
@define-color headerbar_fg_color {fg};
@define-color card_bg_color {card};
@define-color card_fg_color {fg};
@define-color popover_bg_color {lighter};
@define-color popover_fg_color {fg};
@define-color dialog_bg_color {bg};
@define-color dialog_fg_color {fg};
:root {{
  --accent-bg-color: {accent};
  --accent-fg-color: {on_accent};
  --accent-color: {accent};
  --window-bg-color: {bg};
  --window-fg-color: {fg};
  --view-bg-color: {darker};
  --view-fg-color: {fg};
  --headerbar-bg-color: {bg};
  --headerbar-fg-color: {fg};
  --card-bg-color: {card};
  --card-fg-color: {fg};
  --popover-bg-color: {lighter};
  --popover-fg-color: {fg};
  --dialog-bg-color: {bg};
  --dialog-fg-color: {fg};
}}
.touchbar-bezel {{
  background-color: #000000;
  border-radius: 14px;
  padding: 8px 12px;
  box-shadow: inset 0 1px alpha(white, 0.07), 0 10px 28px alpha(black, 0.38);
}}
.preview-caption {{ opacity: 0.62; font-size: 0.92em; }}
.status-dot {{ min-width: 9px; min-height: 9px; border-radius: 99px; background-color: alpha({fg}, 0.35); }}
.status-dot.live {{ background-color: {accent}; box-shadow: 0 0 8px alpha({accent}, 0.8); }}
.fn-key {{ font-weight: bold; min-width: 52px; }}
"#,
        accent = hex(accent),
        on_accent = hex(on_accent),
        destructive = hex(destructive),
        bg = hex(bg),
        fg = hex(fg),
        darker = hex(darker),
        lighter = hex(lighter),
        card = hex(card),
    )
}

/// Applies the theme now and whenever Omarchy switches themes. `changed`
/// receives each new palette (for the preview).
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
            provider.load_from_string(&css(palette));
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
        _monitor: monitor,
    }
}

pub struct Styler {
    _provider: gtk::CssProvider,
    _monitor: Option<gio::FileMonitor>,
}
