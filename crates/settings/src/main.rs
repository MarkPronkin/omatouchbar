//! omatouchbar-settings — the companion app for omatouchbar.

mod daemon;
mod pages;
mod preview;
mod style;

use std::{cell::RefCell, rc::Rc, time::Duration};

use adw::prelude::*;
use gtk::{gio, glib};
use omatouchbar_core::{Config, scene::Command};

use crate::preview::Preview;

const APP_ID: &str = "dev.omatouchbar.Settings";

pub struct App {
    config: RefCell<Config>,
    pub preview: Preview,
    toasts: adw::ToastOverlay,
    save: RefCell<Option<glib::SourceId>>,
    status: RefCell<Option<(adw::ActionRow, gtk::Box)>>,
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
        self.preview.set_config(snapshot);
        if let Some(source) = self.save.borrow_mut().take() {
            source.remove();
        }
        let app = self.clone();
        let source = glib::timeout_add_local_once(Duration::from_millis(250), move || {
            app.save.borrow_mut().take();
            if let Err(e) = app.config.borrow().save() {
                app.toast(&format!("Could not save settings: {e}"));
            }
        });
        *self.save.borrow_mut() = Some(source);
    }

    pub fn toast(&self, text: &str) {
        self.toasts
            .add_toast(adw::Toast::builder().title(text).timeout(3).build());
    }

    pub fn set_status_row(&self, row: adw::ActionRow, dot: gtk::Box) {
        *self.status.borrow_mut() = Some((row, dot));
        self.refresh_status();
    }

    pub fn refresh_status(&self) {
        let Some((row, dot)) = self.status.borrow().clone() else {
            return;
        };
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send((daemon::status(), daemon::is_selected()));
        });
        glib::timeout_add_local(Duration::from_millis(50), move || match rx.try_recv() {
            Ok((status, selected)) => {
                let text = if status.running && status.connected {
                    dot.add_css_class("live");
                    if status.fps >= 1.0 {
                        format!(
                            "Live on the Touch Bar · {}×{} · animating at {:.0} fps",
                            status.width, status.height, status.fps
                        )
                    } else if status.present_ms > 0.0 {
                        format!(
                            "Live on the Touch Bar · {}×{} · frames reach the panel in {:.0} ms",
                            status.width, status.height, status.present_ms
                        )
                    } else {
                        format!("Live on the Touch Bar · {}×{}", status.width, status.height)
                    }
                } else if status.running {
                    dot.remove_css_class("live");
                    "Running, waiting for the Touch Bar hardware service".into()
                } else if selected {
                    dot.remove_css_class("live");
                    "Selected but not running — try Restart".into()
                } else {
                    dot.remove_css_class("live");
                    "Not running — T1Bridge is using its built-in renderer".into()
                };
                row.set_subtitle(&text);
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(_) => glib::ControlFlow::Break,
        });
    }
}

