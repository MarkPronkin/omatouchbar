//! The settings pages, each a scrolling column of Omarchy-style rows.

use std::{cell::RefCell, rc::Rc};

use adw::prelude::*;
use gtk::glib;
use omatouchbar_core::{
    Config, Palette,
    color::Rgba,
    config::{BackgroundMode, ButtonStyle, FnKey, LayoutSource, Mode, WorkspaceStyle},
    omarchy::Widget,
};

use crate::{
    App, daemon, modes,
    ui::{self, glyph},
};

pub struct Entry {
    pub id: &'static str,
    pub glyph: &'static str,
    pub title: &'static str,
}

pub const ENTRIES: [Entry; 7] = [
    Entry {
        id: "mode",
        glyph: glyph::KEYBOARD,
        title: "Mode",
    },
    Entry {
        id: "layout",
        glyph: glyph::LAYOUT,
        title: "Layout",
    },
    Entry {
        id: "look",
        glyph: glyph::LOOK,
        title: "Look",
    },
    Entry {
        id: "motion",
        glyph: glyph::MOTION,
        title: "Motion",
    },
    Entry {
        id: "power",
        glyph: glyph::POWER,
        title: "Power",
    },
    Entry {
        id: "renderer",
        glyph: glyph::RENDERER,
        title: "Renderer",
    },
    Entry {
        id: "about",
        glyph: glyph::ABOUT,
        title: "About",
    },
];

pub fn build(app: &Rc<App>, id: &str) -> gtk::Widget {
    match id {
        "mode" => mode(app),
        "layout" => layout(app),
        "look" => look(app),
        "motion" => motion(app),
        "power" => power(app),
        "renderer" => renderer(app),
        _ => about(app),
    }
}

fn fn_index(c: &Config) -> usize {
    match c.fn_key {
        FnKey::Function => 0,
        FnKey::Controls => 1,
        FnKey::None => 2,
    }
}

fn set_fn(c: &mut Config, v: usize) {
    c.fn_key = [FnKey::Function, FnKey::Controls, FnKey::None][v.min(2)];
}

/// A card per mode, with what it shows and what Fn does in it.
fn mode_cards(app: &Rc<App>) -> (gtk::Grid, Rc<dyn Fn()>) {
    let grid = gtk::Grid::builder()
        .column_spacing(10)
        .row_spacing(10)
        .column_homogeneous(true)
        .build();
    let mut first: Option<gtk::ToggleButton> = None;
    let fn_lines: Rc<RefCell<Vec<(Mode, gtk::Label)>>> = Rc::default();
    for (index, mode) in Mode::ALL.into_iter().enumerate() {
        let info = modes::info(mode);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        top.append(&ui::glyph(info.glyph, &[]));
        let name = ui::label(mode.label(), &["omt-mode-name"]);
        name.set_hexpand(true);
        top.append(&name);
        if index == 0 {
            top.append(&ui::label("default", &["omt-faint", "omt-tiny"]));
        }
        content.append(&top);
        let tagline = ui::wrapped(info.description, &["omt-dim", "omt-small"]);
        tagline.set_lines(3);
        tagline.set_vexpand(true);
        tagline.set_yalign(0.0);
        content.append(&tagline);
        let fn_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        fn_row.set_margin_top(4);
        fn_row.append(&ui::kbd("fn"));
        let fn_text = ui::label("", &["omt-dim", "omt-tiny"]);
        fn_row.append(&fn_text);
        fn_lines.borrow_mut().push((mode, fn_text));
        content.append(&fn_row);

        let button = gtk::ToggleButton::builder().child(&content).build();
        button.add_css_class("omt-mode");
        button.set_group(first.as_ref());
        first.get_or_insert_with(|| button.clone());
        button.set_active(mode == app.config().mode);
        button.update_property(&[
            gtk::accessible::Property::Label(mode.label()),
            gtk::accessible::Property::Description(info.description),
        ]);
        let app = app.clone();
        button.connect_toggled(move |b| {
            if b.is_active() {
                app.update(|c| c.mode = mode);
            }
        });
        grid.attach(&button, (index % 2) as i32, (index / 2) as i32, 1, 1);
    }
    let refresh: Rc<dyn Fn()> = {
        let app = app.clone();
        Rc::new(move || {
            let fn_key = app.config().fn_key;
            for (mode, label) in fn_lines.borrow().iter() {
                let text = match modes::fn_layer(*mode, fn_key) {
                    Some(layer) => format!("hold for {layer}"),
                    None => "does nothing".into(),
                };
                label.set_label(&text);
            }
        })
    };
    refresh();
    (grid, refresh)
}

