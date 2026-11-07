//! Machine-local state (R10.1): what devdeck remembers between runs, as opposed to the
//! configuration a user writes. Lives in `DEVDECK_STATE_DIR`, else
//! `$XDG_STATE_HOME/devdeck`, else `~/.local/state/devdeck`.
//!
//! Layout: `app.json` (active workspace) and `workspaces/<id>/layout.json`. Every write
//! is atomic (temporary file + rename). Reads never fail the app: a missing file means
//! "nothing saved", a broken one is reported and ignored.

use crate::layout::Layout;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub fn state_dir(env: impl Fn(&str) -> Option<String>) -> PathBuf {
    if let Some(d) = env("DEVDECK_STATE_DIR").filter(|s| !s.is_empty()) {
        return PathBuf::from(d);
    }
    if let Some(d) = env("XDG_STATE_HOME").filter(|s| !s.is_empty()) {
        return PathBuf::from(d).join("devdeck");
    }
    let home = env("HOME").unwrap_or_else(|| ".".into());
    PathBuf::from(home).join(".local").join("state").join("devdeck")
}

pub fn default_state_dir() -> PathBuf {
    state_dir(|k| std::env::var(k).ok())
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppState {
    pub active_workspace: Option<String>,
}

/// Outcome of reading a saved file.
#[derive(Debug, PartialEq)]
pub enum Loaded<T> {
    Missing,
    Ok(T),
    Broken(String),
}

pub fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    let dir = path.parent().ok_or("path has no folder")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

fn read(path: &Path) -> Loaded<String> {
    match std::fs::read_to_string(path) {
        Ok(t) => Loaded::Ok(t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Loaded::Missing,
        Err(e) => Loaded::Broken(format!("{}: {e}", path.display())),
    }
}

/// A workspace id usable as a folder name (letters, digits, `-`, `_`).
pub fn safe_id(id: &str) -> String {
    id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
}

pub struct StateStore {
    pub dir: PathBuf,
}

impl StateStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn layout_path(&self, workspace: &str) -> PathBuf {
        self.dir.join("workspaces").join(safe_id(workspace)).join("layout.json")
    }

    pub fn load_layout(&self, workspace: &str) -> Loaded<Layout> {
        let path = self.layout_path(workspace);
        match read(&path) {
            Loaded::Ok(t) => match Layout::from_json(&t) {
                Ok(l) => Loaded::Ok(l),
                Err(e) => Loaded::Broken(format!("{}: {e}", path.display())),
            },
            Loaded::Missing => Loaded::Missing,
            Loaded::Broken(e) => Loaded::Broken(e),
        }
    }

    pub fn save_layout(&self, workspace: &str, layout: &Layout) -> Result<(), String> {
        write_atomic(&self.layout_path(workspace), &layout.to_json())
    }

    pub fn load_app(&self) -> Loaded<AppState> {
        let path = self.dir.join("app.json");
        match read(&path) {
            Loaded::Ok(t) => match serde_json::from_str(&t) {
                Ok(s) => Loaded::Ok(s),
                Err(e) => Loaded::Broken(format!("{}: {e}", path.display())),
            },
            Loaded::Missing => Loaded::Missing,
            Loaded::Broken(e) => Loaded::Broken(e),
        }
    }

    pub fn save_app(&self, state: &AppState) -> Result<(), String> {
        write_atomic(&self.dir.join("app.json"), &serde_json::to_string_pretty(state).map_err(|e| e.to_string())?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::ViewRef;

    fn store(tag: &str) -> StateStore {
        let d = std::env::temp_dir().join(format!("devdeck-state-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        StateStore::new(d)
    }

    #[test]
    fn state_dir_precedence() {
        let env = |pairs: &'static [(&'static str, &'static str)]| move |k: &str| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string());
        assert_eq!(state_dir(env(&[("DEVDECK_STATE_DIR", "/s"), ("XDG_STATE_HOME", "/x")])), PathBuf::from("/s"));
        assert_eq!(state_dir(env(&[("XDG_STATE_HOME", "/x"), ("HOME", "/h")])), PathBuf::from("/x/devdeck"));
        assert_eq!(state_dir(env(&[("HOME", "/h")])), PathBuf::from("/h/.local/state/devdeck"));
    }

    #[test]
    fn layout_round_trip_missing_and_broken() {
        let s = store("layout");
        assert_eq!(s.load_layout("work"), Loaded::Missing);
        let mut l = Layout::new();
        let p = l.main().focused;
        l.open(ViewRef::new("a"), p).unwrap();
        s.save_layout("work", &l).unwrap();
        assert_eq!(s.load_layout("work"), Loaded::Ok(l));
        std::fs::write(s.dir.join("workspaces/work/layout.json"), "{oops").unwrap();
        assert!(matches!(s.load_layout("work"), Loaded::Broken(_)));
        assert!(!s.dir.join("workspaces/work/layout.tmp").exists(), "no temp file left behind");
    }

    #[test]
    fn app_state_and_safe_ids() {
        let s = store("app");
        assert_eq!(s.load_app(), Loaded::Missing);
        s.save_app(&AppState { active_workspace: Some("personal".into()) }).unwrap();
        assert_eq!(s.load_app(), Loaded::Ok(AppState { active_workspace: Some("personal".into()) }));
        assert_eq!(safe_id("../evil name"), "___evil_name");
    }
}
