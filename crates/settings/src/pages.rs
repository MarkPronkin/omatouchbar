//! Preference pages bound to the config.

use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};
use omatouchbar_core::{
    Config,
    config::{BackgroundMode, ButtonStyle, FnKey, LayoutSource, Mode, WorkspaceStyle},
    omarchy::Widget,
};

use crate::{App, daemon};

type Get<T> = fn(&Config) -> T;
type Set<T> = fn(&mut Config, T);

fn switch(
    app: &Rc<App>,
    title: &str,
    subtitle: &str,
    get: Get<bool>,
    set: Set<bool>,
) -> adw::SwitchRow {
    let row = adw::SwitchRow::builder()
        .title(title)
        .subtitle(subtitle)
        .active(get(&app.config()))
        .build();
    let app = app.clone();
    row.connect_active_notify(move |row| app.update(|c| set(c, row.is_active())));
    row
}

fn combo(
    app: &Rc<App>,
    title: &str,
    subtitle: &str,
    items: &[&str],
    get: Get<u32>,
    set: Set<u32>,
) -> adw::ComboRow {
    let model = gtk::StringList::new(items);
    let row = adw::ComboRow::builder()
        .title(title)
        .subtitle(subtitle)
        .model(&model)
        .selected(get(&app.config()))
        .build();
    let app = app.clone();
    row.connect_selected_notify(move |row| app.update(|c| set(c, row.selected())));
    row
}

fn spin(
    app: &Rc<App>,
    title: &str,
    subtitle: &str,
    range: (f64, f64, f64),
    get: Get<f64>,
    set: Set<f64>,
) -> adw::SpinRow {
    let row = adw::SpinRow::with_range(range.0, range.1, range.2);
    row.set_title(title);
    row.set_subtitle(subtitle);
    row.set_value(get(&app.config()));
    let app = app.clone();
    row.connect_value_notify(move |row| app.update(|c| set(c, row.value())));
    row
}

fn scale(
    app: &Rc<App>,
    title: &str,
    subtitle: &str,
    range: (f64, f64, f64),
    digits: i32,
    get: Get<f64>,
    set: Set<f64>,
) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(title)
        .subtitle(subtitle)
        .build();
    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, range.0, range.1, range.2);
    scale.set_value(get(&app.config()));
    scale.set_digits(digits);
    scale.set_draw_value(true);
    scale.set_value_pos(gtk::PositionType::Left);
    scale.set_size_request(280, -1);
    scale.set_valign(gtk::Align::Center);
    let app = app.clone();
    scale.connect_value_changed(move |s| app.update(|c| set(c, s.value())));
    row.add_suffix(&scale);
    row
}

fn page(title: &str, icon: &str) -> adw::PreferencesPage {
    adw::PreferencesPage::builder()
        .title(title)
        .icon_name(icon)
        .build()
}

fn group(title: &str, description: &str) -> adw::PreferencesGroup {
    adw::PreferencesGroup::builder()
        .title(title)
        .description(description)
        .build()
}