pub fn mode(app: &Rc<App>) -> gtk::Widget {
    let page = ui::page(
        "Mode",
        "What the Touch Bar shows when nothing else is open. Switch any time; the preview above follows.",
    );
    page.body.append(&ui::section("Touch Bar mode", ""));
    let (cards, refresh_fn) = mode_cards(app);
    cards.set_margin_start(4);
    cards.set_margin_end(4);
    page.body.append(&cards);

    let (hide_bar, _) = ui::toggle_row(
        app,
        "Hide the on-screen bar",
        "Show Omarchy's bar only on the Touch Bar while this mode is on.",
        |c| c.hide_system_bar,
        |c, v| c.hide_system_bar = v,
    );
    hide_bar.set_margin_top(8);
    hide_bar.set_visible(app.config().mode == Mode::Bar);
    page.body.append(&hide_bar);
    {
        let hide_bar = hide_bar.downgrade();
        app.on_config(move |c| {
            if let Some(row) = hide_bar.upgrade() {
                row.set_visible(c.mode == Mode::Bar);
            }
        });
    }

    page.body.append(&ui::section("Keys", ""));
    {
        let app2 = app.clone();
        let seg = ui::segmented(
            &["Function keys", "Media controls", "Nothing"],
            fn_index(&app.config()),
            move |i| {
                app2.update(|c| set_fn(c, i));
                refresh_fn();
            },
        );
        page.body.append(&ui::row(
            "Holding fn shows",
            "A second layer while the Fn key is held. Modes that already show this swap it for the other.",
            Some(seg.upcast_ref()),
        ));
    }
    page.body.append(
        &ui::toggle_row(
            app,
            "Esc key",
            "Pinned to the left edge of the Touch Bar.",
            |c| c.show_escape,
            |c, v| c.show_escape = v,
        )
        .0,
    );
    page.body.append(
        &ui::toggle_row(
            app,
            "On-screen OSD",
            "Show Omarchy's volume and brightness popups for Touch Bar changes.",
            |c| c.osd,
            |c, v| c.osd = v,
        )
        .0,
    );

    page.body.append(&ui::section("New here?", ""));
    let tour = ui::glyph_button(glyph::PLAY, "Take the tour", false, &[]);
    {
        let app = app.clone();
        tour.connect_clicked(move |_| app.run("tour"));
    }
    page.body.append(&ui::row(
        "The tour",
        "A minute on what each mode does, with the preview showing it.",
        Some(tour.upcast_ref()),
    ));
    page.root.upcast()
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

fn fill_section(app: &Rc<App>, list: &gtk::Box, section: Section) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    let names = section_list(&mut app.config(), section).clone();
    if names.is_empty() {
        let empty = ui::label("Nothing here yet", &["omt-faint", "omt-small"]);
        empty.set_margin_start(14);
        empty.set_margin_top(6);
        empty.set_margin_bottom(6);
        list.append(&empty);
    }
    for (index, name) in names.iter().enumerate() {
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        for (g, tip, delta) in [
            (glyph::UP, "Move left", -1i32),
            (glyph::DOWN, "Move right", 1),
            (glyph::REMOVE, "Remove", 0),
        ] {
            let button = ui::icon_button(g, tip);
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
            buttons.append(&button);
        }
        let row = ui::row(&widget_label(name), "", Some(buttons.upcast_ref()));
        row.add_css_class("compact");
        list.append(&row);
    }
}

