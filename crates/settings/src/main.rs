//! omatouchbar-settings — the companion app for omatouchbar.
//!
//! It opens like Omarchy's launcher: a card floating over a dimmed desktop
//! on a layer-shell overlay, closed with Esc, Super+W, or a click outside. Inside it
//! looks like Omarchy's own apps (Folio): the theme's colors, Hyprland's
//! rounding, the system monospace font, a sidebar and flat rows. The first
//! time it opens it plays a short tour of the modes. `--window` opens it as
//! an ordinary window instead.

mod callouts;
mod daemon;
mod modes;
mod motion;
mod pages;
mod preview;
mod style;
mod tour;
mod ui;

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    path::PathBuf,
    rc::Rc,
    time::Duration,
};

use adw::prelude::*;
use gtk::{gdk, gio, glib, graphene};
use gtk4_layer_shell::LayerShell;
use omatouchbar_core::{Config, paths, scene::Command};

use crate::{
    motion::{Curve, Motion},
    preview::Preview,
    tour::Tour,
    ui::glyph,
};

const APP_ID: &str = "dev.omatouchbar.Settings";
const CARD_W: i32 = 1120;
const CARD_H: i32 = 760;

#[derive(Clone, Debug, Default)]
struct Options {
    window: bool,
    tour: bool,
    screenshot: Option<PathBuf>,
    view: Option<String>,
    delays: Vec<u64>,
}

fn usage() -> &'static str {
    "omatouchbar-settings — configure omatouchbar\n\n\
     Usage: omatouchbar-settings [--window] [--tour]\n\n\
     \x20 --window   open as a normal window instead of over the desktop\n\
     \x20 --tour     play the welcome tour again\n\n\
     For development: --screenshot <out.png> [--view <name>] [--delay <ms>[,<ms>…]]\n\
     draws the app once per delay into numbered PNGs and quits, without saving\n\
     anything. Views: tour, tour-modes[:<mode>], tour-fn, tour-ready, and the\n\
     pages mode, layout, look, motion, power, renderer, about."
}

fn parse_args() -> Result<Options, String> {
    let mut options = Options::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--window" => options.window = true,
            "--tour" => options.tour = true,
            "--screenshot" => {
                options.screenshot = Some(args.next().ok_or("--screenshot needs a path")?.into())
            }
            "--view" => options.view = Some(args.next().ok_or("--view needs a name")?),
            "--delay" => {
                options.delays = args
                    .next()
                    .ok_or("--delay needs milliseconds")?
                    .split(',')
                    .map(|v| {
                        v.trim()
                            .parse::<u64>()
                            .map_err(|_| format!("bad delay {v}"))
                    })
                    .collect::<Result<_, _>>()?
            }
            "-h" | "--help" => return Err(usage().into()),
            other => return Err(format!("Unknown option {other}\n\n{}", usage())),
        }
    }
    Ok(options)
}

type Listener<T> = Rc<dyn Fn(&T)>;
type StatusListener = Rc<dyn Fn(&daemon::Status, bool)>;
type Handlers = HashMap<String, Vec<Rc<dyn Fn()>>>;

pub struct App {
    config: RefCell<Config>,
    pub preview: Preview,
    toasts: adw::ToastOverlay,
    save: RefCell<Option<glib::SourceId>>,
    /// Screenshot runs change nothing on disk or in the system.
    pub dry_run: bool,
    actions: RefCell<Handlers>,
    config_listeners: RefCell<Vec<Listener<Config>>>,
    status_listeners: RefCell<Vec<StatusListener>>,
    last_status: RefCell<Option<(daemon::Status, bool)>>,
    status_busy: Cell<bool>,
}