pub fn general(app: &Rc<App>) -> adw::PreferencesPage {
    let page = page("General", "preferences-system-symbolic");

    let touchbar = group(
        "Touch Bar",
        "omatouchbar runs as the T1Bridge renderer for the Touch Bar.",
    );
    let enable = adw::SwitchRow::builder()
        .title("Use omatouchbar")
        .subtitle("Draw the Touch Bar with omatouchbar instead of the built-in renderer")
        .active(daemon::is_selected())
        .build();
    {
        let app = app.clone();
        enable.connect_active_notify(move |row| {
            let on = row.is_active();
            if on == daemon::is_selected() {
                return;
            }
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(daemon::set_selected(on));
            });
            let app = app.clone();
            glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
                match rx.try_recv() {
                    Ok(result) => {
                        app.toast(&match result {
                            Ok(message) => message,
                            Err(e) => e,
                        });
                        app.refresh_status();
                        glib::ControlFlow::Break
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                    Err(_) => glib::ControlFlow::Break,
                }
            });
        });
    }
    touchbar.add(&enable);
    let status = adw::ActionRow::builder()
        .title("Renderer")
        .subtitle("Checking…")
        .build();
    let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    dot.add_css_class("status-dot");
    dot.set_valign(gtk::Align::Center);
    status.add_prefix(&dot);
    let restart = gtk::Button::builder()
        .label("Restart")
        .valign(gtk::Align::Center)
        .build();
    {
        let app = app.clone();
        restart.connect_clicked(move |_| {
            std::thread::spawn(daemon::restart);
            app.toast("Restarting the Touch Bar renderer…");
        });
    }
    status.add_suffix(&restart);
    touchbar.add(&status);
    app.set_status_row(status, dot);
    page.add(&touchbar);

    let behavior = group("Behavior", "");
    behavior.add(&combo(
        app,
        "Mode",
        "What the Touch Bar shows",
        &Mode::ALL.map(Mode::label),
        |c| Mode::ALL.iter().position(|m| *m == c.mode).unwrap_or(0) as u32,
        |c, v| c.mode = Mode::ALL[(v as usize).min(Mode::ALL.len() - 1)],
    ));
    behavior.add(&combo(
        app,
        "Holding Fn shows",
        "While the Fn key is held",
        &["Function keys", "Media controls", "Nothing"],
        |c| match c.fn_key {
            FnKey::Function => 0,
            FnKey::Controls => 1,
            FnKey::None => 2,
        },
        |c, v| c.fn_key = [FnKey::Function, FnKey::Controls, FnKey::None][(v as usize).min(2)],
    ));
    behavior.add(&switch(
        app,
        "Esc key",
        "Pinned to the left edge of the Touch Bar",
        |c| c.show_escape,
        |c, v| c.show_escape = v,
    ));
    behavior.add(&switch(
        app,
        "Hide the on-screen bar",
        "Move the Omarchy bar to the Touch Bar to free up screen space",
        |c| c.hide_system_bar,
        |c, v| c.hide_system_bar = v,
    ));
    behavior.add(&switch(
        app,
        "On-screen OSD",
        "Show Omarchy's volume and brightness popups for Touch Bar changes",
        |c| c.osd,
        |c, v| c.osd = v,
    ));
    page.add(&behavior);
    page
}

fn widget_label(name: &str) -> String {
    Widget::NAMES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, l)| l.to_string())
        .unwrap_or_else(|| name.to_string())
}

#[derive(Clone, Copy)]
enum Section {
    Left,
    CenterBegin,
    Center,
    CenterEnd,
    Right,
}

fn section_list(c: &mut Config, s: Section) -> &mut Vec<String> {
    match s {
        Section::Left => &mut c.layout.left,
        Section::CenterBegin => &mut c.layout.center_begin,
        Section::Center => &mut c.layout.center,
        Section::CenterEnd => &mut c.layout.center_end,
        Section::Right => &mut c.layout.right,
    }
}

fn fill_section(app: &Rc<App>, list: &gtk::ListBox, section: Section) {
    list.remove_all();
    let names = section_list(&mut app.config(), section).clone();
    if names.is_empty() {
        let row = adw::ActionRow::builder().title("Empty").build();
        row.add_css_class("dim-label");
        list.append(&row);
    }
    for (index, name) in names.iter().enumerate() {
        let row = adw::ActionRow::builder().title(widget_label(name)).build();
        let buttons = [
            ("go-up-symbolic", "Move left", -1i32),
            ("go-down-symbolic", "Move right", 1),
            ("user-trash-symbolic", "Remove", 0),
        ];
        for (icon, tip, delta) in buttons {
            let button = gtk::Button::builder()
                .icon_name(icon)
                .tooltip_text(tip)
                .valign(gtk::Align::Center)
                .build();
            button.add_css_class("flat");
            let (app, list) = (app.clone(), list.clone());
            button.connect_clicked(move |_| {
                app.update(|c| {
                    let items = section_list(c, section);
                    if delta == 0 {
                        if index < items.len() {
                            items.remove(index);
                        }
                    } else {
                        let target = index as i32 + delta;
                        if target >= 0 && (target as usize) < items.len() {
                            items.swap(index, target as usize);
                        }
                    }
                });
                fill_section(&app, &list, section);
            });
            row.add_suffix(&button);
        }
        list.append(&row);
    }
}