pub fn layout(app: &Rc<App>) -> gtk::Widget {
    let page = ui::page(
        "Layout",
        "Which widgets the Omarchy Bar mode shows, and how they look.",
    );
    page.body.append(&ui::section("Source", ""));
    let (mirror_row, mirror) = ui::toggle_row(
        app,
        "Mirror the Omarchy bar",
        "Use the same widgets as ~/.config/omarchy/shell.json. Esc always stays on the left.",
        |c| c.layout.source == LayoutSource::Omarchy,
        |c, v| {
            c.layout.source = if v {
                LayoutSource::Omarchy
            } else {
                LayoutSource::Custom
            }
        },
    );
    page.body.append(&mirror_row);

    let custom = gtk::Box::new(gtk::Orientation::Vertical, 0);
    for (section, title, note) in [
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
    ] {
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let label = ui::section(title, note);
        label.set_hexpand(true);
        head.append(&label);
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        fill_section(app, &list, section);

        let options = gtk::ListBox::new();
        options.set_selection_mode(gtk::SelectionMode::None);
        for (name, label) in Widget::NAMES {
            let row = gtk::ListBoxRow::new();
            row.set_child(Some(&ui::label(label, &[])));
            row.set_widget_name(name);
            options.append(&row);
        }
        let scroller = gtk::ScrolledWindow::builder()
            .child(&options)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .max_content_height(320)
            .build();
        let popover = gtk::Popover::builder()
            .child(&scroller)
            .has_arrow(false)
            .build();
        let add = gtk::MenuButton::builder()
            .popover(&popover)
            .tooltip_text("Add a widget")
            .valign(gtk::Align::End)
            .build();
        add.set_child(Some(&ui::label(&format!("{}  add", glyph::ADD), &[])));
        add.add_css_class("omt-menu");
        {
            let (app, list, popover) = (app.clone(), list.clone(), popover.clone());
            options.connect_row_activated(move |_, row| {
                let name = row.widget_name().to_string();
                app.update(|c| section_list(c, section).push(name.clone()));
                fill_section(&app, &list, section);
                popover.popdown();
            });
        }
        head.append(&add);
        custom.append(&head);
        custom.append(&list);
    }
    let set_custom = {
        let custom = custom.clone();
        move |on: bool| {
            custom.set_sensitive(on);
            custom.set_opacity(if on { 1.0 } else { 0.45 });
        }
    };
    set_custom(app.config().layout.source == LayoutSource::Custom);
    mirror.connect_active_notify(move |s| set_custom(!s.is_active()));
    page.body.append(&custom);

    page.body.append(&ui::section("Widgets", ""));
    page.body.append(&ui::segmented_row(
        app,
        "Workspaces",
        "How workspace buttons look",
        &["Numbers", "Dots"],
        |c| (c.appearance.workspace_style == WorkspaceStyle::Dots) as usize,
        |c, v| {
            c.appearance.workspace_style = if v == 1 {
                WorkspaceStyle::Dots
            } else {
                WorkspaceStyle::Numbers
            }
        },
    ));
    page.body.append(&ui::stepper_row(
        app,
        "Always show",
        "Workspaces shown even when empty",
        (1.0, 10.0, 1.0),
        |c| c.appearance.min_workspaces as f64,
        |c, v| c.appearance.min_workspaces = v as u32,
    ));
    let clock = gtk::Entry::builder()
        .text(app.config().appearance.clock_format.clone())
        .placeholder_text("follow Omarchy")
        .width_chars(16)
        .build();
    {
        let app = app.clone();
        let apply = move |entry: &gtk::Entry| {
            let text = entry.text().trim().to_string();
            app.update(|c| c.appearance.clock_format = text.clone());
        };
        let apply2 = apply.clone();
        clock.connect_activate(move |e| apply(e));
        let focus = gtk::EventControllerFocus::new();
        let entry = clock.clone();
        focus.connect_leave(move |_| apply2(&entry));
        clock.add_controller(focus);
    }
    page.body.append(&ui::row(
        "Clock format",
        "strftime, like %H:%M. Empty follows Omarchy.",
        Some(clock.upcast_ref()),
    ));
    for (title, caption, get, set) in [
        (
            "Battery percentage",
            "",
            (|c: &Config| c.appearance.battery_percentage) as ui::Get<bool>,
            (|c: &mut Config, v| c.appearance.battery_percentage = v) as ui::Set<bool>,
        ),
        (
            "Volume percentage",
            "",
            |c| c.appearance.volume_percentage,
            |c, v| c.appearance.volume_percentage = v,
        ),
        (
            "Brightness percentage",
            "",
            |c| c.appearance.brightness_percentage,
            |c, v| c.appearance.brightness_percentage = v,
        ),
        (
            "Weather temperature",
            "Show the temperature next to the weather icon",
            |c| c.appearance.weather_temperature,
            |c, v| c.appearance.weather_temperature = v,
        ),
        (
            "Show all indicators on the bar",
            "Keep inactive indicators (Passwordless Sudo, Stay Awake, Night Light…) on the bar for one-tap toggling",
            |c| c.appearance.always_show_indicators,
            |c, v| c.appearance.always_show_indicators = v,
        ),
        (
            "Now playing",
            "Show the playing track next to the clock when mirroring Omarchy",
            |c| c.appearance.now_playing,
            |c, v| c.appearance.now_playing = v,
        ),
    ] {
        page.body
            .append(&ui::toggle_row(app, title, caption, get, set).0);
    }
    page.root.upcast()
}

