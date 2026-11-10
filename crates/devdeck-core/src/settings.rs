//! Global, per-machine settings (R9.4, R10.1), stored as TOML in the config folder.
//!
//! Loading never fails: a missing file gives defaults, a broken file gives defaults
//! plus a message to show in the settings view. Saving writes a temporary file and
//! renames it, so a crash never leaves a half-written file.

use serde::{Deserialize, Serialize};
use crate::state::write_atomic;
use std::path::{Path, PathBuf};

/// UI scale steps, applied on top of the desktop's own scale factor. Discrete so the
/// glyph atlas only ever holds a few sizes.
pub const SCALE_STEPS: [f32; 6] = [0.8, 0.9, 1.0, 1.1, 1.25, 1.5];
pub const DEFAULT_SCALE: f32 = 1.0;
pub const DEFAULT_THEME: &str = "devdeck-dark";
pub const SETTINGS_FILE: &str = "settings.toml";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub ui_scale: f32,
    pub theme: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self { ui_scale: DEFAULT_SCALE, theme: DEFAULT_THEME.into() }
    }
}

impl Settings {
    /// The scale snapped to the nearest step (non-finite values give the default).
    pub fn scale(&self) -> f32 {
        snap_scale(self.ui_scale)
    }
}

pub fn snap_scale(v: f32) -> f32 {
    if !v.is_finite() {
        return DEFAULT_SCALE;
    }
    SCALE_STEPS
        .iter()
        .copied()
        .min_by(|a, b| (a - v).abs().total_cmp(&(b - v).abs()))
        .unwrap_or(DEFAULT_SCALE)
}

/// The next larger step, or the largest.
pub fn scale_up(v: f32) -> f32 {
    let v = snap_scale(v);
    SCALE_STEPS.iter().copied().find(|s| *s > v + 1e-4).unwrap_or(v)
}

/// The next smaller step, or the smallest.
pub fn scale_down(v: f32) -> f32 {
    let v = snap_scale(v);
    SCALE_STEPS.iter().rev().copied().find(|s| *s < v - 1e-4).unwrap_or(v)
}

/// The config folder (`devdeck_sys::dirs::config`). Takes a lookup so tests don't touch
/// the real environment.
pub fn config_dir(env: impl Fn(&str) -> Option<String>) -> PathBuf {
    devdeck_sys::dirs::config(env)
}

pub fn default_config_dir() -> PathBuf {
    config_dir(|k| std::env::var(k).ok())
}

/// Settings plus where they live and what went wrong reading or writing them.
#[derive(Clone, Debug)]
pub struct SettingsStore {
    pub path: PathBuf,
    pub settings: Settings,
    pub load_error: Option<String>,
    pub save_error: Option<String>,
}

impl SettingsStore {
    pub fn load(dir: &Path) -> Self {
        let path = dir.join(SETTINGS_FILE);
        let (settings, load_error) = match std::fs::read_to_string(&path) {
            Ok(text) => match toml::from_str::<Settings>(&text) {
                Ok(s) => (s, None),
                Err(e) => (Settings::default(), Some(format!("{}: {}", path.display(), first_line(&e.to_string())))),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Settings::default(), None),
            Err(e) => (Settings::default(), Some(format!("{}: {e}", path.display()))),
        };
        Self { path, settings, load_error, save_error: None }
    }

    /// Writes the settings atomically. On failure the error is kept for display.
    pub fn save(&mut self) -> Result<(), String> {
        let result = write_atomic(&self.path, &toml::to_string_pretty(&self.settings).map_err(|e| e.to_string())?);
        self.save_error = result.as_ref().err().cloned();
        result
    }

    pub fn set_scale(&mut self, scale: f32) -> Result<(), String> {
        self.settings.ui_scale = snap_scale(scale);
        self.save()
    }
}

fn first_line(s: &str) -> String {
    s.lines().find(|l| !l.trim().is_empty()).unwrap_or(s).trim().to_string()
}


#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(name: &str) -> crate::testing::TempDir {
        crate::testing::TempDir::new(name)
    }

    #[test]
    fn snapping_and_steps() {
        assert_eq!(snap_scale(1.0), 1.0);
        assert_eq!(snap_scale(1.3), 1.25);
        assert_eq!(snap_scale(9.0), 1.5);
        assert_eq!(snap_scale(0.1), 0.8);
        assert_eq!(snap_scale(f32::NAN), DEFAULT_SCALE);
        assert_eq!(snap_scale(f32::INFINITY), DEFAULT_SCALE);
        assert_eq!(scale_up(1.0), 1.1);
        assert_eq!(scale_up(1.1), 1.25);
        assert_eq!(scale_up(1.5), 1.5);
        assert_eq!(scale_down(1.25), 1.1);
        assert_eq!(scale_down(0.8), 0.8);
    }

    #[test]
    fn missing_file_gives_defaults_without_error() {
        let s = SettingsStore::load(&tmp_dir("missing"));
        assert_eq!(s.settings, Settings::default());
        assert!(s.load_error.is_none());
    }

    #[test]
    fn broken_file_gives_defaults_with_error() {
        let d = tmp_dir("broken");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(SETTINGS_FILE), "ui_scale = \"big\"\n").unwrap();
        let s = SettingsStore::load(&d);
        assert_eq!(s.settings, Settings::default());
        assert!(s.load_error.as_deref().is_some_and(|e| e.contains("settings.toml")));
    }

    #[test]
    fn unknown_keys_are_ignored_and_missing_keys_default() {
        let d = tmp_dir("unknown");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(SETTINGS_FILE), "ui_scale = 1.25\nfuture_option = true\n").unwrap();
        let s = SettingsStore::load(&d);
        assert!(s.load_error.is_none());
        assert_eq!(s.settings.ui_scale, 1.25);
        assert_eq!(s.settings.theme, DEFAULT_THEME);
    }

    #[test]
    fn save_round_trip_is_atomic_and_snaps() {
        let d = tmp_dir("save");
        let mut s = SettingsStore::load(&d);
        s.set_scale(1.49).unwrap();
        assert!(std::fs::read_dir(&d).unwrap().flatten().all(|e| !e.file_name().to_string_lossy().ends_with(".tmp")));
        let back = SettingsStore::load(&d);
        assert_eq!(back.settings.ui_scale, 1.5);
        assert!(back.load_error.is_none());
    }

    #[test]
    fn save_failure_is_reported_not_fatal() {
        let d = tmp_dir("readonly");
        std::fs::create_dir_all(&d).unwrap();
        // A file where the folder should be makes the write fail on every platform.
        let blocker = d.join("blocked");
        std::fs::write(&blocker, "").unwrap();
        let mut s = SettingsStore::load(&blocker);
        assert!(s.set_scale(1.1).is_err());
        assert!(s.save_error.is_some());
        assert_eq!(s.settings.ui_scale, 1.1, "the in-memory value still changes");
    }
}