pub fn layout(app: &Rc<App>) -> adw::PreferencesPage {
    let page = page("Layout", "view-grid-symbolic");
    let source = group("Bar layout", "");
    let mirror = switch(
        app,
        "Mirror the Omarchy bar",
        "Use the same widgets as ~/.config/omarchy/shell.json (Esc always stays on the left)",
        |c| c.layout.source == LayoutSource::Omarchy,
        |c, v| {
            c.layout.source = if v {
                LayoutSource::Omarchy
            } else {
                LayoutSource::Custom
            }
        },
    );
    source.add(&mirror);
    page.add(&source);

    let sections = [
        (Section::Left, "Left", "After Esc"),
        (
            Section::CenterBegin,
            "Before the center",
            "Hugging the center group's left edge",
        ),
        (Section::Center, "Center", "Centered on the Touch Bar"),
        (
            Section::CenterEnd,
            "After the center",
            "Hugging the center group's right edge",
        ),
        (Section::Right, "Right", "Ending at the right edge"),
    ];
    let mut custom_groups = Vec::new();
    for (section, title, description) in sections {
        let g = group(title, description);
        let list = gtk::ListBox::new();
        list.add_css_class("boxed-list");
        list.set_selection_mode(gtk::SelectionMode::None);
        fill_section(app, &list, section);
        // "Add widget" menu.
        let popover_list = gtk::ListBox::new();
        popover_list.set_selection_mode(gtk::SelectionMode::None);
        popover_list.add_css_class("navigation-sidebar");
        for (name, label) in Widget::NAMES {
            let row = gtk::ListBoxRow::new();
            row.set_child(Some(
                &gtk::Label::builder().label(label).xalign(0.0).build(),
            ));
            row.set_widget_name(name);
            popover_list.append(&row);
        }
        let popover = gtk::Popover::builder().child(&popover_list).build();
        let add = gtk::MenuButton::builder()
            .icon_name("list-add-symbolic")
            .popover(&popover)
            .tooltip_text("Add widget")
            .build();
        add.add_css_class("flat");
        {
            let (app, list, popover) = (app.clone(), list.clone(), popover.clone());
            popover_list.connect_row_activated(move |_, row| {
                let name = row.widget_name().to_string();
                app.update(|c| section_list(c, section).push(name.clone()));
                fill_section(&app, &list, section);
                popover.popdown();
            });
        }
        g.set_header_suffix(Some(&add));
        g.add(&list);
        g.set_sensitive(app.config().layout.source == LayoutSource::Custom);
        page.add(&g);
        custom_groups.push(g);
    }
    mirror.connect_active_notify(move |row| {
        for g in &custom_groups {
            g.set_sensitive(!row.is_active());
        }
    });

    let widgets = group("Widgets", "");
    widgets.add(&combo(
        app,
        "Workspaces",
        "How workspace buttons look",
        &["Numbers", "Dots"],
        |c| (c.appearance.workspace_style == WorkspaceStyle::Dots) as u32,
        |c, v| {
            c.appearance.workspace_style = if v == 1 {
                WorkspaceStyle::Dots
            } else {
                WorkspaceStyle::Numbers
            }
        },
    ));
    widgets.add(&spin(
        app,
        "Always show",
        "Workspaces shown even when empty",
        (1.0, 10.0, 1.0),
        |c| c.appearance.min_workspaces as f64,
        |c, v| c.appearance.min_workspaces = v as u32,
    ));
    let clock = adw::EntryRow::builder()
        .title("Clock format (empty follows Omarchy)")
        .text(app.config().appearance.clock_format.clone())
        .show_apply_button(true)
        .build();
    {
        let app = app.clone();
        clock.connect_apply(move |row| {
            let text = row.text().to_string();
            app.update(|c| c.appearance.clock_format = text.trim().to_string());
        });
    }
    widgets.add(&clock);
    widgets.add(&switch(
        app,
        "Battery percentage",
        "",
        |c| c.appearance.battery_percentage,
        |c, v| c.appearance.battery_percentage = v,
    ));
    widgets.add(&switch(
        app,
        "Volume percentage",
        "",
        |c| c.appearance.volume_percentage,
        |c, v| c.appearance.volume_percentage = v,
    ));
    widgets.add(&switch(
        app,
        "Brightness percentage",
        "",
        |c| c.appearance.brightness_percentage,
        |c, v| c.appearance.brightness_percentage = v,
    ));
    widgets.add(&switch(
        app,
        "Weather temperature",
        "Show the temperature next to the weather icon",
        |c| c.appearance.weather_temperature,
        |c, v| c.appearance.weather_temperature = v,
    ));
    widgets.add(&switch(
        app,
        "Show all indicators on bar",
        "Keep inactive indicators (Passwordless Sudo, Stay Awake, Night Light, …) directly on the bar for one-tap toggling",
        |c| c.appearance.always_show_indicators,
        |c, v| c.appearance.always_show_indicators = v,
    ));
    widgets.add(&switch(
        app,
        "Now playing",
        "Show the playing track next to the clock when mirroring Omarchy",
        |c| c.appearance.now_playing,
        |c, v| c.appearance.now_playing = v,
    ));
    page.add(&widgets);
    page
}