/// A round color swatch that sets the accent.
fn swatch(color: Rgba, current: Rc<RefCell<String>>) -> (gtk::Button, gtk::DrawingArea) {
    let area = gtk::DrawingArea::new();
    area.set_content_width(22);
    area.set_content_height(22);
    let hex = color.to_hex();
    {
        let hex = hex.clone();
        area.set_draw_func(move |_, cr, w, h| {
            let t = crate::style::tokens();
            let (cx, cy) = (w as f64 / 2.0, h as f64 / 2.0);
            let chosen = current.borrow().eq_ignore_ascii_case(&hex);
            let r = if chosen { 7.0 } else { 9.0 };
            cr.set_source_rgb(color.r as f64, color.g as f64, color.b as f64);
            if t.radius > 0.0 {
                cr.arc(cx, cy, r, 0.0, std::f64::consts::TAU);
            } else {
                cr.rectangle(cx - r, cy - r, r * 2.0, r * 2.0);
            }
            let _ = cr.fill();
            if chosen {
                cr.set_source_rgb(t.fg.r as f64, t.fg.g as f64, t.fg.b as f64);
                cr.set_line_width(1.5);
                if t.radius > 0.0 {
                    cr.arc(cx, cy, 10.0, 0.0, std::f64::consts::TAU);
                } else {
                    cr.rectangle(cx - 10.0, cy - 10.0, 20.0, 20.0);
                }
                let _ = cr.stroke();
            }
        });
    }
    let button = gtk::Button::builder()
        .child(&area)
        .tooltip_text(&hex)
        .build();
    button.add_css_class("omt-btn");
    button.add_css_class("flat");
    button.add_css_class("icon");
    (button, area)
}

fn set_segment(strip: &gtk::Box, index: usize) {
    let mut child = strip.first_child();
    let mut i = 0;
    while let Some(widget) = child {
        if i == index
            && let Some(button) = widget.downcast_ref::<gtk::ToggleButton>()
        {
            button.set_active(true);
        }
        child = widget.next_sibling();
        i += 1;
    }
}

