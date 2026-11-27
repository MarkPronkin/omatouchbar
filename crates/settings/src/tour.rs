//! The first-run tour: what omatouchbar is, how the four modes differ, what
//! Fn does, and whether the renderer is on.
//!
//! The real preview stays on stage the whole time while the words around it
//! change, so every mode the tour talks about is shown, not described.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
};

use adw::prelude::*;
use gtk::glib;
use omatouchbar_core::config::{FnKey, Mode};

use crate::{
    App,
    callouts::{Callouts, Measurer, Zone},
    daemon, modes,
    motion::{self, Curve, Motion},
    style,
    ui::{self, glyph},
};

const STEPS: usize = 4;
/// Mode index, Fn setting, Esc shown, Fn held.
type ZoneKey = (usize, usize, bool, bool);
/// How long autoplay dwells on each mode.
const DWELL_US: i64 = 3_800_000;

pub struct Tour {
    pub root: gtk::Box,
    app: Rc<App>,
    step: Cell<usize>,
    dots: Vec<gtk::Box>,
    heroes: gtk::Stack,
    details: gtk::Stack,
    stage: gtk::Box,
    stage_motion: Motion,
    callouts: Callouts,
    stage_caption: gtk::Label,
    caption_motion: Motion,
    back: gtk::Button,
    arrows: gtk::Box,
    next: gtk::Button,
    next_label: gtk::Label,
    measurer: RefCell<Option<Measurer>>,
    zone_cache: RefCell<HashMap<ZoneKey, Vec<Zone>>>,
    finished: RefCell<Option<Box<dyn Fn()>>>,

    // Welcome.
    logo: Motion,
    title: gtk::Label,
    cursor: gtk::Label,
    welcome_sub: Motion,
    features: Vec<Motion>,
    typing: RefCell<Option<glib::SourceId>>,

    // Modes.
    modes_hero: Vec<Motion>,
    tiles: Vec<(Mode, gtk::ToggleButton, gtk::DrawingArea)>,
    tile_motions: Vec<Motion>,
    facts: gtk::Box,
    /// The mode on show.
    chosen: Cell<Mode>,
    /// The mode the user picked, if they have.
    picked: Cell<Option<Mode>>,
    /// Modes autoplay has shown this lap.
    shown: Cell<usize>,
    autoplay: Cell<bool>,
    dwell_from: Cell<i64>,
    programmatic: Cell<bool>,
    ticker: RefCell<Option<gtk::TickCallbackId>>,

    // Fn.
    fn_hero: Vec<Motion>,
    fn_items: Vec<Motion>,
    keycap: gtk::DrawingArea,
    fn_sentence: gtk::Label,
    fn_bar_row: gtk::Box,
    fn_seg: gtk::Box,
    esc_switch: gtk::Switch,
    bar_switch: gtk::Switch,
    key_press: Rc<Cell<f64>>,
    key_ripple: Rc<Cell<f64>>,
    fn_held: Cell<bool>,
    fn_tick: RefCell<Option<gtk::TickCallbackId>>,

    // Ready.
    check: gtk::DrawingArea,
    check_progress: Rc<Cell<f64>>,
    check_tick: RefCell<Option<gtk::TickCallbackId>>,
    ready_hero: Vec<Motion>,
    ready_items: Vec<Motion>,
    ready_title: gtk::Label,
    ready_caption: gtk::Label,
    enable_box: gtk::Box,
    live_box: gtk::Box,
    live_text: gtk::Label,
    enable_switch: gtk::Switch,
}

fn centered(text: &str, classes: &[&str]) -> gtk::Label {
    let label = ui::wrapped(text, classes);
    label.set_xalign(0.5);
    label.set_justify(gtk::Justification::Center);
    label.set_halign(gtk::Align::Center);
    label
}

fn fade(widget: &impl IsA<gtk::Widget>, rise: f64) -> Motion {
    Motion::new(widget).rise(rise).blur(3.0)
}

fn feature(g: &str, title: &str, text: &str) -> gtk::Box {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 8);
    card.add_css_class("omt-feature");
    card.set_hexpand(true);
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    head.append(&ui::label(g, &["omt-fact-glyph"]));
    head.append(&ui::label(title, &["omt-title"]));
    card.append(&head);
    let body = ui::wrapped(text, &["omt-dim", "omt-small"]);
    body.set_yalign(0.0);
    body.set_vexpand(true);
    card.append(&body);
    card
}

fn tip(g: &str, text: &str) -> gtk::Box {
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    line.append(&ui::label(g, &["omt-fact-glyph"]));
    let text = ui::wrapped(text, &[]);
    text.set_hexpand(true);
    line.append(&text);
    line
}