pub fn appearance(app: &Rc<App>) -> adw::PreferencesPage {
    let page = page("Appearance", "applications-graphics-symbolic");
    let look = group(
        "Look",
        "Colors follow your Omarchy theme and change with it.",
    );
    look.add(&combo(
        app,
        "Background",
        "Black blends into the bezel of the OLED Touch Bar",
        &["Black", "Theme"],
        |c| (c.appearance.background == BackgroundMode::Theme) as u32,
        |c, v| {
            c.appearance.background = if v == 1 {
                BackgroundMode::Theme
            } else {
                BackgroundMode::Black
            }
        },
    ));
    look.add(&combo(
        app,
        "Buttons",
        "Filled buttons or flat, Omarchy-bar style",
        &["Filled", "Flat"],
        |c| (c.appearance.button_style == ButtonStyle::Flat) as u32,
        |c, v| {
            c.appearance.button_style = if v == 1 {
                ButtonStyle::Flat
            } else {
                ButtonStyle::Filled
            }
        },
    ));
    let theme_accent = switch(
        app,
        "Theme accent",
        "Use the Omarchy theme's accent color",
        |c| c.appearance.accent == "theme" || c.appearance.accent.is_empty(),
        |c, v| {
            if v {
                c.appearance.accent = "theme".into();
            } else if c.appearance.accent == "theme" || c.appearance.accent.is_empty() {
                c.appearance.accent = "#7aa2f7".into();
            }
        },
    );
    look.add(&theme_accent);
    let accent_row = adw::ActionRow::builder().title("Custom accent").build();
    let color = gtk::ColorDialogButton::new(Some(gtk::ColorDialog::new()));
    color.set_valign(gtk::Align::Center);
    let current = app.config().appearance.accent.clone();
    if let Ok(rgba) = gdk::RGBA::parse(if current.starts_with('#') {
        current.as_str()
    } else {
        "#7aa2f7"
    }) {
        color.set_rgba(&rgba);
    }
    {
        let app = app.clone();
        color.connect_rgba_notify(move |button| {
            let c = button.rgba();
            let hex = format!(
                "#{:02x}{:02x}{:02x}",
                (c.red() * 255.0).round() as u8,
                (c.green() * 255.0).round() as u8,
                (c.blue() * 255.0).round() as u8
            );
            app.update(|config| {
                if config.appearance.accent != "theme" {
                    config.appearance.accent = hex.clone();
                }
            });
        });
    }
    accent_row.add_suffix(&color);
    accent_row.set_sensitive(!theme_accent.is_active());
    {
        let accent_row = accent_row.clone();
        theme_accent.connect_active_notify(move |row| accent_row.set_sensitive(!row.is_active()));
    }
    look.add(&accent_row);
    look.add(&scale(
        app,
        "Text size",
        "",
        (0.75, 1.3, 0.05),
        2,
        |c| c.appearance.text_scale as f64,
        |c, v| c.appearance.text_scale = v as f32,
    ));
    page.add(&look);
    page
}