impl App {
    pub fn config(&self) -> std::cell::RefMut<'_, Config> {
        self.config.borrow_mut()
    }

    /// Applies a change: preview immediately, file after a short debounce.
    pub fn update(self: &Rc<Self>, change: impl FnOnce(&mut Config)) {
        let snapshot = {
            let mut config = self.config.borrow_mut();
            change(&mut config);
            config.sanitize();
            config.clone()
        };
        self.preview.set_config(snapshot.clone());
        let listeners = self.config_listeners.borrow().clone();
        for listener in listeners {
            listener(&snapshot);
        }
        if let Some(source) = self.save.borrow_mut().take() {
            source.remove();
        }
        let app = self.clone();
        let source = glib::timeout_add_local_once(Duration::from_millis(250), move || {
            app.save.borrow_mut().take();
            if app.dry_run {
                return;
            }
            match app.config.borrow().save() {
                Ok(()) => app.run("saved"),
                Err(e) => app.toast(&format!("Could not save settings: {e}")),
            }
        });
        *self.save.borrow_mut() = Some(source);
    }

    pub fn on_config(&self, f: impl Fn(&Config) + 'static) {
        self.config_listeners.borrow_mut().push(Rc::new(f));
    }

    /// Registers a handler for a named action ("tour", "close", …).
    pub fn on(&self, name: &str, f: impl Fn() + 'static) {
        self.actions
            .borrow_mut()
            .entry(name.into())
            .or_default()
            .push(Rc::new(f));
    }

    pub fn run(&self, name: &str) {
        let handlers = self.actions.borrow().get(name).cloned().unwrap_or_default();
        for handler in handlers {
            handler();
        }
    }

    pub fn toast(&self, text: &str) {
        self.toasts
            .add_toast(adw::Toast::builder().title(text).timeout(3).build());
    }

    pub fn on_status(&self, f: impl Fn(&daemon::Status, bool) + 'static) {
        if let Some((status, selected)) = self.last_status.borrow().as_ref() {
            f(status, *selected);
        }
        self.status_listeners.borrow_mut().push(Rc::new(f));
    }

    pub fn refresh_status(self: &Rc<Self>) {
        if self.status_busy.replace(true) {
            return;
        }
        let app = self.clone();
        ui::background(
            || (daemon::status(), daemon::is_selected()),
            move |(status, selected)| {
                app.status_busy.set(false);
                app.last_status.replace(Some((status.clone(), selected)));
                let listeners = app.status_listeners.borrow().clone();
                for listener in listeners {
                    listener(&status, selected);
                }
            },
        );
    }
}

fn tour_marker() -> PathBuf {
    paths::state_home().join("omatouchbar/settings-tour")
}

/// The settings side: header, preview, sidebar and pages.
struct Settings {
    root: gtk::Box,
    preview_slot: gtk::Box,
    sidebar: gtk::ListBox,
    content: gtk::Stack,
    pages: RefCell<HashMap<String, Motion>>,
    rows: Vec<(String, gtk::ListBoxRow)>,
    entrance: Vec<Motion>,
    side_rows: Vec<Motion>,
}

