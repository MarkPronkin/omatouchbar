//! Omarchy-styled building blocks: rows, toggles, segmented controls,
//! sliders, steppers and key hints, each bound to a config field.

use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use omatouchbar_core::Config;

use crate::App;

/// Nerd Font glyphs (Material Design Icons) used across the app.
pub mod glyph {
    pub const LOGO: &str = "\u{e900}";
    pub const KEYBOARD: &str = "\u{F030C}";
    pub const BAR: &str = "\u{F056E}";
    pub const CONTROLS: &str = "\u{F040E}";
    pub const FUNCTION: &str = "\u{F0295}";
    pub const LAYOUT: &str = "\u{F0570}";
    pub const LOOK: &str = "\u{F03D8}";
    pub const MOTION: &str = "\u{F08FF}";
    pub const POWER: &str = "\u{F032A}";
    pub const RENDERER: &str = "\u{F061A}";
    pub const ABOUT: &str = "\u{F02FD}";
    pub const CLOSE: &str = "\u{F0156}";
    pub const UP: &str = "\u{F0143}";
    pub const DOWN: &str = "\u{F0140}";
    pub const REMOVE: &str = "\u{F0A7A}";
    pub const ADD: &str = "\u{F0415}";
    pub const PLAY: &str = "\u{F040A}";
    pub const CHECK: &str = "\u{F012C}";
    pub const ARROW_RIGHT: &str = "\u{F0054}";
    pub const ARROW_LEFT: &str = "\u{F004D}";
    pub const BRIGHTNESS: &str = "\u{F00E0}";
    pub const MUSIC: &str = "\u{F075A}";
    pub const MENU: &str = "\u{F035C}";
    pub const TAP: &str = "\u{F0741}";
    pub const EYE_OFF: &str = "\u{F0209}";
    pub const METER: &str = "\u{F04C5}";
    pub const TERMINAL: &str = "\u{F018D}";
    pub const PALETTE: &str = "\u{F03D8}";
    pub const RESTART: &str = "\u{F0450}";
    pub const FILE: &str = "\u{F09EE}";
    pub const RESET: &str = "\u{F099B}";
    pub const MOUSE: &str = "\u{F037D}";
}

pub type Get<T> = fn(&Config) -> T;
pub type Set<T> = fn(&mut Config, T);

pub fn label(text: &str, classes: &[&str]) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .css_classes(classes.to_vec())
        .build()
}

pub fn wrapped(text: &str, classes: &[&str]) -> gtk::Label {
    let label = label(text, classes);
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_natural_wrap_mode(gtk::NaturalWrapMode::Word);
    label
}

pub fn glyph(g: &str, classes: &[&str]) -> gtk::Label {
    let label = gtk::Label::new(Some(g));
    label.add_css_class("omt-glyph");
    for class in classes {
        label.add_css_class(class);
    }
    label
}

pub fn hair(orientation: gtk::Orientation) -> gtk::Box {
    let line = gtk::Box::new(orientation, 0);
    line.add_css_class("omt-hair");
    line
}

/// A key cap hint, like `esc` or `⏎`.
pub fn kbd(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.add_css_class("omt-kbd");
    label.set_valign(gtk::Align::Center);
    label
}

/// A row of key caps followed by what they do.
pub fn hint(keys: &[&str], what: &str) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    for key in keys {
        row.append(&kbd(key));
    }
    let text = label(what, &["omt-dim", "omt-tiny"]);
    text.set_margin_start(2);
    row.append(&text);
    row
}

pub fn button(text: &str, classes: &[&str]) -> gtk::Button {
    let button = gtk::Button::with_label(text);
    button.add_css_class("omt-btn");
    for class in classes {
        button.add_css_class(class);
    }
    button.set_valign(gtk::Align::Center);
    button
}

/// A button with a glyph before (or after) its label.
pub fn glyph_button(g: &str, text: &str, after: bool, classes: &[&str]) -> gtk::Button {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let icon = gtk::Label::new(Some(g));
    let label = gtk::Label::new(Some(text));
    if after {
        content.append(&label);
        content.append(&icon);
    } else {
        content.append(&icon);
        content.append(&label);
    }
    let button = gtk::Button::builder().child(&content).build();
    button.add_css_class("omt-btn");
    for class in classes {
        button.add_css_class(class);
    }
    button.set_valign(gtk::Align::Center);
    button
}

pub fn icon_button(g: &str, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::with_label(g);
    button.add_css_class("omt-btn");
    button.add_css_class("flat");
    button.add_css_class("icon");
    button.set_tooltip_text(Some(tooltip));
    button.set_valign(gtk::Align::Center);
    button
}

/// A settings page: a scrolling column with a big title.
pub struct Page {
    pub root: gtk::ScrolledWindow,
    pub body: gtk::Box,
}

pub fn page(title: &str, intro: &str) -> Page {
    let body = gtk::Box::new(gtk::Orientation::Vertical, 2);
    body.add_css_class("omt-page");
    let heading = label(title, &["omt-page-title"]);
    heading.set_margin_top(18);
    body.append(&heading);
    if !intro.is_empty() {
        let intro = wrapped(intro, &["omt-dim"]);
        intro.set_margin_top(4);
        intro.set_margin_bottom(6);
        body.append(&intro);
    }
    let clamp = adw::Clamp::builder()
        .maximum_size(760)
        .tightening_threshold(560)
        .child(&body)
        .halign(gtk::Align::Fill)
        .build();
    let root = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&clamp)
        .vexpand(true)
        .hexpand(true)
        .build();
    Page { root, body }
}

