//! Weather the way Omarchy's weather panel gets it: the location stored in
//! `weather.json` (or, with none stored, where geo.omarchy.org places this
//! network), reports from Open-Meteo, Omarchy's helpers as a fallback, and
//! the location search behind the Touch Bar's picker.

use std::{
    collections::HashMap,
    fs,
    process::{Command, Stdio},
    sync::mpsc::{self, Receiver, RecvTimeoutError},
    thread,
    time::{Duration, Instant},
};

use crate::bus::{Bus, Event};
use omatouchbar_core::{
    Scene,
    config::Mode,
    omarchy::Widget,
    paths,
    scene::{Page, Panel},
    state::Weather,
    weather::{self, Location, Place, StoredLocation},
};

/// A report older than this is refreshed when the panel opens.
const FRESH: Duration = Duration::from_secs(5 * 60);
/// Retry after a failed refresh.
const RETRY: Duration = Duration::from_secs(60);
/// Wait for typing to pause before searching (as Omarchy does).
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(300);

#[derive(Clone, Debug, PartialEq)]
struct Options {
    minutes: u64,
    /// Omarchy's `unit` widget setting.
    unit: String,
}

enum Request {
    Configure(Option<Options>),
    Refresh,
    /// `weather.json` changed on disk.
    LocationFile,
    /// The panel opened: refresh if the report is getting old.
    Opened,
    SetLocation(Option<Location>),
}

pub struct Provider {
    tx: mpsc::Sender<Request>,
    search: mpsc::Sender<String>,
    active: Option<Options>,
}

impl Provider {
    pub fn start(bus: Bus) -> Self {
        let (tx, rx) = mpsc::channel();
        let worker = Worker::new(bus.clone());
        thread::Builder::new()
            .name("weather".into())
            .spawn(move || worker.run(rx))
            .ok();
        let (search, queries) = mpsc::channel();
        thread::Builder::new()
            .name("weather-search".into())
            .spawn(move || search_loop(bus, queries))
            .ok();
        Self {
            tx,
            search,
            active: None,
        }
    }

    pub fn configure(&mut self, scene: &Scene) {
        let widget = scene.layout().find("weather");
        let unit = match widget {
            Some(Widget::Weather { unit, .. }) => unit.clone(),
            _ => String::new(),
        };
        let options = match (scene.config().mode, widget) {
            (
                Mode::Bar,
                Some(Widget::Weather {
                    refresh_minutes, ..
                }),
            ) => Some(*refresh_minutes),
            _ => None,
        }
        .or_else(|| (scene.page() == &Page::Panel(Panel::Weather)).then_some(15))
        .map(|minutes| Options { minutes, unit });
        if options != self.active {
            self.active = options.clone();
            let _ = self.tx.send(Request::Configure(options));
        }
    }

    pub fn refresh(&self) {
        if self.active.is_some() {
            let _ = self.tx.send(Request::Refresh);
        }
    }

    pub fn location_changed(&self) {
        let _ = self.tx.send(Request::LocationFile);
    }

    pub fn opened(&self) {
        let _ = self.tx.send(Request::Opened);
    }

    pub fn search(&self, query: String) {
        let _ = self.search.send(query);
    }

    pub fn set_location(&self, location: Option<Location>) {
        let _ = self.tx.send(Request::SetLocation(location));
    }
}

struct Worker {
    bus: Bus,
    options: Option<Options>,
    /// `weather.json` as last read.
    stored: Option<StoredLocation>,
    /// geo.omarchy.org's last answer.
    detected: Option<StoredLocation>,
    /// Country codes of stored places, looked up once per name.
    countries: HashMap<String, Option<String>>,
    /// The place the shown report is for, and whether it used °F.
    reported: Option<(String, bool)>,
    last_success: Option<Instant>,
    next_due: Option<Instant>,
}

impl Worker {
    fn new(bus: Bus) -> Self {
        Self {
            bus,
            options: None,
            stored: None,
            detected: None,
            countries: HashMap::new(),
            reported: None,
            last_success: None,
            next_due: None,
        }
    }

    fn stale_after(&self, age: Duration) -> bool {
        self.last_success.is_none_or(|t| t.elapsed() >= age)
    }