fn build(application: &adw::Application) {
    let config = Config::load().unwrap_or_default();
    let toasts = adw::ToastOverlay::new();
    let preview = Preview::new(&config, &toasts);
    let app = Rc::new(App {
        config: RefCell::new(config),
        preview,
        toasts: toasts.clone(),
        save: RefCell::new(None),
        status: RefCell::new(None),
    });
    let styler = {
        let preview = app.preview.clone();
        style::install(move |palette| preview.set_palette(palette))
    };

    let window = adw::ApplicationWindow::builder()
        .application(application)
        .title("Touch Bar")
        .default_width(1240)
        .default_height(820)
        .width_request(360)
        .height_request(320)
        .build();
    let key_ctl = gtk::EventControllerKey::new();
    key_ctl.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let preview = app.preview.clone();
        key_ctl.connect_key_pressed(move |_, key, _, state| {
            if preview.handle_key(key, state) {
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
    }
    window.add_controller(key_ctl);

    let stack = adw::ViewStack::new();
    for (page, name) in [
        (pages::general(&app), "general"),
        (pages::layout(&app), "layout"),
        (pages::appearance(&app), "appearance"),
        (pages::motion(&app), "motion"),
        (pages::power(&app), "power"),
    ] {
        let title = page.title().to_string();
        let icon = page.icon_name().map(|s| s.to_string()).unwrap_or_default();
        stack.add_titled_with_icon(&page, Some(name), &title, &icon);
    }
    let switcher = adw::ViewSwitcher::builder()
        .stack(&stack)
        .policy(adw::ViewSwitcherPolicy::Wide)
        .build();

    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&switcher));
    let menu = gio::Menu::new();
    menu.append(Some("Open Settings File"), Some("app.open-config"));
    menu.append(Some("Reset to Defaults"), Some("app.reset"));
    menu.append(Some("About omatouchbar"), Some("app.about"));
    let menu_button = gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .menu_model(&menu)
        .primary(true)
        .build();
    header.pack_end(&menu_button);

    // Preview with an Fn key and quick jumps.
    let preview_box = gtk::Box::new(gtk::Orientation::Vertical, 10);
    preview_box.set_margin_top(18);
    preview_box.set_margin_bottom(6);
    preview_box.set_margin_start(24);
    preview_box.set_margin_end(24);
    preview_box.append(&app.preview.widget);
    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let fn_button = gtk::Button::builder()
        .label("fn")
        .tooltip_text("Hold to preview the Fn layer")
        .build();
    fn_button.add_css_class("fn-key");
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
    for (label, command) in [("Menu", Command::OpenMenu(None)), ("Bar", Command::Home)] {
        let button = gtk::Button::builder().label(label).build();
        button.add_css_class("flat");
        let preview = app.preview.clone();
        button.connect_clicked(move |_| preview.command(command.clone()));
        controls.append(&button);
    }
    let caption = gtk::Label::builder()
        .label("Live preview — click and drag it like the Touch Bar. Actions are simulated here.")
        .hexpand(true)
        .xalign(1.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .width_chars(8)
        .build();
    caption.add_css_class("preview-caption");
    controls.append(&caption);
    preview_box.append(&controls);

    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    body.append(&preview_box);
    stack.set_vexpand(true);
    body.append(&stack);
    toasts.set_child(Some(&body));

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&toasts));
    // Narrow windows (tiled halves) move the page tabs to the bottom.
    let switcher_bar = adw::ViewSwitcherBar::builder().stack(&stack).build();
    toolbar.add_bottom_bar(&switcher_bar);
    window.set_content(Some(&toolbar));
    let narrow = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
        adw::BreakpointConditionLengthType::MaxWidth,
        1000.0,
        adw::LengthUnit::Sp,
    ));
    narrow.add_setter(&switcher, "visible", Some(&false.to_value()));
    narrow.add_setter(&switcher_bar, "reveal", Some(&true.to_value()));
    window.add_breakpoint(narrow);

    // Actions.
    let open = gio::SimpleAction::new("open-config", None);
    {
        let app = app.clone();
        open.connect_activate(move |_, _| {
            let path = Config::path();
            if !path.exists() {
                let _ = app.config.borrow().save();
            }
            let uri = gio::File::for_path(&path).uri();
            if gio::AppInfo::launch_default_for_uri(&uri, gio::AppLaunchContext::NONE).is_err() {
                app.toast(&format!("Settings are in {}", path.display()));
            }
        });
    }
    application.add_action(&open);
    let reset = gio::SimpleAction::new("reset", None);
    {
        let (app, window, application) = (app.clone(), window.clone(), application.clone());
        reset.connect_activate(move |_, _| {
            let previous = app.config.borrow().clone();
            app.update(|c| *c = Config::default());
            // Rebuild the pages so every control shows the defaults.
            window.close();
            build(&application);
            let _ = previous;
        });
    }
    application.add_action(&reset);
    let about = gio::SimpleAction::new("about", None);
    {
        let window = window.clone();
        about.connect_activate(move |_, _| {
            let dialog = adw::AboutDialog::builder()
                .application_name("omatouchbar")
                .application_icon(APP_ID)
                .version(env!("CARGO_PKG_VERSION"))
                .comments("The Omarchy bar, menus and controls on the T1 MacBook Pro Touch Bar.")
                .license_type(gtk::License::MitX11)
                .developer_name("Mark Pronkin")
                .build();
            dialog.present(Some(&window));
        });
    }
    application.add_action(&about);

    // Keep the renderer status fresh.
    {
        let app = app.clone();
        glib::timeout_add_local(Duration::from_secs(2), move || {
            app.refresh_status();
            glib::ControlFlow::Continue
        });
    }
    window.connect_close_request(move |_| {
        let _ = &styler;
        glib::Propagation::Proceed
    });
    window.present();
}

fn main() -> glib::ExitCode {
    let application = adw::Application::builder().application_id(APP_ID).build();
    application.connect_activate(|app| {
        if let Some(window) = app.active_window() {
            window.present();
        } else {
            build(app);
        }
    });
    application.run()
}
