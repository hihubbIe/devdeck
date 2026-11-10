//! devdeck's log, and GPUI's: one line per record on stderr (level, target, message).
//! Warnings and errors by default; `DEVDECK_LOG=info|debug|trace` shows more. Installed
//! first thing by the binary, never in tests.

use log::{Level, LevelFilter, Log, Metadata, Record};
use std::io::Write;

struct Stderr;

impl Log for Stderr {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            // Never a panic when stderr is gone (its terminal closed).
            let _ = writeln!(std::io::stderr().lock(), "{}", line(record.level(), record.target(), &record.args().to_string()));
        }
    }

    fn flush(&self) {}
}

/// Installs the logger at the level `DEVDECK_LOG` asks (once; a second call does nothing).
pub fn install() {
    if log::set_logger(&Stderr).is_ok() {
        log::set_max_level(level(std::env::var("DEVDECK_LOG").ok().as_deref()));
    }
}

/// The level `DEVDECK_LOG` asks: warnings when unset, empty or unknown.
pub fn level(asked: Option<&str>) -> LevelFilter {
    match asked.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        Some("error") => LevelFilter::Error,
        Some("info") => LevelFilter::Info,
        Some("debug") => LevelFilter::Debug,
        Some("trace") => LevelFilter::Trace,
        Some("off") => LevelFilter::Off,
        _ => LevelFilter::Warn,
    }
}

/// One record as written: `WARN gpui_macos::platform: no text will be rendered`.
pub fn line(level: Level, target: &str, message: &str) -> String {
    format!("{level} {target}: {message}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warnings_unless_asked() {
        assert_eq!(level(None), LevelFilter::Warn);
        assert_eq!(level(Some("")), LevelFilter::Warn);
        assert_eq!(level(Some("loud")), LevelFilter::Warn);
        assert_eq!(level(Some("error")), LevelFilter::Error);
        assert_eq!(level(Some("info")), LevelFilter::Info);
        assert_eq!(level(Some(" Debug ")), LevelFilter::Debug);
        assert_eq!(level(Some("trace")), LevelFilter::Trace);
        assert_eq!(level(Some("off")), LevelFilter::Off);
    }

    #[test]
    fn a_line_names_level_and_target() {
        assert_eq!(line(Level::Warn, "gpui_macos::platform", "no text"), "WARN gpui_macos::platform: no text");
    }
}