    fn run(mut self, rx: Receiver<Request>) {
        loop {
            let first = if self.options.is_some() {
                let wait = self.next_due.map_or(Duration::ZERO, |due| {
                    due.saturating_duration_since(Instant::now())
                });
                match rx.recv_timeout(wait) {
                    Ok(r) => Some(r),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            } else {
                match rx.recv() {
                    Ok(r) => Some(r),
                    Err(_) => break,
                }
            };
            // A timeout means the next refresh is due.
            let mut fetch = first.is_none();
            for request in first.into_iter().chain(rx.try_iter()) {
                match request {
                    Request::Configure(new) => {
                        if let Some(new) = &new {
                            let interval = Duration::from_secs(new.minutes * 60);
                            match &self.options {
                                None => fetch |= self.stale_after(interval.min(FRESH)),
                                Some(old) => fetch |= old.unit != new.unit,
                            }
                            if let Some(last) = self.last_success {
                                let due = last + interval;
                                if self.next_due.is_none_or(|d| due < d) {
                                    self.next_due = Some(due);
                                }
                            }
                        }
                        self.options = new;
                    }
                    Request::Refresh => fetch = true,
                    Request::Opened => fetch |= self.stale_after(FRESH),
                    Request::LocationFile => {
                        fetch |= self.stored.as_ref() != Some(&read_stored());
                    }
                    Request::SetLocation(location) => {
                        self.save(location);
                        fetch = true;
                    }
                }
            }
            if fetch && self.options.is_some() {
                self.fetch();
            }
        }
    }

    /// Stores the location the way Omarchy's panel does, then shows it as
    /// loading until its report arrives.
    fn save(&mut self, location: Option<Location>) {
        let mut args = Vec::new();
        match &location {
            Some(l) => {
                args.extend(["--set".to_string(), l.name.clone()]);
                if let Some((lat, lon)) = l.coords {
                    args.push(format!("{lat},{lon}"));
                }
            }
            None => args.push("--clear".into()),
        }
        let saved = Command::new("omarchy-weather-location")
            .args(&args)
            .env("OMARCHY_PATH", paths::omarchy_root())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if !saved {
            log::warn!("omarchy-weather-location failed; writing weather.json directly");
            write_stored(location.as_ref());
        }
        log::info!(
            "weather location: {}",
            location.as_ref().map_or("auto", |l| l.name.as_str())
        );
        if let Some(l) = &location
            && let Some(country) = &l.country
        {
            self.countries.insert(l.name.clone(), Some(country.clone()));
        }
        let place = Place {
            name: match &location {
                Some(l) => l.name.clone(),
                None => self
                    .detected
                    .as_ref()
                    .map(|d| d.name.clone())
                    .unwrap_or_default(),
            },
            auto: location.is_none(),
        };
        self.bus.patch(move |s| {
            let w = s.weather.get_or_insert_with(Weather::default);
            w.place = place;
            w.loading = true;
        });
    }

    fn fetch(&mut self) {
        let Some(options) = self.options.clone() else {
            return;
        };
        let stored = read_stored();
        self.stored = Some(stored.clone());
        let auto = stored.is_auto();
        let mut place = Place {
            name: stored.name.clone(),
            auto,
        };
        let mut country = stored.country.clone();
        let coords = if auto {
            if let Some(found) = geo() {
                self.detected = Some(found);
            }
            let detected = self.detected.clone();
            place.name = detected
                .as_ref()
                .map(|d| d.name.clone())
                .unwrap_or_default();
            country = detected.as_ref().and_then(|d| d.country.clone());
            detected.and_then(|d| d.coords)
        } else if let Some(coords) = stored.coords {
            if country.is_none() {
                country = self.country_near(&stored.name, coords);
            }
            Some(coords)
        } else {
            // A name without coordinates (hand-written or typed): find it.
            geocode(&stored.name, 1)
                .and_then(|found| found.into_iter().next())
                .and_then(|l| {
                    country = l.country.clone();
                    l.coords
                })
        };

        // Units like Omarchy: the setting, else the place's country, else the
        // locale. Without a country, keep what this place used last time.
        let imperial = match (&country, &self.reported) {
            (None, Some((name, imperial)))
                if *name == place.name && options.unit.trim().is_empty() =>
            {
                *imperial
            }
            _ => weather::uses_imperial(&options.unit, &locale(), country.as_deref()),
        };
        let report = coords
            .and_then(|(lat, lon)| curl(&weather::forecast_url(lat, lon), 8))
            .and_then(|json| weather::parse_forecast(&json, imperial));

        let same_place = self
            .reported
            .as_ref()
            .is_some_and(|(name, _)| *name == place.name);
        // Omarchy's own helpers, only when there is nothing better to show.
        let fallback = (report.is_none() && !same_place).then(helpers).flatten();
        let now = Instant::now();
        let ok = report.is_some()
            || fallback
                .as_ref()
                .is_some_and(|(_, status)| !status.is_empty());
        if ok {
            self.last_success = Some(now);
        }
        if report.is_some() {
            self.reported = Some((place.name.clone(), imperial));
        } else if !same_place {
            self.reported = None;
        }
        self.next_due = Some(
            now + if report.is_some() {
                Duration::from_secs(options.minutes * 60)
            } else {
                RETRY
            },
        );
        log::debug!(
            "weather for {:?} ({}): {}",
            place.name,
            if auto { "auto" } else { "stored" },
            if report.is_some() { "ok" } else { "no report" }
        );

        let detected = self
            .detected
            .as_ref()
            .map(|d| d.name.clone())
            .filter(|n| !n.is_empty());
        self.bus.patch(move |s| {
            let w = s.weather.get_or_insert_with(Weather::default);
            w.loading = false;
            w.detected = detected;
            w.place = place;
            match report {
                Some(r) => {
                    w.icon = weather::icon(r.now.code, !r.now.day).into();
                    w.report = Some(r);
                    w.status.clear();
                    w.stale = false;
                }
                None if same_place => w.stale = true,
                None => {
                    w.report = None;
                    match fallback {
                        Some((icon, status)) => {
                            if !icon.is_empty() {
                                w.icon = icon;
                            }
                            w.stale = status.is_empty() && !w.status.is_empty();
                            if !status.is_empty() {
                                w.status = status;
                            }
                        }
                        None => w.stale = !w.status.is_empty(),
                    }
                }
            }
        });
    }

    /// The country of a stored place, from the geocoder's match nearest to
    /// its coordinates. Lookup failures are retried next time.
    fn country_near(&mut self, name: &str, (lat, lon): (f64, f64)) -> Option<String> {
        if name.is_empty() {
            return None;
        }
        if let Some(country) = self.countries.get(name) {
            return country.clone();
        }
        let found = geocode(name, 10)?;
        let country = found
            .iter()
            .filter_map(|l| Some((l.coords?, l.country.as_ref()?)))
            .map(|((a, b), c)| ((a - lat).powi(2) + (b - lon).powi(2), c))
            .filter(|(d, _)| *d < 1.0)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, c)| c.clone());
        self.countries.insert(name.to_string(), country.clone());
        country
    }
}

