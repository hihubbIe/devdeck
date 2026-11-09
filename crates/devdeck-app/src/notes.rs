//! A workspace's notes (R12.3), owning their text: views only edit it here, so closing a
//! tab, a layout switch, a detached window's × or a quit loses nothing. Changes are
//! written [`SAVE_AFTER`] after the last one, and all at once by [`Notes::flush`] (the
//! shell's state save and quit). A failed write is said once in a toast; the text stays
//! and is written with the next change or flush.
//!
//! Each note has a revision, raised by every change of its text: the user's typing, a
//! write from elsewhere in devdeck (the control API), or its file changed on disk by
//! another program (its folder is watched; devdeck's own writes read back the same and
//! change nothing). A change not typed in the editor is told with [`NoteWritten`] so open
//! editors take it.
//!
//! Without a folder (the demo, tests that don't give one) notes live in memory only.

use devdeck_core::notes::{NoteMeta, NotesStore, sort};
use devdeck_ui::{ToastKind, toasts};
use gpui::{App, Context, EventEmitter, Task};
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Typing pauses this long before a note is written.
pub const SAVE_AFTER: Duration = Duration::from_millis(500);

/// A note went away (its tabs close).
#[derive(Clone, Debug, PartialEq)]
pub struct NoteDeleted(pub String);

/// A note's text changed other than by typing in its editor (its editors take it).
#[derive(Clone, Debug, PartialEq)]
pub struct NoteWritten(pub String);

pub struct Notes {
    store: Option<NotesStore>,
    index: Vec<NoteMeta>,
    /// Texts read or typed so far, by id.
    texts: HashMap<String, String>,
    unsaved: BTreeSet<String>,
    /// Notes whose file couldn't be read, with why: shown as that, never written over.
    unreadable: HashMap<String, String>,
    save: Option<Task<()>>,
    /// A write failed and that was said (said again only after one succeeds).
    failing: bool,
    /// Each note's revision: raised by every change of its text.
    revisions: HashMap<String, u64>,
    /// When the user last typed in each note.
    typed: HashMap<String, Instant>,
}

impl EventEmitter<NoteDeleted> for Notes {}
impl EventEmitter<NoteWritten> for Notes {}

impl Notes {
    /// The notes in `store` (`None`: in memory). An index that can't be read is said in a
    /// toast and left alone.
    pub fn new(store: Option<NotesStore>, cx: &mut Context<Self>) -> Self {
        let index = match store.as_ref().map(NotesStore::list) {
            Some(Ok(index)) => index,
            Some(Err(e)) => {
                let text = format!("Notes can't be read: {e}");
                cx.defer(move |cx| {
                    toasts(cx).update(cx, |t, cx| t.push(ToastKind::Err, text, vec![], cx));
                });
                Vec::new()
            }
            None => Vec::new(),
        };
        Self { store, index, texts: HashMap::new(), unsaved: BTreeSet::new(), unreadable: HashMap::new(), save: None, failing: false, revisions: HashMap::new(), typed: HashMap::new() }
    }

    /// Keeps the notes in `store` from now on (tests: a folder of their own).
    pub fn set_store(&mut self, store: NotesStore, cx: &mut Context<Self>) {
        self.index = store.list().unwrap_or_default();
        self.texts.clear();
        self.unsaved.clear();
        self.store = Some(store);
        cx.notify();
    }

    /// Notes of `app`, by name then id.
    pub fn of(&self, app: &str) -> Vec<NoteMeta> {
        self.index.iter().filter(|n| n.app == app).cloned().collect()
    }

    /// Every note of the workspace, by name then id.
    pub fn all(&self) -> &[NoteMeta] {
        &self.index
    }

    pub fn meta(&self, id: &str) -> Option<&NoteMeta> {
        self.index.iter().find(|n| n.id == id)
    }

    /// A note's text (read from disk the first time); empty when it can't be read
    /// ([`Notes::unreadable`]).
    pub fn text(&mut self, id: &str) -> String {
        if let Some(t) = self.texts.get(id) {
            return t.clone();
        }
        if self.unreadable.contains_key(id) {
            return String::new();
        }
        match self.store.as_ref().map(|s| s.read(id)).unwrap_or(Ok(String::new())) {
            Ok(text) => {
                self.texts.insert(id.to_string(), text.clone());
                text
            }
            Err(e) => {
                self.unreadable.insert(id.to_string(), e);
                String::new()
            }
        }
    }

    /// Why a note's file couldn't be read (another program wrote it, not as UTF-8 text).
    pub fn unreadable(&self, id: &str) -> Option<&str> {
        self.unreadable.get(id).map(String::as_str)
    }

    /// Makes a note for `app`; its id.
    pub fn create(&mut self, app: &str, name: &str, cx: &mut Context<Self>) -> Option<String> {
        let meta = match &self.store {
            Some(s) => match s.create(app, name) {
                Ok(m) => m,
                Err(e) => {
                    self.say(format!("The note wasn't made: {e}"), cx);
                    return None;
                }
            },
            None => {
                let name = if name.trim().is_empty() { "notes".to_string() } else { name.trim().to_string() };
                NoteMeta { id: uuid_like(self.index.len()), app: app.to_string(), name }
            }
        };
        let id = meta.id.clone();
        self.texts.insert(id.clone(), String::new());
        self.index.push(meta);
        sort(&mut self.index);
        cx.notify();
        Some(id)
    }

