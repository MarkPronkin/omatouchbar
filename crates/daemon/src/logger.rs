//! Minimal stderr logger (stderr lands in the journal of t1-touchbar.service).

use log::{Level, LevelFilter, Metadata, Record};

struct Stderr;

impl log::Log for Stderr {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let level = match record.level() {
            Level::Error => "error",
            Level::Warn => "warn",
            Level::Info => "info",
            Level::Debug => "debug",
            Level::Trace => "trace",
        };
        eprintln!("omatouchbar[{level}] {}", record.args());
    }

    fn flush(&self) {}
}

static LOGGER: Stderr = Stderr;

pub fn init() {
    let level = match std::env::var("OMATOUCHBAR_LOG").as_deref() {
        Ok("trace") => LevelFilter::Trace,
        Ok("debug") => LevelFilter::Debug,
        Ok("warn") => LevelFilter::Warn,
        Ok("error") => LevelFilter::Error,
        _ => LevelFilter::Info,
    };
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(level);
}
