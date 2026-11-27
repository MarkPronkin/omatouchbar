//! The Touch Bar UI: pages, navigation, touch handling and timing.
//!
//! The scene is host-agnostic: the renderer daemon feeds it live desktop
//! state and touches, executes the [`Action`]s it emits, and draws it at
//! the panel's frame rate. The settings app runs the same scene for its
//! interactive preview.

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use chrono::{DateTime, Local, NaiveDateTime, Utc};

use crate::{
    anim::{Animated, Animator, Feel, Spring, SpringParams},
    canvas::ClipRect,
    config::{Config, FnKey, Mode},
    display::DisplayChange,
    menu::{self, MenuItem, MenuTree},
    node::{HitKind, Key, Motion, Rect},
    omarchy::BarLayout,
    screensaver::{Geometry, Grid, Saver},
    speedtest::{SpeedTestEvent, SpeedTestState},
    state::{
        BtOp, DesktopState, DnsProvider, Indicator, NetKind, TouchId, TouchIdStage, TrayMenuEntry,
    },
    text::Fonts,
    theme::{Palette, Theme},
    touchfx::TouchFx,
    weather::Location,
    wipe::ThemeWipe,
    world_clock::{self, Input as WorldInput},
};

/// Keyboard events delivered to the Touch Bar while a text prompt is active.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KeyInput {
    Text(String),
    Backspace,
    Clear,
    Enter,
    Select(i8),
    Escape,
    Paste(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WifiPromptPhase {
    Editing,
    Connecting,
    Connected,
    Failed(String),
}

#[derive(Clone, Debug)]
pub(crate) struct WifiPrompt {
    pub ssid: String,
    pub signal: u8,
    pub password: String,
    pub revealed: bool,
    pub phase: WifiPromptPhase,
    pub opened_at: Instant,
    pub last_edit: Instant,
    pub connected_at: Option<Instant>,
}

/// The Weather panel's location search, typed on the physical keyboard.
#[derive(Clone, Debug)]
pub(crate) struct WeatherSearch {
    pub query: String,
    pub last_edit: Instant,
    /// Suggestions for `results_for`.
    pub results: Vec<Location>,
    pub results_for: String,
    /// A lookup for the current query is in flight.
    pub searching: bool,
    /// The last lookup could not reach the geocoder.
    pub failed: bool,
}

impl WeatherSearch {
    /// Results that match what is typed now.
    pub fn current_results(&self) -> &[Location] {
        if self.results_for == self.query.trim() {
            &self.results
        } else {
            &[]
        }
    }
}

/// Minimum query length before the geocoder is asked (as in Omarchy).
const WEATHER_SEARCH_MIN: usize = 2;
/// Release keyboard capture for an idle city search or clock editor.
const TEXT_ENTRY_IDLE: Duration = Duration::from_secs(45);

/// Something the host must do.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Tap a key through the hardware service (Esc, F1–F12).
    Key(u16),
    /// Run a shell command detached from the renderer.
    Shell(String),
    FocusWorkspace {
        id: i32,
        slot: i32,
        monitor: String,
        same_block: bool,
    },
    SetVolume(u8),
    ToggleMute,
    ToggleMicMute,
    StepVolume(bool),
    /// Set the default input's (microphone's) volume.
    SetMicVolume(u8),
    /// Make an output the default and move playing streams to it.
    SetDefaultOutput {
        id: u32,
        name: String,
    },
    /// Make an input the default and move recording streams to it.
    SetDefaultInput {
        id: u32,
        name: String,
    },
    /// Set one app's playback volume.
    SetStreamVolume {
        id: u32,
        volume: u8,
    },
    ToggleStreamMute(u32),
    MediaPlayPause,
    MediaNext,
    MediaPrevious,
    SetDisplayBrightness(u8),
    ConfigureDisplay(DisplayChange),
    SetKeyboardBacklight(u8),
    StepDisplayBrightness(bool),
    StepKeyboardBacklight(bool),
    SetPowerProfile(String),
    SetWifi(bool),
    SetBluetooth(bool),
    /// Join a known or open Wi-Fi network.
    ConnectWifi {
        ssid: String,
        known: bool,
        secure: bool,
    },
    /// Join a secured Wi-Fi network using a password entered on the Touch Bar.
    ConnectWifiPassword {
        ssid: String,
        password: String,
    },
    /// Abort an in-flight Wi-Fi connection attempt.
    CancelWifiConnect,
    /// Start the live network speed test.
    StartSpeedTest,
    /// Stop an active network speed test.
    StopSpeedTest,
    /// Switch the system DNS provider via `omarchy-dns`.
    SetDns(DnsProvider),
    /// Pin or unpin the active Wi-Fi band via `omarchy-network-band`.
    SetWifiBand(String),
    /// Copy text (e.g. IP address or gateway) to the Wayland clipboard.
    CopyToClipboard(String),
    /// Summon Omarchy's Wi-Fi QR share overlay.
    ShareWifiQr {
        iface: String,
        ssid: String,
    },
    /// Open Omarchy's captive portal sign-in window.
    OpenCaptivePortal,
    /// Pair, connect, disconnect or forget a Bluetooth device (answered
    /// with [`Scene::apply_bluetooth_done`]).
    BluetoothDevice {
        address: String,
        op: BtOp,
    },
    ActivateTray {
        address: String,
        context_menu: bool,
    },
    LoadTrayMenu(TrayMenu),
    ClickTrayMenu {
        menu: TrayMenu,
        id: i32,
    },
    WorldClock(world_clock::Request),
    RefreshWeather,
    /// Look up places matching the typed text (answered with
    /// [`Scene::apply_weather_suggestions`]).
    SearchWeatherLocation(String),
    /// Store the weather location via `omarchy-weather-location`
    /// (`None` returns to auto-detection).
    SetWeatherLocation(Option<Location>),
    RefreshAgents {
        disabled: Vec<String>,
    },
    CycleKeyboardLayout,
    /// Trigger or toggle one of Omarchy's status indicators.
    TriggerIndicator {
        indicator: Indicator,
        currently_active: bool,
    },
    /// Open Omarchy's interactive reminder prompt.
    SetReminder,
    /// Clear all queued Omarchy reminders.
    ClearReminders,
    CancelTouchId,
    /// The strip was touched during Omarchy's screensaver: end it on the
    /// desktop too.
    DismissScreensaver,
    /// Evaluate the menu's `when`/`checked`/`disabled` shell conditions.
    EvaluateGuards {
        menu: String,
        guards: Vec<Guard>,
    },
    /// Run a menu provider script and report its rows.
    LoadProvider {
        menu: String,
        provider: String,
    },
    /// A panel opened; the host may refresh data it shows.
    PanelOpened(Panel),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GuardKind {
    When,
    Checked,
    Disabled,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Guard {
    pub id: String,
    pub kind: GuardKind,
    pub expr: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrayMenu {
    pub address: String,
    pub path: String,
    /// Root first, followed by each submenu selected on the strip.
    pub parents: Vec<i32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Panel {
    Audio,
    Brightness,
    Power,
    Network,
    Bluetooth,
    Clock,
    Tray,
    Weather,
    Agents,
    Indicators,
}

impl Panel {
    pub fn title(self) -> &'static str {
        match self {
            Panel::Audio => "Sound",
            Panel::Brightness => "Display",
            Panel::Power => "Battery",
            Panel::Network => "Network",
            Panel::Bluetooth => "Bluetooth",
            Panel::Clock => "Clock",
            Panel::Tray => "Tray",
            Panel::Weather => "Weather",
            Panel::Agents => "Agents",
            Panel::Indicators => "Indicators",
        }
    }

    /// The bar widget a panel grows out of.
    pub fn origin(self) -> Key {
        Key::new(match self {
            Panel::Audio => "w/audio",
            Panel::Brightness => "w/brightness",
            Panel::Power => "w/battery",
            Panel::Network => "w/network",
            Panel::Bluetooth => "w/bluetooth",
            Panel::Clock => "w/clock",
            Panel::Tray => "w/tray",
            Panel::Weather => "w/weather",
            Panel::Agents => "w/agents",
            Panel::Indicators => "w/indicators",
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Slider {
    Volume,
    Display,
    /// A panel brightness drag stays bound to the connector it began on.
    Monitor(Key),
    Keyboard,
    /// The default input's volume.
    Mic,
    /// One app's playback volume (sink-input index).
    Stream(u32),
}

impl Slider {
    fn minimum_percent(self) -> u8 {
        match self {
            Self::Monitor(_) => 1,
            _ => 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LevelFeedback {
    SliderOpened,
    Tick,
    Minimum,
    Maximum,
}

impl LevelFeedback {
    fn at(percent: u8, minimum: u8) -> Self {
        match percent {
            p if p == minimum => Self::Minimum,
            100 => Self::Maximum,
            _ => Self::Tick,
        }
    }

    // One actuator plays one cue per frame. Limits take priority over the
    // opening click, which takes priority over ordinary adjustment ticks.
    fn queue(self, pending: &mut Option<Self>) {
        match self {
            Self::Tick if pending.is_some() => {}
            Self::SliderOpened if matches!(pending, Some(Self::Minimum | Self::Maximum)) => {}
            _ => *pending = Some(self),
        }
    }
}

/// The Sound panel's sections, as in Omarchy's audio panel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SoundTab {
    #[default]
    Output,
    Input,
    Sources,
}

impl SoundTab {
    pub const ALL: [SoundTab; 3] = [SoundTab::Output, SoundTab::Input, SoundTab::Sources];

    pub fn label(self) -> &'static str {
        match self {
            SoundTab::Output => "Output",
            SoundTab::Input => "Input",
            SoundTab::Sources => "Sources",
        }
    }
}

/// Internal command bound to an interactive node.
#[derive(Clone, Debug, PartialEq)]
pub enum Cmd {
    Esc,
    FKey(u8),
    OpenMenu,
    MenuOpen(String),
    MenuRun(String),
    Back,
    Close,
    OpenPanel(Panel),
    TrayMenu(TrayMenu),
    Workspace(i32),
    Slider(Slider),
    WifiPromptOpen {
        ssid: String,
        signal: u8,
    },
    WifiPromptToggleReveal,
    WifiPromptBackspace,
    WifiPromptSubmit,
    WifiPromptCancel,
    SpeedTestStart,
    SpeedTestStop,
    ToggleWifiNetworksExpanded,
    CopyField {
        field: &'static str,
        value: String,
    },
    TriggerIndicator(Indicator),
    /// Switch the Sound panel's section.
    SoundTab(SoundTab),
    /// Open the Weather panel's location search.
    WeatherSearch,
    WeatherSearchBackspace,
    WorldTap(String),
    /// Choose a weather location (`None` = auto).
    WeatherPick(Option<Location>),
    /// Forget a Bluetooth device: the first tap asks, the second forgets.
    BluetoothForget(String),
    Do(Action),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Page {
    Bar,
    /// The keyboard-style function row (see [`Mode::Keyboard`]).
    Keyboard,
    Controls,
    FnKeys,
    Menu(String),
    Panel(Panel),
}

/// Commands from outside (control socket, keybindings, settings app).
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    OpenMenu(Option<String>),
    OpenPanel(Panel),
    /// Open the Sound panel on one of its sections.
    OpenSound(SoundTab),
    OpenSpeedTest,
    Home,
    SetMode(Mode),
    Wake,
    /// Play a scripted Touch ID prompt to preview its animation.
    TouchIdDemo(TouchIdDemo),
}

/// Scripted Touch ID prompts (`omatouchbar ctl touchid …`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TouchIdDemo {
    /// Authenticate, miss once, then succeed.
    Unlock,
    /// Enroll a finger from 0 to 100 %, then succeed.
    Enroll,
}

impl TouchIdDemo {
    /// The prompt `elapsed` seconds in, or `None` once it is over.
    pub fn at(self, elapsed: f32) -> Option<TouchId> {
        let stage = |stage| {
            Some(TouchId {
                stage,
                progress: None,
            })
        };
        match self {
            TouchIdDemo::Unlock => match elapsed {
                t if t < 3.6 => stage(TouchIdStage::Authenticate),
                t if t < 5.4 => stage(TouchIdStage::Retry),
                t if t < 8.2 => stage(TouchIdStage::Authenticate),
                t if t < 8.6 => stage(TouchIdStage::Success),
                _ => None,
            },
            TouchIdDemo::Enroll => {
                const STEPS: [u8; 12] = [8, 17, 25, 33, 42, 50, 58, 67, 75, 83, 92, 100];
                let first = 1.8;
                let every = 0.95;
                let done = first + every * STEPS.len() as f32;
                if elapsed < done {
                    let i = ((elapsed - first) / every).floor();
                    let progress = if i < 0.0 { 0 } else { STEPS[i as usize] };
                    Some(TouchId {
                        stage: TouchIdStage::Enrollment,
                        progress: Some(progress),
                    })
                } else if elapsed < done + 0.4 {
                    stage(TouchIdStage::Success)
                } else {
                    None
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct QuickSlider {
    pub slider: Slider,
    pub key: Key,
    pub touch_id: u8,
    pub rect: Rect,
    pub fraction: f32,
    pub last_x: f32,
    pub changed: bool,
    pub last_sent: Option<u8>,
}

struct Touch {
    x0: f32,
    x: f32,
    y: f32,
    key: Option<Key>,
    kind: Option<HitKind>,
    cmd: Option<Cmd>,
    cancelled: bool,
    scrolling: bool,
    scroll0: f32,
    last_x: f32,
    last_t: Instant,
    vx: f32,
    repeat_at: Option<Instant>,
    /// Last confirmed percentage since this key started repeating.
    repeat_level: Option<u8>,
    hold_at: Option<Instant>,
    swallowed: bool,
}

#[derive(Default)]
pub(crate) struct Scroll {
    pub offset: f32,
    pub velocity: f32,
    pub max: f32,
    pub x0: f32,
    pub x1: f32,
    pub dragging: bool,
}

impl Scroll {
    pub(crate) fn clip(&self) -> Option<ClipRect> {
        (self.x0 < self.x1).then_some(ClipRect::band(self.x0, self.x1))
    }

    fn step(&mut self, dt: f32) -> bool {
        if self.dragging {
            return false;
        }
        let mut moving = false;
        if self.velocity.abs() > 2.0 {
            self.offset += self.velocity * dt;
            self.velocity *= (-dt * 4.2).exp();
            moving = true;
        } else {
            self.velocity = 0.0;
        }
        let bound = if self.offset < 0.0 {
            Some(0.0)
        } else if self.offset > self.max {
            Some(self.max)
        } else {
            None
        };
        if let Some(bound) = bound {
            self.velocity *= (-dt * 18.0).exp();
            self.offset += (bound - self.offset) * (1.0 - (-dt * 16.0).exp());
            if (self.offset - bound).abs() < 0.3 {
                self.offset = bound;
                self.velocity = 0.0;
            } else {
                moving = true;
            }
        }
        moving
    }

    fn rubber(&self, raw: f32) -> f32 {
        if raw < 0.0 {
            -(-raw).powf(0.82)
        } else if raw > self.max {
            self.max + (raw - self.max).powf(0.82)
        } else {
            raw
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct GuardResult {
    pub when: Option<bool>,
    pub checked: Option<bool>,
    pub disabled: Option<bool>,
}

/// The Bluetooth panel's transient device states.
#[derive(Default)]
pub(crate) struct BtUi {
    /// Last time the strip was touched or an operation was in flight. The
    /// scan ends this long after it, however busy the desktop is (desktop
    /// activity keeps panels open, and a lingering scan makes Bluetooth
    /// audio stutter).
    pub active_at: Option<Instant>,
    /// Operations in flight, by device address, and when each started.
    pub pending: HashMap<String, (BtOp, Instant)>,
    /// Operations that didn't take, shown on the device until the instant.
    pub failed: HashMap<String, (BtOp, Instant)>,
    /// The device whose Forget was tapped once, awaiting the second tap.
    pub forget_armed: Option<(String, Instant)>,
    /// A device that just connected: its audio output becomes the default
    /// once PipeWire lists it (as Omarchy's panel does), until the instant.
    pub audio_switch: Option<(String, Instant)>,
}

pub struct Scene {
    pub(crate) width: f32,
    pub(crate) height: f32,
    pub(crate) config: Config,
    palette: Palette,
    theme_from: Theme,
    theme_to: Theme,
    theme_t: f32,
    pub(crate) theme: Theme,
    /// The reveal of a new Omarchy theme, while it runs.
    pub(crate) theme_wipe: Option<ThemeWipe>,
    pub(crate) fonts: Fonts,
    pub(crate) state: DesktopState,
    pub(crate) menu: MenuTree,
    omarchy_layout: Option<BarLayout>,
    pub(crate) layout: BarLayout,
    pub(crate) page: Page,
    pub(crate) fn_held: bool,
    pub(crate) animator: Animator,
    touches: HashMap<u8, Touch>,
    pub(crate) scroll: Scroll,
    pub(crate) tray_menu: Option<TrayMenu>,
    pub(crate) tray_entries: Option<Vec<TrayMenuEntry>>,
    pub(crate) wifi_prompt: Option<WifiPrompt>,
    pub(crate) wifi_networks_expanded: bool,
    pub(crate) sound_tab: SoundTab,
    pub(crate) weather_search: Option<WeatherSearch>,
    pub(crate) world: world_clock::Frame,
    world_session: u64,
    pub(crate) bt: BtUi,
    pub(crate) speedtest: Option<SpeedTestState>,
    pub(crate) quick_slider: Option<QuickSlider>,
    pub(crate) quick_slider_closing: Option<(Key, Instant)>,
    pub(crate) guards: HashMap<String, GuardResult>,
    pub(crate) provider_rows: HashMap<String, Vec<MenuItem>>,
    actions: Vec<Action>,
    level_feedback: Option<LevelFeedback>,
    dirty: bool,
    last_update: Option<Instant>,
    pub(crate) now: Instant,
    pub(crate) utc: DateTime<Utc>,
    pub(crate) local: NaiveDateTime,
    last_clock_text: String,
    last_agents_key: u64,
    last_touch: Instant,
    brightness: Spring,
    pub(crate) overrides: HashMap<Slider, (u8, Option<Instant>)>,
    last_slider_send: HashMap<Slider, Instant>,
    pub(crate) pulses: HashMap<&'static str, Instant>,
    pub(crate) copied_field: Option<(&'static str, Instant)>,
    pub(crate) panel_origin: Option<f32>,
    pub(crate) entered_at: Instant,
    /// The Touch ID overlay's timeline (outlives the prompt for its outro).
    pub(crate) touch_fx: Option<TouchFx>,
    /// A scripted prompt and when it started (set on the next update).
    touch_demo: Option<(TouchIdDemo, Option<Instant>)>,
    /// Omarchy's screensaver, while it runs (see `screensaver`).
    pub(crate) saver: Option<Saver>,
    animating: bool,
}

const REPEAT_DELAY: Duration = Duration::from_millis(350);
const REPEAT_EVERY: Duration = Duration::from_millis(90);
const QUICK_SLIDER_HOLD: Duration = Duration::from_millis(240);
const QUICK_SLIDER_DRAG: f32 = 10.0;
const QUICK_SLIDER_WIDTH: f32 = 580.0;
const PULSE: Duration = Duration::from_millis(1800);
const COPY_FEEDBACK: Duration = Duration::from_millis(1600);
const SLIDER_HOLD: Duration = Duration::from_millis(900);
const SLIDER_RATE: Duration = Duration::from_millis(45);
/// The Bluetooth panel stays open at least this long while it scans, so a
/// device has time to show up after it is put in pairing mode.
const BT_SCAN_IDLE: Duration = Duration::from_secs(60);
/// A Bluetooth operation nobody answered is dropped after this (pairing can
/// take two 20 s `bluetoothctl` timeouts).
const BT_PENDING: Duration = Duration::from_secs(45);
/// How long a failed operation is shown on its device.
const BT_FAILED: Duration = Duration::from_secs(4);
/// How long Forget waits for its confirming tap.
const BT_FORGET_CONFIRM: Duration = Duration::from_secs(3);
/// How long a new connection waits for its audio output to appear.
const BT_AUDIO_SWITCH: Duration = Duration::from_secs(6);

impl Scene {
    pub fn new(
        width: u32,
        height: u32,
        config: Config,
        palette: Palette,
        fonts: Fonts,
        menu: MenuTree,
        omarchy_layout: Option<BarLayout>,
    ) -> Scene {
        let theme = Theme::from_palette(&palette, &config);
        let layout = BarLayout::resolve(&config, omarchy_layout.as_ref());
        let now = Instant::now();
        let mut scene = Scene {
            width: width as f32,
            height: height as f32,
            page: base_page(config.mode),
            animator: Animator::new(feel(&config)),
            config,
            palette,
            theme_from: theme.clone(),
            theme_to: theme.clone(),
            theme_t: 1.0,
            theme,
            theme_wipe: None,
            fonts,
            state: DesktopState::default(),
            menu,
            omarchy_layout,
            layout,
            fn_held: false,
            touches: HashMap::new(),
            scroll: Scroll::default(),
            tray_menu: None,
            tray_entries: None,
            wifi_prompt: None,
            wifi_networks_expanded: false,
            sound_tab: SoundTab::Output,
            weather_search: None,
            world: Default::default(),
            world_session: 0,
            bt: BtUi::default(),
            speedtest: None,
            quick_slider: None,
            quick_slider_closing: None,
            guards: HashMap::new(),
            provider_rows: HashMap::new(),
            actions: Vec::new(),
            level_feedback: None,
            dirty: true,
            last_update: None,
            now,
            utc: Utc::now(),
            local: Local::now().naive_local(),
            last_clock_text: String::new(),
            last_agents_key: 0,
            last_touch: now,
            brightness: Spring::new(1.0),
            overrides: HashMap::new(),
            last_slider_send: HashMap::new(),
            pulses: HashMap::new(),
            copied_field: None,
            panel_origin: None,
            entered_at: now,
            touch_fx: None,
            touch_demo: None,
            saver: None,
            animating: true,
        };
        scene.animator.enter = Motion::rise(10.0);
        scene
    }

    // ---------------------------------------------------------------- inputs

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn layout(&self) -> &BarLayout {
        &self.layout
    }

    pub fn state(&self) -> &DesktopState {
        &self.state
    }

    pub fn page(&self) -> &Page {
        &self.page
    }

    pub fn node_rect(&self, key: Key) -> Option<Rect> {
        self.animator.get(key).map(|a| a.rect())
    }

    pub fn set_config(&mut self, config: Config) {
        if config == self.config {
            return;
        }
        let mode_changed = config.mode != self.config.mode;
        self.config = config;
        self.animator.set_feel(feel(&self.config));
        self.theme_to = Theme::from_palette(&self.palette, &self.config);
        self.theme_from = self.theme.clone();
        self.theme_t = 0.0;
        self.layout = BarLayout::resolve(&self.config, self.omarchy_layout.as_ref());
        if mode_changed {
            self.navigate(
                base_page(self.config.mode),
                Motion::rise(14.0),
                Motion::rise(-10.0),
            );
        }
        self.dirty = true;
    }

    pub fn palette(&self) -> &Palette {
        &self.palette
    }

    /// Switches to a new Omarchy theme, revealing it the way the desktop
    /// does (see [`ThemeWipe`]); call it when the desktop starts its reveal.
    pub fn set_palette(&mut self, palette: Palette) {
        if palette == self.palette {
            return;
        }
        self.palette = palette;
        let to = Theme::from_palette(&self.palette, &self.config);
        // The wipe starts from what is on screen. One already running gives
        // way to the new one from its own target.
        self.theme_wipe = self
            .config
            .motion
            .enabled
            .then(|| ThemeWipe::new(self.theme.clone()));
        self.theme = to.clone();
        self.theme_from = to.clone();
        self.theme_to = to;
        self.theme_t = 1.0;
        // Someone just switched themes: show them the switch.
        self.wake();
    }

    pub fn set_omarchy_layout(&mut self, layout: Option<BarLayout>) {
        if self.page == Page::Panel(Panel::Clock) {
            self.world_input(WorldInput::Reload);
        }
        self.omarchy_layout = layout;
        self.layout = BarLayout::resolve(&self.config, self.omarchy_layout.as_ref());
        self.dirty = true;
    }

    pub fn set_menu(&mut self, menu: MenuTree) {
        self.menu = menu;
        self.dirty = true;
    }

    pub fn set_fonts(&mut self, fonts: Fonts) {
        self.fonts = fonts;
        self.dirty = true;
    }

    /// Replaces the desktop state, keeping values the user is dragging.
    pub fn set_state(&mut self, mut state: DesktopState) {
        let old = &self.state;
        let now = self.now;
        let audio_changed = match (&old.audio, &state.audio) {
            (Some(a), Some(b)) => a.volume != b.volume || a.muted != b.muted,
            _ => false,
        };
        if audio_changed && !self.overrides.contains_key(&Slider::Volume) {
            self.pulses.insert("audio", now + PULSE);
        }
        if old.display_brightness.is_some()
            && old.display_brightness != state.display_brightness
            && !self.overrides.contains_key(&Slider::Display)
        {
            self.pulses.insert("brightness", now + PULSE);
        }
        if old.keyboard_backlight.is_some()
            && old.keyboard_backlight != state.keyboard_backlight
            && !self.overrides.contains_key(&Slider::Keyboard)
        {
            self.pulses.insert("keyboard", now + PULSE);
        }
        let plugged = |s: &DesktopState| {
            s.battery.as_ref().is_some_and(|b| {
                matches!(
                    b.charge,
                    crate::state::Charge::Charging | crate::state::Charge::Full
                )
            })
        };
        if old.battery.is_some() && !plugged(old) && plugged(&state) {
            self.pulses.insert("battery", now + PULSE);
        }
        // Keep the user's in-flight slider values.
        for (slider, (value, _)) in &self.overrides {
            set_slider(&mut state, *slider, *value, false);
        }
        if self.touch_demo.is_some() {
            if state.touch_id.is_some() {
                self.touch_demo = None;
            } else {
                state.touch_id = self.state.touch_id.clone();
            }
        }
        if state.touch_id.is_some() && self.state.touch_id.is_none() {
            self.finish_quick_slider();
            self.wake();
        }
        if let Some(qs) = self.quick_slider.as_mut()
            && !qs.changed
        {
            qs.fraction = slider_fraction(&state, qs.slider);
        }
        if self.tray_menu.as_ref().is_some_and(|menu| {
            !state.tray.iter().any(|item| {
                item.address == menu.address && item.menu_path.as_ref() == Some(&menu.path)
            })
        }) {
            self.tray_menu = None;
            self.tray_entries = None;
            self.scroll = Scroll::default();
        }
        if state.agents != self.state.agents
            && self
                .pulses
                .remove(crate::build::AGENTS_REFRESH_PULSE)
                .is_some()
        {
            self.dirty = true;
        }
        if state != self.state {
            self.state = state;
            self.dirty = true;
            self.sync_bluetooth();
        }
    }

    /// Settles Bluetooth operations the new state shows finished, and moves
    /// audio to a newly connected device once its output appears.
    fn sync_bluetooth(&mut self) {
        let bt = &self.state.bluetooth;
        let mut connected = Vec::new();
        self.bt.pending.retain(|address, (op, _)| {
            let done = op.done(bt, address);
            if done && matches!(op, BtOp::Pair | BtOp::Connect) {
                connected.push(address.clone());
            }
            !done
        });
        for address in connected {
            self.bt.failed.remove(&address);
            self.bt.audio_switch = Some((address, self.now + BT_AUDIO_SWITCH));
        }
        if let Some((address, until)) = &self.bt.audio_switch {
            if self.now >= *until {
                self.bt.audio_switch = None;
            } else if let Some(output) = self
                .state
                .mixer
                .outputs
                .iter()
                .find(|d| bluez_node_matches(&d.name, address))
            {
                if !output.default {
                    let action = Action::SetDefaultOutput {
                        id: output.id,
                        name: output.name.clone(),
                    };
                    self.optimistic(&action);
                    self.actions.push(action);
                }
                self.bt.audio_switch = None;
            }
        }
    }

    /// The host finished a Bluetooth operation: `op` and `ok` as
    /// [`BtOp::outcome`] reports them.
    pub fn apply_bluetooth_done(&mut self, address: &str, op: BtOp, ok: bool) {
        // A device takes no new operation while one is in flight, so
        // whatever is pending for it is what finished.
        self.bt.pending.remove(address);
        if ok {
            if matches!(op, BtOp::Pair | BtOp::Connect) && self.bt.audio_switch.is_none() {
                self.bt.audio_switch = Some((address.to_string(), self.now + BT_AUDIO_SWITCH));
                self.sync_bluetooth();
            }
        } else {
            self.bt
                .failed
                .insert(address.to_string(), (op, self.now + BT_FAILED));
        }
        self.dirty = true;
    }

    /// Whether the host should keep Bluetooth discovery running.
    pub fn wants_bluetooth_scan(&self) -> bool {
        self.page == Page::Panel(Panel::Bluetooth)
            && self.state.bluetooth.powered
            && self.touch_fx.is_none()
    }

    fn bluetooth_scanning_panel(&self) -> bool {
        self.page == Page::Panel(Panel::Bluetooth) && self.state.bluetooth.powered
    }

    pub fn apply_tray_menu(&mut self, menu: &TrayMenu, entries: Vec<TrayMenuEntry>) {
        if self.tray_menu.as_ref() == Some(menu) {
            self.tray_entries = Some(entries);
            self.dirty = true;
        }
    }

    pub fn state_mut(&mut self) -> &mut DesktopState {
        self.dirty = true;
        &mut self.state
    }

    pub fn apply_guards(&mut self, menu_id: &str, results: Vec<(String, GuardKind, bool)>) {
        let _ = menu_id;
        for (id, kind, value) in results {
            let entry = self.guards.entry(id).or_default();
            match kind {
                GuardKind::When => entry.when = Some(value),
                GuardKind::Checked => entry.checked = Some(value),
                GuardKind::Disabled => entry.disabled = Some(value),
            }
        }
        self.dirty = true;
    }

    /// Provider rows: `(label, value, current)`.
    pub fn apply_provider(
        &mut self,
        menu_id: &str,
        provider: &str,
        rows: Vec<(String, String, bool)>,
    ) {
        let prefix = menu::provider_script(provider)
            .map(|(_, p)| p)
            .unwrap_or("");
        let icon = self
            .menu
            .get(menu_id)
            .map(|m| m.icon.clone())
            .unwrap_or_default();
        let items = rows
            .into_iter()
            .enumerate()
            .map(|(i, (label, value, current))| MenuItem {
                id: format!("{menu_id}.~{i}"),
                icon: icon.clone(),
                label,
                action: Some(format!("{prefix} {}", menu::shell_quote(&value))),
                current,
                ..MenuItem::default()
            })
            .collect();
        self.provider_rows.insert(menu_id.to_string(), items);
        self.dirty = true;
    }

    pub fn command(&mut self, command: Command) {
        self.wake();
        match command {
            Command::OpenMenu(id) => {
                if self.config.mode.opens_panels() {
                    self.open_menu(id.unwrap_or_default(), true);
                }
            }
            Command::OpenPanel(panel) => {
                if self.config.mode.opens_panels() {
                    self.open_panel(panel);
                }
            }
            Command::OpenSound(tab) => {
                if self.config.mode.opens_panels() {
                    self.open_panel(Panel::Audio);
                    self.exec(Cmd::SoundTab(tab));
                }
            }
            Command::OpenSpeedTest => {
                if self.config.mode.opens_panels() {
                    self.open_panel(Panel::Network);
                    self.start_speedtest();
                }
            }
            Command::Home => self.go_home(),
            Command::SetMode(mode) => {
                let mut config = self.config.clone();
                config.mode = mode;
                self.set_config(config);
            }
            Command::Wake => {}
            Command::TouchIdDemo(demo) => {
                if self.touch_demo.is_some() || self.state.touch_id.is_none() {
                    self.touch_demo = Some((demo, None));
                    // Restart the effect from the top.
                    self.touch_fx = None;
                    self.state.touch_id = None;
                    self.dirty = true;
                }
            }
        }
    }

    /// Whether the Touch Bar currently wants exclusive physical keyboard input
    /// (for typing a Wi-Fi password without leaking keystrokes to the desktop).
    pub fn wants_keyboard(&self) -> bool {
        if self.state.touch_id.is_some() {
            return false;
        }
        match self.shown_page() {
            Page::Panel(Panel::Network) => self.wifi_prompt.as_ref().is_some_and(|p| {
                matches!(
                    p.phase,
                    WifiPromptPhase::Editing
                        | WifiPromptPhase::Connecting
                        | WifiPromptPhase::Failed(_)
                )
            }),
            Page::Panel(Panel::Weather) => self.weather_search.is_some(),
            Page::Panel(Panel::Clock) => self.world.editing,
            _ => false,
        }
    }

    fn world_input(&mut self, command: WorldInput) {
        self.actions.push(Action::WorldClock(world_clock::Request {
            session: self.world_session,
            command,
        }));
    }

    pub fn apply_world_clock(&mut self, frame: world_clock::Frame) {
        if self.page != Page::Panel(Panel::Clock) || frame.session != self.world_session {
            return;
        }
        if self.world.view != frame.view {
            self.scroll = Scroll::default();
        }
        self.world = frame;
        self.dirty = true;
    }

    /// Opens the Weather panel's location search (on tap, and for snapshots).
    pub fn open_weather_search(&mut self) {
        if !matches!(self.page, Page::Panel(Panel::Weather)) {
            self.open_panel(Panel::Weather);
        }
        self.last_touch = self.now;
        self.weather_search = Some(WeatherSearch {
            query: String::new(),
            last_edit: self.now,
            results: Vec::new(),
            results_for: String::new(),
            searching: false,
            failed: false,
        });
        self.animator.enter = Motion::slide(40.0);
        self.animator.exit = Motion::slide(-40.0);
        self.scroll = Scroll::default();
        self.dirty = true;
    }

    fn close_weather_search(&mut self) {
        if self.weather_search.take().is_some() {
            self.animator.enter = Motion::slide(-40.0);
            self.animator.exit = Motion::slide(40.0);
            self.scroll = Scroll::default();
            self.dirty = true;
        }
    }

    /// Places found for `query` (`None`: the lookup failed).
    pub fn apply_weather_suggestions(&mut self, query: &str, results: Option<Vec<Location>>) {
        let Some(search) = self.weather_search.as_mut() else {
            return;
        };
        if search.query.trim() != query {
            return;
        }
        search.searching = false;
        search.failed = results.is_none();
        search.results = results.unwrap_or_default();
        search.results_for = query.to_string();
        self.scroll = Scroll::default();
        self.dirty = true;
    }

    fn weather_search_edited(&mut self) {
        let Some(search) = self.weather_search.as_mut() else {
            return;
        };
        let query = search.query.trim().to_string();
        search.last_edit = self.now;
        search.failed = false;
        if query.chars().count() >= WEATHER_SEARCH_MIN {
            search.searching = query != search.results_for;
            if search.searching {
                self.actions.push(Action::SearchWeatherLocation(query));
            }
        } else {
            search.searching = false;
            search.results.clear();
            search.results_for.clear();
        }
        self.dirty = true;
    }

    /// Enter in the search: the first suggestion, a typed name as-is, or
    /// auto when nothing is typed (Omarchy's `locationCommit`).
    fn commit_weather_search(&mut self) {
        let Some(search) = self.weather_search.as_ref() else {
            return;
        };
        let query = search.query.trim();
        let pick = if query.is_empty() {
            None
        } else if query.chars().count() < WEATHER_SEARCH_MIN {
            return;
        } else if let Some(first) = search.current_results().first() {
            Some(first.clone())
        } else if search.searching {
            // Wait for the suggestions rather than store a bare name.
            return;
        } else {
            Some(Location {
                name: query.to_string(),
                ..Location::default()
            })
        };
        self.pick_weather_location(pick);
    }

    fn pick_weather_location(&mut self, location: Option<Location>) {
        self.close_weather_search();
        if let Some(w) = self.state.weather.as_mut() {
            w.place = crate::weather::Place {
                name: match &location {
                    Some(l) => l.name.clone(),
                    None => w.detected.clone().unwrap_or_default(),
                },
                auto: location.is_none(),
            };
            w.loading = true;
        }
        self.actions.push(Action::SetWeatherLocation(location));
        self.dirty = true;
    }

    /// Whether the Network panel is open and showing live connection telemetry
    /// (ping, throughput, IP/gateway, band and DNS status).
    pub fn wants_network_details(&self) -> bool {
        matches!(self.shown_page(), Page::Panel(Panel::Network))
            && self.speedtest.is_none()
            && self.wifi_prompt.is_none()
    }

    pub fn wants_display_panel(&self) -> bool {
        matches!(self.shown_page(), Page::Panel(Panel::Brightness)) && self.state.touch_id.is_none()
    }

    /// Sets the horizontal scroll position of the active scrollable row (used by snapshots/tests).
    pub fn set_scroll_offset(&mut self, offset: f32) {
        self.scroll.offset = offset.clamp(0.0, self.scroll.max);
        self.scroll.velocity = 0.0;
        self.dirty = true;
    }

    /// Sets whether all unknown Wi-Fi networks are expanded inline (used by snapshots/tests).
    pub fn set_wifi_networks_expanded(&mut self, expanded: bool) {
        self.wifi_networks_expanded = expanded;
        self.dirty = true;
    }

    pub(crate) fn is_copied(&self, field: &str) -> bool {
        self.copied_field
            .is_some_and(|(f, until)| f == field && self.now < until)
    }

    /// Opens the in-Touch-Bar Wi-Fi password prompt directly (used on tap,
    /// auth failure on a saved profile, and snapshots).
    pub fn open_wifi_prompt(&mut self, ssid: String, signal: u8, revealed: bool) {
        self.stop_speedtest_if_running();
        self.speedtest = None;
        if !matches!(self.page, Page::Panel(Panel::Network)) {
            self.open_panel(Panel::Network);
        }
        let now = self.now;
        self.last_touch = now;
        self.wifi_prompt = Some(WifiPrompt {
            ssid,
            signal,
            password: String::new(),
            revealed,
            phase: WifiPromptPhase::Editing,
            opened_at: now,
            last_edit: now,
            connected_at: None,
        });
        self.scroll = Scroll::default();
        self.dirty = true;
    }

    /// Delivers a physical keyboard event to the active Touch Bar prompt.
    pub fn key_input(&mut self, input: KeyInput, now: Instant) {
        self.now = now;
        self.last_touch = now;
        if self.page == Page::Panel(Panel::Clock) && self.world.editing {
            let command = match input {
                KeyInput::Text(text) | KeyInput::Paste(text) => WorldInput::Text(text),
                KeyInput::Escape => WorldInput::Back,
                KeyInput::Enter => WorldInput::Enter,
                KeyInput::Select(delta) => WorldInput::Select(delta),
                KeyInput::Backspace => WorldInput::Backspace,
                KeyInput::Clear => WorldInput::Clear,
            };
            self.world_input(command);
            return;
        }
        if let Some(search) = self.weather_search.as_mut() {
            match input {
                KeyInput::Select(_) => {}
                KeyInput::Escape => self.close_weather_search(),
                KeyInput::Enter => self.commit_weather_search(),
                KeyInput::Text(text) | KeyInput::Paste(text) => {
                    let room = 64usize.saturating_sub(search.query.chars().count());
                    let clean: String = text
                        .chars()
                        .map(|c| if c.is_whitespace() { ' ' } else { c })
                        .filter(|c| !c.is_control())
                        .take(room)
                        .collect();
                    if !clean.is_empty() {
                        search.query.push_str(&clean);
                        self.weather_search_edited();
                    }
                }
                KeyInput::Backspace => {
                    if search.query.pop().is_some() {
                        self.weather_search_edited();
                    }
                }
                KeyInput::Clear => {
                    if !search.query.is_empty() {
                        search.query.clear();
                        self.weather_search_edited();
                    }
                }
            }
            return;
        }
        let Some(prompt) = self.wifi_prompt.as_mut() else {
            return;
        };
        if matches!(input, KeyInput::Escape) {
            self.cancel_wifi_prompt();
            return;
        }
        if !matches!(
            prompt.phase,
            WifiPromptPhase::Editing | WifiPromptPhase::Failed(_)
        ) {
            return;
        }
        match input {
            KeyInput::Text(text) | KeyInput::Paste(text) => {
                let clean: String = text
                    .chars()
                    .filter(|c| !c.is_control() && *c != '\u{7f}')
                    .collect();
                if !clean.is_empty() {
                    let count = prompt.password.chars().count();
                    let room = 128usize.saturating_sub(count);
                    if room > 0 {
                        prompt.password.extend(clean.chars().take(room));
                        prompt.phase = WifiPromptPhase::Editing;
                        prompt.last_edit = now;
                        self.dirty = true;
                    }
                }
            }
            KeyInput::Backspace => {
                if prompt.password.pop().is_some()
                    || matches!(prompt.phase, WifiPromptPhase::Failed(_))
                {
                    prompt.phase = WifiPromptPhase::Editing;
                    prompt.last_edit = now;
                    self.dirty = true;
                }
            }
            KeyInput::Clear => {
                if !prompt.password.is_empty() || matches!(prompt.phase, WifiPromptPhase::Failed(_))
                {
                    prompt.password.clear();
                    prompt.phase = WifiPromptPhase::Editing;
                    prompt.last_edit = now;
                    self.dirty = true;
                }
            }
            KeyInput::Enter => {
                self.submit_wifi_prompt();
            }
            KeyInput::Select(_) | KeyInput::Escape => {}
        }
    }

    /// Reports the outcome of a Wi-Fi connection attempt from the daemon.
    pub fn apply_wifi_connect_result(
        &mut self,
        ssid: &str,
        result: Result<(), String>,
        auth_failed: bool,
    ) {
        let now = self.now;
        if let Some(prompt) = self.wifi_prompt.as_mut().filter(|p| p.ssid == ssid) {
            self.last_touch = now;
            match result {
                Ok(()) => {
                    prompt.phase = WifiPromptPhase::Connected;
                    prompt.connected_at = Some(now);
                    prompt.password.clear();
                }
                Err(msg) => {
                    prompt.phase = WifiPromptPhase::Failed(msg);
                    prompt.last_edit = now;
                }
            }
            self.dirty = true;
            return;
        }
        // A saved/known network failed auth — pop open the prompt so the user
        // can immediately enter the updated password on the Touch Bar.
        if auth_failed
            && let Err(msg) = result
            && matches!(self.page, Page::Panel(Panel::Network))
        {
            let signal = self
                .state
                .network
                .networks
                .iter()
                .find(|n| n.ssid == ssid)
                .map(|n| n.signal)
                .unwrap_or(60);
            self.open_wifi_prompt(ssid.to_string(), signal, false);
            if let Some(prompt) = self.wifi_prompt.as_mut() {
                prompt.phase = WifiPromptPhase::Failed(msg);
            }
        }
    }

    /// Feeds a live speed test event from the daemon or preview simulator.
    pub fn apply_speedtest_event(&mut self, event: SpeedTestEvent) {
        let Some(st) = self.speedtest.as_mut() else {
            return;
        };
        let was_running = st.is_running();
        st.apply(event);
        if was_running && !st.is_running() {
            // Reset idle clock when results land so the user has full time to read them.
            self.last_touch = self.now;
        }
        self.dirty = true;
    }

    /// Starts (or restarts) the full-strip Touch Bar speed test.
    pub fn start_speedtest(&mut self) {
        self.cancel_wifi_connect_only();
        self.wifi_prompt = None;
        let conn = match self.state.network.kind {
            NetKind::Wifi if !self.state.network.name.is_empty() => self.state.network.name.clone(),
            NetKind::Wifi => "Wi-Fi".into(),
            NetKind::Ethernet => {
                if self.state.network.name.is_empty() {
                    "Ethernet".into()
                } else {
                    self.state.network.name.clone()
                }
            }
            _ => "Network".into(),
        };
        self.speedtest = Some(SpeedTestState::new(conn));
        self.scroll = Scroll::default();
        self.last_touch = self.now;
        self.actions.push(Action::StartSpeedTest);
        self.dirty = true;
    }

    fn stop_speedtest_if_running(&mut self) {
        if self.speedtest.as_ref().is_some_and(|s| s.is_running()) {
            self.actions.push(Action::StopSpeedTest);
        }
    }

    fn close_speedtest(&mut self) {
        self.stop_speedtest_if_running();
        if self.speedtest.take().is_some() {
            self.scroll = Scroll::default();
            self.dirty = true;
        }
    }

    fn cancel_wifi_connect_only(&mut self) {
        if self
            .wifi_prompt
            .as_ref()
            .is_some_and(|p| matches!(p.phase, WifiPromptPhase::Connecting))
        {
            self.actions.push(Action::CancelWifiConnect);
        }
    }

    fn cancel_wifi_prompt(&mut self) {
        self.cancel_wifi_connect_only();
        if self.wifi_prompt.take().is_some() {
            self.scroll = Scroll::default();
            self.dirty = true;
        }
    }

    fn submit_wifi_prompt(&mut self) {
        let Some(prompt) = self.wifi_prompt.as_mut() else {
            return;
        };
        if prompt.password.is_empty()
            || matches!(
                prompt.phase,
                WifiPromptPhase::Connecting | WifiPromptPhase::Connected
            )
        {
            return;
        }
        let ssid = prompt.ssid.clone();
        let password = prompt.password.clone();
        prompt.phase = WifiPromptPhase::Connecting;
        prompt.opened_at = self.now;
        self.last_touch = self.now;
        self.actions
            .push(Action::ConnectWifiPassword { ssid, password });
        self.dirty = true;
    }

    /// Keeps the strip awake (e.g. on desktop activity).
    pub fn wake(&mut self) {
        self.last_touch = Instant::now().max(self.last_touch);
        self.dirty = true;
    }

    pub fn take_actions(&mut self) -> Vec<Action> {
        std::mem::take(&mut self.actions)
    }

    pub fn take_level_feedback(&mut self) -> Option<LevelFeedback> {
        self.level_feedback.take()
    }

    // ----------------------------------------------------------- screensaver

    /// Omarchy's screensaver started: the strip becomes its canvas, divided
    /// as `geometry`, and shows the frames [`Scene::screensaver_frame`]
    /// hands it until [`Scene::screensaver_stop`] or a touch. Touch ID
    /// prompts still take over.
    pub fn screensaver_start(&mut self, geometry: Geometry) {
        if self.saver.as_ref().is_some_and(|s| s.geometry == geometry) {
            return;
        }
        self.finish_quick_slider();
        self.saver = Some(Saver::new(geometry));
        self.dirty = true;
    }

    /// The effect's latest frame (ignored unless the screensaver is up).
    pub fn screensaver_frame(&mut self, grid: Arc<Grid>) {
        if let Some(saver) = self.saver.as_mut() {
            saver.frame = Some(grid);
            saver.fresh = true;
        }
    }

    /// The screensaver ended on the desktop: someone is back.
    pub fn screensaver_stop(&mut self) {
        if self.saver.take().is_some() {
            self.wake();
            self.last_touch = self.last_touch.max(self.now);
        }
    }

    pub fn screensaver_active(&self) -> bool {
        self.saver.is_some()
    }

    /// Whether the screensaver owns the strip right now.
    pub(crate) fn shows_screensaver(&self) -> bool {
        self.saver.is_some() && self.touch_fx.is_none()
    }

    /// Glyphs in the font cache (diagnostics).
    pub fn cached_glyphs(&self) -> usize {
        self.fonts.cached_glyphs()
    }

    /// Current output brightness (idle dimming), 0–1.
    pub fn brightness(&self) -> f32 {
        self.brightness.value.clamp(0.0, 1.0)
    }

    // ---------------------------------------------------------------- touch

    /// Cancel a lost input source without firing release actions on its behalf.
    pub fn cancel_input(&mut self, now: Instant) {
        self.now = now;
        if self
            .touches
            .values()
            .any(|touch| touch.kind == Some(HitKind::WorldDrag))
        {
            self.world_input(WorldInput::CancelDrag);
        }
        for (_, touch) in self.touches.drain() {
            if let Some(key) = touch.key {
                self.animator.press(key, false, None, false);
            }
        }
        self.scroll.dragging = false;
        self.finish_quick_slider();
        for (_, release) in self.overrides.values_mut() {
            release.get_or_insert(now + SLIDER_HOLD);
        }
        self.input(false, &[], now);
        self.dirty = true;
    }

    /// Feeds one input frame: every contact currently touching, in display
    /// coordinates. Returns true when a new contact presses a button.
    pub fn input(&mut self, fn_pressed: bool, contacts: &[(u8, f32, f32)], now: Instant) -> bool {
        self.input_press_position(fn_pressed, contacts, now)
            .is_some()
    }

    /// Processes a frame and returns the first new button press's horizontal
    /// position, from 0 (left edge) to 1 (right edge). A single actuator can
    /// play only one click when several fingers land in the same frame.
    pub fn input_press_position(
        &mut self,
        fn_pressed: bool,
        contacts: &[(u8, f32, f32)],
        now: Instant,
    ) -> Option<f32> {
        self.now = now;
        if !contacts.is_empty() {
            self.bt.active_at = Some(now);
        }
        if fn_pressed != self.fn_held {
            self.fn_held = fn_pressed;
            self.last_touch = now;
            if self.config.fn_key != FnKey::None {
                self.finish_quick_slider();
                if fn_pressed {
                    self.animator.enter = Motion::rise(16.0);
                    self.animator.exit = Motion::rise(-12.0);
                } else {
                    self.animator.enter = Motion::rise(-14.0);
                    self.animator.exit = Motion::rise(12.0);
                }
            }
            self.dirty = true;
        }
        let ended: Vec<u8> = self
            .touches
            .keys()
            .copied()
            .filter(|id| !contacts.iter().any(|c| c.0 == *id))
            .collect();
        for id in ended {
            self.touch_up(id, now);
        }
        let mut press_position = None;
        for &(id, x, y) in contacts {
            if self.touches.contains_key(&id) {
                self.touch_move(id, x, y, now);
            } else if self.touch_down(id, x, y, now) && press_position.is_none() {
                press_position = Some((x / self.width).clamp(0.0, 1.0));
            }
        }
        press_position
    }

    fn group_offset(&self, group: Option<u8>) -> f32 {
        match group {
            Some(0) => self.scroll.offset,
            _ => 0.0,
        }
    }

    fn hit_test(&self, x: f32, y: f32, pad: f32) -> Option<Key> {
        self.animator
            .hit_test_visible(x, y, pad, |a| self.touch_rect(a, x))
    }

    /// Visible input targets, in the same pixel coordinates as a hardware frame.
    pub fn controls(&self) -> serde_json::Value {
        use crate::node::Part;
        let controls: Vec<_> = self
            .animator
            .items
            .iter()
            .filter_map(|item| {
                let kind = item.node.hit.as_ref()?.kind;
                let bounds = item.rect().offset(-self.group_offset(item.node.group));
                let mut left = bounds.x.max(0.0);
                let mut right = bounds.right().min(self.width);
                if item.node.group == Some(0)
                    && let Some(row) = self.scroll.clip()
                {
                    left = left.max(row.x0);
                    right = right.min(row.x1);
                }
                let top = bounds.y.max(0.0);
                let bottom = (bounds.y + bounds.h).min(self.height);
                if right <= left || bottom <= top {
                    return None;
                }
                let x = ((left + right) * 0.5).floor();
                let y = ((top + bottom) * 0.5).floor();
                if self.hit_test(x, y, 0.0) != Some(item.node.key) {
                    return None;
                }
                let label = item
                    .node
                    .parts
                    .iter()
                    .filter_map(|part| match part {
                        Part::Text { text, .. } | Part::SplitText { text, .. } => {
                            Some(text.as_str())
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                let kind = match kind {
                    HitKind::WorldDrag => "drag",
                    HitKind::Tap => "tap",
                    HitKind::TapSlide(_) => "tap-slide",
                    HitKind::Down => "press",
                    HitKind::Repeat => "repeat",
                    HitKind::Slider => "slider",
                    HitKind::Scrub(_) => "scrub",
                };
                Some(
                    serde_json::json!({"id": format!("{:016x}", item.node.key.0), "label": label,
                "kind": kind, "x": x as u32, "y": y as u32, "bounds": [left, top, right, bottom]}),
                )
            })
            .collect();
        serde_json::json!({"width": self.width as u32, "height": self.height as u32, "controls": controls})
    }

    fn touch_rect(&self, a: &Animated, x: f32) -> Option<Rect> {
        let rect = a.rect().offset(-self.group_offset(a.node.group));
        if a.node.group == Some(0)
            && let Some(row) = self.scroll.clip()
        {
            // Items scrolled behind pinned controls cannot receive their taps.
            if !(row.x0..row.x1).contains(&x) {
                return None;
            }
            let left = rect.x.max(row.x0);
            let right = rect.right().min(row.x1);
            return (left < right).then(|| Rect::new(left, rect.y, right - left, rect.h));
        }
        Some(rect)
    }

    fn still_over(&self, key: Key, x: f32, y: f32) -> bool {
        self.animator
            .get(key)
            .filter(|a| a.node.hit.is_some())
            .and_then(|a| self.touch_rect(a, x))
            .is_some_and(|rect| rect.contains(x, y, 28.0))
    }

    fn touch_down(&mut self, id: u8, x: f32, y: f32, now: Instant) -> bool {
        let was_off = self.brightness.value < 0.08 && self.brightness.target < 0.08;
        self.last_touch = now;
        self.dirty = true;
        let mut touch = Touch {
            x0: x,
            x,
            y,
            key: None,
            kind: None,
            cmd: None,
            cancelled: false,
            scrolling: false,
            scroll0: self.scroll.offset,
            last_x: x,
            last_t: now,
            vx: 0.0,
            repeat_at: None,
            repeat_level: None,
            hold_at: None,
            swallowed: was_off,
        };
        // A touch ends the screensaver (on the desktop too) and goes no
        // further, like a key press on the desktop.
        if self.shows_screensaver() {
            touch.swallowed = true;
            self.saver = None;
            self.actions.push(Action::DismissScreensaver);
        }
        if touch.swallowed {
            self.touches.insert(id, touch);
            return false;
        }
        // Catch a flinging row.
        if self.scroll.velocity.abs() > 40.0 && x >= self.scroll.x0 && x <= self.scroll.x1 {
            self.scroll.velocity = 0.0;
            self.scroll.dragging = true;
            touch.scrolling = true;
            touch.scroll0 = self.scroll.offset;
            self.touches.insert(id, touch);
            return false;
        }
        let key = self.hit_test(x, y, 10.0);
        if let Some(key) = key {
            let (hit, ripple) = match self.animator.get(key) {
                Some(a) => (
                    a.node.hit.clone(),
                    a.node.ripple && self.config.motion.ripple,
                ),
                None => (None, false),
            };
            if let Some(hit) = hit {
                let grouped = self.animator.get(key).and_then(|a| a.node.group).is_some();
                let offset = self.group_offset(self.animator.get(key).and_then(|a| a.node.group));
                self.animator
                    .press(key, true, Some((x + offset, y)), ripple);
                touch.key = Some(key);
                touch.kind = Some(hit.kind);
                touch.cmd = Some(hit.cmd.clone());
                match hit.kind {
                    HitKind::Down | HitKind::Scrub(_) => self.exec(hit.cmd),
                    HitKind::Repeat => {
                        self.exec(hit.cmd);
                        touch.repeat_at = Some(now + REPEAT_DELAY);
                    }
                    HitKind::Slider => {
                        if let Cmd::Slider(slider) = hit.cmd {
                            self.slide(slider, key, x, now, false);
                        }
                    }
                    HitKind::TapSlide(_) if grouped => {
                        touch.kind = Some(HitKind::Tap);
                    }
                    HitKind::TapSlide(_) => {
                        touch.hold_at = Some(now + QUICK_SLIDER_HOLD);
                    }
                    HitKind::WorldDrag => self.drag_world(&touch, false),
                    HitKind::Tap => {}
                }
            }
        }
        let button_pressed = matches!(
            touch.kind,
            Some(
                HitKind::Down
                    | HitKind::Repeat
                    | HitKind::Tap
                    | HitKind::TapSlide(_)
                    | HitKind::Scrub(_)
            )
        );
        self.touches.insert(id, touch);
        button_pressed
    }

    fn touch_move(&mut self, id: u8, x: f32, y: f32, now: Instant) {
        let Some(mut touch) = self.touches.remove(&id) else {
            return;
        };
        let dt = now.saturating_duration_since(touch.last_t).as_secs_f32();
        if dt > 0.0 {
            let instant = (x - touch.last_x) / dt;
            touch.vx = touch.vx * 0.6 + instant * 0.4;
        }
        touch.last_x = x;
        touch.last_t = now;
        touch.x = x;
        touch.y = y;
        if touch.swallowed {
            self.touches.insert(id, touch);
            return;
        }
        self.last_touch = now;
        if self
            .quick_slider
            .as_ref()
            .is_some_and(|qs| qs.touch_id == id)
        {
            self.move_quick_slider(x, now, false);
            self.touches.insert(id, touch);
            return;
        }
        let in_row =
            touch.x0 >= self.scroll.x0 && touch.x0 <= self.scroll.x1 && self.scroll.max > 0.5;
        let grouped = touch
            .key
            .and_then(|k| self.animator.get(k))
            .and_then(|a| a.node.group)
            .is_some();
        match touch.kind {
            Some(HitKind::WorldDrag) => self.drag_world(&touch, false),
            Some(HitKind::TapSlide(slider)) => {
                if !touch.cancelled {
                    if (x - touch.x0).abs() >= QUICK_SLIDER_DRAG {
                        touch.hold_at = None;
                        touch.cancelled = true;
                        if let Some(key) = touch.key {
                            self.activate_quick_slider(id, slider, key, touch.x0, Some(x), now);
                        }
                    } else if let Some(key) = touch.key
                        && !self.still_over(key, x, y)
                    {
                        touch.cancelled = true;
                        touch.hold_at = None;
                        self.animator.press(key, false, None, false);
                    }
                }
            }
            Some(HitKind::Slider) => {
                if let (Some(Cmd::Slider(slider)), Some(key)) = (touch.cmd.clone(), touch.key) {
                    self.slide(slider, key, x, now, false);
                }
            }
            Some(HitKind::Scrub(_)) => {
                if let Some(key) = self.hit_test(x, y.clamp(1.0, self.height - 1.0), 6.0)
                    && Some(key) != touch.key
                    && let Some(hit) = self.animator.get(key).and_then(|a| a.node.hit.clone())
                    && let HitKind::Scrub(cue) = hit.kind
                {
                    if let Some(old) = touch.key {
                        self.animator.press(old, false, None, false);
                    }
                    self.animator.press(key, true, None, false);
                    touch.key = Some(key);
                    touch.cmd = Some(hit.cmd.clone());
                    self.exec(hit.cmd);
                    cue.queue(&mut self.level_feedback);
                }
            }
            _ => {
                if touch.scrolling {
                    self.scroll.offset = self.scroll.rubber(touch.scroll0 - (x - touch.x0));
                    self.dirty = true;
                } else if (in_row && (grouped || touch.key.is_none()))
                    && (x - touch.x0).abs() > 12.0
                {
                    touch.scrolling = true;
                    touch.cancelled = true;
                    touch.repeat_at = None;
                    touch.scroll0 = self.scroll.offset + (x - touch.x0);
                    self.scroll.dragging = true;
                    self.scroll.velocity = 0.0;
                    if let Some(key) = touch.key {
                        self.animator.press(key, false, None, false);
                    }
                    self.scroll.offset = self.scroll.rubber(touch.scroll0 - (x - touch.x0));
                } else if let Some(key) = touch.key
                    && !touch.cancelled
                    && !self.still_over(key, x, y)
                {
                    touch.cancelled = true;
                    touch.repeat_at = None;
                    self.animator.press(key, false, None, false);
                }
            }
        }
        self.touches.insert(id, touch);
    }

    fn touch_up(&mut self, id: u8, now: Instant) {
        let Some(touch) = self.touches.remove(&id) else {
            return;
        };
        self.dirty = true;
        if touch.swallowed {
            return;
        }
        self.last_touch = now;
        if self
            .quick_slider
            .as_ref()
            .is_some_and(|qs| qs.touch_id == id)
        {
            self.move_quick_slider(touch.x, now, true);
            self.finish_quick_slider();
            return;
        }
        if let Some(key) = touch.key {
            self.animator.press(key, false, None, false);
        }
        if touch.scrolling {
            self.scroll.dragging = false;
            self.scroll.velocity = (-touch.vx).clamp(-7000.0, 7000.0);
            if now.saturating_duration_since(touch.last_t) > Duration::from_millis(80) {
                self.scroll.velocity = 0.0;
            }
            return;
        }
        if touch.kind == Some(HitKind::WorldDrag) {
            self.drag_world(&touch, true);
            return;
        }
        match (touch.kind, touch.cmd) {
            (Some(HitKind::Tap | HitKind::TapSlide(_)), Some(cmd)) if !touch.cancelled => {
                if touch
                    .key
                    .is_some_and(|key| self.still_over(key, touch.x, touch.y))
                {
                    self.exec(cmd);
                }
            }
            (Some(HitKind::Slider), Some(Cmd::Slider(slider))) => {
                if let Some(key) = touch.key {
                    self.slide(slider, key, touch.x, now, true);
                }
            }
            _ => {}
        }
    }

    fn drag_world(&mut self, touch: &Touch, released: bool) {
        if let Some(Cmd::WorldTap(id)) = &touch.cmd
            && let Some(node) = touch.key.and_then(|k| self.animator.get(k))
        {
            let rect = node.rect().offset(-self.group_offset(node.node.group));
            self.world_input(WorldInput::Drag {
                id: id.clone(),
                fraction: ((touch.x - rect.x) / rect.w).clamp(0.0, 1.0),
                released,
            });
        }
    }

    fn slide(&mut self, slider: Slider, key: Key, x: f32, now: Instant, last: bool) {
        let Some(a) = self.animator.get(key) else {
            return;
        };
        // Sliders in a scrolling row sit where the row has scrolled them.
        let rect = a.rect().offset(-self.group_offset(a.node.group));
        let inset = 6.0;
        let fraction = ((x - rect.x - inset) / (rect.w - inset * 2.0)).clamp(0.0, 1.0);
        if let Slider::Monitor(target) = slider
            && monitor_target(&self.state, target).is_none()
        {
            self.overrides.remove(&slider);
            return;
        }
        let value = ((fraction * 100.0).round() as u8).max(slider.minimum_percent());
        self.preview_slider_level(slider, value, now, last);
        let due = self
            .last_slider_send
            .get(&slider)
            .is_none_or(|t| now.saturating_duration_since(*t) >= SLIDER_RATE);
        if due || last {
            self.last_slider_send.insert(slider, now);
            self.actions
                .extend(slider_action(&self.state, slider, value));
        }
        self.dirty = true;
    }

    fn activate_quick_slider(
        &mut self,
        touch_id: u8,
        slider: Slider,
        key: Key,
        anchor_x: f32,
        drag_to: Option<f32>,
        now: Instant,
    ) {
        self.animator.press(key, false, None, false);
        self.animator.clear_ripples(key);
        if self.quick_slider.is_some()
            || !matches!(self.shown_page(), Page::Bar)
            || self.state.touch_id.is_some()
        {
            return;
        }
        let v0 = slider_fraction(&self.state, slider);
        let m = self.m();
        let min_x = if self.config.show_escape {
            m.edge + m.esc + m.gap * 1.5
        } else {
            m.edge
        };
        let max_x = self.width - m.edge;
        let w = QUICK_SLIDER_WIDTH.min((max_x - min_x).max(160.0));
        let inset = 6.0;
        let track_w = (w - inset * 2.0).max(1.0);
        let ideal_x = anchor_x - inset - v0 * track_w;
        let x = ideal_x.clamp(min_x, (max_x - w).max(min_x)).round();
        let rect = Rect::new(x, m.inset, w, m.bh);

        match slider {
            Slider::Volume => {
                self.pulses.remove("audio");
            }
            Slider::Display => {
                self.pulses.remove("brightness");
            }
            Slider::Monitor(_) | Slider::Keyboard | Slider::Mic | Slider::Stream(_) => {}
        }
        self.animator.exit = Motion {
            dx: 0.0,
            dy: 0.0,
            scale: 0.9,
            delay: 0.0,
        };
        let track_l = rect.x + inset;
        let track_r = rect.right() - inset;
        self.quick_slider = Some(QuickSlider {
            slider,
            key,
            touch_id,
            rect,
            fraction: v0,
            last_x: anchor_x.clamp(track_l, track_r),
            changed: false,
            last_sent: None,
        });
        LevelFeedback::SliderOpened.queue(&mut self.level_feedback);
        self.dirty = true;
        if let Some(x) = drag_to {
            self.move_quick_slider(x, now, false);
        }
    }

    fn move_quick_slider(&mut self, x: f32, now: Instant, last: bool) {
        let Some(qs) = self.quick_slider.as_mut() else {
            return;
        };
        let inset = 6.0;
        let track_l = qs.rect.x + inset;
        let track_r = qs.rect.right() - inset;
        let track_w = (track_r - track_l).max(1.0);
        let clamped_x = x.clamp(track_l, track_r);
        let dx = clamped_x - qs.last_x;
        if dx.abs() > 0.01 {
            let fill_x = track_l + qs.fraction * track_w;
            let gain = if dx > 0.0 {
                let d_finger = (track_r - qs.last_x).max(1.0);
                let d_fill = (track_r - fill_x).max(0.0);
                (d_fill / d_finger).clamp(1.0, 8.0)
            } else {
                let d_finger = (qs.last_x - track_l).max(1.0);
                let d_fill = (fill_x - track_l).max(0.0);
                (d_fill / d_finger).clamp(1.0, 8.0)
            };
            let new_fill = (fill_x + dx * gain).clamp(track_l, track_r);
            let new_fraction = ((new_fill - track_l) / track_w).clamp(0.0, 1.0);
            if (new_fraction - qs.fraction).abs() > 1e-5 {
                qs.fraction = new_fraction;
                qs.changed = true;
            }
            qs.last_x = clamped_x;
        }
        if !qs.changed {
            return;
        }
        let slider = qs.slider;
        let value = (qs.fraction * 100.0).round() as u8;
        let last_sent = qs.last_sent;
        self.preview_slider_level(slider, value, now, last);
        let due = self
            .last_slider_send
            .get(&slider)
            .is_none_or(|t| now.saturating_duration_since(*t) >= SLIDER_RATE);
        let changed_val = last_sent != Some(value);
        if (due && changed_val) || (last && changed_val) {
            self.last_slider_send.insert(slider, now);
            if let Some(qs) = self.quick_slider.as_mut() {
                qs.last_sent = Some(value);
            }
            self.actions
                .extend(slider_action(&self.state, slider, value));
        }
        self.dirty = true;
    }

    fn preview_slider_level(&mut self, slider: Slider, value: u8, now: Instant, last: bool) {
        let release = last.then(|| now + SLIDER_HOLD);
        let previous = self.overrides.insert(slider, (value, release));
        if let Some(old) = previous
            .map(|(v, _)| v)
            .or_else(|| slider_percent(&self.state, slider))
            && old != value
        {
            LevelFeedback::at(value, slider.minimum_percent()).queue(&mut self.level_feedback);
        }
        set_slider(&mut self.state, slider, value, true);
    }

    fn finish_quick_slider(&mut self) {
        let Some(qs) = self.quick_slider.take() else {
            return;
        };
        self.quick_slider_closing = Some((qs.key, self.now + Duration::from_millis(350)));
        if let Some((_, release)) = self.overrides.get_mut(&qs.slider)
            && release.is_none()
        {
            *release = Some(self.now + SLIDER_HOLD);
        }
        self.animator.enter = Motion {
            dx: 0.0,
            dy: 0.0,
            scale: 0.9,
            delay: 0.14,
        };
        self.dirty = true;
    }

    // ---------------------------------------------------------- navigation

    fn exec(&mut self, cmd: Cmd) {
        self.dirty = true;
        match cmd {
            Cmd::WorldTap(id) => self.world_input(WorldInput::Tap(id)),
            Cmd::Esc => {
                if self.shown_page() == Page::Panel(Panel::Clock) && self.world.editing {
                    self.world_input(WorldInput::Back);
                } else if matches!(self.shown_page(), Page::Panel(Panel::Network))
                    && self.wifi_prompt.is_some()
                {
                    self.cancel_wifi_prompt();
                } else if matches!(self.shown_page(), Page::Panel(Panel::Network))
                    && self.speedtest.is_some()
                {
                    self.close_speedtest();
                } else if matches!(self.shown_page(), Page::Panel(Panel::Weather))
                    && self.weather_search.is_some()
                {
                    self.close_weather_search();
                } else {
                    self.actions.push(Action::Key(t1_key::ESC));
                }
            }
            Cmd::FKey(n) => self.actions.push(Action::Key(t1_key::function(n))),
            Cmd::OpenMenu => self.open_menu(String::new(), true),
            Cmd::MenuOpen(id) => self.open_menu(id, true),
            Cmd::MenuRun(id) => self.run_menu_item(&id),
            Cmd::Back => self.back(),
            Cmd::Close => self.go_home(),
            Cmd::OpenPanel(panel) => self.open_panel(panel),
            Cmd::TrayMenu(menu) => self.open_tray_menu(menu),
            Cmd::Workspace(id) => {
                let ws = &self.state.workspaces;
                let block = ws.block;
                let same_block = (id - 1).div_euclid(10) == block;
                let action = Action::FocusWorkspace {
                    id,
                    slot: (id - 1).rem_euclid(10) + 1,
                    monitor: ws.monitor.clone(),
                    same_block,
                };
                if self.state.workspaces.active != id {
                    self.state.workspaces.active = id;
                    self.actions.push(action);
                }
            }
            Cmd::Slider(_) => {}
            Cmd::WifiPromptOpen { ssid, signal } => self.open_wifi_prompt(ssid, signal, false),
            Cmd::WifiPromptToggleReveal => {
                if let Some(prompt) = self.wifi_prompt.as_mut() {
                    prompt.revealed = !prompt.revealed;
                    prompt.last_edit = self.now;
                }
            }
            Cmd::WifiPromptBackspace => {
                let now = self.now;
                self.key_input(KeyInput::Backspace, now);
            }
            Cmd::WifiPromptSubmit => self.submit_wifi_prompt(),
            Cmd::WifiPromptCancel => self.cancel_wifi_prompt(),
            Cmd::SpeedTestStart => self.start_speedtest(),
            Cmd::SpeedTestStop => self.close_speedtest(),
            Cmd::ToggleWifiNetworksExpanded => {
                self.wifi_networks_expanded = !self.wifi_networks_expanded;
            }
            Cmd::CopyField { field, value } => {
                self.copied_field = Some((field, self.now + COPY_FEEDBACK));
                self.actions.push(Action::CopyToClipboard(value));
            }
            Cmd::TriggerIndicator(indicator) => self.trigger_indicator(indicator),
            Cmd::SoundTab(tab) => {
                if self.sound_tab != tab {
                    self.sound_tab = tab;
                    self.scroll = Scroll::default();
                }
            }
            Cmd::WeatherSearch => self.open_weather_search(),
            Cmd::WeatherSearchBackspace => {
                let now = self.now;
                self.key_input(KeyInput::Backspace, now);
            }
            Cmd::WeatherPick(location) => self.pick_weather_location(location),
            Cmd::BluetoothForget(address) => {
                let armed = self
                    .bt
                    .forget_armed
                    .as_ref()
                    .is_some_and(|(a, until)| *a == address && self.now < *until);
                if armed {
                    self.bt.forget_armed = None;
                    let action = Action::BluetoothDevice {
                        address,
                        op: BtOp::Forget,
                    };
                    self.optimistic(&action);
                    self.actions.push(action);
                } else {
                    self.bt.forget_armed = Some((address, self.now + BT_FORGET_CONFIRM));
                }
            }
            Cmd::Do(Action::CancelTouchId) if self.touch_demo.is_some() => {
                self.touch_demo = None;
                self.state.touch_id = None;
            }
            Cmd::Do(action) => {
                if matches!(action, Action::ClickTrayMenu { .. }) {
                    self.open_panel(Panel::Tray);
                }
                let close_after = matches!(
                    action,
                    Action::SetReminder
                        | Action::SetDns(DnsProvider::Custom)
                        | Action::ShareWifiQr { .. }
                        | Action::OpenCaptivePortal
                );
                if matches!(action, Action::RefreshAgents { .. }) {
                    self.pulses.insert(
                        crate::build::AGENTS_REFRESH_PULSE,
                        self.now + crate::build::AGENTS_REFRESH,
                    );
                }
                self.optimistic(&action);
                self.actions.push(action);
                if close_after {
                    self.go_home();
                }
            }
        }
    }

    fn trigger_indicator(&mut self, indicator: Indicator) {
        let active = self.state.indicators.is_active(indicator);
        match indicator {
            Indicator::RemoteSession => {
                if !matches!(self.page, Page::Panel(Panel::Indicators)) {
                    self.open_panel(Panel::Indicators);
                }
            }
            Indicator::ScreenRecording if !active => {
                if self.menu.has_children("trigger.capture.screenrecord") {
                    self.open_menu("trigger.capture.screenrecord".into(), true);
                } else {
                    self.actions.push(Action::TriggerIndicator {
                        indicator,
                        currently_active: false,
                    });
                    self.go_home();
                }
            }
            Indicator::ScreenRecording => {
                self.state.indicators.recording = false;
                self.actions.push(Action::TriggerIndicator {
                    indicator,
                    currently_active: true,
                });
            }
            Indicator::PasswordlessSudo => {
                self.actions.push(Action::TriggerIndicator {
                    indicator,
                    currently_active: active,
                });
                if !active {
                    self.go_home();
                }
            }
            Indicator::Reminder => {
                if active {
                    self.actions.push(Action::TriggerIndicator {
                        indicator,
                        currently_active: true,
                    });
                } else {
                    self.actions.push(Action::SetReminder);
                    self.go_home();
                }
            }
            Indicator::NightLight => {
                self.state.indicators.nightlight = !active;
                self.actions.push(Action::TriggerIndicator {
                    indicator,
                    currently_active: active,
                });
            }
            Indicator::Dnd => {
                self.state.indicators.dnd = !active;
                self.actions.push(Action::TriggerIndicator {
                    indicator,
                    currently_active: active,
                });
            }
            Indicator::StayAwake => {
                self.state.indicators.stay_awake = !active;
                self.actions.push(Action::TriggerIndicator {
                    indicator,
                    currently_active: active,
                });
            }
        }
    }

    /// Reflects an action in the UI before the desktop confirms it.
    fn optimistic(&mut self, action: &Action) {
        let s = &mut self.state;
        match action {
            Action::ToggleMute => {
                if let Some(a) = s.audio.as_mut() {
                    a.muted = !a.muted;
                }
            }
            Action::ToggleMicMute => {
                if let Some(m) = s.mic_muted.as_mut() {
                    *m = !*m;
                }
            }
            Action::SetDefaultOutput { id, .. } => {
                for d in &mut s.mixer.outputs {
                    d.default = d.id == *id;
                }
                if let (Some(a), Some(d)) = (
                    s.audio.as_mut(),
                    s.mixer.outputs.iter().find(|d| d.id == *id),
                ) {
                    a.sink = d.label.clone();
                }
            }
            Action::SetDefaultInput { id, .. } => {
                for d in &mut s.mixer.inputs {
                    d.default = d.id == *id;
                }
            }
            Action::ToggleStreamMute(id) => {
                if let Some(st) = s.mixer.streams.iter_mut().find(|st| st.id == *id) {
                    st.muted = !st.muted;
                }
            }
            Action::MediaPlayPause => {
                if let Some(m) = s.media.as_mut() {
                    m.playing = !m.playing;
                }
            }
            Action::SetPowerProfile(p) => s.power_profile = Some(p.clone()),
            Action::SetWifi(on) => s.network.wifi_enabled = *on,
            Action::SetDns(provider) if *provider != DnsProvider::Custom => {
                s.network.pending_dns = Some(*provider);
            }
            Action::SetWifiBand(band) => {
                s.network.pending_band = Some(band.clone());
            }
            Action::SetBluetooth(on) => s.bluetooth.powered = *on,
            Action::BluetoothDevice { address, op } => {
                self.bt.failed.remove(address);
                self.bt.pending.insert(address.clone(), (*op, self.now));
                if self
                    .bt
                    .forget_armed
                    .as_ref()
                    .is_some_and(|(a, _)| a == address)
                {
                    self.bt.forget_armed = None;
                }
            }
            Action::ClearReminders => {
                s.indicators.reminder_count = 0;
                s.indicators.reminder_tooltip.clear();
            }
            _ => {}
        }
    }

    fn navigate(&mut self, page: Page, enter: Motion, exit: Motion) {
        if page == self.page {
            return;
        }
        if self.page == Page::Panel(Panel::Clock) {
            self.world_input(WorldInput::Close);
            self.world = Default::default();
        }
        self.cancel_wifi_connect_only();
        self.wifi_prompt = None;
        self.wifi_networks_expanded = false;
        self.sound_tab = SoundTab::Output;
        self.weather_search = None;
        self.bt.forget_armed = None;
        self.bt.active_at = Some(self.now);
        self.stop_speedtest_if_running();
        self.speedtest = None;
        self.finish_quick_slider();
        self.animator.enter = enter;
        self.animator.exit = exit;
        self.tray_menu = None;
        self.tray_entries = None;
        self.copied_field = None;
        self.page = page;
        self.scroll = Scroll::default();
        self.entered_at = self.now;
        self.dirty = true;
    }

    pub(crate) fn go_home(&mut self) {
        self.panel_origin = None;
        let from_menu = matches!(self.page, Page::Menu(_));
        let enter = if from_menu {
            Motion::slide(40.0)
        } else {
            Motion::FADE
        };
        let exit = if from_menu {
            Motion::slide(-40.0)
        } else {
            Motion {
                dx: 0.0,
                dy: 0.0,
                scale: 0.92,
                delay: 0.0,
            }
        };
        self.navigate(base_page(self.config.mode), enter, exit);
    }

    fn open_tray_menu(&mut self, menu: TrayMenu) {
        self.tray_entries = None;
        self.scroll = Scroll::default();
        self.tray_menu = Some(menu.clone());
        self.actions.push(Action::LoadTrayMenu(menu));
        self.dirty = true;
    }

    fn back(&mut self) {
        if self.page == Page::Panel(Panel::Clock)
            && !matches!(self.world.view.as_str(), "" | "list")
        {
            self.world_input(WorldInput::Back);
            return;
        }
        if matches!(self.page, Page::Panel(Panel::Weather)) && self.weather_search.is_some() {
            self.close_weather_search();
            return;
        }
        if matches!(self.page, Page::Panel(Panel::Network)) {
            if self.wifi_prompt.is_some() {
                self.cancel_wifi_prompt();
                return;
            }
            if self.speedtest.is_some() {
                self.close_speedtest();
                return;
            }
        }
        if matches!(self.page, Page::Panel(Panel::Tray))
            && let Some(mut menu) = self.tray_menu.take()
        {
            menu.parents.pop();
            if menu.parents.is_empty() {
                self.open_panel(Panel::Tray);
            } else {
                self.open_tray_menu(menu);
            }
            return;
        }
        match self.page.clone() {
            Page::Menu(id) if !id.is_empty() => {
                let parent = menu::parent_of(&id).to_string();
                self.open_menu(parent, false);
            }
            _ => self.go_home(),
        }
    }

    pub(crate) fn open_menu(&mut self, id: String, forward: bool) {
        let resolved = match self.menu.get(&id) {
            Some(item) if item.target.is_some() => item.target.clone().unwrap(),
            _ => id,
        };
        let from_bar = !matches!(self.page, Page::Menu(_));
        let (enter, exit) = if from_bar {
            (
                Motion::slide(-56.0),
                Motion {
                    dx: 30.0,
                    dy: 0.0,
                    scale: 0.94,
                    delay: 0.0,
                },
            )
        } else if forward {
            (Motion::slide(80.0), Motion::slide(-60.0))
        } else {
            (Motion::slide(-80.0), Motion::slide(60.0))
        };
        self.navigate(Page::Menu(resolved.clone()), enter, exit);
        self.request_menu_data(&resolved);
    }

    fn request_menu_data(&mut self, id: &str) {
        let mut guards: Vec<Guard> = self
            .menu
            .children(id)
            .flat_map(|item| {
                [
                    (GuardKind::Checked, &item.checked),
                    (GuardKind::Disabled, &item.disabled),
                ]
                .into_iter()
                .filter_map(|(kind, expr)| {
                    expr.as_ref().map(|e| Guard {
                        id: item.id.clone(),
                        kind,
                        expr: e.clone(),
                    })
                })
                .collect::<Vec<_>>()
            })
            .collect();
        guards.extend(
            self.menu
                .visibility_guards(id)
                .into_iter()
                .map(|(id, expr)| Guard {
                    id: id.into(),
                    kind: GuardKind::When,
                    expr: expr.into(),
                }),
        );
        if !guards.is_empty() {
            self.actions.push(Action::EvaluateGuards {
                menu: id.to_string(),
                guards,
            });
        }
        if let Some(provider) = self.menu.get(id).and_then(|m| m.provider.clone())
            && menu::provider_script(&provider).is_some()
        {
            self.actions.push(Action::LoadProvider {
                menu: id.to_string(),
                provider,
            });
        }
    }

    fn run_menu_item(&mut self, id: &str) {
        let item = self.menu.get(id).cloned().or_else(|| {
            self.provider_rows
                .values()
                .flatten()
                .find(|i| i.id == id)
                .cloned()
        });
        let Some(item) = item else { return };
        if let Some(action) = &item.action {
            self.actions.push(Action::Shell(action.clone()));
            self.go_home();
        } else if item.provider.is_some()
            && !self.menu.has_children(&item.id)
            && menu::provider_script(item.provider.as_deref().unwrap_or("")).is_none()
        {
            // Runtime rows we can't enumerate: open the on-screen menu there.
            let payload = format!("{{\"menu\":\"{}\"}}", item.id.replace('"', ""));
            self.actions.push(Action::Shell(format!(
                "omarchy-shell shell toggle omarchy.menu {}",
                menu::shell_quote(&payload)
            )));
            self.go_home();
        } else {
            self.open_menu(item.id.clone(), true);
        }
    }

    fn open_panel(&mut self, panel: Panel) {
        self.tray_menu = None;
        self.tray_entries = None;
        self.scroll = Scroll::default();
        // Keyboard mode has no bar widgets: Sound grows out of its player.
        let origin = match (panel, self.config.mode) {
            (Panel::Audio, Mode::Keyboard) => self.keyboard_player_key(),
            _ => panel.origin(),
        };
        self.panel_origin = self.animator.get(origin).map(|a| a.rect().cx());
        self.navigate(
            Page::Panel(panel),
            Motion::FADE,
            Motion {
                dx: 0.0,
                dy: 0.0,
                scale: 0.9,
                delay: 0.0,
            },
        );
        if panel == Panel::Clock {
            self.world_session += 1;
            self.world = Default::default();
            self.world_input(WorldInput::Open);
            self.dirty = true;
        }
        self.actions.push(Action::PanelOpened(panel));
    }

    /// The page actually shown (Fn overrides the base page).
    pub(crate) fn shown_page(&self) -> Page {
        if self.fn_held {
            match (self.config.fn_key, self.config.mode) {
                (FnKey::Function, Mode::Function) => return Page::Controls,
                (FnKey::Function, _) => return Page::FnKeys,
                // The keyboard row already has the controls.
                (FnKey::Controls, Mode::Controls | Mode::Keyboard) => return Page::FnKeys,
                (FnKey::Controls, _) => return Page::Controls,
                (FnKey::None, _) => {}
            }
        }
        self.page.clone()
    }

    // ---------------------------------------------------------------- time

    /// Advances timers and animations to `now`. Returns whether the frame
    /// changed and should be drawn.
    pub fn update(&mut self, now: Instant, utc: DateTime<Utc>) -> bool {
        let dt = match self.last_update {
            Some(t) => now.saturating_duration_since(t).as_secs_f32().min(0.05),
            None => 1.0 / 60.0,
        };
        self.last_update = Some(now);
        self.now = now;
        self.utc = utc;
        self.local = utc.with_timezone(&Local).naive_local();

        // Hold-to-repeat.
        let mut repeats = Vec::new();
        let level_keys_visible = matches!(self.shown_page(), Page::Keyboard | Page::Controls)
            && self.state.touch_id.is_none()
            && self.touch_fx.is_none()
            && !self.shows_screensaver();
        for touch in self.touches.values_mut() {
            let (Some(at), Some(cmd)) = (touch.repeat_at, touch.cmd.as_ref()) else {
                continue;
            };
            if touch.cancelled {
                continue;
            }
            if level_keys_visible
                && (now >= at || touch.repeat_level.is_some())
                && let Some((level, up)) = step_level(&self.state, cmd)
            {
                let previous = touch.repeat_level.replace(level);
                let at_limit = level == if up { 100 } else { 0 };
                if previous.map_or(at_limit, |old| if up { level > old } else { level < old }) {
                    LevelFeedback::at(level, 0).queue(&mut self.level_feedback);
                }
            }
            if now >= at {
                touch.repeat_at = Some(at + REPEAT_EVERY);
                repeats.push(cmd.clone());
            }
        }
        for cmd in repeats {
            self.exec(cmd);
        }

        // Hold-to-transform into an inline slider.
        let mut held_slider = None;
        for (&id, touch) in self.touches.iter_mut() {
            if let Some(at) = touch.hold_at
                && now >= at
                && !touch.cancelled
            {
                touch.hold_at = None;
                if let (Some(HitKind::TapSlide(slider)), Some(key)) = (touch.kind, touch.key)
                    && held_slider.is_none()
                {
                    touch.cancelled = true;
                    held_slider = Some((id, slider, key, touch.x));
                }
            }
        }
        if let Some((id, slider, key, x)) = held_slider {
            self.activate_quick_slider(id, slider, key, x, None, now);
        }

        // Slider overrides end shortly after release.
        let before = self.overrides.len();
        self.overrides
            .retain(|_, (_, release)| release.is_none_or(|t| now < t));
        if self.overrides.len() != before {
            self.dirty = true;
        }
        let before = self.pulses.len();
        self.pulses.retain(|_, until| now < *until);
        if self.pulses.len() != before {
            self.dirty = true;
        }
        if self.copied_field.is_some_and(|(_, until)| now >= until) {
            self.copied_field = None;
            self.dirty = true;
        }
        if self
            .quick_slider_closing
            .is_some_and(|(_, until)| now >= until)
        {
            self.quick_slider_closing = None;
            self.dirty = true;
        }
        if self.step_bluetooth(now) {
            self.dirty = true;
        }

        // Auto-close the Wi-Fi password prompt shortly after a successful join.
        if let Some(done_at) = self.wifi_prompt.as_ref().and_then(|p| p.connected_at)
            && now.saturating_duration_since(done_at) >= Duration::from_millis(1100)
        {
            self.wifi_prompt = None;
            self.dirty = true;
        }

        // The speed test's opening sweep runs on a timer, and a gauge waiting
        // for its first reading shimmers; otherwise its springs animate it.
        let mut live_fx = false;
        if let Some(st) = self.speedtest.as_mut() {
            if st.step(dt) {
                self.dirty = true;
            }
            live_fx = st.sweep().is_some() || st.waiting();
        }
        if self.wifi_prompt.is_some() || self.weather_search.is_some() {
            live_fx = true;
        }
        if matches!(self.page, Page::Panel(Panel::Weather))
            && self.state.weather.as_ref().is_some_and(|w| w.loading)
        {
            live_fx = true;
        }
        if matches!(self.page, Page::Panel(Panel::Agents))
            && self.pulses.contains_key(crate::build::AGENTS_REFRESH_PULSE)
        {
            live_fx = true;
        }
        // Devices being paired or connected, and the scan, shimmer.
        if matches!(self.page, Page::Panel(Panel::Bluetooth))
            && (!self.bt.pending.is_empty() || self.state.bluetooth.discovering)
        {
            live_fx = true;
        }

        // Touch ID: a scripted demo stands in for T1Bridge, and the overlay
        // follows the prompt (and plays its outro after it closes).
        if let Some((demo, since)) = self.touch_demo.as_mut() {
            let since = *since.get_or_insert(now);
            let tid = demo.at(now.saturating_duration_since(since).as_secs_f32());
            if tid.is_none() {
                self.touch_demo = None;
            }
            if tid != self.state.touch_id {
                self.state.touch_id = tid;
                self.dirty = true;
            }
        }
        match (&mut self.touch_fx, &self.state.touch_id) {
            (None, Some(tid)) => {
                self.touch_fx = Some(TouchFx::new(tid, now, &self.fonts, self.height));
                self.dirty = true;
            }
            (Some(fx), tid) => {
                if fx.sync(tid.as_ref(), now) {
                    self.dirty = true;
                }
                if fx.done(now) {
                    self.touch_fx = None;
                    self.dirty = true;
                }
            }
            (None, None) => {}
        }
        if let Some(fx) = &self.touch_fx {
            live_fx |= fx.animating(self.config.motion.enabled);
            self.last_touch = now;
        }
        // The screensaver draws whenever its effect prints a new frame.
        if let Some(saver) = self.saver.as_mut()
            && std::mem::take(&mut saver.fresh)
        {
            live_fx = true;
        }

        // Return to the bar after inactivity (never while typing a password or
        // while a speed test is actively measuring).
        let idle = now.saturating_duration_since(self.last_touch);
        // Release keyboard capture for forgotten text editors.
        if (self.weather_search.is_some() || self.world.editing)
            && self.touches.is_empty()
            && idle > TEXT_ENTRY_IDLE
        {
            self.go_home();
        }
        let keep_open = self.world.editing
            || self.wifi_prompt.is_some()
            || self.weather_search.is_some()
            || self.speedtest.as_ref().is_some_and(|s| s.is_running())
            || (self.page == Page::Panel(Panel::Bluetooth) && !self.bt.pending.is_empty());
        if !keep_open
            && matches!(self.page, Page::Menu(_) | Page::Panel(_))
            && self.touches.is_empty()
            && (idle > self.panel_timeout() || self.bluetooth_scan_lapsed(now))
        {
            self.go_home();
        }

        // Idle dimming.
        let p = &self.config.power;
        // The screensaver plays at full brightness, as on the desktop.
        let target = if self.state.touch_id.is_some()
            || !self.touches.is_empty()
            || keep_open
            || self.saver.is_some()
        {
            1.0
        } else if p.off_after > 0 && idle > Duration::from_secs(p.off_after as u64) {
            0.0
        } else if p.dim_after > 0 && idle > Duration::from_secs(p.dim_after as u64) {
            p.dim_level
        } else {
            1.0
        };
        self.brightness.target = target;
        let waking = target > self.brightness.value;
        let params = if waking {
            SpringParams::new(0.16, 1.0)
        } else {
            SpringParams::new(1.4, 1.0)
        };
        let dimming = self.brightness.step(dt, params, 0.002);

        // Theme crossfade.
        let mut theming = false;
        if self.theme_t < 1.0 {
            let speed = if self.config.motion.enabled {
                2.2 * self.config.motion.speed
            } else {
                1000.0
            };
            self.theme_t = (self.theme_t + dt * speed).min(1.0);
            let t = 1.0 - (1.0 - self.theme_t).powi(3);
            self.theme = Theme::lerp(&self.theme_from, &self.theme_to, t);
            theming = true;
            self.dirty = true;
        }

        // Theme switch reveal.
        let wiping = self.theme_wipe.is_some();
        if self.theme_wipe.as_mut().is_some_and(|wipe| !wipe.step(now)) {
            // Finished: one more frame draws the new theme everywhere.
            self.theme_wipe = None;
        }

        // The clock text changes every minute (or second).
        let clock = self.clock_text();
        if clock != self.last_clock_text {
            self.last_clock_text = clock;
            self.dirty = true;
        }

        // Agent limit reset countdowns and expirations can change between wall-clock minutes.
        let agents_key = self.agents_countdown_key();
        if agents_key != self.last_agents_key {
            self.last_agents_key = agents_key;
            self.dirty = true;
        }

        let rebuilt = self.dirty;
        if self.dirty {
            self.dirty = false;
            let nodes = self.build();
            if let Some(from) = self.theme_wipe.as_ref().map(|w| w.from.clone()) {
                // The same nodes in the outgoing theme, for outside the band.
                let to = std::mem::replace(&mut self.theme, from);
                let old = self.build();
                self.theme = to;
                if let Some(wipe) = self.theme_wipe.as_mut() {
                    wipe.nodes = old.into_iter().map(|n| (n.key, n)).collect();
                }
            }
            self.animator.sync(nodes);
            self.animator.enter = Motion::FADE;
            self.animator.exit = Motion::FADE;
        }
        let scrolling = self.scroll.step(dt);
        let moving = self.animator.step(dt);
        let changed = rebuilt
            || moving
            || scrolling
            || dimming
            || theming
            || wiping
            || live_fx
            || self.animating;
        self.animating = moving || scrolling || dimming || theming || wiping || live_fx;
        changed
    }

    /// Expires Bluetooth operation feedback. Returns whether anything changed.
    fn step_bluetooth(&mut self, now: Instant) -> bool {
        let bt = &mut self.bt;
        if !bt.pending.is_empty() {
            bt.active_at = Some(now);
        }
        let before = (bt.pending.len(), bt.failed.len());
        bt.pending
            .retain(|_, (_, since)| now.saturating_duration_since(*since) < BT_PENDING);
        bt.failed.retain(|_, (_, until)| now < *until);
        let mut changed = before != (bt.pending.len(), bt.failed.len());
        if bt
            .forget_armed
            .as_ref()
            .is_some_and(|(_, until)| now >= *until)
        {
            bt.forget_armed = None;
            changed = true;
        }
        if bt
            .audio_switch
            .as_ref()
            .is_some_and(|(_, until)| now >= *until)
        {
            bt.audio_switch = None;
        }
        changed
    }

    /// The Bluetooth panel has scanned for [`BT_SCAN_IDLE`] since the strip
    /// was last touched.
    fn bluetooth_scan_lapsed(&self, now: Instant) -> bool {
        self.bluetooth_scanning_panel()
            && self
                .bt
                .active_at
                .is_some_and(|at| now.saturating_duration_since(at) > BT_SCAN_IDLE)
    }

    /// Idle time after which an open menu or panel returns to the bar.
    fn panel_timeout(&self) -> Duration {
        let configured = Duration::from_secs(self.config.power.panel_timeout as u64);
        if self.bluetooth_scanning_panel() {
            configured.max(BT_SCAN_IDLE)
        } else {
            configured
        }
    }

    /// Whether another frame is needed soon (animations in flight).
    pub fn animating(&self) -> bool {
        self.animating || self.dirty
    }

    /// When the scene next needs an update while idle.
    pub fn next_deadline(&self, now: Instant) -> Duration {
        let mut next = Duration::from_secs(60);
        let seconds = self.clock_has_seconds();
        let sub = self.utc.timestamp_subsec_millis() as u64;
        let to_tick = if seconds {
            Duration::from_millis(1000 - sub.min(999))
        } else {
            let s = self.local.and_utc().timestamp().rem_euclid(60) as u64;
            Duration::from_millis((60 - s) * 1000 - sub.min(999))
        };
        next = next.min(to_tick + Duration::from_millis(5));
        let idle = now.saturating_duration_since(self.last_touch);
        let p = &self.config.power;
        for limit in [
            Duration::from_secs(p.dim_after as u64),
            Duration::from_secs(p.off_after as u64),
            self.panel_timeout(),
        ] {
            if limit > Duration::ZERO && idle < limit {
                next = next.min(limit - idle + Duration::from_millis(10));
            }
        }
        let bt_deadlines = self
            .bt
            .pending
            .values()
            .map(|(_, since)| *since + BT_PENDING)
            .chain(self.bt.failed.values().map(|(_, until)| *until))
            .chain(self.bt.forget_armed.as_ref().map(|(_, until)| *until));
        let scan_end = self
            .bt
            .active_at
            .filter(|_| self.bluetooth_scanning_panel())
            .map(|at| at + BT_SCAN_IDLE);
        for at in bt_deadlines.chain(scan_end) {
            next = next.min(at.saturating_duration_since(now) + Duration::from_millis(5));
        }
        if (self.weather_search.is_some() || self.world.editing) && idle < TEXT_ENTRY_IDLE {
            next = next.min(TEXT_ENTRY_IDLE - idle + Duration::from_millis(10));
        }
        for touch in self.touches.values() {
            if let Some(at) = touch.repeat_at {
                next = next.min(at.saturating_duration_since(now));
            }
            if let Some(at) = touch.hold_at {
                next = next.min(at.saturating_duration_since(now));
            }
        }
        for until in self.pulses.values() {
            next = next.min(until.saturating_duration_since(now));
        }
        if let Some((_, until)) = self.copied_field {
            next = next.min(until.saturating_duration_since(now));
        }
        for (_, release) in self.overrides.values() {
            if let Some(t) = release {
                next = next.min(t.saturating_duration_since(now));
            }
        }
        let on_agents = matches!(self.shown_page(), Page::Panel(Panel::Agents));
        for agent in &self.state.agents {
            if on_agents
                && let Some(wait) = agent
                    .reset_credits
                    .as_ref()
                    .and_then(|c| c.next_expires_at)
                    .and_then(|at| crate::agents::next_countdown_change(at, self.utc))
            {
                next = next.min(wait + Duration::from_millis(5));
            }
            for limit in agent.all_limits() {
                if on_agents {
                    if let Some(wait) = limit.next_reset_change(self.utc) {
                        next = next.min(wait + Duration::from_millis(5));
                    }
                } else if limit.used >= 90
                    && let Some(reset_at) = limit.resets_at
                {
                    // The bar's alarm clears when the window resets.
                    let ms = reset_at.signed_duration_since(self.utc).num_milliseconds();
                    if ms > 0 {
                        next = next.min(Duration::from_millis(ms as u64 + 5));
                    }
                }
            }
        }
        next
    }

    fn agents_countdown_key(&self) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        let mut mix = |v: u64| {
            h ^= v;
            h = h.wrapping_mul(0x0100_0000_01b3);
        };
        let mut bucket = |used: u8, b: Option<(u8, i64)>| {
            mix(u64::from(used));
            match b {
                Some((unit, value)) => {
                    mix(u64::from(unit) + 1);
                    mix(value as u64);
                }
                None => mix(0),
            }
        };
        for agent in &self.state.agents {
            let credits = agent.reset_credits.as_ref().and_then(|c| c.next_expires_at);
            bucket(
                0,
                credits.and_then(|at| crate::agents::countdown_bucket(at, self.utc)),
            );
            for limit in agent.all_limits() {
                bucket(
                    limit.used_at(self.utc),
                    limit.reset_display_bucket(self.utc),
                );
            }
        }
        h
    }
}

fn monitor_target(state: &DesktopState, key: Key) -> Option<&str> {
    let panel = state.display_panel.as_ref()?;
    let m = panel.focused()?;
    (Key::new(&m.name) == key && panel.brightness.is_some()).then_some(&m.name)
}

/// A slider's level (0–1) as the desktop reports it.
fn slider_fraction(state: &DesktopState, slider: Slider) -> f32 {
    let fallback = if matches!(slider, Slider::Display | Slider::Keyboard) {
        50
    } else {
        0
    };
    slider_percent(state, slider).unwrap_or(fallback) as f32 / 100.0
}

fn slider_percent(state: &DesktopState, slider: Slider) -> Option<u8> {
    match slider {
        Slider::Volume => state
            .audio
            .as_ref()
            .map(|a| if a.muted { 0 } else { a.volume }),
        Slider::Display => state.display_brightness,
        Slider::Monitor(_) => state.display_panel.as_ref().and_then(|p| p.brightness),
        Slider::Keyboard => state.keyboard_backlight,
        Slider::Mic => state.mic_volume,
        Slider::Stream(id) => state
            .mixer
            .streams
            .iter()
            .find(|s| s.id == id)
            .map(|s| s.volume),
    }
}

fn step_level(state: &DesktopState, cmd: &Cmd) -> Option<(u8, bool)> {
    let (slider, up) = match cmd {
        Cmd::Do(Action::StepVolume(up)) => (Slider::Volume, *up),
        Cmd::Do(Action::StepDisplayBrightness(up)) => (Slider::Display, *up),
        Cmd::Do(Action::StepKeyboardBacklight(up)) => (Slider::Keyboard, *up),
        _ => return None,
    };
    Some((slider_percent(state, slider)?, up))
}

/// Shows a slider's value before the desktop confirms it. Dragging the
/// output volume unmutes it, like Omarchy's volume keys; the microphone and
/// apps keep their mute state, as in Omarchy's audio panel.
fn set_slider(state: &mut DesktopState, slider: Slider, value: u8, dragged: bool) {
    match slider {
        Slider::Volume => {
            if let Some(a) = state.audio.as_mut() {
                a.volume = value;
                if dragged {
                    a.muted = false;
                }
            }
        }
        Slider::Display => state.display_brightness = Some(value),
        Slider::Monitor(key) => {
            if monitor_target(state, key).is_some()
                && let Some(panel) = &mut state.display_panel
            {
                panel.brightness = Some(value);
            }
        }
        Slider::Keyboard => state.keyboard_backlight = Some(value),
        Slider::Mic => state.mic_volume = Some(value),
        Slider::Stream(id) => {
            if let Some(s) = state.mixer.streams.iter_mut().find(|s| s.id == id) {
                s.volume = value;
            }
        }
    }
}

fn slider_action(state: &DesktopState, slider: Slider, value: u8) -> Option<Action> {
    Some(match slider {
        Slider::Volume => Action::SetVolume(value),
        Slider::Display => Action::SetDisplayBrightness(value),
        Slider::Monitor(key) => Action::ConfigureDisplay(DisplayChange::Brightness {
            monitor: monitor_target(state, key)?.to_owned(),
            percent: value,
        }),
        Slider::Keyboard => Action::SetKeyboardBacklight(value),
        Slider::Mic => Action::SetMicVolume(value),
        Slider::Stream(id) => Action::SetStreamVolume { id, volume: value },
    })
}

/// Whether a PipeWire output node belongs to the Bluetooth device at
/// `address` (`bluez_output.AC_80_0A_00_00_01.1`).
fn bluez_node_matches(node: &str, address: &str) -> bool {
    let Some(rest) = node.strip_prefix("bluez_output.") else {
        return false;
    };
    let wanted = address.replace(':', "_");
    rest.get(..wanted.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(&wanted))
}

fn base_page(mode: Mode) -> Page {
    match mode {
        Mode::Bar => Page::Bar,
        Mode::Keyboard => Page::Keyboard,
        Mode::Controls => Page::Controls,
        Mode::Function => Page::FnKeys,
    }
}

fn feel(config: &Config) -> Feel {
    Feel {
        enabled: config.motion.enabled,
        speed: config.motion.speed,
        bounce: config.motion.bounce,
    }
}

/// Linux key codes accepted by the T1Bridge `TapKeys` action.
pub mod t1_key {
    pub const ESC: u16 = 1;

    pub const fn function(n: u8) -> u16 {
        match n {
            1..=10 => 58 + n as u16,
            11 => 87,
            _ => 88,
        }
    }
}

#[cfg(test)]
mod t1_menus;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{T1_HEIGHT, T1_WIDTH, canvas::Canvas, color::Rgba, speedtest::SpeedPhase};

    /// Most tests exercise the Omarchy bar.
    fn bar_config() -> Config {
        Config {
            mode: Mode::Bar,
            ..Config::default()
        }
    }

    fn test_scene() -> Scene {
        test_scene_with_config(bar_config())
    }

    fn test_scene_with_config(config: Config) -> Scene {
        let mut scene = Scene::new(
            T1_WIDTH,
            T1_HEIGHT,
            config,
            Palette::fallback(),
            Fonts::from_files(None, None, None, None, None),
            MenuTree::default(),
            None,
        );
        scene.set_state(DesktopState::sample());
        scene
    }

    #[test]
    fn world_clock_edit_releases_keyboard_and_ignores_frames_from_a_closed_panel() {
        let mut scene = test_scene();
        let now = Instant::now();
        scene.command(Command::OpenPanel(Panel::Clock));
        let first = scene.world_session;
        let edit = world_clock::Frame {
            session: first,
            view: "people".into(),
            editing: true,
            ..Default::default()
        };
        scene.apply_world_clock(edit.clone());
        assert!(scene.wants_keyboard());
        scene.key_input(KeyInput::Text("Ada".into()), now);
        assert!(
            scene
                .take_actions()
                .contains(&Action::WorldClock(world_clock::Request {
                    session: first,
                    command: WorldInput::Text("Ada".into())
                }))
        );
        scene.command(Command::Home);
        assert!(!scene.wants_keyboard());
        scene.command(Command::OpenPanel(Panel::Clock));
        scene.apply_world_clock(edit);
        assert!(!scene.wants_keyboard());
        scene.apply_world_clock(world_clock::Frame {
            session: scene.world_session,
            view: "add".into(),
            editing: true,
            ..Default::default()
        });
        scene.update(now + TEXT_ENTRY_IDLE + Duration::from_secs(1), Utc::now());
        assert!(!scene.wants_keyboard());
    }

    #[test]
    fn theme_switch_opens_out_of_the_middle() {
        // Sample the exposed backdrop to track the wipe across the panel.
        let mut config = bar_config();
        config.appearance.background = crate::config::BackgroundMode::Theme;
        let mut scene = test_scene_with_config(config);
        let utc = Utc::now();
        let mut now = Instant::now();
        scene.update(now, utc);
        let old = scene.theme.bar;
        let light = Palette::parse(
            r##"
            mode = "light"
            background = "#eff1f5"
            darker_background = "#dce0e8"
            foreground = "#4c4f69"
            accent = "#1e66f5"
            "##,
        )
        .unwrap();
        scene.set_palette(light.clone());
        let new = scene.theme.bar;
        assert_ne!(old, new);

        let mut canvas = Canvas::new(T1_WIDTH, T1_HEIGHT);
        let mut at = |scene: &Scene, x: u32| {
            scene.render(&mut canvas);
            let i = (x * 4) as usize;
            canvas.rgba()[i..i + 3].to_vec()
        };
        let rgb = |c: Rgba| [c.r, c.g, c.b].map(|v| (v * 255.0).round() as u8).to_vec();
        let close = |a: Vec<u8>, b: Vec<u8>| a.iter().zip(&b).all(|(x, y)| x.abs_diff(*y) <= 2);

        // Part way: the middle shows the new theme, the ends still the old.
        for _ in 0..8 {
            now += Duration::from_millis(16);
            assert!(scene.update(now, utc));
        }
        assert!(close(at(&scene, T1_WIDTH / 2), rgb(new)));
        assert!(close(at(&scene, 2), rgb(old)));
        assert!(close(at(&scene, T1_WIDTH - 3), rgb(old)));

        // It runs as long as the desktop's reveal, then the new theme is everywhere.
        for _ in 0..60 {
            now += Duration::from_millis(16);
            scene.update(now, utc);
        }
        assert!(scene.theme_wipe.is_none());
        assert!(close(at(&scene, 2), rgb(new)));

        // Without motion the theme just changes.
        scene.config.motion.enabled = false;
        scene.set_palette(Palette::fallback());
        assert!(scene.theme_wipe.is_none());
        scene.set_palette(light);
        assert!(scene.theme_wipe.is_none());
    }

    #[test]
    fn wifi_prompt_lifecycle_and_keyboard_grab() {
        let mut scene = test_scene();
        let mut now = Instant::now();
        assert!(!scene.wants_keyboard());

        scene.open_wifi_prompt("Szv".into(), 70, false);
        assert!(scene.wants_keyboard());

        scene.key_input(KeyInput::Text("abc12345".into()), now);
        scene.key_input(KeyInput::Backspace, now);
        assert_eq!(
            scene.wifi_prompt.as_ref().map(|p| p.password.as_str()),
            Some("abc1234")
        );

        // Toggle reveal via internal command
        scene.exec(Cmd::WifiPromptToggleReveal);
        assert!(scene.wifi_prompt.as_ref().unwrap().revealed);

        // Submit emits ConnectWifiPassword
        scene.key_input(KeyInput::Enter, now);
        let actions = scene.take_actions();
        assert!(actions.contains(&Action::ConnectWifiPassword {
            ssid: "Szv".into(),
            password: "abc1234".into(),
        }));
        assert!(scene.wants_keyboard());

        // Wrong password returns to Failed while keeping keyboard active
        scene.apply_wifi_connect_result("Szv", Err("Wrong password".into()), true);
        assert!(matches!(
            scene.wifi_prompt.as_ref().map(|p| &p.phase),
            Some(WifiPromptPhase::Failed(_))
        ));
        assert!(scene.wants_keyboard());

        // Resubmit & succeed -> Connected releases keyboard and auto-closes
        scene.key_input(KeyInput::Text("6".into()), now);
        scene.key_input(KeyInput::Enter, now);
        let _ = scene.take_actions();
        scene.apply_wifi_connect_result("Szv", Ok(()), false);
        assert!(!scene.wants_keyboard());
        now += Duration::from_millis(1200);
        scene.update(now, Utc::now());
        assert!(scene.wifi_prompt.is_none());
    }

    #[test]
    fn speedtest_phases_and_cancel() {
        let mut scene = test_scene();
        let mut now = Instant::now();
        scene.command(Command::OpenSpeedTest);
        assert!(scene.take_actions().contains(&Action::StartSpeedTest));
        assert!(matches!(
            scene.speedtest.as_ref().map(|s| &s.phase),
            Some(SpeedPhase::Ignition)
        ));

        scene.apply_speedtest_event(SpeedTestEvent::Ping(12.5));
        scene.apply_speedtest_event(SpeedTestEvent::DownloadSample(420.0));
        assert!(matches!(
            scene.speedtest.as_ref().map(|s| &s.phase),
            Some(SpeedPhase::Download)
        ));
        assert!(scene.speedtest.as_ref().unwrap().scale >= 500.0);

        scene.apply_speedtest_event(SpeedTestEvent::DownloadDone(420.0));
        scene.apply_speedtest_event(SpeedTestEvent::UploadStart);
        scene.apply_speedtest_event(SpeedTestEvent::UploadSample(180.0));
        assert!(matches!(
            scene.speedtest.as_ref().map(|s| &s.phase),
            Some(SpeedPhase::Upload)
        ));

        // Esc stops an active speedtest cleanly
        scene.exec(Cmd::Esc);
        assert!(scene.speedtest.is_none());
        assert!(scene.take_actions().contains(&Action::StopSpeedTest));

        // Verify a completed run settles cleanly
        scene.command(Command::OpenSpeedTest);
        let _ = scene.take_actions();
        now += Duration::from_millis(1000);
        scene.update(now, Utc::now());
        scene.apply_speedtest_event(SpeedTestEvent::DownloadDone(512.0));
        scene.apply_speedtest_event(SpeedTestEvent::UploadDone(240.0));
        assert!(matches!(
            scene.speedtest.as_ref().map(|s| &s.phase),
            Some(SpeedPhase::Complete)
        ));
    }

    #[test]
    fn bar_defaults_hide_media_and_show_brightness_percent_and_weather_temp() {
        use crate::node::Part;

        let omarchy = BarLayout::parse_shell_json(
            r#"{"bar":{"layout":{
                "left":[{"id":"omarchy.menu"},{"id":"omarchy.workspaces"}],
                "center":[{"id":"omarchy.clock","format":"dddd HH:mm"}],
                "centerEnd":[{"id":"omarchy.elsewhen"},{"id":"omarchy.weather"}],
                "right":[{"id":"omarchy.audio"},{"id":"omarchy.monitor"},{"id":"omarchy.power"}]
            }}}"#,
        );
        let mut scene = Scene::new(
            T1_WIDTH,
            T1_HEIGHT,
            bar_config(),
            Palette::fallback(),
            Fonts::from_files(None, None, None, None, None),
            MenuTree::default(),
            omarchy,
        );
        scene.set_state(DesktopState::sample());
        let nodes = scene.build();

        assert!(
            !nodes.iter().any(|n| n.key == Key::new("w/media")),
            "now playing should not be shown on the bar by default"
        );

        let brightness = nodes
            .iter()
            .find(|n| n.key == Key::new("w/brightness"))
            .expect("brightness widget");
        assert!(brightness.parts.iter().any(|p| matches!(
            p,
            Part::Text { text, .. } if text == "70%"
        )));

        let weather = nodes
            .iter()
            .find(|n| n.key == Key::new("w/weather"))
            .expect("weather widget");
        assert!(weather.parts.iter().any(|p| matches!(
            p,
            Part::Text { text, .. } if text == "12°C"
        )));
        let m = scene.m();
        assert!(weather.parts.iter().any(|p| matches!(
            p,
            Part::Text { role: crate::text::Role::Icon, size, .. } if *size < m.icon
        )));
    }

    fn place(name: &str) -> Location {
        Location {
            name: name.into(),
            region: "Somewhere".into(),
            coords: Some((54.7, 20.5)),
            country: Some("RU".into()),
        }
    }

    #[test]
    fn weather_location_search_types_suggests_and_picks() {
        let mut scene = test_scene();
        let now = Instant::now();
        scene.open_weather_search();
        assert!(scene.wants_keyboard());
        assert_eq!(scene.page, Page::Panel(Panel::Weather));
        scene.take_actions();

        // One letter is not enough to search; two are.
        scene.key_input(KeyInput::Text("K".into()), now);
        assert!(
            !scene
                .take_actions()
                .iter()
                .any(|a| matches!(a, Action::SearchWeatherLocation(_)))
        );
        scene.key_input(KeyInput::Text("al".into()), now);
        assert_eq!(
            scene.take_actions(),
            vec![Action::SearchWeatherLocation("Kal".into())]
        );

        // Enter waits for the answer rather than storing a bare name.
        scene.key_input(KeyInput::Enter, now);
        assert!(scene.weather_search.is_some());
        assert!(scene.take_actions().is_empty());

        // Answers for an older query are ignored.
        scene.apply_weather_suggestions("Ka", Some(vec![place("Kazan")]));
        assert!(
            scene
                .weather_search
                .as_ref()
                .unwrap()
                .current_results()
                .is_empty()
        );
        scene.apply_weather_suggestions("Kal", Some(vec![place("Kaliningrad"), place("Kalinin")]));
        assert_eq!(
            scene
                .weather_search
                .as_ref()
                .unwrap()
                .current_results()
                .len(),
            2
        );

        scene.key_input(KeyInput::Enter, now);
        assert!(scene.weather_search.is_none());
        assert!(!scene.wants_keyboard());
        assert_eq!(
            scene.take_actions(),
            vec![Action::SetWeatherLocation(Some(place("Kaliningrad")))]
        );
        let w = scene.state.weather.as_ref().unwrap();
        assert!(w.loading);
        assert_eq!(w.place.name, "Kaliningrad");
        assert!(!w.place.auto);
        assert_eq!(scene.page, Page::Panel(Panel::Weather));
    }

    #[test]
    fn weather_location_search_auto_cancel_and_idle_release() {
        let (mut scene, mut now) = settled_scene();
        let utc = Utc::now();

        // Enter with nothing typed returns to auto, like Omarchy.
        scene.open_weather_search();
        scene.take_actions();
        scene.key_input(KeyInput::Enter, now);
        assert_eq!(scene.take_actions(), vec![Action::SetWeatherLocation(None)]);
        assert!(scene.state.weather.as_ref().unwrap().place.auto);

        // Tapping a suggestion picks it.
        scene.open_weather_search();
        scene.exec(Cmd::WeatherPick(Some(place("Tokyo"))));
        assert!(
            scene
                .take_actions()
                .contains(&Action::SetWeatherLocation(Some(place("Tokyo"))))
        );

        // Esc and Back leave the search but stay in the panel.
        scene.open_weather_search();
        scene.key_input(KeyInput::Escape, now);
        assert!(scene.weather_search.is_none());
        assert_eq!(scene.page, Page::Panel(Panel::Weather));
        scene.open_weather_search();
        scene.exec(Cmd::Esc);
        assert!(scene.weather_search.is_none());
        scene.open_weather_search();
        scene.exec(Cmd::Back);
        assert!(scene.weather_search.is_none());
        assert_eq!(scene.page, Page::Panel(Panel::Weather));
        assert!(
            !scene
                .take_actions()
                .iter()
                .any(|a| matches!(a, Action::Key(_)))
        );

        // Typing keeps it open past the panel timeout; a forgotten search
        // releases the keyboard.
        scene.open_weather_search();
        for _ in 0..30 {
            now += Duration::from_secs(1);
            scene.key_input(KeyInput::Text("a".into()), now);
            scene.update(now, utc);
        }
        assert!(scene.wants_keyboard());
        now += TEXT_ENTRY_IDLE + Duration::from_secs(1);
        scene.update(now, utc);
        assert!(!scene.wants_keyboard());
        assert!(matches!(scene.page, Page::Bar));
    }

    #[test]
    fn weather_panel_shows_the_full_report_and_location() {
        use crate::node::Part;
        let mut scene = test_scene();
        scene.command(Command::OpenPanel(Panel::Weather));
        let nodes = scene.build();
        let texts: Vec<&str> = nodes
            .iter()
            .flat_map(|n| &n.parts)
            .filter_map(|p| match p {
                Part::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        for expected in [
            "12°C",
            "Partly cloudy",
            "Copenhagen",
            "Auto-detected",
            "Feels like",
            "9°C",
            "Wind",
            "14 km/h SW",
            "Humidity",
            "81%",
            "Sunrise",
            "Sunset",
            "UV index",
            "Refresh",
        ] {
            assert!(
                texts.contains(&expected),
                "missing {expected:?} in {texts:?}"
            );
        }
        let place = nodes
            .iter()
            .find(|n| n.key == Key::new("p/weather").with("place"))
            .expect("location chip");
        assert_eq!(
            place.hit.as_ref().map(|h| &h.cmd),
            Some(&Cmd::WeatherSearch)
        );
        // Six forecast days follow today.
        let days =
            nodes
                .iter()
                .filter(|n| {
                    n.parts.iter().any(|p| matches!(
                    p,
                    Part::Text { text, .. } if ["Monday", "Tuesday", "Wednesday", "Thursday",
                        "Friday", "Saturday", "Sunday"].contains(&text.as_str())
                ))
                })
                .count();
        assert_eq!(days, 6);
    }

    fn settled_scene() -> (Scene, Instant) {
        let mut scene = test_scene();
        let mut now = Instant::now();
        let utc = Utc::now();
        for _ in 0..80 {
            now += Duration::from_millis(16);
            scene.update(now, utc);
        }
        (scene, now)
    }

    #[test]
    fn held_level_keys_follow_confirmed_changes_and_signal_each_limit_once() {
        let (mut scene, mut now) = settled_scene();
        scene.command(Command::SetMode(Mode::Keyboard));
        for _ in 0..60 {
            now += Duration::from_millis(16);
            scene.update(now, Utc::now());
        }
        for (key, slider, start, step, end, limit) in [
            ("vu", Slider::Volume, 90, 95, 100, LevelFeedback::Maximum),
            ("bd", Slider::Display, 12, 6, 0, LevelFeedback::Minimum),
            ("ku", Slider::Keyboard, 88, 94, 100, LevelFeedback::Maximum),
        ] {
            let r = scene.node_rect(Key::new("ctl").with(key)).unwrap();
            let mut reported = scene.state.clone();
            set_slider(&mut reported, slider, start, true);
            scene.set_state(reported.clone());
            assert!(scene.input(false, &[(0, r.cx(), r.cy())], now));
            now += REPEAT_DELAY;
            scene.update(now, Utc::now());
            assert_eq!(scene.take_level_feedback(), None);
            for (percent, effect) in [
                (step, Some(LevelFeedback::Tick)),
                (step, None),
                (end, Some(limit)),
                (end, None),
            ] {
                set_slider(&mut reported, slider, percent, true);
                scene.set_state(reported.clone());
                now += REPEAT_EVERY;
                scene.update(now, Utc::now());
                assert_eq!(scene.take_level_feedback(), effect);
            }
            scene.input(false, &[], now);
            // A fresh hold already at the limit gets one stop cue as repeat begins.
            scene.input(false, &[(0, r.cx(), r.cy())], now);
            now += REPEAT_DELAY;
            scene.update(now, Utc::now());
            assert_eq!(scene.take_level_feedback(), Some(limit));
            scene.input(false, &[], now);
            set_slider(&mut reported, slider, start, true);
            scene.set_state(reported);
            now += REPEAT_EVERY;
            scene.update(now, Utc::now());
            assert_eq!(scene.take_level_feedback(), None);
        }
    }

    #[test]
    fn panel_sliders_tick_per_new_percentage_and_stop_at_their_own_limits() {
        // The last case has no monitor helper: the laptop backlight
        // slider, keyed by Hyprland's focused screen, goes down to 0.
        for (panel, key, minimum, without_helper) in [
            (Panel::Audio, Key::new("p/volume"), 0, false),
            (
                Panel::Brightness,
                Key::new("p/display").with("eDP-1"),
                1,
                false,
            ),
            (Panel::Brightness, Key::new("p/keyboard"), 0, false),
            (
                Panel::Brightness,
                Key::new("p/display").with("eDP-1"),
                0,
                true,
            ),
        ] {
            let (mut scene, mut now) = settled_scene();
            if without_helper {
                scene
                    .state_mut()
                    .display_panel
                    .as_mut()
                    .unwrap()
                    .monitors
                    .clear();
            }
            scene.command(Command::OpenPanel(panel));
            for _ in 0..60 {
                now += Duration::from_millis(16);
                scene.update(now, Utc::now());
            }
            let r = scene.node_rect(key).unwrap();
            for (percent, effect) in [
                (50, Some(LevelFeedback::Tick)),
                (51, Some(LevelFeedback::Tick)),
                (51, None),
                (100, Some(LevelFeedback::Maximum)),
                (100, None),
                (99, Some(LevelFeedback::Tick)),
                (minimum + 1, Some(LevelFeedback::Tick)),
                (minimum, Some(LevelFeedback::Minimum)),
                (0, None),
            ] {
                now += Duration::from_millis(50);
                let x = r.x + 6.0 + (r.w - 12.0) * percent as f32 / 100.0;
                scene.input(false, &[(0, x, r.cy())], now);
                assert_eq!(scene.take_level_feedback(), effect, "{key:?} at {percent}%");
            }
            scene.input(false, &[], now);
            assert_eq!(scene.take_level_feedback(), None);
        }
    }

    #[test]
    fn bluetooth_panel_pairs_forgets_and_reports_failures() {
        let (mut scene, mut now) = settled_scene();
        let utc = Utc::now();
        let wait = |scene: &mut Scene, now: &mut Instant, ms: u64| {
            for _ in 0..ms / 16 {
                *now += Duration::from_millis(16);
                scene.update(*now, utc);
            }
        };
        let touch = |scene: &mut Scene, now: &mut Instant, key: Key| {
            let r = scene.node_rect(key).expect("node on the Bluetooth panel");
            scene.input(false, &[(0, r.cx(), r.cy())], *now);
            *now += Duration::from_millis(40);
            scene.update(*now, utc);
            scene.input(false, &[], *now);
            *now += Duration::from_millis(16);
            scene.update(*now, utc);
            scene.take_actions()
        };
        let dev = |address: &str| Key::new("p/bt/dev").with(address);
        let forget = |address: &str| Key::new("p/bt/forget").with(address);
        assert!(!scene.wants_bluetooth_scan());
        scene.command(Command::OpenPanel(Panel::Bluetooth));
        wait(&mut scene, &mut now, 1000);
        scene.take_actions();
        assert!(scene.wants_bluetooth_scan());

        // Tapping a nearby device pairs it; while busy it takes no taps.
        let speaker = "AA:BB:CC:DD:EE:04";
        assert_eq!(
            touch(&mut scene, &mut now, dev(speaker)),
            [Action::BluetoothDevice {
                address: speaker.into(),
                op: BtOp::Pair
            }]
        );
        assert!(scene.bt.pending.contains_key(speaker));
        assert!(touch(&mut scene, &mut now, dev(speaker)).is_empty());
        // The panel waits for it well past the usual timeout.
        wait(&mut scene, &mut now, 15_000);
        assert_eq!(scene.page, Page::Panel(Panel::Bluetooth));

        // Once it shows up connected it settles, and audio moves to it as
        // soon as PipeWire lists its output.
        let mut state = scene.state.clone();
        let i = state
            .bluetooth
            .nearby
            .iter()
            .position(|d| d.address == speaker)
            .unwrap();
        let mut device = state.bluetooth.nearby.remove(i);
        device.paired = true;
        device.connected = true;
        state.bluetooth.devices.push(device);
        scene.set_state(state.clone());
        assert!(scene.bt.pending.is_empty());
        assert!(scene.take_actions().is_empty());
        let node = "bluez_output.AA_BB_CC_DD_EE_04.1";
        state.mixer.outputs.push(crate::state::AudioDevice {
            id: 120,
            name: node.into(),
            label: "JBL Flip 6".into(),
            kind: crate::state::AudioDeviceKind::Bluetooth,
            default: false,
        });
        scene.set_state(state);
        assert_eq!(
            scene.take_actions(),
            [Action::SetDefaultOutput {
                id: 120,
                name: node.into()
            }]
        );
        wait(&mut scene, &mut now, 600);

        // Forget asks first, and the question lapses.
        let mouse = "AA:BB:CC:DD:EE:02";
        let keyboard = "AA:BB:CC:DD:EE:03";
        assert!(touch(&mut scene, &mut now, forget(keyboard)).is_empty());
        wait(&mut scene, &mut now, 3200);
        assert!(scene.bt.forget_armed.is_none());
        assert!(touch(&mut scene, &mut now, forget(mouse)).is_empty());
        wait(&mut scene, &mut now, 400);
        assert_eq!(
            touch(&mut scene, &mut now, forget(mouse)),
            [Action::BluetoothDevice {
                address: mouse.into(),
                op: BtOp::Forget
            }]
        );
        wait(&mut scene, &mut now, 600);
        assert!(scene.node_rect(forget(mouse)).is_none());

        // A failure shows on the device, then clears.
        assert_eq!(
            touch(&mut scene, &mut now, dev(keyboard)),
            [Action::BluetoothDevice {
                address: keyboard.into(),
                op: BtOp::Connect
            }]
        );
        scene.apply_bluetooth_done(keyboard, BtOp::Connect, false);
        assert!(!scene.bt.pending.contains_key(keyboard));
        assert!(scene.bt.failed.contains_key(keyboard));
        wait(&mut scene, &mut now, 4500);
        assert!(scene.bt.failed.is_empty());

        // Scanning holds the panel open for a minute, then it closes and
        // the scan stops.
        scene.apply_bluetooth_done(mouse, BtOp::Forget, true);
        wait(&mut scene, &mut now, 30_000);
        assert_eq!(scene.page, Page::Panel(Panel::Bluetooth));
        wait(&mut scene, &mut now, 32_000);
        assert_ne!(scene.page, Page::Panel(Panel::Bluetooth));
        assert!(!scene.wants_bluetooth_scan());

        // Desktop activity keeps panels open, but not the scan.
        scene.command(Command::OpenPanel(Panel::Bluetooth));
        for _ in 0..70 {
            // What `wake` does on a Hyprland event, on the test's clock.
            scene.last_touch = now;
            wait(&mut scene, &mut now, 1000);
        }
        assert_ne!(scene.page, Page::Panel(Panel::Bluetooth));

        // With Bluetooth off the panel closes on the usual timeout.
        scene.command(Command::OpenPanel(Panel::Bluetooth));
        scene.state.bluetooth.powered = false;
        assert!(!scene.wants_bluetooth_scan());
        wait(&mut scene, &mut now, 11_000);
        assert_ne!(scene.page, Page::Panel(Panel::Bluetooth));
    }

    #[test]
    fn bluez_outputs_match_their_device() {
        let airpods = "AC:80:0A:00:00:01";
        assert!(bluez_node_matches(
            "bluez_output.AC_80_0A_00_00_01.1",
            airpods
        ));
        assert!(bluez_node_matches(
            "bluez_output.ac_80_0a_00_00_01.a2dp-sink",
            airpods
        ));
        assert!(!bluez_node_matches(
            "bluez_input.AC_80_0A_00_00_01.0",
            airpods
        ));
        assert!(!bluez_node_matches(
            "bluez_output.AC_80_0A_00_00_02.1",
            airpods
        ));
        assert!(!bluez_node_matches("bluez_output.AC_80", airpods));
    }

    #[test]
    fn sound_panel_picks_devices_and_sets_mic_and_app_volume() {
        let (mut scene, mut now) = settled_scene();
        let utc = Utc::now();
        let settle = |scene: &mut Scene, now: &mut Instant| {
            for _ in 0..60 {
                *now += Duration::from_millis(16);
                scene.update(*now, utc);
            }
        };
        let touch = |scene: &mut Scene, now: &mut Instant, key: Key, at: f32| {
            let r = scene.node_rect(key).expect("node on the Sound panel");
            scene.input(false, &[(0, r.x + r.w * at, r.cy())], *now);
            *now += Duration::from_millis(40);
            scene.update(*now, utc);
            scene.input(false, &[], *now);
            *now += Duration::from_millis(16);
            scene.update(*now, utc);
            scene.take_actions()
        };
        scene.command(Command::OpenPanel(Panel::Audio));
        settle(&mut scene, &mut now);
        scene.take_actions();

        // Output: choose another device.
        let airpods = "bluez_output.AC_80_0A_00_00_01.1";
        let actions = touch(
            &mut scene,
            &mut now,
            Key::new("p/snd/out").with(airpods),
            0.5,
        );
        assert!(actions.contains(&Action::SetDefaultOutput {
            id: 91,
            name: airpods.into()
        }));
        let outputs = &scene.state.mixer.outputs;
        assert!(outputs.iter().all(|d| d.default == (d.id == 91)));
        assert_eq!(scene.state.audio.as_ref().unwrap().sink, "AirPods Pro");

        // Input: the microphone slider keeps the mute state.
        let tab = |name: &str| Key::new("p/snd/tab").with("opt").with(name);
        touch(&mut scene, &mut now, tab("Input"), 0.5);
        assert_eq!(scene.sound_tab, SoundTab::Input);
        settle(&mut scene, &mut now);
        scene.state.mic_muted = Some(true);
        let actions = touch(&mut scene, &mut now, Key::new("p/mic"), 0.25);
        let Some(Action::SetMicVolume(level)) = actions.last() else {
            panic!("expected a mic volume, got {actions:?}");
        };
        assert!((20..=30).contains(level), "mic volume {level}");
        assert_eq!(scene.state.mic_muted, Some(true));
        assert!(
            scene
                .node_rect(Key::new("p/snd/in").with("bluez_input.AC_80_0A_00_00_01.0"))
                .is_some()
        );

        // Sources: each app has its own volume and mute.
        touch(&mut scene, &mut now, tab("Sources"), 0.5);
        settle(&mut scene, &mut now);
        let app = |id: i64| Key::new("p/snd/app").index(id);
        let actions = touch(&mut scene, &mut now, app(133).with("volume"), 0.8);
        let Some(Action::SetStreamVolume { id: 133, volume }) = actions.last() else {
            panic!("expected Firefox's volume, got {actions:?}");
        };
        assert!((75..=85).contains(volume), "app volume {volume}");
        let actions = touch(&mut scene, &mut now, app(141), 0.5);
        assert_eq!(actions, [Action::ToggleStreamMute(141)]);
        assert!(
            !scene
                .state
                .mixer
                .streams
                .iter()
                .find(|s| s.id == 141)
                .unwrap()
                .muted
        );

        // Leaving the panel returns it to Output next time.
        scene.command(Command::Home);
        scene.command(Command::OpenPanel(Panel::Audio));
        assert_eq!(scene.sound_tab, SoundTab::Output);
    }

    #[test]
    fn display_panel_targets_the_focused_monitor_and_protects_the_last_output() {
        let (mut scene, mut now) = settled_scene();
        scene.command(Command::OpenPanel(Panel::Brightness));
        let settle = |scene: &mut Scene, now: &mut Instant| {
            for _ in 0..60 {
                *now += Duration::from_millis(16);
                scene.update(*now, Utc::now());
            }
        };
        settle(&mut scene, &mut now);
        scene.take_actions();
        let tap = |scene: &mut Scene, key: Key, fraction: f32, now: Instant| {
            let r = scene.node_rect(key).unwrap();
            let x = r.x + r.w * fraction - scene.scroll.offset;
            assert!(x >= scene.scroll.x0 && x < scene.scroll.x1);
            scene.input(false, &[(0, x, r.cy())], now);
            scene.input(false, &[], now);
            scene.take_actions()
        };
        let brightness = Key::new("p/display").with("eDP-1");
        let actions = tap(&mut scene, brightness, 0.01, now);
        assert!(
            actions.contains(&Action::ConfigureDisplay(DisplayChange::Brightness {
                monitor: "eDP-1".into(),
                percent: 1,
            }))
        );
        let size = Key::new("p/display/text").with("opt").with("16");
        assert_eq!(
            tap(&mut scene, size, 0.5, now),
            [Action::ConfigureDisplay(DisplayChange::TextSize(16))]
        );
        scene.set_scroll_offset(f32::MAX);
        settle(&mut scene, &mut now);
        let scale = Key::new("p/display/scale").with("opt").with("160");
        assert_eq!(
            tap(&mut scene, scale, 0.5, now),
            [Action::ConfigureDisplay(DisplayChange::Scale {
                monitor: "eDP-1".into(),
                preset: 160,
            })]
        );
        let output = Key::new("p/display/output");
        assert!(tap(&mut scene, output.with("eDP-1"), 0.5, now).is_empty());
        assert_eq!(
            tap(&mut scene, output.with("DP-1"), 0.5, now),
            [Action::ConfigureDisplay(DisplayChange::Enabled {
                monitor: "DP-1".into(),
                enabled: true,
            })]
        );

        // A fixed-brightness output still has text and scale controls. An
        // old laptop drag cannot become a command for the new monitor.
        let panel = scene.state.display_panel.as_mut().unwrap();
        panel.monitors[0].focused = false;
        panel.monitors[1].focused = true;
        panel.monitors[1].enabled = true;
        panel.brightness = None;
        let nodes = scene.build();
        assert!(!nodes.iter().any(|n| n.key == brightness));
        assert!(nodes.iter().any(|n| n.key == size));
        assert!(nodes.iter().any(|n| n.key == scale));
        scene.slide(
            Slider::Monitor(Key::new("eDP-1")),
            brightness,
            100.0,
            now,
            true,
        );
        assert!(scene.take_actions().is_empty());
    }

    #[test]
    fn display_panel_fits_a_lone_laptop_and_names_its_brightness() {
        let (mut scene, mut now) = settled_scene();
        scene
            .state_mut()
            .display_panel
            .as_mut()
            .unwrap()
            .monitors
            .truncate(1);
        scene.command(Command::OpenPanel(Panel::Brightness));
        for _ in 0..60 {
            now += Duration::from_millis(16);
            scene.update(now, Utc::now());
        }
        scene.take_actions();
        // Every control is on the strip without scrolling.
        assert_eq!(scene.scroll.max, 0.0);

        let texts = |scene: &mut Scene, key: Key| -> (Vec<String>, f32) {
            let nodes = scene.build();
            let node = nodes.iter().find(|n| n.key == key).expect("node");
            let texts = node
                .parts
                .iter()
                .filter_map(|p| match p {
                    crate::node::Part::Text { text, role, .. }
                        if *role != crate::text::Role::Icon =>
                    {
                        Some(text.clone())
                    }
                    _ => None,
                })
                .collect();
            (texts, node.rect.w)
        };
        let hero = Key::new("p/display/hero");
        let (before, width) = texts(&mut scene, hero);
        assert_eq!(before, ["Built-in Display", "Golden hour"]);

        // Dimming renames the mood without moving the slider under the finger.
        let r = scene
            .node_rect(Key::new("p/display").with("eDP-1"))
            .unwrap();
        scene.input(false, &[(0, r.x + 2.0, r.cy())], now);
        let (during, during_width) = texts(&mut scene, hero);
        assert_eq!(during[1], "Night owl");
        assert_eq!(during_width, width);
        scene.input(false, &[], now);
        scene.take_actions();

        // Night Light toggles from the panel, like the indicator.
        let r = scene.node_rect(Key::new("p/display/night")).unwrap();
        scene.input(false, &[(0, r.cx(), r.cy())], now);
        scene.input(false, &[], now);
        assert_eq!(
            scene.take_actions(),
            [Action::TriggerIndicator {
                indicator: Indicator::NightLight,
                currently_active: true,
            }]
        );
        assert!(!scene.state.indicators.nightlight);
    }

    #[test]
    fn panel_controls_grow_in_as_one_piece() {
        let (mut scene, _) = settled_scene();
        scene.command(Command::OpenPanel(Panel::Brightness));
        let nodes = scene.build();
        let enter = |key: Key| {
            let node = nodes.iter().find(|n| n.key == key).expect("node");
            (node.rect, node.enter.expect("enter motion"))
        };
        let well = Key::new("p/display/text");
        let (outer, base) = enter(well);
        for key in [
            well.with("pill"),
            well.with("opt").with("9"),
            well.with("opt").with("20"),
        ] {
            let (rect, motion) = enter(key);
            assert_eq!(motion.delay, base.delay, "{key:?} waits with its control");
            assert_eq!(motion.scale, base.scale);
            // Scaled about the control's center, so it keeps its place in it.
            let gap = (rect.cx() + motion.dx) - (outer.cx() + base.dx);
            assert!((gap - (rect.cx() - outer.cx()) * base.scale).abs() < 0.01);
        }
        // Controls still cascade one after another.
        let (_, scale) = enter(Key::new("p/display/scale"));
        assert!(scale.delay > base.delay);
    }

    #[test]
    fn display_panel_names_external_screens_by_model() {
        let (mut scene, mut now) = settled_scene();
        scene.command(Command::OpenPanel(Panel::Brightness));
        for _ in 0..60 {
            now += Duration::from_millis(16);
            scene.update(now, Utc::now());
        }
        let nodes = scene.build();
        let card = |name: &str| {
            nodes
                .iter()
                .find(|n| n.key == Key::new("p/display/output").with(name))
                .unwrap()
                .parts
                .iter()
                .filter_map(|p| match p {
                    crate::node::Part::Text { text, role, .. }
                        if *role != crate::text::Role::Icon =>
                    {
                        Some(text.as_str())
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(card("eDP-1"), ["Built-in Display", "Focused"]);
        assert_eq!(card("DP-1"), ["LG ULTRAFINE", "Off · DP-1"]);
    }

    #[test]
    fn quick_tap_on_audio_and_brightness_still_opens_panels() {
        let (mut scene, mut now) = settled_scene();
        let utc = Utc::now();
        let r = scene.node_rect(Key::new("w/audio")).expect("audio button");
        scene.input(false, &[(0, r.cx(), r.cy())], now);
        now += Duration::from_millis(80);
        scene.update(now, utc);
        assert!(scene.quick_slider.is_none());
        scene.input(false, &[], now);
        assert_eq!(scene.page(), &Page::Panel(Panel::Audio));
        assert_eq!(scene.take_level_feedback(), None);

        scene.command(Command::Home);
        for _ in 0..80 {
            now += Duration::from_millis(16);
            scene.update(now, utc);
        }
        let r = scene
            .node_rect(Key::new("w/brightness"))
            .expect("brightness button");
        scene.input(false, &[(0, r.cx(), r.cy())], now);
        now += Duration::from_millis(90);
        scene.update(now, utc);
        assert!(scene.quick_slider.is_none());
        scene.input(false, &[], now);
        assert_eq!(scene.page(), &Page::Panel(Panel::Brightness));
        assert_eq!(scene.take_level_feedback(), None);
    }

    #[test]
    fn holding_audio_transforms_into_slider_and_adjusts_on_slide() {
        let (mut scene, mut now) = settled_scene();
        let utc = Utc::now();
        let r = scene.node_rect(Key::new("w/audio")).expect("audio button");
        let start_vol = scene.state.audio.as_ref().unwrap().volume;

        scene.input(false, &[(0, r.cx(), r.cy())], now);
        // next_deadline wakes for the hold timer even when idle.
        assert!(scene.next_deadline(now) <= QUICK_SLIDER_HOLD);

        // Hold past threshold without moving -> transforms without changing volume.
        now += QUICK_SLIDER_HOLD + Duration::from_millis(10);
        scene.update(now, utc);
        let qs = scene
            .quick_slider
            .as_ref()
            .expect("hold activates quick slider");
        assert_eq!(qs.slider, Slider::Volume);
        assert_eq!(qs.key, Key::new("w/audio"));
        assert!(!qs.changed);
        assert_eq!(scene.state.audio.as_ref().unwrap().volume, start_vol);
        assert!(scene.take_actions().is_empty());
        assert_eq!(
            scene.take_level_feedback(),
            Some(LevelFeedback::SliderOpened)
        );
        now += Duration::from_millis(60);
        scene.input(false, &[(0, r.cx(), r.cy())], now);
        scene.update(now, utc);
        assert_eq!(scene.take_level_feedback(), None);

        // Built bar replaces w/audio with expanded slider and hides overlapped neighbors.
        let nodes = scene.build();
        let slider_node = nodes
            .iter()
            .find(|n| n.key == Key::new("w/audio"))
            .expect("morphed audio slider node");
        assert!(slider_node.rect.w > r.w * 3.0);
        assert!(
            !nodes.iter().any(|n| n.key == Key::new("w/brightness")),
            "overlapping neighbor should be tucked away"
        );
        assert!(
            nodes.iter().any(|n| n.key == Key::new("w/menu")),
            "non-overlapping widgets should stay on the bar"
        );

        // Slide right to increase volume.
        now += Duration::from_millis(60);
        scene.input(false, &[(0, r.cx() + 120.0, r.cy())], now);
        scene.update(now, utc);
        let raised = scene.state.audio.as_ref().unwrap().volume;
        assert_eq!(
            scene.take_level_feedback(),
            Some(LevelFeedback::at(raised, 0))
        );
        assert!(
            raised > start_vol + 10,
            "expected volume > {}, got {raised}",
            start_vol + 10
        );
        assert!(
            scene
                .take_actions()
                .iter()
                .any(|a| matches!(a, Action::SetVolume(v) if *v == raised))
        );

        // Release returns to normal bar without opening the panel.
        now += Duration::from_millis(50);
        scene.input(false, &[], now);
        scene.update(now, utc);
        assert!(scene.quick_slider.is_none());
        assert_eq!(scene.page(), &Page::Bar);
        let nodes = scene.build();
        assert!(nodes.iter().any(|n| n.key == Key::new("w/brightness")));
    }

    #[test]
    fn immediate_horizontal_drag_auto_detects_slider_and_reaches_full_range() {
        let (mut scene, mut now) = settled_scene();
        let utc = Utc::now();
        // Start near the right edge at low brightness (15%) to exercise catch-up to 100%.
        scene.state_mut().display_brightness = Some(15);
        for _ in 0..40 {
            now += Duration::from_millis(16);
            scene.update(now, utc);
        }
        let r = scene
            .node_rect(Key::new("w/brightness"))
            .expect("brightness button");
        let _ = scene.take_actions();

        // Press down and immediately slide right 18px (well before QUICK_SLIDER_HOLD).
        scene.input(false, &[(0, r.cx(), r.cy())], now);
        now += Duration::from_millis(16);
        scene.input(false, &[(0, r.cx() + 18.0, r.cy())], now);
        scene.update(now, utc);
        let qs = scene
            .quick_slider
            .as_ref()
            .expect("horizontal drag immediately activates quick slider");
        assert_eq!(qs.slider, Slider::Display);
        let track_l = qs.rect.x + 6.0;
        let track_r = qs.rect.right() - 6.0;
        let after_start = scene.state.display_brightness.unwrap();
        assert_eq!(
            scene.take_level_feedback(),
            Some(LevelFeedback::SliderOpened)
        );
        assert!(
            after_start > 15 && after_start < 40,
            "smooth initial move, got {after_start}"
        );

        // Drag smoothly to the right edge -> ticks, then reaches the maximum.
        for step in 1..=12 {
            now += Duration::from_millis(20);
            let x = r.cx() + 18.0 + (track_r - (r.cx() + 18.0)) * (step as f32 / 12.0);
            scene.input(false, &[(0, x, r.cy())], now);
            scene.update(now, utc);
            let level = scene.state.display_brightness.unwrap();
            assert_eq!(
                scene.take_level_feedback(),
                Some(LevelFeedback::at(level, 0))
            );
        }
        assert_eq!(scene.state.display_brightness, Some(100));

        // Drag smoothly back to the left edge -> reaches 0% cleanly.
        for step in 1..=16 {
            now += Duration::from_millis(20);
            let x = track_r - (track_r - track_l) * (step as f32 / 16.0);
            scene.input(false, &[(0, x, r.cy())], now);
            scene.update(now, utc);
        }
        assert_eq!(scene.state.display_brightness, Some(0));
        assert_eq!(scene.take_level_feedback(), Some(LevelFeedback::Minimum));

        // Release -> final action emitted, morphs back to bar.
        now += Duration::from_millis(20);
        scene.input(false, &[], now);
        scene.update(now, utc);
        assert!(scene.quick_slider.is_none());
        assert_eq!(scene.page(), &Page::Bar);
        assert!(
            scene
                .take_actions()
                .contains(&Action::SetDisplayBrightness(0))
        );
        assert_eq!(scene.take_level_feedback(), None);
    }

    #[test]
    fn hold_without_move_keeps_muted_and_fn_interrupt_does_not_ghost_tap() {
        let (mut scene, mut now) = settled_scene();
        let utc = Utc::now();
        if let Some(a) = scene.state_mut().audio.as_mut() {
            a.muted = true;
            a.volume = 60;
        }
        for _ in 0..20 {
            now += Duration::from_millis(16);
            scene.update(now, utc);
        }
        let r = scene.node_rect(Key::new("w/audio")).expect("audio button");
        let _ = scene.take_actions();

        // Hold and lift without moving -> remains muted, no action, no panel open.
        scene.input(false, &[(0, r.cx(), r.cy())], now);
        now += QUICK_SLIDER_HOLD + Duration::from_millis(10);
        scene.update(now, utc);
        assert!(scene.quick_slider.is_some());
        now += Duration::from_millis(30);
        scene.input(false, &[], now);
        scene.update(now, utc);
        assert!(scene.quick_slider.is_none());
        assert!(scene.state.audio.as_ref().unwrap().muted);
        assert!(scene.take_actions().is_empty());
        assert_eq!(scene.page(), &Page::Bar);

        // Hold, then press Fn while finger still down -> cancels cleanly, lifting finger does not open panel.
        scene.input(false, &[(0, r.cx(), r.cy())], now);
        now += QUICK_SLIDER_HOLD + Duration::from_millis(10);
        scene.update(now, utc);
        assert!(scene.quick_slider.is_some());
        scene.input(true, &[(0, r.cx(), r.cy())], now);
        scene.update(now, utc);
        assert!(scene.quick_slider.is_none());
        scene.input(false, &[], now);
        scene.update(now, utc);
        assert_eq!(scene.page(), &Page::Bar);
    }

    #[test]
    fn agents_panel_shows_reset_countdown_and_updates_on_tick_and_expiry() {
        use crate::{
            node::Part,
            state::{AgentLimit, AgentUsage},
        };

        let t0 = DateTime::parse_from_rfc3339("2026-11-26T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let mut scene = test_scene();
        let mut cfg = scene.config().clone();
        cfg.power.panel_timeout = 600;
        cfg.power.dim_after = 600;
        cfg.power.off_after = 600;
        scene.set_config(cfg);
        let mut state = DesktopState::sample();
        state.agents = vec![AgentUsage {
            id: "claude".into(),
            name: "Claude Code".into(),
            plan: "Pro".into(),
            has_local_stats: true,
            today_tokens: 148_121_357,
            limits: vec![
                AgentLimit {
                    window: "Session".into(),
                    used: 92,
                    resets_at: Some(
                        t0 + chrono::Duration::minutes(2) + chrono::Duration::seconds(30),
                    ),
                },
                AgentLimit {
                    window: "Weekly".into(),
                    used: 40,
                    resets_at: Some(t0 + chrono::Duration::hours(4 * 24 + 19)),
                },
                AgentLimit {
                    window: "Extra".into(),
                    used: 15,
                    resets_at: None,
                },
            ],
            limits_stale: false,
            ..AgentUsage::default()
        }];
        scene.set_state(state);

        let mut now = Instant::now();
        scene.command(Command::OpenPanel(Panel::Agents));
        scene.update(now, t0);

        let texts = |scene: &mut Scene| -> Vec<String> {
            scene
                .build()
                .into_iter()
                .flat_map(|n| n.parts)
                .filter_map(|p| match p {
                    Part::Text { text, .. } => Some(text),
                    _ => None,
                })
                .collect()
        };
        let has = |texts: &[String], t: &str| texts.iter().any(|x| x == t);
        let countdowns =
            |texts: &[String]| texts.iter().filter(|t| t.starts_with("resets in")).count();

        let initial = texts(&mut scene);
        for expected in [
            "Pro · 148.1M tokens today",
            "92%",
            "resets in 2m",
            "40%",
            "resets in 4d 19h",
            "15%",
        ] {
            assert!(
                has(&initial, expected),
                "missing {expected:?} in {initial:?}"
            );
        }
        assert_eq!(
            countdowns(&initial),
            2,
            "a limit without reset has no countdown"
        );

        // Next deadline should wake right when 2m -> 1m (30s later), before the 60s clock tick.
        assert!(scene.next_deadline(now) <= Duration::from_secs(31));

        // Advance past the 2m -> 1m boundary (within the same wall-clock minute).
        now += Duration::from_secs(31);
        assert!(scene.update(now, t0 + chrono::Duration::seconds(31)));
        let ticked = texts(&mut scene);
        assert!(
            has(&ticked, "resets in 1m"),
            "countdown should tick in {ticked:?}"
        );

        // Advance past expiry -> resets to 0% used without a reset countdown.
        now += Duration::from_secs(125);
        assert!(scene.update(now, t0 + chrono::Duration::seconds(156)));
        let expired = texts(&mut scene);
        assert!(
            has(&expired, "0%"),
            "expired limit should drop to 0% in {expired:?}"
        );
        assert!(
            !has(&expired, "92%"),
            "expired limit still shows 92% in {expired:?}"
        );
        assert_eq!(countdowns(&expired), 1);

        // Stale limits keep their countdown and fade their meters.
        let tracks = |scene: &mut Scene| -> Vec<crate::color::Rgba> {
            scene
                .build()
                .into_iter()
                .flat_map(|n| n.parts)
                .filter_map(|p| match p {
                    Part::Progress { color, .. } => Some(color),
                    _ => None,
                })
                .collect()
        };
        let live = tracks(&mut scene);
        assert_eq!(live.len(), 3);
        assert!(live.iter().all(|c| *c == scene.theme.accent));
        scene.state_mut().agents[0].limits_stale = true;
        scene.update(now, t0 + chrono::Duration::seconds(156));
        assert!(has(&texts(&mut scene), "resets in 4d 18h"));
        assert!(
            tracks(&mut scene)
                .iter()
                .all(|c| *c == scene.theme.fg_faint)
        );
    }

    #[test]
    fn agents_refresh_shimmers_until_new_records_land() {
        use crate::node::Part;

        let (mut scene, mut now) = settled_scene();
        let utc = Utc::now();
        let mut cfg = scene.config().clone();
        cfg.power.panel_timeout = 600;
        scene.set_config(cfg);
        scene.command(Command::OpenPanel(Panel::Agents));
        scene.update(now, utc);
        let shimmering = |scene: &mut Scene| {
            scene
                .build()
                .iter()
                .find(|n| n.key == Key::new("p/agents/refresh"))
                .expect("refresh button")
                .parts
                .iter()
                .any(|p| matches!(p, Part::Shimmer { .. }))
        };
        assert!(!shimmering(&mut scene));

        scene.take_actions();
        scene.exec(Cmd::Do(Action::RefreshAgents { disabled: vec![] }));
        assert!(matches!(
            scene.take_actions().as_slice(),
            [Action::RefreshAgents { .. }]
        ));
        assert!(shimmering(&mut scene));
        // It keeps animating while it waits.
        now += Duration::from_millis(16);
        scene.update(now, utc);
        assert!(scene.animating());

        let mut state = scene.state().clone();
        state.agents[0].today_tokens += 1;
        scene.set_state(state);
        assert!(!shimmering(&mut scene));

        // Without new records it gives up on its own.
        scene.exec(Cmd::Do(Action::RefreshAgents { disabled: vec![] }));
        now += crate::build::AGENTS_REFRESH + Duration::from_millis(10);
        scene.update(now, utc);
        assert!(!shimmering(&mut scene));
    }

    #[test]
    fn compact_indicators_button_always_present_and_opens_panel() {
        let (mut scene, mut now) = settled_scene();
        let utc = Utc::now();
        // Turn off every indicator: the trigger pill must still be present on the bar.
        scene.state_mut().indicators = Default::default();
        for _ in 0..20 {
            now += Duration::from_millis(16);
            scene.update(now, utc);
        }
        let r = scene
            .node_rect(Key::new("w/indicators"))
            .expect("indicators pill should be visible even when none are active");
        scene.input(false, &[(0, r.cx(), r.cy())], now);
        now += Duration::from_millis(60);
        scene.update(now, utc);
        scene.input(false, &[], now);
        assert_eq!(scene.page(), &Page::Panel(Panel::Indicators));
        assert!(
            scene
                .take_actions()
                .contains(&Action::PanelOpened(Panel::Indicators))
        );

        // Inside Panel::Indicators, Stay Awake, Passwordless Sudo, Night Light,
        // DND, Screen Record, and Reminder are all present, while inactive
        // RemoteSession stays hidden.
        let nodes = scene.build();
        for key in [
            "p/ind/sudo",
            "p/ind/rec",
            "p/ind/reminder",
            "p/ind/night",
            "p/ind/dnd",
            "p/ind/awake",
        ] {
            assert!(
                nodes.iter().any(|n| n.key == Key::new(key)),
                "missing indicator control {key}"
            );
        }
        assert!(
            !nodes.iter().any(|n| n.key == Key::new("p/ind/remote")),
            "inactive remote session should not appear"
        );

        // Toggling Stay Awake flips state optimistically and emits TriggerIndicator.
        scene.exec(Cmd::TriggerIndicator(Indicator::StayAwake));
        assert!(scene.state.indicators.stay_awake);
        assert!(scene.take_actions().contains(&Action::TriggerIndicator {
            indicator: Indicator::StayAwake,
            currently_active: false,
        }));
        assert_eq!(scene.page(), &Page::Panel(Panel::Indicators));

        // Tapping inactive Passwordless Sudo emits enable action and returns home.
        scene.exec(Cmd::TriggerIndicator(Indicator::PasswordlessSudo));
        assert!(scene.take_actions().contains(&Action::TriggerIndicator {
            indicator: Indicator::PasswordlessSudo,
            currently_active: false,
        }));
        assert_eq!(scene.page(), &Page::Bar);

        // When active, Passwordless Sudo emits disable action without leaving panel.
        scene.command(Command::OpenPanel(Panel::Indicators));
        let _ = scene.take_actions();
        scene.state_mut().indicators.passwordless_sudo = true;
        scene.exec(Cmd::TriggerIndicator(Indicator::PasswordlessSudo));
        assert!(scene.take_actions().contains(&Action::TriggerIndicator {
            indicator: Indicator::PasswordlessSudo,
            currently_active: true,
        }));
        assert_eq!(scene.page(), &Page::Panel(Panel::Indicators));
    }

    #[test]
    fn inline_indicators_render_dimmed_inactive_buttons_and_trigger_directly() {
        let mut config = bar_config();
        config.appearance.always_show_indicators = true;
        let mut scene = Scene::new(
            T1_WIDTH,
            T1_HEIGHT,
            config,
            Palette::fallback(),
            Fonts::from_files(None, None, None, None, None),
            MenuTree::default(),
            None,
        );
        let mut state = DesktopState::sample();
        state.indicators.stay_awake = false;
        state.indicators.nightlight = true;
        state.indicators.remote_session = false;
        scene.set_state(state);

        let nodes = scene.build();
        let awake = nodes
            .iter()
            .find(|n| n.key == Key::new("w/ind").with("awake"))
            .expect("inline stay awake button");
        assert!(
            awake.opacity < 0.7,
            "inactive inline indicator should be dimmed"
        );
        let night = nodes
            .iter()
            .find(|n| n.key == Key::new("w/ind").with("night"))
            .expect("inline night light button");
        assert_eq!(
            night.opacity, 1.0,
            "active inline indicator should be full opacity"
        );
        assert!(
            nodes
                .iter()
                .any(|n| n.key == Key::new("w/ind").with("sudo")),
            "inline passwordless sudo button should be present"
        );
        assert!(
            !nodes
                .iter()
                .any(|n| n.key == Key::new("w/ind").with("remote")),
            "inactive remote session must not be revealed inline"
        );

        // When a remote session becomes active, it appears inline too.
        scene.state_mut().indicators.remote_session = true;
        let nodes = scene.build();
        assert!(
            nodes
                .iter()
                .any(|n| n.key == Key::new("w/ind").with("remote"))
        );
    }

    #[test]
    fn screen_recording_indicator_opens_touchbar_submenu_when_available() {
        let menu = MenuTree::from_jsonc(
            r#"{
                "trigger.capture.screenrecord": {"label": "Screenrecord"},
                "trigger.capture.screenrecord.no-audio": {"label": "With no audio", "action": "omarchy-capture-screenrecording"}
            }"#,
        )
        .unwrap();
        let mut scene = Scene::new(
            T1_WIDTH,
            T1_HEIGHT,
            bar_config(),
            Palette::fallback(),
            Fonts::from_files(None, None, None, None, None),
            menu,
            None,
        );
        scene.set_state(DesktopState::sample());
        scene.exec(Cmd::TriggerIndicator(Indicator::ScreenRecording));
        assert_eq!(
            scene.page(),
            &Page::Menu("trigger.capture.screenrecord".into())
        );

        // When already recording, triggering stops it immediately instead.
        scene.command(Command::OpenPanel(Panel::Indicators));
        let _ = scene.take_actions();
        scene.state_mut().indicators.recording = true;
        scene.exec(Cmd::TriggerIndicator(Indicator::ScreenRecording));
        assert!(!scene.state.indicators.recording);
        assert!(scene.take_actions().contains(&Action::TriggerIndicator {
            indicator: Indicator::ScreenRecording,
            currently_active: true,
        }));
    }

    #[test]
    fn network_panel_shows_all_omarchy_details_dns_band_and_actions() {
        use crate::{
            node::Part,
            state::{Connectivity, DnsProvider},
        };

        let mut scene = test_scene();
        let mut now = Instant::now();
        let utc = Utc::now();

        assert!(!scene.wants_network_details());
        scene.command(Command::OpenPanel(Panel::Network));
        scene.update(now, utc);
        assert!(scene.wants_network_details());

        let nodes = scene.build();
        let texts: Vec<String> = nodes
            .iter()
            .flat_map(|n| &n.parts)
            .filter_map(|p| match p {
                Part::Text { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect();

        // All 8 Omarchy network metrics + DNS selector + Band selector + QR + Speed Test.
        for expected in [
            "Ping",
            "12 ms",
            "Packet loss",
            "0%",
            "Receiving",
            "1.4 MB/s",
            "Sending",
            "240.5 KB/s",
            "Downloaded",
            "1.42 GB",
            "Uploaded",
            "184.5 MB",
            "IP address",
            "192.168.1.42",
            "Gateway",
            "192.168.1.1",
            "Band",
            "Auto · 5GHz",
            "2.4GHz",
            "5GHz",
            "DNS",
            "DHCP",
            "Cloudflare",
            "Google",
            "Custom",
            "Speed Test",
        ] {
            assert!(
                texts.iter().any(|t| t == expected),
                "missing '{expected}' in network panel texts: {texts:?}"
            );
        }

        // Wi-Fi networks appear before telemetry stats, Band, and DNS, showing all
        // known networks plus up to 3 unknown networks with an expand arrow for the rest.
        let x_of = |key: Key| nodes.iter().find(|n| n.key == key).unwrap().rect.x;
        assert!(x_of(Key::new("p/net/ssid").with("Studio 5G")) < x_of(Key::new("p/net/stat/ping")));
        assert!(x_of(Key::new("p/net/stat/gateway")) < x_of(Key::new("p/net/dns")));
        assert!(
            nodes
                .iter()
                .any(|n| n.key == Key::new("p/net/ssid").with("Studio"))
        );
        assert!(
            nodes
                .iter()
                .any(|n| n.key == Key::new("p/net/ssid").with("Café Libre"))
        );
        assert!(
            nodes
                .iter()
                .any(|n| n.key == Key::new("p/net/ssid").with("NETGEAR-4F2A"))
        );
        assert!(
            nodes
                .iter()
                .any(|n| n.key == Key::new("p/net/ssid").with("Guest Wi-Fi"))
        );
        assert!(
            !nodes
                .iter()
                .any(|n| n.key == Key::new("p/net/ssid").with("TP-Link_5G_9A"))
        );
        assert!(
            !nodes
                .iter()
                .any(|n| n.key == Key::new("p/net/ssid").with("NeighborNet"))
        );
        assert!(nodes.iter().any(|n| n.key == Key::new("p/net/wifi/more")));
        assert!(!nodes.iter().any(|n| n.key == Key::new("p/net/more")));

        // Tapping the expand arrow reveals the remaining unknown networks inline,
        // and tapping it again collapses them back.
        scene.exec(Cmd::ToggleWifiNetworksExpanded);
        let expanded = scene.build();
        assert!(
            expanded
                .iter()
                .any(|n| n.key == Key::new("p/net/ssid").with("TP-Link_5G_9A"))
        );
        assert!(
            expanded
                .iter()
                .any(|n| n.key == Key::new("p/net/ssid").with("NeighborNet"))
        );
        assert!(
            expanded
                .iter()
                .any(|n| n.key == Key::new("p/net/wifi/more"))
        );
        scene.exec(Cmd::ToggleWifiNetworksExpanded);
        let collapsed = scene.build();
        assert!(
            !collapsed
                .iter()
                .any(|n| n.key == Key::new("p/net/ssid").with("TP-Link_5G_9A"))
        );

        // Dynamic rate/ping cards keep stable widths when numbers tick so scrolling never jitters.
        let width_of = |nodes: &[crate::node::Node], key: &str| {
            nodes
                .iter()
                .find(|n| n.key == Key::new(key))
                .expect(key)
                .rect
                .w
        };
        let ping_w1 = width_of(&nodes, "p/net/stat/ping");
        let rx_w1 = width_of(&nodes, "p/net/stat/rx_rate");
        scene.state_mut().network.details.ping_ms = None; // Timeout
        scene.state_mut().network.details.download_rate = 0.0; // 0 B/s
        let nodes2 = scene.build();
        assert_eq!(width_of(&nodes2, "p/net/stat/ping"), ping_w1);
        assert_eq!(width_of(&nodes2, "p/net/stat/rx_rate"), rx_w1);

        // Tapping IP copies to clipboard and displays temporary "Copied" feedback.
        let _ = scene.take_actions();
        scene.exec(Cmd::CopyField {
            field: "ip",
            value: "192.168.1.42".into(),
        });
        assert!(
            scene
                .take_actions()
                .contains(&Action::CopyToClipboard("192.168.1.42".into()))
        );
        assert!(scene.is_copied("ip"));
        assert!(scene.next_deadline(now) <= COPY_FEEDBACK);
        now += COPY_FEEDBACK + Duration::from_millis(20);
        scene.update(now, utc);
        assert!(!scene.is_copied("ip"));

        // Selecting Google DNS updates effective_dns optimistically without leaving the panel.
        scene.exec(Cmd::Do(Action::SetDns(DnsProvider::Google)));
        assert_eq!(scene.state.network.effective_dns(), DnsProvider::Google);
        assert!(
            scene
                .take_actions()
                .contains(&Action::SetDns(DnsProvider::Google))
        );
        assert_eq!(scene.page(), &Page::Panel(Panel::Network));

        // Selecting a Wi-Fi band sets pending_band optimistically without leaving the panel.
        scene.exec(Cmd::Do(Action::SetWifiBand("5".into())));
        assert_eq!(scene.state.network.effective_band(), "5");
        assert!(
            scene
                .take_actions()
                .contains(&Action::SetWifiBand("5".into()))
        );
        assert_eq!(scene.page(), &Page::Panel(Panel::Network));

        // Captive portal surfaces the Sign in button and hides ping behind "--".
        scene.state_mut().network.connectivity = Connectivity::Portal;
        let portal_nodes = scene.build();
        assert!(
            portal_nodes
                .iter()
                .any(|n| n.key == Key::new("p/net/portal"))
        );
        let ping_node = portal_nodes
            .iter()
            .find(|n| n.key == Key::new("p/net/stat/ping"))
            .unwrap();
        assert!(ping_node.parts.iter().any(|p| matches!(
            p,
            Part::Text { text, .. } if text == "--"
        )));

        // Selecting Custom DNS emits SetDns(Custom) and returns home for the floating terminal.
        scene.exec(Cmd::Do(Action::SetDns(DnsProvider::Custom)));
        assert!(
            scene
                .take_actions()
                .contains(&Action::SetDns(DnsProvider::Custom))
        );
        assert_eq!(scene.page(), &Page::Bar);
        assert!(!scene.wants_network_details());
    }

    #[test]
    fn touch_id_demo_plays_cancels_and_yields_to_a_real_prompt() {
        let (mut scene, mut now) = settled_scene();
        let utc = Utc::now();
        let mut step = |scene: &mut Scene, ms: u64| {
            for _ in 0..(ms / 16).max(1) {
                now += Duration::from_millis(16);
                scene.update(now, utc);
            }
            now
        };
        let fx = Key::new("tid/fx");
        let cancel = Key::new("tid/cancel");

        // The demo takes over the strip with the effect and a Cancel button.
        scene.command(Command::TouchIdDemo(TouchIdDemo::Unlock));
        step(&mut scene, 800);
        assert!(scene.touch_fx.is_some());
        assert!(scene.node_rect(fx).is_some_and(|r| r.w > 1500.0));
        let r = scene.node_rect(cancel).expect("cancel button");
        // A desktop update without a prompt does not end the demo.
        scene.set_state(DesktopState::sample());
        step(&mut scene, 32);
        assert!(scene.state.touch_id.is_some());

        // Cancel ends the demo without asking T1Bridge to cancel anything.
        let t = step(&mut scene, 16);
        scene.input(false, &[(0, r.cx(), r.cy())], t);
        scene.input(false, &[], t + Duration::from_millis(60));
        step(&mut scene, 600);
        assert!(scene.touch_fx.is_none());
        assert!(scene.state.touch_id.is_none());
        assert!(
            !scene
                .take_actions()
                .iter()
                .any(|a| matches!(a, Action::CancelTouchId))
        );

        // The demo runs to a success that outlives the prompt, then the bar returns.
        scene.command(Command::TouchIdDemo(TouchIdDemo::Unlock));
        step(&mut scene, 8400);
        assert_eq!(
            scene.state.touch_id.as_ref().map(|t| t.stage),
            Some(TouchIdStage::Success)
        );
        step(&mut scene, 400);
        assert!(scene.state.touch_id.is_none());
        assert!(scene.touch_fx.is_some(), "success outro still playing");
        step(&mut scene, 2000);
        assert!(scene.touch_fx.is_none());

        // A real prompt replaces a running demo.
        scene.command(Command::TouchIdDemo(TouchIdDemo::Enroll));
        step(&mut scene, 300);
        let mut state = DesktopState::sample();
        state.touch_id = Some(TouchId {
            stage: TouchIdStage::Authenticate,
            progress: None,
        });
        scene.set_state(state);
        step(&mut scene, 5000);
        assert_eq!(
            scene.state.touch_id.as_ref().map(|t| t.stage),
            Some(TouchIdStage::Authenticate)
        );
        assert!(scene.touch_demo.is_none());
    }

    #[test]
    fn keyboard_mode_keys_player_fn_and_touch_id() {
        let (mut scene, mut now) = settled_scene();
        let utc = Utc::now();
        let mut step = |scene: &mut Scene, ms: u64| {
            for _ in 0..(ms / 16).max(1) {
                now += Duration::from_millis(16);
                scene.update(now, utc);
            }
            now
        };
        let tap = |scene: &mut Scene, key: Key, t: Instant| {
            let r = scene.node_rect(key).expect("node on screen");
            let finger = [(0, r.cx(), r.cy())];
            assert_eq!(
                scene.input_press_position(false, &finger, t),
                Some(r.cx() / scene.width)
            );
            assert!(!scene.input(false, &finger, t + Duration::from_millis(30)));
            assert!(!scene.input(false, &[], t + Duration::from_millis(60)));
        };
        scene.command(Command::SetMode(Mode::Keyboard));
        step(&mut scene, 800);
        assert_eq!(scene.page(), &Page::Keyboard);
        // Keys left of the player, which ends the strip on the right.
        let player = scene.keyboard_player_key();
        let p = scene.node_rect(player).expect("player");
        assert!((p.right() - (scene.width - scene.m().edge)).abs() < 1.0);
        let vu = scene
            .node_rect(Key::new("ctl").with("vu"))
            .expect("volume up");
        assert!(vu.right() < p.x);
        assert!(scene.node_rect(Key::new("w/clock")).is_none());
        // The player's text is part of its card, so they move as one when
        // Fn swaps the page.
        let overlapping: Vec<Key> = scene
            .build()
            .iter()
            .filter(|n| n.key != player && n.rect.right() > p.x && n.rect.x < p.right())
            .map(|n| n.key)
            .collect();
        assert!(overlapping.is_empty(), "{overlapping:?} on the player");

        // Keys act like keys.
        scene.take_actions();
        let t = step(&mut scene, 16);
        tap(&mut scene, Key::new("ctl").with("bu"), t);
        let t = step(&mut scene, 16);
        tap(&mut scene, Key::new("ctl").with("play"), t);
        let actions = scene.take_actions();
        assert!(actions.contains(&Action::StepDisplayBrightness(true)));
        assert!(actions.contains(&Action::MediaPlayPause));

        // The player opens the full Sound panel, growing out of it, and
        // Back returns to the keys.
        let t = step(&mut scene, 300);
        tap(&mut scene, player, t);
        assert_eq!(scene.page(), &Page::Panel(Panel::Audio));
        assert!(scene.panel_origin.is_some_and(|x| (x - p.cx()).abs() < 1.0));
        step(&mut scene, 600);
        let t = step(&mut scene, 16);
        tap(&mut scene, Key::new("nav"), t);
        assert_eq!(scene.page(), &Page::Keyboard);

        // Panels can also be opened from outside, and time out to the keys.
        scene.command(Command::OpenPanel(Panel::Brightness));
        assert_eq!(scene.page(), &Page::Panel(Panel::Brightness));
        let timeout = scene.config.power.panel_timeout as u64 * 1000;
        step(&mut scene, timeout + 500);
        assert_eq!(scene.page(), &Page::Keyboard);

        // Holding Fn shows F1–F12 (with either Fn setting).
        let t = step(&mut scene, 16);
        scene.input(true, &[], t);
        assert_eq!(scene.shown_page(), Page::FnKeys);
        scene.input(false, &[], t);
        let mut config = scene.config.clone();
        config.fn_key = FnKey::Controls;
        scene.set_config(config);
        scene.input(true, &[], t);
        assert_eq!(scene.shown_page(), Page::FnKeys);
        scene.input(false, &[], t);
        assert_eq!(scene.shown_page(), Page::Keyboard);

        // Touch ID takes over the strip as in every mode.
        scene.command(Command::TouchIdDemo(TouchIdDemo::Unlock));
        step(&mut scene, 800);
        assert!(scene.touch_fx.is_some());
        assert!(scene.node_rect(Key::new("tid/fx")).is_some());
        assert!(scene.node_rect(player).is_none());
        step(&mut scene, 11000);
        assert!(scene.touch_fx.is_none());
        assert!(scene.node_rect(player).is_some());

        // Nothing playing: the player still names the output and opens Sound,
        // growing out of it.
        let mut state = scene.state.clone();
        state.media = None;
        scene.set_state(state);
        step(&mut scene, 600);
        let idle = scene.keyboard_player_key();
        assert_ne!(idle, player);
        assert!(scene.node_rect(player).is_none());
        let t = step(&mut scene, 16);
        tap(&mut scene, idle, t);
        assert_eq!(scene.page(), &Page::Panel(Panel::Audio));
        assert!(scene.panel_origin.is_some_and(|x| (x - p.cx()).abs() < 1.0));
    }

    #[test]
    fn workspace_swipe_cues_follow_the_displayed_order_in_both_directions() {
        use crate::{
            config::WorkspaceStyle,
            state::{Workspace, Workspaces},
        };

        for style in [WorkspaceStyle::Numbers, WorkspaceStyle::Dots] {
            let mut config = bar_config();
            config.appearance.workspace_style = style;
            config.appearance.min_workspaces = 2;
            let mut scene = test_scene_with_config(config);
            // A guest from another block follows the two local workspaces.
            scene.state_mut().workspaces = Workspaces {
                block: 1,
                monitor: "DP-1".into(),
                active: 11,
                list: vec![Workspace {
                    id: 2,
                    monitor: "DP-1".into(),
                    ..Workspace::default()
                }],
            };
            let mut now = Instant::now();
            let utc = Utc::now();
            for _ in 0..80 {
                now += Duration::from_millis(16);
                scene.update(now, utc);
            }

            let first = scene.node_rect(Key::new("w/ws").index(11)).unwrap();
            assert!(scene.input(false, &[(0, first.cx(), first.cy())], now));
            assert_eq!(scene.take_level_feedback(), None);
            for (id, cue) in [
                (11, None),
                (12, Some(LevelFeedback::Tick)),
                (2, Some(LevelFeedback::Maximum)),
                (2, None),
                (12, Some(LevelFeedback::Tick)),
                (11, Some(LevelFeedback::Minimum)),
                (11, None),
            ] {
                let r = scene.node_rect(Key::new("w/ws").index(id)).unwrap();
                now += Duration::from_millis(80);
                assert!(!scene.input(false, &[(0, r.cx(), r.cy())], now));
                assert_eq!(scene.take_level_feedback(), cue, "{style:?} workspace {id}");
                assert_eq!(i64::from(scene.state().workspaces.active), id);
            }
            scene.input(false, &[], now);
            assert_eq!(scene.take_level_feedback(), None);
            let focused: Vec<_> = scene
                .take_actions()
                .into_iter()
                .filter_map(|action| match action {
                    Action::FocusWorkspace { id, .. } => Some(id),
                    _ => None,
                })
                .collect();
            assert_eq!(focused, [12, 2, 12, 11]);
        }
    }

    #[test]
    fn tenth_workspace_displays_as_zero_at_the_end_of_its_block() {
        use crate::{
            node::{Hit, Part},
            state::{Workspace, Workspaces},
        };

        let mut scene = test_scene();
        let ws = |id: i32, windows: u32, monitor: &str| Workspace {
            id,
            name: id.to_string(),
            windows,
            monitor: monitor.into(),
        };
        let ws_labels = |nodes: &[crate::node::Node]| -> Vec<(f32, String)> {
            let mut items: Vec<(f32, String)> = nodes
                .iter()
                .filter_map(|n| {
                    let hit_ws = matches!(
                        n.hit,
                        Some(Hit {
                            cmd: Cmd::Workspace(_),
                            ..
                        })
                    );
                    if !hit_ws {
                        return None;
                    }
                    let text = n.parts.iter().find_map(|p| match p {
                        Part::Text { text, .. } => Some(text.clone()),
                        _ => None,
                    })?;
                    Some((n.rect.x, text))
                })
                .collect();
            items.sort_by(|a, b| a.0.total_cmp(&b.0));
            items
        };

        // Block 0: workspace 10 appears at the end labeled "0", and guests keep their global id.
        scene.state_mut().workspaces = Workspaces {
            list: vec![ws(1, 1, "eDP-1"), ws(10, 1, "eDP-1"), ws(12, 1, "eDP-1")],
            active: 10,
            monitor: "eDP-1".into(),
            block: 0,
        };
        let nodes = scene.build();
        let labels: Vec<String> = ws_labels(&nodes).into_iter().map(|(_, t)| t).collect();
        assert_eq!(labels, vec!["1", "2", "3", "4", "5", "0", "12"]);

        // Activating workspace 10 still targets Hyprland id 10 / slot 10.
        scene.state_mut().workspaces.active = 1;
        let _ = scene.take_actions();
        scene.exec(Cmd::Workspace(10));
        assert_eq!(
            scene.take_actions(),
            vec![Action::FocusWorkspace {
                id: 10,
                slot: 10,
                monitor: "eDP-1".into(),
                same_block: true,
            }]
        );

        // Block 1 (second monitor): workspace 20 is slot 10 and also displays as "0" at the end.
        scene.state_mut().workspaces = Workspaces {
            list: vec![ws(11, 1, "DP-1"), ws(20, 1, "DP-1")],
            active: 20,
            monitor: "DP-1".into(),
            block: 1,
        };
        let nodes = scene.build();
        let labels: Vec<String> = ws_labels(&nodes).into_iter().map(|(_, t)| t).collect();
        assert_eq!(labels, vec!["1", "2", "3", "4", "5", "0"]);
    }

    #[test]
    fn screensaver_takes_the_strip_and_a_touch_ends_it() {
        let (mut scene, mut now) = settled_scene();
        let utc = Utc::now();
        let mut step = |scene: &mut Scene, ms: u64| {
            for _ in 0..(ms / 16).max(1) {
                now += Duration::from_millis(16);
                scene.update(now, utc);
            }
            now
        };
        let saver = Key::new("screensaver");
        let geometry = Geometry::fit(10, 2170, 60).unwrap();

        // The whole strip, Esc included, is the screensaver's.
        scene.screensaver_start(geometry);
        step(&mut scene, 600);
        let nodes = scene.build();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].key, saver);
        assert_eq!(scene.node_rect(saver).map(|r| r.w), Some(2170.0));
        assert!(scene.node_rect(Key::new("esc")).is_none());

        // Frames make the strip draw; without them it rests.
        let mut grid = Grid::new(geometry.cols as usize, geometry.rows as usize);
        grid.cells[0].ch = '█';
        let t = step(&mut scene, 16);
        scene.screensaver_frame(Arc::new(grid));
        assert!(scene.update(t + Duration::from_millis(8), utc));
        step(&mut scene, 400);
        assert!(!scene.animating());

        // It plays at full brightness however long nobody touched the strip.
        scene.config.power.dim_after = 1;
        step(&mut scene, 1500);
        assert!(scene.brightness() > 0.99);

        // A touch ends it on the desktop too and presses nothing beneath.
        let t = step(&mut scene, 16);
        assert!(!scene.input(false, &[(0, 1085.0, 30.0)], t));
        scene.input(false, &[], t + Duration::from_millis(60));
        assert!(!scene.screensaver_active());
        assert_eq!(scene.take_actions(), vec![Action::DismissScreensaver]);
        step(&mut scene, 600);
        assert!(scene.node_rect(saver).is_none());
        assert!(scene.node_rect(Key::new("esc")).is_some());

        // Touch ID prompts still take over while it runs.
        scene.screensaver_start(geometry);
        let mut state = DesktopState::sample();
        state.touch_id = Some(TouchId {
            stage: TouchIdStage::Authenticate,
            progress: None,
        });
        scene.set_state(state);
        step(&mut scene, 600);
        assert!(scene.node_rect(Key::new("tid/fx")).is_some());
        assert!(scene.node_rect(saver).is_none());

        // The desktop's screensaver ending wakes the strip.
        scene.set_state(DesktopState::sample());
        step(&mut scene, 3000);
        assert!(scene.node_rect(saver).is_some());
        scene.screensaver_stop();
        step(&mut scene, 600);
        assert!(scene.node_rect(saver).is_none());
        assert!(scene.brightness() > 0.99);
    }
}