impl Tour {
    pub fn new(app: &Rc<App>) -> Rc<Tour> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);

        // Top bar: name, progress, skip.
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        top.add_css_class("omt-header");
        let logo_small = ui::label(glyph::LOGO, &["omt-logo", "omt-heading"]);
        top.append(&logo_small);
        top.append(&ui::label("omatouchbar", &["omt-title"]));
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        top.append(&spacer);
        let steps = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        steps.add_css_class("omt-steps");
        steps.set_valign(gtk::Align::Center);
        let dots: Vec<gtk::Box> = (0..STEPS)
            .map(|_| {
                let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                steps.append(&dot);
                dot
            })
            .collect();
        top.append(&steps);
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        top.append(&spacer);
        let skip = ui::button("Skip tour", &["flat"]);
        top.append(&skip);
        root.append(&top);
        root.append(&ui::hair(gtk::Orientation::Horizontal));

        let heroes = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .transition_duration(220)
            .vhomogeneous(false)
            .interpolate_size(true)
            .margin_top(34)
            .margin_start(48)
            .margin_end(48)
            .build();
        root.append(&heroes);

        // The stage: the live preview and its callouts.
        let stage = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let stage_motion = Motion::new(&stage).rise(46.0).scale_from(0.94);
        let stage_wrap = gtk::Box::new(gtk::Orientation::Vertical, 0);
        stage_wrap.set_margin_top(26);
        stage_wrap.set_margin_start(40);
        stage_wrap.set_margin_end(40);
        stage_wrap.append(&stage_motion);
        let callouts = Callouts::new();
        callouts.track(&app.preview.picture());
        callouts.widget.set_margin_top(6);
        // Steps without callouts say something about the strip instead.
        let stage_caption = ui::label("", &["omt-dim", "omt-small"]);
        stage_caption.set_halign(gtk::Align::Center);
        stage_caption.set_valign(gtk::Align::Start);
        stage_caption.set_margin_top(16);
        let caption_motion = Motion::new(&stage_caption).rise(-6.0);
        caption_motion.set_halign(gtk::Align::Center);
        caption_motion.set_valign(gtk::Align::Start);
        let under = gtk::Overlay::new();
        under.set_child(Some(&callouts.widget));
        under.add_overlay(&caption_motion);
        stage_wrap.append(&under);
        root.append(&stage_wrap);

        let details = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .transition_duration(220)
            .vexpand(true)
            .margin_start(48)
            .margin_end(48)
            .margin_top(8)
            .build();
        root.append(&details);

        root.append(&ui::hair(gtk::Orientation::Horizontal));
        let bottom = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        bottom.add_css_class("omt-header");
        bottom.set_margin_end(10);
        let back = ui::glyph_button(glyph::ARROW_LEFT, "Back", false, &["flat"]);
        bottom.append(&back);
        let hints = gtk::Box::new(gtk::Orientation::Horizontal, 18);
        hints.set_hexpand(true);
        hints.set_halign(gtk::Align::Center);
        hints.append(&ui::hint(&["⏎"], "continue"));
        let arrows = ui::hint(&["←", "→"], "pick a mode");
        hints.append(&arrows);
        hints.append(&ui::hint(&["esc"], "close"));
        bottom.append(&hints);
        let next_content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let next_label = gtk::Label::new(Some("Get started"));
        next_content.append(&next_label);
        next_content.append(&gtk::Label::new(Some(glyph::ARROW_RIGHT)));
        let next = gtk::Button::builder().child(&next_content).build();
        next.add_css_class("omt-btn");
        next.add_css_class("primary");
        next.set_valign(gtk::Align::Center);
        bottom.append(&next);
        root.append(&bottom);

        // ---------------------------------------------------------- welcome
        let welcome = gtk::Box::new(gtk::Orientation::Vertical, 6);
        welcome.set_halign(gtk::Align::Center);
        welcome.set_margin_top(18);
        let logo_label = ui::label(glyph::LOGO, &["omt-logo", "omt-logo-xl"]);
        logo_label.set_halign(gtk::Align::Center);
        let logo = Motion::new(&logo_label)
            .rise(0.0)
            .scale_from(0.35)
            .blur(12.0);
        welcome.append(&logo);
        let title_row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        title_row.set_halign(gtk::Align::Center);
        title_row.set_margin_top(8);
        let name = "omatouchbar";
        let title = ui::label("", &["omt-hero"]);
        title.set_width_chars(name.chars().count() as i32);
        title.set_max_width_chars(name.chars().count() as i32);
        title_row.append(&title);
        let cursor = ui::label("▌", &["omt-hero", "omt-cursor"]);
        title_row.append(&cursor);
        welcome.append(&title_row);
        let sub = centered(
            "Omarchy's bar, menus and controls, on your Touch Bar.",
            &["omt-dim", "omt-heading"],
        );
        sub.set_margin_top(2);
        let welcome_sub = fade(&sub, 10.0);
        welcome.append(&welcome_sub);
        heroes.add_named(&welcome, Some("0"));

        let feature_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        feature_row.set_homogeneous(true);
        feature_row.set_margin_top(30);
        feature_row.set_valign(gtk::Align::Start);
        let features: Vec<Motion> = [
            (
                glyph::KEYBOARD,
                "Four modes",
                "From a Mac-style function row to the whole Omarchy bar. The next step shows each one.",
            ),
            (
                glyph::TAP,
                "Panels at a tap",
                "Wi-Fi, Bluetooth, Sound, Battery and the Omarchy menu open right on the strip.",
            ),
            (
                glyph::PALETTE,
                "Your theme",
                "Colors follow Omarchy and change the moment your theme does.",
            ),
        ]
        .into_iter()
        .map(|(g, t, x)| {
            let m = fade(&feature(g, t, x), 18.0);
            feature_row.append(&m);
            m
        })
        .collect();
        details.add_named(&feature_row, Some("0"));

        // ------------------------------------------------------------ modes
        let modes_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let h1 = centered("Four ways to use the strip", &["omt-display"]);
        let h2 = centered(
            "Watch each one on the preview. Pick the one it starts in; you can switch any time.",
            &["omt-dim"],
        );
        let modes_hero = vec![fade(&h1, 12.0), fade(&h2, 10.0)];
        for m in &modes_hero {
            modes_box.append(m);
        }
        heroes.add_named(&modes_box, Some("1"));

        let modes_detail = gtk::Box::new(gtk::Orientation::Vertical, 14);
        modes_detail.set_margin_top(4);
        let tile_row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        tile_row.set_homogeneous(true);
        let mut tiles = Vec::new();
        let mut tile_motions = Vec::new();
        let mut first: Option<gtk::ToggleButton> = None;
        for mode in Mode::ALL {
            let info = modes::info(mode);
            let content = gtk::Box::new(gtk::Orientation::Vertical, 4);
            let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            head.append(&ui::glyph(info.glyph, &[]));
            head.append(&ui::label(mode.label(), &["omt-mode-name"]));
            content.append(&head);
            let tagline = ui::label(info.tagline, &["omt-dim", "omt-small"]);
            tagline.set_ellipsize(gtk::pango::EllipsizeMode::End);
            content.append(&tagline);
            let bar = gtk::DrawingArea::new();
            bar.set_content_height(2);
            bar.set_margin_top(8);
            bar.add_css_class("omt-tile-progress");
            content.append(&bar);
            let button = gtk::ToggleButton::builder().child(&content).build();
            button.add_css_class("omt-mode");
            button.set_group(first.as_ref());
            first.get_or_insert_with(|| button.clone());
            let m = fade(&button, 16.0);
            tile_row.append(&m);
            tile_motions.push(m);
            tiles.push((mode, button, bar));
        }
        modes_detail.append(&tile_row);
        let facts = gtk::Box::new(gtk::Orientation::Vertical, 8);
        facts.set_margin_start(6);
        facts.set_margin_top(6);
        modes_detail.append(&facts);
        details.add_named(&modes_detail, Some("1"));

        // --------------------------------------------------------------- fn
        let fn_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let f1 = centered("Hold fn for more", &["omt-display"]);
        let f2 = centered(
            "While you hold the Fn key, the strip swaps to a second layer. Let go and it's back.",
            &["omt-dim"],
        );
        let fn_hero = vec![fade(&f1, 12.0), fade(&f2, 10.0)];
        for m in &fn_hero {
            fn_box.append(m);
        }
        heroes.add_named(&fn_box, Some("2"));

        let fn_detail = gtk::Box::new(gtk::Orientation::Horizontal, 28);
        fn_detail.set_margin_top(10);
        let keycap = gtk::DrawingArea::new();
        keycap.set_content_width(150);
        keycap.set_content_height(118);
        keycap.set_valign(gtk::Align::Start);
        let key_press = Rc::new(Cell::new(0.0));
        let key_ripple = Rc::new(Cell::new(-1.0));
        {
            let (press, ripple) = (key_press.clone(), key_ripple.clone());
            keycap
                .set_draw_func(move |_, cr, w, h| draw_keycap(cr, w, h, press.get(), ripple.get()));
        }
        let keycap_motion = Motion::new(&keycap).rise(0.0).scale_from(0.6).blur(6.0);
        fn_detail.append(&keycap_motion);
        let fn_column = gtk::Box::new(gtk::Orientation::Vertical, 2);
        fn_column.set_hexpand(true);
        let fn_sentence = ui::wrapped("", &["omt-title"]);
        fn_sentence.set_margin_start(14);
        fn_sentence.set_margin_bottom(6);
        let sentence_motion = fade(&fn_sentence, 8.0);
        fn_column.append(&sentence_motion);
        let fn_seg = {
            let selected = match app.config().fn_key {
                FnKey::Function => 0,
                FnKey::Controls => 1,
                FnKey::None => 2,
            };
            let app = app.clone();
            ui::segmented(
                &["Function keys", "Media controls", "Nothing"],
                selected,
                move |i| {
                    app.update(|c| {
                        c.fn_key = [FnKey::Function, FnKey::Controls, FnKey::None][i.min(2)]
                    });
                    app.run("tour-fn-changed");
                },
            )
        };
        let fn_seg_row = ui::row(
            "Holding fn shows",
            "Modes that already show this swap it for the other",
            Some(fn_seg.upcast_ref()),
        );
        let seg_motion = fade(&fn_seg_row, 10.0);
        fn_column.append(&seg_motion);
        let (esc_row, esc_switch) = ui::toggle_row(
            app,
            "Esc key",
            "Pinned to the left edge, in every mode",
            |c| c.show_escape,
            |c, v| c.show_escape = v,
        );
        let esc_motion = fade(&esc_row, 10.0);
        fn_column.append(&esc_motion);
        let (fn_bar_row, bar_switch) = ui::toggle_row(
            app,
            "Hide the on-screen bar",
            "With the Omarchy Bar on the strip, the top of the screen can go",
            |c| c.hide_system_bar,
            |c, v| c.hide_system_bar = v,
        );
        let bar_motion = fade(&fn_bar_row, 10.0);
        fn_column.append(&bar_motion);
        fn_detail.append(&fn_column);
        details.add_named(&fn_detail, Some("2"));
        let fn_items = vec![
            keycap_motion,
            sentence_motion,
            seg_motion,
            esc_motion,
            bar_motion,
        ];

        // ------------------------------------------------------------ ready
        let ready_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
        ready_box.set_halign(gtk::Align::Center);
        let check = gtk::DrawingArea::new();
        check.set_content_width(64);
        check.set_content_height(64);
        check.set_halign(gtk::Align::Center);
        let check_progress = Rc::new(Cell::new(0.0));
        {
            let p = check_progress.clone();
            check.set_draw_func(move |_, cr, w, h| draw_check(cr, w, h, p.get()));
        }
        ready_box.append(&check);
        let ready_title = centered("You're all set", &["omt-display"]);
        ready_title.set_margin_top(6);
        let ready_caption = centered("", &["omt-dim"]);
        let ready_hero = vec![fade(&ready_title, 12.0), fade(&ready_caption, 10.0)];
        for m in &ready_hero {
            ready_box.append(m);
        }
        heroes.add_named(&ready_box, Some("3"));

        let ready_detail = gtk::Box::new(gtk::Orientation::Vertical, 10);
        ready_detail.set_margin_top(6);
        let enable_box = gtk::Box::new(gtk::Orientation::Horizontal, 14);
        enable_box.add_css_class("omt-feature");
        let enable_text = gtk::Box::new(gtk::Orientation::Vertical, 3);
        enable_text.set_hexpand(true);
        enable_text.append(&ui::label("Turn on omatouchbar", &["omt-title"]));
        enable_text.append(&ui::wrapped(
            "T1Bridge is drawing the Touch Bar with its built-in renderer. Switch to omatouchbar to use what you just picked.",
            &["omt-dim", "omt-small"],
        ));
        enable_box.append(&enable_text);
        let enable_switch = gtk::Switch::builder()
            .active(false)
            .valign(gtk::Align::Center)
            .build();
        enable_box.append(&enable_switch);
        let live_box = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        live_box.add_css_class("omt-feature");
        let live_dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        live_dot.add_css_class("omt-dot");
        live_dot.add_css_class("live");
        live_dot.add_css_class("big");
        live_dot.set_valign(gtk::Align::Center);
        live_box.append(&live_dot);
        let live_text = ui::label("Live on the Touch Bar", &["omt-title"]);
        live_text.set_hexpand(true);
        live_box.append(&live_text);
        let status_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        status_box.append(&enable_box);
        status_box.append(&live_box);
        let mut ready_items = vec![fade(&status_box, 12.0)];
        for (g, text) in [
            (
                glyph::MENU,
                "Open this again from the Omarchy menu: Setup › Touch Bar.",
            ),
            (
                glyph::MOUSE,
                "Drag across the preview to try the strip. Actions are only simulated here.",
            ),
            (
                glyph::KEYBOARD,
                "Press esc to close, like the launcher. Settings save as you change them.",
            ),
        ] {
            let line = tip(g, text);
            line.set_margin_start(18);
            ready_items.push(fade(&line, 10.0));
        }
        for m in &ready_items {
            ready_detail.append(m);
        }
        details.add_named(&ready_detail, Some("3"));

        let tour = Rc::new(Tour {
            root,
            app: app.clone(),
            step: Cell::new(0),
            dots,
            heroes,
            details,
            stage,
            stage_motion,
            callouts,
            stage_caption,
            caption_motion,
            back,
            arrows,
            next,
            next_label,
            measurer: RefCell::new(None),
            zone_cache: RefCell::default(),
            finished: RefCell::new(None),
            logo,
            title,
            cursor,
            welcome_sub,
            features,
            typing: RefCell::new(None),
            modes_hero,
            tiles,
            tile_motions,
            facts,
            chosen: Cell::new(app.config().mode),
            picked: Cell::new(None),
            shown: Cell::new(0),
            autoplay: Cell::new(false),
            dwell_from: Cell::new(0),
            programmatic: Cell::new(false),
            ticker: RefCell::new(None),
            fn_hero,
            fn_items,
            keycap,
            fn_sentence,
            fn_bar_row,
            fn_seg,
            esc_switch,
            bar_switch,
            key_press,
            key_ripple,
            fn_held: Cell::new(false),
            fn_tick: RefCell::new(None),
            check,
            check_progress,
            check_tick: RefCell::new(None),
            ready_hero,
            ready_items,
            ready_title,
            ready_caption,
            enable_box,
            live_box,
            live_text,
            enable_switch,
        });
        tour.wire(&skip);
        tour
    }

    fn wire(self: &Rc<Self>, skip: &gtk::Button) {
        {
            let this = Rc::downgrade(self);
            self.next.connect_clicked(move |_| {
                if let Some(t) = this.upgrade() {
                    t.forward();
                }
            });
        }
        {
            let this = Rc::downgrade(self);
            self.back.connect_clicked(move |_| {
                if let Some(t) = this.upgrade() {
                    t.backward();
                }
            });
        }
        {
            let this = Rc::downgrade(self);
            skip.connect_clicked(move |_| {
                if let Some(t) = this.upgrade() {
                    t.finish();
                }
            });
        }
        for (mode, button, bar) in &self.tiles {
            let mode = *mode;
            let this = Rc::downgrade(self);
            button.connect_toggled(move |b| {
                let Some(t) = this.upgrade() else {
                    return;
                };
                if !b.is_active() {
                    return;
                }
                if !t.programmatic.get() {
                    // The user picked: stop the show and keep their choice.
                    t.stop_autoplay();
                    t.picked.set(Some(mode));
                    t.app.update(|c| c.mode = mode);
                }
                t.show_mode(mode);
            });
            let this = Rc::downgrade(self);
            bar.set_draw_func(move |area, cr, w, h| {
                let Some(t) = this.upgrade() else {
                    return;
                };
                if !t.autoplay.get() || t.chosen.get() != mode {
                    return;
                }
                let now = area
                    .frame_clock()
                    .map(|c| c.frame_time())
                    .unwrap_or_else(glib::monotonic_time);
                let p = ((now - t.dwell_from.get()) as f64 / DWELL_US as f64).clamp(0.0, 1.0);
                let tk = style::tokens();
                cr.set_source_rgba(tk.fg.r as f64, tk.fg.g as f64, tk.fg.b as f64, 0.12);
                cr.rectangle(0.0, 0.0, w as f64, h as f64);
                let _ = cr.fill();
                cr.set_source_rgb(tk.accent.r as f64, tk.accent.g as f64, tk.accent.b as f64);
                cr.rectangle(0.0, 0.0, w as f64 * p, h as f64);
                let _ = cr.fill();
            });
        }
        {
            let this = Rc::downgrade(self);
            self.app.on("tour-fn-changed", move || {
                if let Some(t) = this.upgrade() {
                    t.refresh_fn_sentence();
                    t.zone_cache.borrow_mut().clear();
                }
            });
        }
        {
            let this = Rc::downgrade(self);
            self.app.on_status(move |status, selected| {
                if let Some(t) = this.upgrade() {
                    t.apply_status(status, selected);
                }
            });
        }
        {
            let this = Rc::downgrade(self);
            self.enable_switch.connect_active_notify(move |s| {
                let Some(t) = this.upgrade() else {
                    return;
                };
                if !s.is_active() || daemon::is_selected() || t.app.dry_run {
                    return;
                }
                let app = t.app.clone();
                ui::background(
                    || daemon::set_selected(true),
                    move |result| {
                        app.toast(&match result {
                            Ok(message) => message,
                            Err(e) => e,
                        });
                        app.refresh_status();
                    },
                );
            });
        }
    }

    pub fn connect_finished(&self, f: impl Fn() + 'static) {
        self.finished.replace(Some(Box::new(f)));
    }

    /// Starts (or restarts) the tour at `step`.
    pub fn start(self: &Rc<Self>, step: usize) {
        self.app.preview.place(&self.stage);
        self.callouts.track(&self.app.preview.picture());
        self.chosen.set(self.app.config().mode);
        self.picked.set(None);
        self.sync_controls();
        self.go(step.min(STEPS - 1), true);
    }

    /// Matches the tour's own controls to the config, which settings may
    /// have changed since the tour was built.
    fn sync_controls(&self) {
        let c = self.app.config().clone();
        let index = match c.fn_key {
            FnKey::Function => 0,
            FnKey::Controls => 1,
            FnKey::None => 2,
        };
        let mut child = self.fn_seg.first_child();
        let mut i = 0;
        while let Some(widget) = child {
            if let Some(button) = widget.downcast_ref::<gtk::ToggleButton>()
                && i == index
                && !button.is_active()
            {
                button.set_active(true);
            }
            child = widget.next_sibling();
            i += 1;
        }
        if self.esc_switch.is_active() != c.show_escape {
            self.esc_switch.set_active(c.show_escape);
        }
        if self.bar_switch.is_active() != c.hide_system_bar {
            self.bar_switch.set_active(c.hide_system_bar);
        }
    }

    pub fn forward(self: &Rc<Self>) {
        let step = self.step.get();
        if step == 1
            && let Some(mode) = self.picked.get()
        {
            self.app.update(|c| c.mode = mode);
        }
        if step + 1 >= STEPS {
            self.finish();
        } else {
            self.go(step + 1, false);
        }
    }

    pub fn backward(self: &Rc<Self>) {
        let step = self.step.get();
        if step > 0 {
            self.go(step - 1, false);
        }
    }

    /// Left/right on the modes step.
    pub fn nudge(self: &Rc<Self>, delta: i32) -> bool {
        if self.step.get() != 1 {
            return false;
        }
        let index = Mode::ALL
            .iter()
            .position(|m| *m == self.chosen.get())
            .unwrap_or(0) as i32;
        let next = (index + delta).rem_euclid(Mode::ALL.len() as i32) as usize;
        self.tiles[next].1.set_active(true);
        true
    }

    pub fn finish(self: &Rc<Self>) {
        self.leave_step();
        self.callouts.clear();
        self.app.preview.set_config(self.app.config().clone());
        if let Some(f) = self.finished.borrow().as_ref() {
            f();
        }
    }

    fn leave_step(&self) {
        self.stop_autoplay();
        if let Some(id) = self.fn_tick.take() {
            id.remove();
        }
        if self.fn_held.replace(false) {
            self.app.preview.set_fn(false);
        }
        if let Some(source) = self.typing.take() {
            source.remove();
        }
        if let Some(id) = self.check_tick.take() {
            id.remove();
        }
    }

    fn go(self: &Rc<Self>, step: usize, first: bool) {
        self.leave_step();
        self.step.set(step);
        for (i, dot) in self.dots.iter().enumerate() {
            dot.set_css_classes(match i.cmp(&step) {
                std::cmp::Ordering::Less => &["done"],
                std::cmp::Ordering::Equal => &["current"],
                std::cmp::Ordering::Greater => &[],
            });
        }
        let name = step.to_string();
        self.heroes.set_visible_child_name(&name);
        self.details.set_visible_child_name(&name);
        self.back.set_opacity(if step == 0 { 0.0 } else { 1.0 });
        self.back.set_sensitive(step > 0);
        self.next_label.set_label(match step {
            0 => "Get started",
            3 => "Open settings",
            _ => "Continue",
        });
        self.arrows.set_visible(step == 1);
        self.app.preview.set_config(self.app.config().clone());
        self.callouts.set_shown(step == 1 || step == 2);
        if step != 1 && step != 2 {
            self.callouts.clear();
        }
        let caption = match step {
            0 => "This is the real renderer, live. Drag across it to try it.",
            3 => "Your Touch Bar, exactly as it will look.",
            _ => "",
        };
        if caption.is_empty() {
            self.caption_motion.animate_to(0.0, 0, Curve::Ease(160));
        } else {
            self.stage_caption.set_label(caption);
            self.caption_motion
                .reveal(if step == 0 { 2200 } else { 300 }, Curve::SOFT);
        }
        match step {
            0 => self.enter_welcome(first),
            1 => self.enter_modes(),
            2 => self.enter_fn(),
            _ => self.enter_ready(),
        }
        self.next.grab_focus();
    }

    // ------------------------------------------------------------ welcome

    fn enter_welcome(self: &Rc<Self>, first: bool) {
        self.logo.reveal(80, Curve::Spring(0.5, 180.0));
        let name: Vec<char> = "omatouchbar".chars().collect();
        self.title.set_label("");
        self.cursor.set_opacity(1.0);
        let typed = Rc::new(Cell::new(0usize));
        let blink = Rc::new(Cell::new(0u32));
        let (title, cursor) = (self.title.clone(), self.cursor.clone());
        let this = Rc::downgrade(self);
        let source = glib::timeout_add_local(std::time::Duration::from_millis(62), move || {
            let n = typed.get();
            if n < name.len() {
                // Start typing once the logo has landed.
                if blink.get() < 6 {
                    blink.set(blink.get() + 1);
                    cursor.set_opacity(if blink.get() % 4 < 2 { 1.0 } else { 0.0 });
                    return glib::ControlFlow::Continue;
                }
                typed.set(n + 1);
                title.set_label(&name[..n + 1].iter().collect::<String>());
                cursor.set_opacity(1.0);
                return glib::ControlFlow::Continue;
            }
            // Blink a few times, then go.
            let b = blink.get() + 1;
            blink.set(b);
            cursor.set_opacity(if (b / 5).is_multiple_of(2) { 1.0 } else { 0.0 });
            if b > 40 {
                cursor.set_opacity(0.0);
                if let Some(t) = this.upgrade() {
                    t.typing.take();
                }
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });
        self.typing.replace(Some(source));
        self.welcome_sub.reveal(1250, Curve::SOFT);
        self.stage_motion
            .reveal(if first { 1500 } else { 900 }, Curve::Spring(0.7, 140.0));
        let app = self.app.clone();
        glib::timeout_add_local_once(
            std::time::Duration::from_millis(if first { 1560 } else { 960 }),
            move || app.preview.replay(),
        );
        motion::stagger(&self.features, 1900, 110, Curve::SOFT);
    }

    // -------------------------------------------------------------- modes

    fn zones(&self, mode: Mode, fn_held: bool) -> Vec<Zone> {
        let mut config = self.app.config().clone();
        config.mode = mode;
        let key = (
            Mode::ALL.iter().position(|m| *m == mode).unwrap_or(0),
            config.fn_key as usize,
            config.show_escape,
            fn_held,
        );
        if let Some(z) = self.zone_cache.borrow().get(&key) {
            return z.clone();
        }
        let mut measurer = self.measurer.borrow_mut();
        let m = measurer.get_or_insert_with(|| Measurer::new(&config));
        let zones = m.zones(&config, fn_held);
        self.zone_cache.borrow_mut().insert(key, zones.clone());
        zones
    }

    fn enter_modes(self: &Rc<Self>) {
        self.stage_motion.show_now();
        motion::stagger(&self.modes_hero, 40, 80, Curve::SOFT);
        motion::stagger(&self.tile_motions, 160, 60, Curve::POP);
        self.autoplay.set(true);
        let start = Mode::ALL[0];
        self.programmatic.set(true);
        if let Some((_, button, _)) = self.tiles.iter().find(|(m, ..)| *m == start) {
            if button.is_active() {
                self.show_mode(start);
            } else {
                button.set_active(true);
            }
        }
        self.programmatic.set(false);
        self.dwell_from.set(glib::monotonic_time());
        self.shown.set(1);
        let this = Rc::downgrade(self);
        let id = self.root.add_tick_callback(move |_, clock| {
            let Some(t) = this.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if !t.autoplay.get() {
                return glib::ControlFlow::Break;
            }
            for (_, _, bar) in &t.tiles {
                bar.queue_draw();
            }
            if clock.frame_time() - t.dwell_from.get() > DWELL_US {
                let index = Mode::ALL
                    .iter()
                    .position(|m| *m == t.chosen.get())
                    .unwrap_or(0);
                let lap_done = t.shown.get() >= Mode::ALL.len();
                // After one lap, settle back on the mode in use and stop.
                let next = if lap_done {
                    let current = t.app.config().mode;
                    Mode::ALL.iter().position(|m| *m == current).unwrap_or(0)
                } else {
                    (index + 1) % Mode::ALL.len()
                };
                t.shown.set(t.shown.get() + 1);
                t.dwell_from.set(clock.frame_time());
                t.programmatic.set(true);
                if t.tiles[next].1.is_active() {
                    t.show_mode(Mode::ALL[next]);
                } else {
                    t.tiles[next].1.set_active(true);
                }
                t.programmatic.set(false);
                if lap_done {
                    // Returning Break removes this callback; don't remove it twice.
                    t.autoplay.set(false);
                    t.ticker.take();
                    for (_, _, bar) in &t.tiles {
                        bar.queue_draw();
                    }
                    return glib::ControlFlow::Break;
                }
            }
            glib::ControlFlow::Continue
        });
        self.ticker.replace(Some(id));
    }

    fn stop_autoplay(&self) {
        self.autoplay.set(false);
        if let Some(id) = self.ticker.take() {
            id.remove();
        }
        for (_, _, bar) in &self.tiles {
            bar.queue_draw();
        }
    }

    fn show_mode(self: &Rc<Self>, mode: Mode) {
        let changed = self.chosen.replace(mode) != mode;
        self.dwell_from.set(glib::monotonic_time());
        let mut config = self.app.config().clone();
        config.mode = mode;
        self.app.preview.set_config(config.clone());
        self.callouts.set_zones(self.zones(mode, false), 200);
        for (_, _, bar) in &self.tiles {
            bar.queue_draw();
        }

        // The facts for this mode, one line at a time.
        while let Some(child) = self.facts.first_child() {
            self.facts.remove(&child);
        }
        let info = modes::info(mode);
        let mut lines: Vec<Motion> = info
            .facts
            .iter()
            .map(|(g, text)| fade(&tip(g, text), 8.0))
            .collect();
        let fn_line = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let key = ui::kbd("fn");
        key.set_width_request(26);
        fn_line.append(&key);
        fn_line.append(&ui::label(
            &modes::fn_sentence(mode, config.fn_key),
            &["omt-dim"],
        ));
        lines.push(fade(&fn_line, 8.0));
        for line in &lines {
            self.facts.append(line);
        }
        motion::stagger(&lines, if changed { 120 } else { 300 }, 70, Curve::SOFT);
        self.next_label.set_label(&match self.picked.get() {
            Some(picked) => format!("Use {}", picked.label()),
            None => "Continue".into(),
        });
    }

    // ----------------------------------------------------------------- fn

    fn refresh_fn_sentence(&self) {
        let c = self.app.config().clone();
        let text = match modes::fn_layer(c.mode, c.fn_key) {
            Some(layer) => format!("In {} mode, holding fn shows {layer}.", c.mode.label()),
            None => "Fn leaves the strip alone.".into(),
        };
        self.fn_sentence.set_label(&text);
    }

    fn enter_fn(self: &Rc<Self>) {
        self.stage_motion.show_now();
        self.refresh_fn_sentence();
        let bar = self.app.config().mode == Mode::Bar;
        self.fn_bar_row.set_visible(bar);
        motion::stagger(&self.fn_hero, 40, 80, Curve::SOFT);
        self.fn_items[0].reveal(120, Curve::POP);
        motion::stagger(&self.fn_items[1..], 200, 70, Curve::SOFT);
        let mode = self.app.config().mode;
        self.callouts.set_zones(self.zones(mode, false), 150);

        // Press and release Fn on a loop, on the key and the preview alike.
        let start = glib::monotonic_time();
        let this = Rc::downgrade(self);
        let id = self.keycap.add_tick_callback(move |area, clock| {
            let Some(t) = this.upgrade() else {
                return glib::ControlFlow::Break;
            };
            const CYCLE: f64 = 3.4;
            let elapsed = (clock.frame_time() - start) as f64 / 1e6;
            let phase = elapsed % CYCLE;
            let (down_at, up_at) = (0.9, 2.5);
            let held = elapsed > 0.6 && phase >= down_at && phase < up_at;
            let press = if phase < down_at {
                0.0
            } else if phase < down_at + 0.09 {
                (phase - down_at) / 0.09
            } else if phase < up_at {
                1.0
            } else if phase < up_at + 0.14 {
                1.0 - (phase - up_at) / 0.14
            } else {
                0.0
            };
            t.key_press.set(if elapsed > 0.6 { press } else { 0.0 });
            t.key_ripple.set(if elapsed > 0.6 && phase >= down_at {
                (phase - down_at) / 0.9
            } else {
                -1.0
            });
            if held != t.fn_held.get() {
                t.fn_held.set(held);
                t.app.preview.set_fn(held);
                let mode = t.app.config().mode;
                t.callouts.set_zones(t.zones(mode, held), 90);
            }
            area.queue_draw();
            glib::ControlFlow::Continue
        });
        self.fn_tick.replace(Some(id));
    }

    // -------------------------------------------------------------- ready

    fn apply_status(&self, status: &daemon::Status, selected: bool) {
        let live = status.running && status.connected;
        self.enable_box.set_visible(!selected);
        self.live_box.set_visible(selected);
        if selected && self.enable_switch.is_active() {
            self.enable_switch.set_active(false);
        }
        let (_, headline, detail) = crate::pages::status_text(status, selected);
        self.live_text.set_label(&if live {
            format!("{headline} · {detail}")
        } else {
            format!("{headline}. {detail}")
        });
        self.ready_caption.set_label(if live {
            "Your Touch Bar is already showing it. Everything else lives in settings."
        } else if selected {
            "omatouchbar is selected; it starts with the Touch Bar service."
        } else {
            "One last switch and the Touch Bar is yours."
        });
        self.ready_title.set_label(if live || !selected {
            "You're all set"
        } else {
            "Almost there"
        });
    }

    fn enter_ready(self: &Rc<Self>) {
        self.stage_motion.show_now();
        self.app.refresh_status();
        self.check_progress.set(0.0);
        let start = glib::monotonic_time() + 120_000;
        let this = Rc::downgrade(self);
        let id = self.check.add_tick_callback(move |area, clock| {
            let Some(t) = this.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let p = ((clock.frame_time() - start) as f64 / 900_000.0).max(0.0);
            t.check_progress.set(p.min(1.0));
            area.queue_draw();
            if p >= 1.0 {
                t.check_tick.take();
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });
        self.check_tick.replace(Some(id));
        motion::stagger(&self.ready_hero, 380, 80, Curve::SOFT);
        motion::stagger(&self.ready_items, 520, 80, Curve::SOFT);
    }
}

fn rgba(cr: &gtk::cairo::Context, c: omatouchbar_core::color::Rgba, a: f64) {
    cr.set_source_rgba(c.r as f64, c.g as f64, c.b as f64, c.a as f64 * a);
}

fn rounded(cr: &gtk::cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0);
    if r <= 0.0 {
        cr.rectangle(x, y, w, h);
        return;
    }
    use std::f64::consts::{FRAC_PI_2, PI};
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, FRAC_PI_2, PI);
    cr.arc(x + r, y + r, r, PI, PI * 1.5);
    cr.close_path();
}