impl Settings {
    fn new(app: &Rc<App>) -> Rc<Settings> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);

        let header = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        header.add_css_class("omt-header");
        header.append(&ui::label(glyph::LOGO, &["omt-logo", "omt-heading"]));
        header.append(&ui::label("Touch Bar", &["omt-title"]));
        let sub = ui::label("omatouchbar", &["omt-faint", "omt-small"]);
        sub.set_hexpand(true);
        header.append(&sub);
        let status_content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        dot.add_css_class("omt-dot");
        dot.set_valign(gtk::Align::Center);
        status_content.append(&dot);
        let status_label = gtk::Label::new(Some("checking"));
        status_content.append(&status_label);
        let status = gtk::Button::builder().child(&status_content).build();
        status.add_css_class("omt-status");
        status.add_css_class("flat");
        header.append(&status);
        let close = ui::icon_button(glyph::CLOSE, "Close (Esc or Super+W)");
        {
            let app = app.clone();
            close.connect_clicked(move |_| app.run("close"));
        }
        header.append(&close);
        {
            let (dot, status_label, status) = (dot.clone(), status_label.clone(), status.clone());
            app.on_status(move |s, selected| {
                let (live, headline, detail) = pages::status_text(s, selected);
                if live {
                    dot.add_css_class("live");
                } else {
                    dot.remove_css_class("live");
                }
                status_label.set_label(&if live {
                    format!("live · {}×{}", s.width, s.height)
                } else {
                    headline.to_lowercase()
                });
                status.set_tooltip_text(Some(&format!("{headline}: {detail}")));
            });
        }
        let header_motion = Motion::new(&header).rise(-8.0);
        root.append(&header_motion);
        root.append(&ui::hair(gtk::Orientation::Horizontal));

        // The live preview with an Fn key and quick jumps.
        let zone = gtk::Box::new(gtk::Orientation::Vertical, 10);
        zone.set_margin_top(18);
        zone.set_margin_bottom(14);
        zone.set_margin_start(24);
        zone.set_margin_end(24);
        let preview_slot = gtk::Box::new(gtk::Orientation::Vertical, 0);
        zone.append(&preview_slot);
        let controls = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let fn_button = ui::button("fn", &[]);
        fn_button.set_tooltip_text(Some("Hold to preview the Fn layer"));
        let press = gtk::GestureClick::new();
        {
            let preview = app.preview.clone();
            press.connect_pressed(move |_, _, _, _| preview.set_fn(true));
        }
        {
            let preview = app.preview.clone();
            press.connect_released(move |_, _, _, _| preview.set_fn(false));
        }
        {
            let preview = app.preview.clone();
            press.connect_cancel(move |_, _| preview.set_fn(false));
        }
        fn_button.add_controller(press);
        controls.append(&fn_button);
        for (label, command) in [("menu", Command::OpenMenu(None)), ("home", Command::Home)] {
            let button = ui::button(label, &["flat"]);
            let preview = app.preview.clone();
            button.connect_clicked(move |_| preview.command(command.clone()));
            controls.append(&button);
        }
        let caption = ui::label(
            "Live preview. Drag it like the Touch Bar; actions are simulated.",
            &["omt-faint", "omt-small"],
        );
        caption.set_hexpand(true);
        caption.set_xalign(1.0);
        caption.set_ellipsize(gtk::pango::EllipsizeMode::End);
        caption.set_width_chars(8);
        controls.append(&caption);
        zone.append(&controls);
        let zone_motion = Motion::new(&zone).rise(18.0).scale_from(0.98);
        root.append(&zone_motion);
        root.append(&ui::hair(gtk::Orientation::Horizontal));

        // Sidebar and pages.
        let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        body.set_vexpand(true);
        let sidebar = gtk::ListBox::new();
        sidebar.add_css_class("omt-sidebar");
        sidebar.set_selection_mode(gtk::SelectionMode::Single);
        let side_scroll = gtk::ScrolledWindow::builder()
            .child(&sidebar)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .width_request(206)
            .build();
        body.append(&side_scroll);
        body.append(&ui::hair(gtk::Orientation::Vertical));
        let content = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .transition_duration(140)
            .hexpand(true)
            .vexpand(true)
            .build();
        body.append(&content);
        root.append(&body);

        let mut rows = Vec::new();
        let mut side_rows = Vec::new();
        let label_row = |text: &str| {
            let row = gtk::ListBoxRow::new();
            row.set_selectable(false);
            row.set_activatable(false);
            row.set_focusable(false);
            row.add_css_class("omt-side-label");
            row.set_child(Some(&ui::label(&text.to_uppercase(), &["omt-section"])));
            row
        };
        sidebar.append(&label_row("Settings"));
        for (index, entry) in pages::ENTRIES.iter().enumerate() {
            if index == 5 {
                sidebar.append(&label_row("omatouchbar"));
            }
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            line.append(&ui::glyph(entry.glyph, &[]));
            line.append(&ui::label(entry.title, &[]));
            let motion = Motion::new(&line).rise(0.0).blur(2.0);
            side_rows.push(motion.clone());
            let row = gtk::ListBoxRow::new();
            row.set_child(Some(&motion));
            row.set_widget_name(entry.id);
            sidebar.append(&row);
            rows.push((entry.id.to_string(), row));
        }

        let footer_line = ui::hair(gtk::Orientation::Horizontal);
        root.append(&footer_line);
        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 18);
        footer.add_css_class("omt-footer");
        let saved = ui::label(
            &format!("{}  saved", glyph::CHECK),
            &["omt-accent", "omt-tiny"],
        );
        let saved_motion = Motion::new(&saved).rise(4.0);
        saved_motion.hide();
        footer.append(&saved_motion);
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        footer.append(&spacer);
        footer.append(&ui::hint(&["↑", "↓"], "pages"));
        footer.append(&ui::hint(&["ctrl", "o"], "settings file"));
        footer.append(&ui::hint(&["esc"], "close"));
        root.append(&footer);
        {
            let hide_at = Rc::new(RefCell::new(None::<glib::SourceId>));
            let saved_motion = saved_motion.clone();
            app.on("saved", move || {
                if saved_motion.progress() < 0.5 {
                    saved_motion.reveal(0, Curve::SOFT);
                }
                if let Some(source) = hide_at.borrow_mut().take() {
                    source.remove();
                }
                let (m, slot) = (saved_motion.clone(), hide_at.clone());
                *hide_at.borrow_mut() = Some(glib::timeout_add_local_once(
                    Duration::from_millis(1600),
                    move || {
                        slot.borrow_mut().take();
                        m.animate_to(0.0, 0, Curve::Ease(360));
                    },
                ));
            });
        }

        let settings = Rc::new(Settings {
            root,
            preview_slot,
            sidebar,
            content,
            pages: RefCell::default(),
            rows,
            entrance: vec![header_motion, zone_motion],
            side_rows,
        });
        settings.build_pages(app);
        {
            let this = Rc::downgrade(&settings);
            settings.sidebar.connect_row_selected(move |_, row| {
                if let (Some(s), Some(row)) = (this.upgrade(), row) {
                    s.show(&row.widget_name());
                }
            });
        }
        {
            let this = Rc::downgrade(&settings);
            status.connect_clicked(move |_| {
                if let Some(s) = this.upgrade() {
                    s.select("renderer");
                }
            });
        }
        settings.select("mode");
        settings
    }

    fn build_pages(&self, app: &Rc<App>) {
        while let Some(child) = self.content.first_child() {
            self.content.remove(&child);
        }
        let mut pages = self.pages.borrow_mut();
        pages.clear();
        for entry in pages::ENTRIES.iter() {
            let page = pages::build(app, entry.id);
            let motion = Motion::new(&page).rise(12.0);
            self.content.add_named(&motion, Some(entry.id));
            pages.insert(entry.id.into(), motion);
        }
    }

    /// Rebuilds every page from the config, staying on the current one.
    fn rebuild(&self, app: &Rc<App>) {
        let current = self
            .content
            .visible_child_name()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "mode".into());
        self.build_pages(app);
        self.content.set_visible_child_name(&current);
    }

    fn select(&self, id: &str) {
        if let Some((_, row)) = self.rows.iter().find(|(r, _)| r == id) {
            self.sidebar.select_row(Some(row));
        }
    }

    fn show(&self, id: &str) {
        if self.content.visible_child_name().as_deref() == Some(id) {
            return;
        }
        self.content.set_visible_child_name(id);
        if let Some(motion) = self.pages.borrow().get(id) {
            motion.reveal(0, Curve::Ease(380));
        }
    }

    fn enter(&self, app: &App) {
        app.preview.place(&self.preview_slot);
        motion::stagger(&self.entrance, 0, 70, Curve::SOFT);
        motion::stagger(&self.side_rows, 120, 28, Curve::SOFT);
        if let Some(id) = self.content.visible_child_name()
            && let Some(motion) = self.pages.borrow().get(id.as_str())
        {
            motion.reveal(220, Curve::Ease(460));
        }
        if let Some(row) = self.sidebar.selected_row() {
            row.grab_focus();
        }
    }
}

