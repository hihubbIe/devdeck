//! Notes (R12.3): Markdown notes belonging to an app, kept in devdeck's data folder, never
//! in the project. A workspace's notes are a folder: `index.json` (each note's id, app and
//! name) and one `<id>.md` each. Ids are UUIDs; names are labels (two notes may share one).
//!
//! Writes are atomic. Making a note writes its file before the index, deleting one
//! removes it from the index before its file: a crash leaves at most a file no note lists
//! (kept, ignored), never a listed note without its file (read as empty if it happens).

use crate::state::{safe_id, write_atomic};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// What the user writes in devdeck (R10.1): `DEVDECK_DATA_DIR`, else
/// `$XDG_DATA_HOME/devdeck`, else `~/.local/share/devdeck`.
pub fn data_dir(env: impl Fn(&str) -> Option<String>) -> PathBuf {
    if let Some(d) = env("DEVDECK_DATA_DIR").filter(|s| !s.is_empty()) {
        return PathBuf::from(d);
    }
    if let Some(d) = env("XDG_DATA_HOME").filter(|s| !s.is_empty()) {
        return PathBuf::from(d).join("devdeck");
    }
    let home = env("HOME").unwrap_or_else(|| ".".into());
    PathBuf::from(home).join(".local").join("share").join("devdeck")
}

pub fn default_data_dir() -> PathBuf {
    data_dir(|k| std::env::var(k).ok())
}

/// A workspace's notes folder under `data`.
pub fn notes_dir(data: &Path, workspace: &str) -> PathBuf {
    data.join("workspaces").join(safe_id(workspace)).join("notes")
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteMeta {
    pub id: String,
    /// The app it belongs to (its id).
    pub app: String,
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct Index {
    notes: Vec<NoteMeta>,
}

/// A workspace's notes on disk.
#[derive(Clone, Debug)]
pub struct NotesStore {
    pub dir: PathBuf,
}

impl NotesStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn index_path(&self) -> PathBuf {
        self.dir.join("index.json")
    }

    pub fn file(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{}.md", safe_id(id)))
    }

    /// The notes listed, by name then id. A missing index is no notes; a broken one is
    /// an error (and nothing is written over it).
    pub fn list(&self) -> Result<Vec<NoteMeta>, String> {
        let path = self.index_path();
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        let mut notes = serde_json::from_str::<Index>(&text).map_err(|e| format!("{}: {e}", path.display()))?.notes;
        sort(&mut notes);
        Ok(notes)
    }

    fn save(&self, notes: &[NoteMeta]) -> Result<(), String> {
        let text = serde_json::to_string_pretty(&Index { notes: notes.to_vec() }).map_err(|e| e.to_string())?;
        write_atomic(&self.index_path(), &text)
    }

    /// A note's text; empty when its file is missing.
    pub fn read(&self, id: &str) -> Result<String, String> {
        match std::fs::read_to_string(self.file(id)) {
            Ok(t) => Ok(t),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
            Err(e) => Err(format!("{}: {e}", self.file(id).display())),
        }
    }

    pub fn write(&self, id: &str, text: &str) -> Result<(), String> {
        write_atomic(&self.file(id), text)
    }

    /// Makes a note for `app` named `name` (empty: "notes"), file first.
    pub fn create(&self, app: &str, name: &str) -> Result<NoteMeta, String> {
        let mut notes = self.list()?;
        let name = if name.trim().is_empty() { "notes".to_string() } else { name.trim().to_string() };
        let meta = NoteMeta { id: uuid::Uuid::new_v4().to_string(), app: app.to_string(), name };
        self.write(&meta.id, "")?;
        notes.push(meta.clone());
        self.save(&notes)?;
        Ok(meta)
    }

    pub fn rename(&self, id: &str, name: &str) -> Result<(), String> {
        let mut notes = self.list()?;
        let note = notes.iter_mut().find(|n| n.id == id).ok_or("no such note")?;
        let name = name.trim();
        if !name.is_empty() {
            note.name = name.to_string();
        }
        self.save(&notes)
    }

    /// Deletes a note, index first.
    pub fn delete(&self, id: &str) -> Result<(), String> {
        let mut notes = self.list()?;
        notes.retain(|n| n.id != id);
        self.save(&notes)?;
        match std::fs::remove_file(self.file(id)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("{}: {e}", self.file(id).display())),
            _ => Ok(()),
        }
    }
}

/// By name (case aside), then id.
pub fn sort(notes: &mut [NoteMeta]) {
    notes.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then_with(|| a.id.cmp(&b.id)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    #[test]
    fn data_dir_order() {
        let env = |pairs: &'static [(&'static str, &'static str)]| move |k: &str| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string());
        assert_eq!(data_dir(env(&[("DEVDECK_DATA_DIR", "/d"), ("XDG_DATA_HOME", "/x")])), PathBuf::from("/d"));
        assert_eq!(data_dir(env(&[("XDG_DATA_HOME", "/x"), ("HOME", "/h")])), PathBuf::from("/x/devdeck"));
        assert_eq!(data_dir(env(&[("HOME", "/h")])), PathBuf::from("/h/.local/share/devdeck"));
    }

    #[test]
    fn make_write_rename_delete() {
        let dir = TempDir::new("notes");
        let store = NotesStore::new(notes_dir(&dir, "work"));
        assert_eq!(store.list().unwrap(), vec![]);
        let b = store.create("front-end/graph", "todo").unwrap();
        let a = store.create("front-end/graph", "  ").unwrap();
        assert_eq!(a.name, "notes");
        assert_eq!(store.list().unwrap().iter().map(|n| n.name.as_str()).collect::<Vec<_>>(), ["notes", "todo"]);
        store.write(&b.id, "- [ ] fix the zoom\n").unwrap();
        assert_eq!(store.read(&b.id).unwrap(), "- [ ] fix the zoom\n");
        store.rename(&b.id, "bugs").unwrap();
        assert_eq!(store.list().unwrap()[0].name, "bugs");
        store.delete(&b.id).unwrap();
        assert_eq!(store.list().unwrap(), vec![a.clone()]);
        assert!(!store.file(&b.id).exists());
        assert_eq!(store.read(&b.id).unwrap(), "", "a missing file reads empty");
    }

    #[test]
    fn same_names_order_by_id_and_a_broken_index_is_kept() {
        let dir = TempDir::new("notes");
        let store = NotesStore::new(dir.to_path_buf());
        let (x, y) = (store.create("a", "same").unwrap(), store.create("a", "same").unwrap());
        let ids: Vec<String> = store.list().unwrap().into_iter().map(|n| n.id).collect();
        let mut want = vec![x.id, y.id];
        want.sort();
        assert_eq!(ids, want);
        std::fs::write(dir.join("index.json"), "{ not json").unwrap();
        assert!(store.list().is_err());
        assert!(store.create("a", "c").is_err(), "nothing is written over it");
        assert_eq!(std::fs::read_to_string(dir.join("index.json")).unwrap(), "{ not json");
    }
}