pub fn look(app: &Rc<App>) -> gtk::Widget {
    let page = ui::page(
        "Look",
        "Colors follow your Omarchy theme and change with it.",
    );
    page.body.append(&ui::section("Strip", ""));
    page.body.append(&ui::segmented_row(
        app,
        "Background",
        "Black blends into the bezel of the OLED Touch Bar",
        &["Black", "Theme"],
        |c| (c.appearance.background == BackgroundMode::Theme) as usize,
        |c, v| {
            c.appearance.background = if v == 1 {
                BackgroundMode::Theme
            } else {
                BackgroundMode::Black
            }
        },
    ));
    page.body.append(&ui::segmented_row(
        app,
        "Buttons",
        "Filled keys, or flat like Omarchy's bar",
        &["Filled", "Flat"],
        |c| (c.appearance.button_style == ButtonStyle::Flat) as usize,
        |c, v| {
            c.appearance.button_style = if v == 1 {
                ButtonStyle::Flat
            } else {
                ButtonStyle::Filled
            }
        },
    ));
    page.body.append(&ui::slider_row(
        app,
        "Text size",
        "",
        (0.75, 1.3, 0.05),
        |v| format!("{:.0}%", v * 100.0),
        |c| c.appearance.text_scale as f64,
        |c, v| c.appearance.text_scale = v as f32,
    ));

    page.body.append(&ui::section("Accent", ""));
    let is_theme = |c: &Config| c.appearance.accent == "theme" || c.appearance.accent.is_empty();
    let current = Rc::new(RefCell::new(app.config().appearance.accent.clone()));
    let entry = gtk::Entry::builder()
        .placeholder_text("#rrggbb")
        .width_chars(9)
        .max_length(7)
        .build();
    if !is_theme(&app.config()) {
        entry.set_text(&app.config().appearance.accent);
    }
    let swatches: Rc<RefCell<Vec<gtk::DrawingArea>>> = Rc::default();
    let redraw = {
        let swatches = swatches.clone();
        move || swatches.borrow().iter().for_each(|s| s.queue_draw())
    };
    let seg = {
        let (app2, current, entry, redraw) =
            (app.clone(), current.clone(), entry.clone(), redraw.clone());
        ui::segmented(
            &["Theme", "Custom"],
            (!is_theme(&app.config())) as usize,
            move |i| {
                let custom = entry.text().to_string();
                app2.update(|c| {
                    if i == 0 {
                        c.appearance.accent = "theme".into();
                    } else if c.appearance.accent == "theme" || c.appearance.accent.is_empty() {
                        c.appearance.accent = if Rgba::hex(&custom).is_some() {
                            custom.clone()
                        } else {
                            "#7aa2f7".into()
                        };
                    }
                });
                *current.borrow_mut() = app2.config().appearance.accent.clone();
                redraw();
            },
        )
    };
    page.body.append(&ui::row(
        "Accent color",
        "The theme's accent, or one of your own",
        Some(seg.upcast_ref()),
    ));

    let palette = Palette::load().unwrap_or_else(Palette::fallback);
    let mut colors: Vec<Rgba> = Vec::new();
    for name in [
        "accent", "red", "orange", "yellow", "green", "cyan", "blue", "magenta",
    ] {
        if let Some(c) = palette.colors.get(name)
            && !colors.iter().any(|x| x.to_hex() == c.to_hex())
        {
            colors.push(*c);
        }
    }
    let strip = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    for color in colors {
        let (button, area) = swatch(color, current.clone());
        swatches.borrow_mut().push(area);
        let (app, current, entry, seg, redraw) = (
            app.clone(),
            current.clone(),
            entry.clone(),
            seg.clone(),
            redraw.clone(),
        );
        button.connect_clicked(move |_| {
            let hex = color.to_hex();
            entry.set_text(&hex);
            app.update(|c| c.appearance.accent = hex.clone());
            *current.borrow_mut() = hex.clone();
            set_segment(&seg, 1);
            redraw();
        });
        strip.append(&button);
    }
    {
        let (app, current, seg, redraw) = (app.clone(), current.clone(), seg.clone(), redraw);
        entry.connect_activate(move |e| {
            let text = e.text().trim().to_string();
            let text = if text.starts_with('#') {
                text
            } else {
                format!("#{text}")
            };
            if Rgba::hex(&text).is_some() {
                app.update(|c| c.appearance.accent = text.clone());
                *current.borrow_mut() = text;
                set_segment(&seg, 1);
                redraw();
            } else {
                app.toast("Colors look like #ff9f0a");
            }
        });
    }
    let custom = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    custom.append(&strip);
    custom.append(&entry);
    page.body.append(&ui::row(
        "Palette",
        "From your theme, or type a hex color and press Enter",
        Some(custom.upcast_ref()),
    ));
    page.root.upcast()
}