pub fn motion(app: &Rc<App>) -> adw::PreferencesPage {
    let page = page("Motion", "media-playlist-shuffle-symbolic");
    let g = group(
        "Animation",
        "Spring physics drive every movement on the Touch Bar.",
    );
    g.add(&switch(
        app,
        "Animations",
        "Turn off for instant changes",
        |c| c.motion.enabled,
        |c, v| c.motion.enabled = v,
    ));
    g.add(&scale(
        app,
        "Speed",
        "",
        (0.25, 3.0, 0.05),
        2,
        |c| c.motion.speed as f64,
        |c, v| c.motion.speed = v as f32,
    ));
    g.add(&scale(
        app,
        "Bounce",
        "How much things overshoot",
        (0.0, 1.0, 0.05),
        2,
        |c| c.motion.bounce as f64,
        |c, v| c.motion.bounce = v as f32,
    ));
    g.add(&scale(
        app,
        "Stagger",
        "Delay between items appearing, in ms",
        (0.0, 60.0, 1.0),
        0,
        |c| c.motion.stagger_ms as f64,
        |c, v| c.motion.stagger_ms = v as f32,
    ));
    g.add(&switch(
        app,
        "Touch ripples",
        "Ink spreads from where you touch",
        |c| c.motion.ripple,
        |c, v| c.motion.ripple = v,
    ));
    g.add(&spin(
        app,
        "Frame rate",
        "Frames per second while animating",
        (20.0, 120.0, 5.0),
        |c| c.motion.fps as f64,
        |c, v| c.motion.fps = v as u32,
    ));
    let replay = adw::ActionRow::builder()
        .title("Replay")
        .subtitle("Play the entrance animation in the preview")
        .build();
    let button = gtk::Button::builder()
        .icon_name("media-playback-start-symbolic")
        .valign(gtk::Align::Center)
        .build();
    button.add_css_class("flat");
    {
        let app = app.clone();
        button.connect_clicked(move |_| app.preview.replay());
    }
    replay.add_suffix(&button);
    replay.set_activatable_widget(Some(&button));
    g.add(&replay);
    page.add(&g);
    page
}

pub fn power(app: &Rc<App>) -> adw::PreferencesPage {
    let page = page("Power", "battery-good-symbolic");
    let g = group(
        "Dimming",
        "The Touch Bar is an OLED panel; dimming it when idle saves power and prevents burn-in.",
    );
    g.add(&spin(
        app,
        "Dim after",
        "Seconds without touches (0 never dims)",
        (0.0, 900.0, 5.0),
        |c| c.power.dim_after as f64,
        |c, v| c.power.dim_after = v as u32,
    ));
    g.add(&scale(
        app,
        "Dimmed brightness",
        "",
        (5.0, 100.0, 1.0),
        0,
        |c| (c.power.dim_level * 100.0) as f64,
        |c, v| c.power.dim_level = v as f32 / 100.0,
    ));
    g.add(&spin(
        app,
        "Turn off after",
        "Seconds without touches (0 keeps it on)",
        (0.0, 3600.0, 30.0),
        |c| c.power.off_after as f64,
        |c, v| c.power.off_after = v as u32,
    ));
    page.add(&g);
    let nav = group("Navigation", "");
    nav.add(&spin(
        app,
        "Return to the bar after",
        "Seconds before an open menu or panel closes itself",
        (3.0, 120.0, 1.0),
        |c| c.power.panel_timeout as f64,
        |c, v| c.power.panel_timeout = v as u32,
    ));
    page.add(&nav);
    page
}