/// Geocodes what is typed in the picker, newest query first.
fn search_loop(bus: Bus, queries: Receiver<String>) {
    while let Ok(mut query) = queries.recv() {
        loop {
            match queries.recv_timeout(SEARCH_DEBOUNCE) {
                Ok(newer) => query = newer,
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
        let results = geocode(&query, 6);
        bus.send(Event::WeatherSuggestions { query, results });
    }
}

fn curl(url: &str, timeout_secs: u32) -> Option<String> {
    let output = Command::new("curl")
        .args(["-fsS", "--max-time", &timeout_secs.to_string(), url])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

fn geocode(query: &str, count: u8) -> Option<Vec<Location>> {
    if query.trim().is_empty() {
        return Some(Vec::new());
    }
    weather::parse_geocoding(&curl(&weather::geocoding_url(query, count), 5)?)
}

/// Omarchy's network location lookup (geo.omarchy.org).
fn geo() -> Option<StoredLocation> {
    let found = weather::parse_location(&curl("https://geo.omarchy.org/", 4)?);
    (found.coords.is_some() && !found.name.is_empty()).then_some(found)
}

fn read_stored() -> StoredLocation {
    fs::read_to_string(paths::weather_location_file())
        .map(|text| weather::parse_location(&text))
        .unwrap_or_default()
}

/// Writes `weather.json` in `omarchy-weather-location`'s format.
fn write_stored(location: Option<&Location>) {
    let path = paths::weather_location_file();
    let Some(l) = location else {
        let _ = fs::remove_file(&path);
        return;
    };
    let mut data = serde_json::Map::new();
    data.insert("name".into(), l.name.clone().into());
    if let Some((lat, lon)) = l.coords {
        data.insert("latitude".into(), lat.into());
        data.insert("longitude".into(), lon.into());
    }
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let _ = fs::write(&path, format!("{}\n", serde_json::Value::Object(data)));
}

fn locale() -> String {
    ["LC_ALL", "LC_MEASUREMENT", "LANG"]
        .iter()
        .find_map(|var| std::env::var(var).ok().filter(|v| !v.is_empty()))
        .unwrap_or_default()
}

fn helper(program: &str) -> Option<String> {
    let output = Command::new(program)
        .env("OMARCHY_PATH", paths::omarchy_root())
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    (output.status.success() && !text.is_empty()).then_some(text)
}

/// `(icon, status)` from `omarchy-weather-icon` and `omarchy-weather-status`.
fn helpers() -> Option<(String, String)> {
    thread::scope(|scope| {
        let icon = scope.spawn(|| helper("omarchy-weather-icon"));
        let status = helper("omarchy-weather-status");
        let icon = icon.join().ok().flatten();
        if status.is_none() && icon.is_none() {
            return None;
        }
        Some((icon.unwrap_or_default(), status.unwrap_or_default()))
    })
}