pub fn motion(app: &Rc<App>) -> gtk::Widget {
    let page = ui::page(
        "Motion",
        "Spring physics drive every movement on the Touch Bar.",
    );
    page.body.append(&ui::section("Touch feedback", ""));
    page.body.append(
        &ui::toggle_row(
            app,
            "Touch Bar haptics",
            "Feel Touch Bar clicks and ticks through the trackpad. Requires Trackpad Haptics.",
            |c| c.haptics,
            |c, v| c.haptics = v,
        )
        .0,
    );
    page.body.append(
        &ui::toggle_row(
            app,
            "Touch ripples",
            "Ink spreads from where you touch",
            |c| c.motion.ripple,
            |c, v| c.motion.ripple = v,
        )
        .0,
    );
    page.body.append(&ui::section("Animation", ""));
    page.body.append(
        &ui::toggle_row(
            app,
            "Animations",
            "Turn off for instant changes",
            |c| c.motion.enabled,
            |c, v| c.motion.enabled = v,
        )
        .0,
    );
    page.body.append(&ui::slider_row(
        app,
        "Speed",
        "",
        (0.25, 3.0, 0.05),
        |v| format!("{v:.2}×"),
        |c| c.motion.speed as f64,
        |c, v| c.motion.speed = v as f32,
    ));
    page.body.append(&ui::slider_row(
        app,
        "Bounce",
        "How much things overshoot",
        (0.0, 1.0, 0.05),
        |v| format!("{:.0}%", v * 100.0),
        |c| c.motion.bounce as f64,
        |c, v| c.motion.bounce = v as f32,
    ));
    page.body.append(&ui::slider_row(
        app,
        "Stagger",
        "Delay between items appearing",
        (0.0, 60.0, 1.0),
        |v| format!("{v:.0} ms"),
        |c| c.motion.stagger_ms as f64,
        |c, v| c.motion.stagger_ms = v as f32,
    ));
    page.body.append(&ui::stepper_row(
        app,
        "Frame rate",
        "Frames per second while animating",
        (20.0, 120.0, 5.0),
        |c| c.motion.fps as f64,
        |c, v| c.motion.fps = v as u32,
    ));
    let replay = ui::glyph_button(glyph::PLAY, "Replay", false, &[]);
    {
        let app = app.clone();
        replay.connect_clicked(move |_| app.preview.replay());
    }
    page.body.append(&ui::row(
        "Entrance",
        "Play the strip's entrance animation in the preview",
        Some(replay.upcast_ref()),
    ));
    page.root.upcast()
}

pub fn power(app: &Rc<App>) -> gtk::Widget {
    let page = ui::page(
        "Power",
        "The Touch Bar is an OLED panel: dimming it when idle saves power and prevents burn-in.",
    );
    page.body.append(&ui::section("Screensaver", ""));
    page.body.append(
        &ui::toggle_row(
            app,
            "Follow the desktop screensaver",
            "Animate the Touch Bar when Omarchy's screensaver starts. Tap the strip to dismiss it on both screens.",
            |c| c.screensaver,
            |c, v| c.screensaver = v,
        )
        .0,
    );
    page.body.append(&ui::section("Dimming", ""));
    page.body.append(&ui::stepper_row(
        app,
        "Dim after",
        "Seconds without touches (0 never dims)",
        (0.0, 900.0, 5.0),
        |c| c.power.dim_after as f64,
        |c, v| c.power.dim_after = v as u32,
    ));
    page.body.append(&ui::slider_row(
        app,
        "Dimmed brightness",
        "",
        (5.0, 100.0, 1.0),
        |v| format!("{v:.0}%"),
        |c| (c.power.dim_level * 100.0) as f64,
        |c, v| c.power.dim_level = v as f32 / 100.0,
    ));
    page.body.append(&ui::stepper_row(
        app,
        "Turn off after",
        "Seconds without touches (0 keeps it on)",
        (0.0, 3600.0, 30.0),
        |c| c.power.off_after as f64,
        |c, v| c.power.off_after = v as u32,
    ));
    page.body.append(&ui::section("Navigation", ""));
    page.body.append(&ui::stepper_row(
        app,
        "Return to the bar after",
        "Seconds before an open menu or panel closes itself",
        (3.0, 120.0, 1.0),
        |c| c.power.panel_timeout as f64,
        |c, v| c.power.panel_timeout = v as u32,
    ));
    page.root.upcast()
}