/// A big Fn keycap that is pressed (`press` 0–1) with a ring spreading out.
fn draw_keycap(cr: &gtk::cairo::Context, w: i32, h: i32, press: f64, ripple: f64) {
    let t = style::tokens();
    let (kw, kh) = (96.0, 72.0);
    let x = (w as f64 - kw) / 2.0;
    let travel = 4.0 * press;
    let y = (h as f64 - kh) / 2.0 - 6.0 + travel;
    let r = t.radius.clamp(2.0, 6.0);

    if (0.0..=1.0).contains(&ripple) {
        let e = motion::ease_out_cubic(ripple);
        let grow = 4.0 + e * 22.0;
        rgba(cr, t.accent, (1.0 - ripple) * 0.55);
        cr.set_line_width(1.5);
        rounded(
            cr,
            x - grow,
            y - grow,
            kw + grow * 2.0,
            kh + grow * 2.0,
            r + grow,
        );
        let _ = cr.stroke();
    }
    // The key's side, which the top sinks into when pressed.
    let lip = 6.0 - travel;
    rgba(cr, t.fg, 0.10);
    rounded(cr, x, y + lip, kw, kh, r);
    let _ = cr.fill();
    // The top.
    let accent = press;
    rgba(cr, t.bg, 1.0);
    rounded(cr, x, y, kw, kh, r);
    let _ = cr.fill_preserve();
    rgba(cr, t.accent, 0.16 * accent);
    let _ = cr.fill_preserve();
    let border = t.fg.alpha(0.4).mix(t.accent, accent as f32);
    rgba(cr, border, 1.0);
    cr.set_line_width(1.5);
    let _ = cr.stroke();

    let layout = pangocairo::functions::create_layout(cr);
    layout.set_font_description(Some(&gtk::pango::FontDescription::from_string(&format!(
        "monospace Bold {}px",
        (t.text * 1.7).round()
    ))));
    layout.set_text("fn");
    let (lw, lh) = layout.pixel_size();
    rgba(cr, t.fg.mix(t.accent, accent as f32), 1.0);
    cr.move_to(x + 14.0, y + kh - lh as f64 - 10.0);
    pangocairo::functions::show_layout(cr, &layout);
    let _ = lw;
    // A little globe like the Mac key, top right.
    layout.set_font_description(Some(&gtk::pango::FontDescription::from_string(&format!(
        "monospace {}px",
        (t.text * 1.15).round()
    ))));
    layout.set_text("\u{F01E7}");
    let (gw, _) = layout.pixel_size();
    rgba(cr, t.secondary.mix(t.accent, accent as f32), 1.0);
    cr.move_to(x + kw - gw as f64 - 12.0, y + 10.0);
    pangocairo::functions::show_layout(cr, &layout);

    // What it is doing, under the key.
    layout.set_font_description(Some(&gtk::pango::FontDescription::from_string(&format!(
        "monospace {}px",
        (t.text * 0.8).round()
    ))));
    layout.set_text(if press > 0.5 { "holding" } else { "released" });
    let (tw, _) = layout.pixel_size();
    rgba(cr, if press > 0.5 { t.accent } else { t.secondary }, 0.9);
    cr.move_to((w as f64 - tw as f64) / 2.0, h as f64 - 16.0);
    pangocairo::functions::show_layout(cr, &layout);
}