struct Ui {
    app: Rc<App>,
    window: adw::ApplicationWindow,
    card: gtk::Box,
    card_motion: Option<Motion>,
    scrim: Option<Motion>,
    root: gtk::Stack,
    tour: Rc<Tour>,
    settings: Rc<Settings>,
    closing: Cell<bool>,
}

impl Ui {
    fn in_tour(&self) -> bool {
        self.root.visible_child_name().as_deref() == Some("tour")
    }

    fn start_tour(&self) {
        self.root.set_visible_child_name("tour");
        self.tour.start(0);
    }

    fn show_settings(&self) {
        // The tour may have changed the mode, Fn and Esc.
        self.settings.rebuild(&self.app);
        self.root.set_visible_child_name("settings");
        self.settings.enter(&self.app);
    }

    fn close(&self) {
        if self.closing.replace(true) {
            return;
        }
        match (&self.card_motion, &self.scrim) {
            (Some(card), Some(scrim)) => {
                card.animate_to(0.0, 0, Curve::Ease(200));
                scrim.animate_to(0.0, 0, Curve::Ease(200));
                let window = self.window.clone();
                glib::timeout_add_local_once(Duration::from_millis(150), move || window.close());
            }
            _ => self.window.close(),
        }
    }

    fn handle_key(&self, event: &gdk::KeyEvent) -> bool {
        if event.matches(gdk::Key::w, gdk::ModifierType::SUPER_MASK) != gdk::KeyMatch::None {
            self.close();
            return true;
        }
        let (key, state) = (event.keyval(), event.modifier_state());
        if self.app.preview.handle_key(key, state) {
            return true;
        }
        let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
        let focus = GtkWindowExt::focus(&self.window);
        let editing = focus.as_ref().is_some_and(|w| w.is::<gtk::Text>());
        match key {
            gdk::Key::Escape => {
                self.close();
                return true;
            }
            gdk::Key::q | gdk::Key::Q if ctrl => {
                self.close();
                return true;
            }
            gdk::Key::o | gdk::Key::O if ctrl => {
                self.app.run("open-config");
                return true;
            }
            _ => {}
        }
        if self.in_tour() {
            if editing {
                return false;
            }
            return match key {
                gdk::Key::Left => self.tour.nudge(-1),
                gdk::Key::Right => self.tour.nudge(1),
                gdk::Key::Return | gdk::Key::KP_Enter => {
                    let on_button = focus
                        .as_ref()
                        .is_some_and(|w| w.is::<gtk::Button>() || w.is::<gtk::Switch>());
                    if on_button {
                        return false;
                    }
                    self.tour.forward();
                    true
                }
                _ => false,
            };
        }
        if ctrl
            && let Some(digit) = key.to_unicode().and_then(|c| c.to_digit(10))
            && (1..=pages::ENTRIES.len() as u32).contains(&digit)
        {
            self.settings.select(pages::ENTRIES[digit as usize - 1].id);
            if let Some(row) = self.settings.sidebar.selected_row() {
                row.grab_focus();
            }
            return true;
        }
        false
    }
}