/// An uppercase section label, Folio-style, with an optional note.
pub fn section(title: &str, note: &str) -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 3);
    column.set_margin_top(24);
    column.set_margin_bottom(6);
    column.set_margin_start(14);
    column.append(&label(&title.to_uppercase(), &["omt-section"]));
    if !note.is_empty() {
        column.append(&wrapped(note, &["omt-dim", "omt-small"]));
    }
    column
}

/// A row with a title, a caption and a control on the right. Clicking
/// anywhere on a row whose control is a switch flips the switch.
pub fn row(title: &str, caption: &str, control: Option<&gtk::Widget>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    row.add_css_class("omt-row");
    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    text.append(&label(title, &[]));
    if !caption.is_empty() {
        text.append(&wrapped(caption, &["omt-row-caption"]));
    }
    row.append(&text);
    if let Some(control) = control {
        control.set_valign(gtk::Align::Center);
        row.append(control);
        if let Some(switch) = control.downcast_ref::<gtk::Switch>() {
            switch.update_property(&[gtk::accessible::Property::Label(title)]);
            let click = gtk::GestureClick::new();
            let (switch, row_ref) = (switch.clone(), row.downgrade());
            click.connect_released(move |gesture, _, x, y| {
                let Some(row) = row_ref.upgrade() else {
                    return;
                };
                let on_switch = row
                    .pick(x, y, gtk::PickFlags::DEFAULT)
                    .is_some_and(|w| w == switch || w.is_ancestor(&switch));
                if !on_switch && switch.is_sensitive() {
                    switch.set_active(!switch.is_active());
                    gesture.set_state(gtk::EventSequenceState::Claimed);
                }
            });
            row.add_controller(click);
            row.set_cursor_from_name(Some("pointer"));
        }
    }
    row
}

pub fn toggle(app: &Rc<App>, get: Get<bool>, set: Set<bool>) -> gtk::Switch {
    let switch = gtk::Switch::builder().active(get(&app.config())).build();
    let app = app.clone();
    switch.connect_active_notify(move |s| app.update(|c| set(c, s.is_active())));
    switch
}

pub fn toggle_row(
    app: &Rc<App>,
    title: &str,
    caption: &str,
    get: Get<bool>,
    set: Set<bool>,
) -> (gtk::Box, gtk::Switch) {
    let switch = toggle(app, get, set);
    (row(title, caption, Some(switch.upcast_ref())), switch)
}

/// A strip of options where one is chosen (the shell's ButtonGroup).
pub fn segmented(options: &[&str], selected: usize, changed: impl Fn(usize) + 'static) -> gtk::Box {
    let strip = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    strip.add_css_class("omt-seg");
    strip.set_valign(gtk::Align::Center);
    let changed = Rc::new(changed);
    let mut first: Option<gtk::ToggleButton> = None;
    for (index, option) in options.iter().enumerate() {
        let button = gtk::ToggleButton::with_label(option);
        button.add_css_class("omt-btn");
        button.set_group(first.as_ref());
        first.get_or_insert_with(|| button.clone());
        button.set_active(index == selected);
        let changed = changed.clone();
        button.connect_toggled(move |b| {
            if b.is_active() {
                changed(index);
            }
        });
        strip.append(&button);
    }
    strip
}

pub fn segmented_row(
    app: &Rc<App>,
    title: &str,
    caption: &str,
    options: &[&str],
    get: Get<usize>,
    set: Set<usize>,
) -> gtk::Box {
    let app2 = app.clone();
    let seg = segmented(options, get(&app.config()), move |i| {
        app2.update(|c| set(c, i))
    });
    row(title, caption, Some(seg.upcast_ref()))
}

/// A slider with its value printed beside it.
#[allow(clippy::too_many_arguments)]
pub fn slider_row(
    app: &Rc<App>,
    title: &str,
    caption: &str,
    range: (f64, f64, f64),
    format: fn(f64) -> String,
    get: Get<f64>,
    set: Set<f64>,
) -> gtk::Box {
    let control = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, range.0, range.1, range.2);
    scale.set_value(get(&app.config()));
    scale.set_draw_value(false);
    scale.set_size_request(220, -1);
    let value = label(&format(scale.value()), &["omt-value"]);
    value.set_xalign(1.0);
    {
        let app = app.clone();
        let value = value.clone();
        scale.connect_value_changed(move |s| {
            value.set_label(&format(s.value()));
            app.update(|c| set(c, s.value()));
        });
    }
    control.append(&scale);
    control.append(&value);
    row(title, caption, Some(control.upcast_ref()))
}

pub fn stepper_row(
    app: &Rc<App>,
    title: &str,
    caption: &str,
    range: (f64, f64, f64),
    get: Get<f64>,
    set: Set<f64>,
) -> gtk::Box {
    let spin = gtk::SpinButton::with_range(range.0, range.1, range.2);
    spin.set_value(get(&app.config()));
    spin.set_numeric(true);
    {
        let app = app.clone();
        spin.connect_value_changed(move |s| app.update(|c| set(c, s.value())));
    }
    row(title, caption, Some(spin.upcast_ref()))
}

/// Polls a background job and hands its result to `done` on the main loop.
pub fn background<T: Send + 'static>(
    job: impl FnOnce() -> T + Send + 'static,
    done: impl FnOnce(T) + 'static,
) {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(job());
    });
    let mut done = Some(done);
    glib::timeout_add_local(std::time::Duration::from_millis(40), move || {
        match rx.try_recv() {
            Ok(value) => {
                if let Some(done) = done.take() {
                    done(value);
                }
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(_) => glib::ControlFlow::Break,
        }
    });
}