pub fn status_text(status: &daemon::Status, selected: bool) -> (bool, String, String) {
    if status.running && status.connected {
        let detail = if status.fps >= 1.0 {
            format!(
                "{}×{} · animating at {:.0} fps",
                status.width, status.height, status.fps
            )
        } else if status.present_ms > 0.0 {
            format!(
                "{}×{} · frames reach the panel in {:.0} ms",
                status.width, status.height, status.present_ms
            )
        } else {
            format!("{}×{}", status.width, status.height)
        };
        (true, "Live on the Touch Bar".into(), detail)
    } else if status.running {
        (
            false,
            "Waiting for the Touch Bar".into(),
            "Running, waiting for the Touch Bar hardware service".into(),
        )
    } else if selected {
        (
            false,
            "Not running".into(),
            "Selected but not running. Try restarting it.".into(),
        )
    } else {
        (
            false,
            "Off".into(),
            "T1Bridge is using its built-in renderer".into(),
        )
    }
}

pub fn renderer(app: &Rc<App>) -> gtk::Widget {
    let page = ui::page(
        "Renderer",
        "omatouchbar draws the Touch Bar as T1Bridge's renderer.",
    );
    page.body.append(&ui::section("Status", ""));
    let block = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    block.add_css_class("omt-row");
    let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    dot.add_css_class("omt-dot");
    dot.add_css_class("big");
    dot.set_valign(gtk::Align::Center);
    block.append(&dot);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_hexpand(true);
    let headline = ui::label("Checking…", &["omt-title"]);
    let detail = ui::label("", &["omt-row-caption"]);
    text.append(&headline);
    text.append(&detail);
    block.append(&text);
    let restart = ui::glyph_button(glyph::RESTART, "Restart", false, &[]);
    {
        let app = app.clone();
        restart.connect_clicked(move |_| {
            if !app.dry_run {
                std::thread::spawn(daemon::restart);
            }
            app.toast("Restarting the Touch Bar renderer…");
        });
    }
    block.append(&restart);
    page.body.append(&block);

    let (enable_row, enable) = {
        let switch = gtk::Switch::builder().active(daemon::is_selected()).build();
        let app = app.clone();
        switch.connect_active_notify(move |s| {
            let on = s.is_active();
            if on == daemon::is_selected() || app.dry_run {
                return;
            }
            let app = app.clone();
            ui::background(
                move || daemon::set_selected(on),
                move |result| {
                    app.toast(&match result {
                        Ok(message) => message,
                        Err(e) => e,
                    });
                    app.refresh_status();
                },
            );
        });
        (
            ui::row(
                "Use omatouchbar",
                "Draw the Touch Bar with omatouchbar instead of T1Bridge's built-in renderer",
                Some(switch.upcast_ref()),
            ),
            switch,
        )
    };
    page.body.append(&enable_row);
    {
        let (dot, headline, detail, enable) = (
            dot.downgrade(),
            headline.downgrade(),
            detail.downgrade(),
            enable.downgrade(),
        );
        app.on_status(move |status, selected| {
            let (Some(dot), Some(headline), Some(detail), Some(enable)) = (
                dot.upgrade(),
                headline.upgrade(),
                detail.upgrade(),
                enable.upgrade(),
            ) else {
                return;
            };
            let (live, title, text) = status_text(status, selected);
            if live {
                dot.add_css_class("live");
            } else {
                dot.remove_css_class("live");
            }
            headline.set_label(&title);
            detail.set_label(&text);
            if enable.is_active() != selected {
                enable.set_active(selected);
            }
        });
    }

    page.body.append(&ui::section("Settings file", ""));
    let open = ui::glyph_button(glyph::FILE, "Open", false, &[]);
    {
        let app = app.clone();
        open.connect_clicked(move |_| app.run("open-config"));
    }
    page.body.append(&ui::row(
        "config.toml",
        &Config::path().display().to_string(),
        Some(open.upcast_ref()),
    ));
    let reset = ui::glyph_button(glyph::RESET, "Reset", false, &["danger"]);
    {
        let app = app.clone();
        let armed = Rc::new(std::cell::Cell::new(None::<glib::SourceId>));
        reset.connect_clicked(move |button| {
            if let Some(source) = armed.take() {
                source.remove();
                button.remove_css_class("armed");
                app.run("reset");
                return;
            }
            button.add_css_class("armed");
            set_button_text(button, "Click again");
            let (button, armed2) = (button.clone(), armed.clone());
            armed.set(Some(glib::timeout_add_local_once(
                std::time::Duration::from_secs(3),
                move || {
                    armed2.take();
                    button.remove_css_class("armed");
                    set_button_text(&button, "Reset");
                },
            )));
        });
    }
    page.body.append(&ui::row(
        "Reset to defaults",
        "Every setting back to how omatouchbar ships",
        Some(reset.upcast_ref()),
    ));
    page.root.upcast()
}