    pub fn rename(&mut self, id: &str, name: &str, cx: &mut Context<Self>) {
        if name.trim().is_empty() {
            return;
        }
        if let Some(s) = &self.store
            && let Err(e) = s.rename(id, name)
        {
            return self.say(format!("The note wasn't renamed: {e}"), cx);
        }
        if let Some(n) = self.index.iter_mut().find(|n| n.id == id) {
            n.name = name.trim().to_string();
        }
        sort(&mut self.index);
        cx.notify();
    }

    pub fn delete(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(s) = &self.store
            && let Err(e) = s.delete(id)
        {
            return self.say(format!("The note wasn't deleted: {e}"), cx);
        }
        self.index.retain(|n| n.id != id);
        self.texts.remove(id);
        self.unsaved.remove(id);
        self.unreadable.remove(id);
        self.revisions.remove(id);
        self.typed.remove(id);
        cx.emit(NoteDeleted(id.to_string()));
        cx.notify();
    }

    /// The user typed in a note: it's written soon.
    pub fn set_text(&mut self, id: &str, text: String, cx: &mut Context<Self>) {
        if self.change(id, text, cx).is_ok_and(|changed| changed) {
            self.typed.insert(id.to_string(), cx.background_executor().now());
        }
    }

    /// Writes a note's text from elsewhere than its editor (the control API): written soon,
    /// told to its editors. Its new revision.
    pub fn write(&mut self, id: &str, text: String, cx: &mut Context<Self>) -> Result<u64, String> {
        if self.change(id, text, cx)? {
            cx.emit(NoteWritten(id.to_string()));
        }
        Ok(self.revision(id))
    }

    /// A note's revision: 0 until its text first changes.
    pub fn revision(&self, id: &str) -> u64 {
        self.revisions.get(id).copied().unwrap_or(0)
    }

    /// When the user last typed in a note.
    pub fn typed_at(&self, id: &str) -> Option<Instant> {
        self.typed.get(id).copied()
    }

    /// The folder notes are kept in, watched for other programs' changes.
    pub fn dir(&self) -> Option<PathBuf> {
        self.store.as_ref().map(|s| s.dir.clone())
    }

    /// Files in `paths` changed on disk: a note read already whose file now says something
    /// else takes it. One with text not written yet keeps it (it's written over the file
    /// next).
    pub fn disk_changed(&mut self, paths: &[PathBuf], cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else { return };
        let ids: Vec<String> = self.texts.keys().filter(|id| !self.unsaved.contains(*id) && paths.contains(&store.file(id))).cloned().collect();
        for id in ids {
            let Ok(text) = store.read(&id) else { continue };
            if self.texts.get(&id) != Some(&text) {
                self.texts.insert(id.clone(), text);
                *self.revisions.entry(id.clone()).or_default() += 1;
                cx.emit(NoteWritten(id));
                cx.notify();
            }
        }
    }

    /// Changes a note's text, to be written soon; whether it changed.
    fn change(&mut self, id: &str, text: String, cx: &mut Context<Self>) -> Result<bool, String> {
        if self.meta(id).is_none() {
            return Err(format!("no note {id}"));
        }
        if let Some(why) = self.unreadable.get(id) {
            return Err(format!("the note can't be read: {why}"));
        }
        if self.texts.get(id) == Some(&text) {
            return Ok(false);
        }
        self.texts.insert(id.to_string(), text);
        *self.revisions.entry(id.to_string()).or_default() += 1;
        self.unsaved.insert(id.to_string());
        self.save = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SAVE_AFTER).await;
            let _ = this.update(cx, |this, cx| {
                this.save = None;
                this.flush(cx);
            });
        }));
        cx.notify();
        Ok(true)
    }

    /// Whether a note has text not written yet.
    pub fn is_unsaved(&self, id: &str) -> bool {
        self.unsaved.contains(id)
    }

    /// Writes every unsaved note now.
    pub fn flush(&mut self, cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else {
            self.unsaved.clear();
            return;
        };
        let mut failed = None;
        for id in std::mem::take(&mut self.unsaved) {
            let text = self.texts.get(&id).cloned().unwrap_or_default();
            if let Err(e) = store.write(&id, &text) {
                failed = Some(e);
                self.unsaved.insert(id);
            }
        }
        match failed {
            Some(e) if !self.failing => {
                self.failing = true;
                self.say(format!("A note couldn't be saved ({e}); it's kept and tried again."), cx);
            }
            Some(_) => {}
            None => self.failing = false,
        }
    }

    fn say(&self, text: String, cx: &mut App) {
        toasts(cx).update(cx, |t, cx| t.push(ToastKind::Err, text, vec![], cx));
    }
}

/// An id for a note kept in memory (the demo).
fn uuid_like(n: usize) -> String {
    format!("memory-{n}")
}