/// A ring that draws itself, then a check mark, with a little pop.
fn draw_check(cr: &gtk::cairo::Context, w: i32, h: i32, p: f64) {
    let t = style::tokens();
    let (cx, cy) = (w as f64 / 2.0, h as f64 / 2.0);
    let ring = motion::ease_in_out((p / 0.55).min(1.0));
    let tick = motion::ease_out_cubic(((p - 0.45) / 0.4).clamp(0.0, 1.0));
    let pop = if p > 0.8 {
        1.0 + 0.08
            * (1.0 - ((p - 0.8) / 0.2)).max(0.0)
            * ((p - 0.8) / 0.2 * std::f64::consts::PI).sin()
    } else {
        1.0
    };
    let radius = 26.0 * pop;
    cr.set_line_cap(gtk::cairo::LineCap::Round);
    cr.set_line_join(gtk::cairo::LineJoin::Round);
    // Faint full ring under the drawn one.
    rgba(cr, t.fg, 0.08);
    cr.set_line_width(2.5);
    cr.arc(cx, cy, radius, 0.0, std::f64::consts::TAU);
    let _ = cr.stroke();
    if ring > 0.0 {
        rgba(cr, t.accent, 1.0);
        let start = -std::f64::consts::FRAC_PI_2;
        cr.arc(cx, cy, radius, start, start + ring * std::f64::consts::TAU);
        let _ = cr.stroke();
    }
    if tick > 0.0 {
        let s = radius / 26.0;
        let points = [(-10.0, 0.5), (-3.0, 7.5), (11.0, -7.0)];
        let lengths = [
            ((7.0f64).powi(2) * 2.0).sqrt(),
            ((14.0f64).powi(2) + (14.5f64).powi(2)).sqrt(),
        ];
        let total: f64 = lengths.iter().sum();
        let mut left = tick * total;
        rgba(cr, t.accent, 1.0);
        cr.set_line_width(3.5);
        cr.move_to(cx + points[0].0 * s, cy + points[0].1 * s);
        for i in 0..2 {
            let seg = lengths[i];
            let f = (left / seg).min(1.0);
            let (ax, ay) = points[i];
            let (bx, by) = points[i + 1];
            cr.line_to(cx + (ax + (bx - ax) * f) * s, cy + (ay + (by - ay) * f) * s);
            left -= seg;
            if left <= 0.0 {
                break;
            }
        }
        let _ = cr.stroke();
    }
}