fn set_button_text(button: &gtk::Button, text: &str) {
    if let Some(content) = button.child()
        && let Some(label) = content.last_child().and_downcast::<gtk::Label>()
    {
        label.set_label(text);
    }
}

pub fn about(app: &Rc<App>) -> gtk::Widget {
    let page = ui::page("About", "");
    let hero = gtk::Box::new(gtk::Orientation::Horizontal, 18);
    hero.set_margin_top(10);
    hero.set_margin_start(14);
    let logo = ui::label(glyph::LOGO, &["omt-logo"]);
    logo.add_css_class("omt-display");
    hero.append(&logo);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 4);
    text.set_valign(gtk::Align::Center);
    text.append(&ui::label("omatouchbar", &["omt-heading"]));
    text.append(&ui::label(
        &format!(
            "Version {} · MIT license · Mark Pronkin",
            env!("CARGO_PKG_VERSION")
        ),
        &["omt-dim", "omt-small"],
    ));
    hero.append(&text);
    page.body.append(&hero);
    let blurb = ui::wrapped(
        "The Omarchy bar, menus and controls on the T1 MacBook Pro Touch Bar. Drawn in Rust at up to 120 fps, following your theme as it changes.",
        &["omt-dim"],
    );
    blurb.set_margin_start(14);
    blurb.set_margin_top(14);
    page.body.append(&blurb);
    let tour = ui::glyph_button(glyph::PLAY, "Take the tour", false, &["primary"]);
    tour.set_halign(gtk::Align::Start);
    tour.set_margin_start(14);
    tour.set_margin_top(16);
    {
        let app = app.clone();
        tour.connect_clicked(move |_| app.run("tour"));
    }
    page.body.append(&tour);

    page.body.append(&ui::section("Keyboard", ""));
    for (keys, what) in [
        (&["esc"][..], "Close"),
        (&["ctrl", "1–7"][..], "Jump to a page"),
        (&["↑", "↓"][..], "Move through the pages"),
        (&["ctrl", "q"][..], "Quit"),
        (&["ctrl", "o"][..], "Open the settings file"),
    ] {
        let line = ui::hint(keys, what);
        line.add_css_class("omt-row");
        page.body.append(&line);
    }
    page.root.upcast()
}