fn capture(widget: &gtk::Widget, path: &std::path::Path) -> Result<(), String> {
    let (w, h) = (widget.width(), widget.height());
    let paintable = gtk::WidgetPaintable::new(Some(widget));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, w as f64, h as f64);
    let node = snapshot.to_node().ok_or("nothing was drawn")?;
    let renderer = widget
        .native()
        .and_then(|n| n.renderer())
        .ok_or("no renderer")?;
    let texture = renderer.render_texture(
        &node,
        Some(&graphene::Rect::new(0.0, 0.0, w as f32, h as f32)),
    );
    texture.save_to_png(path).map_err(|e| e.to_string())
}

fn build(application: &adw::Application, options: &Options) {
    let config = Config::load().unwrap_or_default();
    let toasts = adw::ToastOverlay::new();
    let preview = Preview::new(&config, &toasts);
    let dry_run = options.screenshot.is_some();
    let app = Rc::new(App {
        config: RefCell::new(config),
        preview,
        toasts: toasts.clone(),
        save: RefCell::new(None),
        dry_run,
        actions: RefCell::default(),
        config_listeners: RefCell::default(),
        status_listeners: RefCell::default(),
        last_status: RefCell::new(None),
        status_busy: Cell::new(false),
    });
    let styler = {
        let preview = app.preview.clone();
        style::install(move |palette| preview.set_palette(palette))
    };

    let window = adw::ApplicationWindow::builder()
        .application(application)
        .title("Touch Bar")
        .build();
    window.add_css_class("omt");
    let layered =
        !options.window && options.screenshot.is_none() && gtk4_layer_shell::is_supported();
    if layered {
        window.init_layer_shell();
        window.set_layer(gtk4_layer_shell::Layer::Overlay);
        window.set_namespace(Some("omatouchbar-settings"));
        for edge in [
            gtk4_layer_shell::Edge::Top,
            gtk4_layer_shell::Edge::Bottom,
            gtk4_layer_shell::Edge::Left,
            gtk4_layer_shell::Edge::Right,
        ] {
            window.set_anchor(edge, true);
        }
        window.set_exclusive_zone(-1);
        window.set_keyboard_mode(gtk4_layer_shell::KeyboardMode::Exclusive);
    } else {
        window.add_css_class("windowed");
        window.set_default_size(CARD_W, CARD_H);
        window.set_size_request(720, 520);
    }

    let tour = Tour::new(&app);
    let settings = Settings::new(&app);
    let root = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .transition_duration(260)
        .build();
    root.add_named(&tour.root, Some("tour"));
    root.add_named(&settings.root, Some("settings"));
    toasts.set_child(Some(&root));
    let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
    card.add_css_class("omt-card");
    card.set_overflow(gtk::Overflow::Hidden);
    toasts.set_vexpand(true);
    card.append(&toasts);

    let (card_motion, scrim) = if layered {
        let scrim_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        scrim_box.add_css_class("omt-scrim");
        scrim_box.set_hexpand(true);
        scrim_box.set_vexpand(true);
        let scrim = Motion::new(&scrim_box).rise(0.0);
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&scrim));
        let card_motion = Motion::new(&card).rise(-10.0).scale_from(0.965);
        overlay.add_overlay(&card_motion);
        // Center the card at its size, shrunk to fit small screens.
        overlay.connect_get_child_position(|overlay, _| {
            let (w, h) = (overlay.width(), overlay.height());
            let cw = CARD_W.min(w - 48).max(320);
            let ch = CARD_H.min(h - 48).max(320);
            Some(gdk::Rectangle::new((w - cw) / 2, (h - ch) / 2, cw, ch))
        });
        let click = gtk::GestureClick::new();
        {
            let app = app.clone();
            click.connect_released(move |_, _, _, _| app.run("close"));
        }
        scrim_box.add_controller(click);
        window.set_content(Some(&overlay));
        (Some(card_motion), Some(scrim))
    } else {
        window.set_content(Some(&card));
        (None, None)
    };

    let ui = Rc::new(Ui {
        app: app.clone(),
        window: window.clone(),
        card: card.clone(),
        card_motion,
        scrim,
        root,
        tour: tour.clone(),
        settings: settings.clone(),
        closing: Cell::new(false),
    });

    {
        let ui = Rc::downgrade(&ui);
        tour.connect_finished(move || {
            let Some(ui) = ui.upgrade() else {
                return;
            };
            if !ui.app.dry_run {
                let marker = tour_marker();
                if let Some(dir) = marker.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let _ = std::fs::write(&marker, "1\n");
            }
            ui.show_settings();
        });
    }
    {
        let ui = Rc::downgrade(&ui);
        app.on("tour", move || {
            if let Some(ui) = ui.upgrade() {
                ui.start_tour();
            }
        });
    }
    {
        let ui = Rc::downgrade(&ui);
        app.on("close", move || {
            if let Some(ui) = ui.upgrade() {
                ui.close();
            }
        });
    }
    {
        let ui = Rc::downgrade(&ui);
        app.on("open-config", move || {
            let Some(ui) = ui.upgrade() else {
                return;
            };
            let path = Config::path();
            if ui.app.dry_run {
                return;
            }
            if !path.exists() {
                let _ = ui.app.config.borrow().save();
            }
            let uri = gio::File::for_path(&path).uri();
            if gio::AppInfo::launch_default_for_uri(&uri, gio::AppLaunchContext::NONE).is_ok() {
                // The editor opens under the overlay, so get out of its way.
                if ui.card_motion.is_some() {
                    ui.close();
                }
            } else {
                ui.app.toast(&format!("Settings are in {}", path.display()));
            }
        });
    }
    {
        let ui = Rc::downgrade(&ui);
        app.on("reset", move || {
            let Some(ui) = ui.upgrade() else {
                return;
            };
            ui.app.update(|c| *c = Config::default());
            ui.settings.rebuild(&ui.app);
            ui.app.toast("Everything is back to the defaults");
        });
    }

    let key_ctl = gtk::EventControllerKey::new();
    key_ctl.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let ui = Rc::downgrade(&ui);
        key_ctl.connect_key_pressed(move |controller, _, _, _| {
            let key = controller
                .current_event()
                .and_then(|event| event.downcast::<gdk::KeyEvent>().ok());
            match (ui.upgrade(), key) {
                (Some(ui), Some(key)) if ui.handle_key(&key) => glib::Propagation::Stop,
                _ => glib::Propagation::Proceed,
            }
        });
    }
    window.add_controller(key_ctl);

    // Keep the renderer status fresh.
    app.refresh_status();
    {
        let app = app.clone();
        glib::timeout_add_local(Duration::from_secs(2), move || {
            app.refresh_status();
            glib::ControlFlow::Continue
        });
    }

    // What to show first.
    let view = options.view.clone().unwrap_or_default();
    let first_run = !tour_marker().exists();
    let tour_view = view.starts_with("tour") || (view.is_empty() && (options.tour || first_run));
    if tour_view {
        ui.root.set_visible_child_name("tour");
        let step = match view.split(':').next().unwrap_or("") {
            "tour-modes" => 1,
            "tour-fn" => 2,
            "tour-ready" => 3,
            _ => 0,
        };
        tour.start(step);
        if let Some(name) = view.strip_prefix("tour-modes:") {
            let wanted = name.to_lowercase();
            if let Some(index) = omatouchbar_core::config::Mode::ALL
                .iter()
                .position(|m| m.label().to_lowercase().contains(&wanted))
            {
                for _ in 0..index {
                    tour.nudge(1);
                }
            }
        }
    } else {
        ui.root.set_visible_child_name("settings");
        if !view.is_empty() {
            settings.select(&view);
        }
        settings.enter(&app);
    }

    if let Some(card_motion) = &ui.card_motion {
        card_motion.hide();
        if let Some(scrim) = &ui.scrim {
            scrim.hide();
            scrim.reveal(0, Curve::Ease(220));
        }
        let card_motion = card_motion.clone();
        window.connect_map(move |window| {
            // The compositor's close binding targets regular windows. Let the
            // focused overlay receive Super+W directly.
            if let Some(surface) = window.surface().and_downcast::<gdk::Toplevel>() {
                surface.inhibit_system_shortcuts(None::<gdk::Event>);
            }
            card_motion.reveal(0, Curve::Spring(0.78, 320.0));
        });
    }

    if let Some(path) = options.screenshot.clone() {
        let delays = if options.delays.is_empty() {
            vec![1500]
        } else {
            options.delays.clone()
        };
        let count = delays.len();
        for (index, delay) in delays.into_iter().enumerate() {
            let (card, application, path) = (card.clone(), application.clone(), path.clone());
            glib::timeout_add_local_once(Duration::from_millis(delay), move || {
                let out = if count == 1 {
                    path.clone()
                } else {
                    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
                    path.with_file_name(format!("{stem}-{index}.png"))
                };
                // Capture right after the next frame is painted, when the
                // layout is current; between frames a pending resize makes
                // the widget draw nothing.
                let Some(clock) = card.frame_clock() else {
                    eprintln!("screenshot failed: not on screen");
                    return;
                };
                let handler: Rc<RefCell<Option<glib::SignalHandlerId>>> = Rc::default();
                let slot = handler.clone();
                let id = clock.connect_after_paint(move |clock| {
                    if let Some(id) = slot.borrow_mut().take() {
                        clock.disconnect(id);
                    }
                    match capture(card.upcast_ref(), &out) {
                        Ok(()) => println!("{}", out.display()),
                        Err(e) => eprintln!("screenshot failed: {e}"),
                    }
                    if index + 1 == count {
                        application.quit();
                    }
                });
                handler.replace(Some(id));
                clock.request_phase(gdk::FrameClockPhase::PAINT);
            });
        }
    }

    {
        let ui = ui.clone();
        window.connect_close_request(move |window| {
            let _ = (&styler, &ui.card);
            if layered && let Some(surface) = window.surface().and_downcast::<gdk::Toplevel>() {
                surface.restore_system_shortcuts();
            }
            glib::Propagation::Proceed
        });
    }
    window.present();
}

fn main() -> glib::ExitCode {
    let options = match parse_args() {
        Ok(options) => options,
        Err(message) => {
            let help = message.starts_with("omatouchbar-settings");
            if help {
                println!("{message}");
                return glib::ExitCode::SUCCESS;
            }
            eprintln!("{message}");
            return glib::ExitCode::FAILURE;
        }
    };
    let flags = if options.screenshot.is_some() {
        gio::ApplicationFlags::NON_UNIQUE
    } else {
        gio::ApplicationFlags::empty()
    };
    let application = adw::Application::builder()
        .application_id(APP_ID)
        .flags(flags)
        .build();
    application.connect_activate(move |app| {
        if let Some(window) = app.active_window() {
            window.present();
        } else {
            build(app, &options);
        }
    });
    application.run_with_args(&["omatouchbar-settings"])
}
