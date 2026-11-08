//! Named layouts of one workspace (R3.10): save the arrangement under a name, switch
//! between layouts, and keep some views in every layout. The live layout is the
//! workspace's `layout.json`; the book holds the others. Layouts are known by an id;
//! their name is only a label, so renaming one keeps what refers to it (sidebar pins).

use crate::layout::{Layout, ViewRef};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub type LayoutId = String;

pub const DEFAULT_ID: &str = "default";
pub const DEFAULT_NAME: &str = "default";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Named {
    pub name: String,
    /// `None` for a layout never stored yet.
    #[serde(default)]
    pub layout: Option<Layout>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LayoutBook {
    /// The layout the live one is saved under when switching away; always a key of
    /// `layouts`.
    pub current: LayoutId,
    /// Every layout (the current one's entry may be stale: the live layout is the
    /// truth for it).
    pub layouts: BTreeMap<LayoutId, Named>,
    /// Views that follow into every layout.
    pub keep: BTreeSet<ViewRef>,
}

impl Default for LayoutBook {
    fn default() -> Self {
        let layouts = BTreeMap::from([(DEFAULT_ID.to_string(), Named { name: DEFAULT_NAME.into(), layout: None })]);
        Self { current: DEFAULT_ID.into(), layouts, keep: BTreeSet::new() }
    }
}

/// The book as saved before layouts had ids: layouts by name, `current` a name.
#[derive(Deserialize)]
struct OldBook {
    #[serde(default)]
    current: String,
    #[serde(default)]
    layouts: BTreeMap<String, Layout>,
    #[serde(default)]
    keep: BTreeSet<ViewRef>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BookError {
    EmptyName,
    NameTaken(String),
    Unknown(String),
    DeleteCurrent,
}

impl std::fmt::Display for BookError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BookError::EmptyName => write!(f, "a layout needs a name"),
            BookError::NameTaken(n) => write!(f, "a layout called {n} exists"),
            BookError::Unknown(n) => write!(f, "no such layout: {n}"),
            BookError::DeleteCurrent => write!(f, "switch to another layout before deleting this one"),
        }
    }
}

/// Trims a name; refuses empty ones. Names keep their case; comparisons ignore it.
pub fn clean_name(name: &str) -> Result<String, BookError> {
    let n = name.trim();
    if n.is_empty() { Err(BookError::EmptyName) } else { Ok(n.to_string()) }
}

fn new_id() -> LayoutId {
    uuid::Uuid::new_v4().to_string()
}

impl LayoutBook {
    /// Reads a saved book, in today's shape or the one from before layouts had ids.
    pub fn from_json(text: &str) -> Result<LayoutBook, String> {
        let mut book = match serde_json::from_str::<LayoutBook>(text) {
            Ok(b) => b,
            // A saved `Layout` has no `name`, so an old book never reads as a new one.
            Err(e) => serde_json::from_str::<OldBook>(text).map(LayoutBook::from_old).map_err(|_| e.to_string())?,
        };
        book.repair();
        Ok(book)
    }

    fn from_old(old: OldBook) -> LayoutBook {
        let layouts = old
            .layouts
            .into_iter()
            .map(|(name, l)| {
                let id = if name.eq_ignore_ascii_case(DEFAULT_NAME) { DEFAULT_ID.to_string() } else { new_id() };
                (id, Named { name, layout: Some(l) })
            })
            .collect();
        // The old current is a name; `repair` finds its id or gives it an entry.
        LayoutBook { current: old.current, layouts, keep: old.keep }
    }

    /// Makes `current` a key of `layouts`: a current that is a layout's name points at
    /// it; otherwise it gets an entry named after it, or the default.
    fn repair(&mut self) {
        if self.layouts.contains_key(&self.current) {
            return;
        }
        let name = clean_name(&self.current).unwrap_or_else(|_| DEFAULT_NAME.into());
        if let Some(id) = self.id_named(&name) {
            self.current = id;
            return;
        }
        let id = if name.eq_ignore_ascii_case(DEFAULT_NAME) && !self.layouts.contains_key(DEFAULT_ID) { DEFAULT_ID.to_string() } else { new_id() };
        self.layouts.insert(id.clone(), Named { name, layout: None });
        self.current = id;
    }

    /// Every layout as (id, name), by name then id, the current one included.
    pub fn entries(&self) -> Vec<(LayoutId, String)> {
        let mut v: Vec<(LayoutId, String)> = self.layouts.iter().map(|(id, n)| (id.clone(), n.name.clone())).collect();
        v.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()).then_with(|| a.0.cmp(&b.0)));
        v
    }

    pub fn name(&self, id: &str) -> Option<&str> {
        self.layouts.get(id).map(|n| n.name.as_str())
    }

    pub fn current_name(&self) -> &str {
        self.name(&self.current).unwrap_or(DEFAULT_NAME)
    }

    /// The layout called `name`, ignoring case.
    pub fn id_named(&self, name: &str) -> Option<LayoutId> {
        self.layouts.iter().find(|(_, n)| n.name.eq_ignore_ascii_case(name)).map(|(id, _)| id.clone())
    }

    fn store_live(&mut self, live: &Layout) {
        let current = self.current.clone();
        self.layouts.entry(current).or_insert_with(|| Named { name: DEFAULT_NAME.into(), layout: None }).layout = Some(live.clone());
    }

    /// Saves the live layout under a new name and makes it current.
    pub fn save_as(&mut self, name: &str, live: &Layout) -> Result<LayoutId, BookError> {
        let name = clean_name(name)?;
        if let Some(existing) = self.id_named(&name) {
            return Err(BookError::NameTaken(self.layouts[&existing].name.clone()));
        }
        self.store_live(live);
        let id = new_id();
        self.layouts.insert(id.clone(), Named { name, layout: Some(live.clone()) });
        self.current = id.clone();
        Ok(id)
    }

    /// Switches to `id`: stores the live layout under the current one and returns the
    /// layout to show, with kept views it lacks added to its main focused pane.
    pub fn switch(&mut self, id: &str, live: &Layout) -> Result<Layout, BookError> {
        if !self.layouts.contains_key(id) {
            return Err(BookError::Unknown(id.into()));
        }
        self.store_live(live);
        let mut target = self.layouts[id].layout.clone().unwrap_or_else(|| live.clone());
        // Kept views land where they'll be seen: the maximized pane, else the focused
        // pane, else the first expanded one; they don't take the active tab.
        let main = target.main();
        let into = main.maximized.unwrap_or(main.focused);
        let into = if target.pane(into).is_ok_and(|p| p.collapsed) {
            target.panes_in(crate::layout::MAIN_WINDOW).unwrap_or_default().into_iter().find(|p| target.pane(*p).is_ok_and(|p| !p.collapsed)).unwrap_or(into)
        } else {
            into
        };
        for v in &self.keep {
            if live.find_view(v).is_some() && target.find_view(v).is_none() {
                let active = target.pane(into).map(|p| p.active).unwrap_or(0);
                let had_tabs = target.pane(into).is_ok_and(|p| !p.tabs.is_empty());
                if target.open(v.clone(), into).is_ok() && had_tabs {
                    let _ = target.activate(into, active);
                }
            }
        }
        // Kept views that the target doesn't have and the live layout dropped stay away.
        self.current = id.to_string();
        Ok(target)
    }

    /// Renames `id`; only its name changes.
    pub fn rename(&mut self, id: &str, to: &str) -> Result<(), BookError> {
        if !self.layouts.contains_key(id) {
            return Err(BookError::Unknown(id.into()));
        }
        let to = clean_name(to)?;
        if let Some(existing) = self.id_named(&to)
            && existing != id
        {
            return Err(BookError::NameTaken(self.layouts[&existing].name.clone()));
        }
        if let Some(n) = self.layouts.get_mut(id) {
            n.name = to;
        }
        Ok(())
    }

    pub fn delete(&mut self, id: &str) -> Result<(), BookError> {
        if !self.layouts.contains_key(id) {
            return Err(BookError::Unknown(id.into()));
        }
        if id == self.current {
            return Err(BookError::DeleteCurrent);
        }
        self.layouts.remove(id);
        Ok(())
    }

    /// Marks or unmarks a view as kept in every layout; returns whether it is kept.
    pub fn toggle_keep(&mut self, view: &ViewRef) -> bool {
        if self.keep.remove(view) {
            false
        } else {
            self.keep.insert(view.clone());
            true
        }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(views: &[&str]) -> Layout {
        let mut l = Layout::new();
        let p = l.main().focused;
        for v in views {
            l.open(ViewRef::new(*v), p).unwrap();
        }
        l
    }

    fn names(book: &LayoutBook) -> Vec<String> {
        book.entries().into_iter().map(|(_, n)| n).collect()
    }

    #[test]
    fn save_switch_and_back() {
        let mut book = LayoutBook::default();
        let a = layout(&["a", "b"]);
        let triage = book.save_as("Triage", &a).unwrap();
        assert_eq!(book.current, triage);
        assert_eq!(book.current_name(), "Triage");
        assert_eq!(names(&book), ["default", "Triage"]);
        assert_eq!(book.id_named("triage"), Some(triage.clone()), "names ignore case");
        let changed = layout(&["c"]);
        let shown = book.switch(DEFAULT_ID, &changed).unwrap();
        assert_eq!(shown, a, "default was saved when Triage was created");
        assert_eq!(book.layouts[&triage].layout, Some(changed.clone()), "the live layout is stored under the current id");
        let back = book.switch(&triage, &shown).unwrap();
        assert_eq!(back, changed);
    }

    #[test]
    fn kept_views_follow_into_every_layout() {
        let mut book = LayoutBook::default();
        book.save_as("one", &layout(&["a"])).unwrap();
        // The live layout changed to b, then is saved as two: "one" is stored as b too
        // (the live layout always wins for the current one).
        let two = book.save_as("two", &layout(&["b"])).unwrap();
        let shown = book.switch(DEFAULT_ID, &layout(&["b"])).unwrap();
        assert_eq!(shown, layout(&["a"]));
        assert!(book.toggle_keep(&ViewRef::new("logs")));
        let live = layout(&["a", "logs"]);
        let shown = book.switch(&two, &live).unwrap();
        assert!(shown.find_view(&ViewRef::new("logs")).is_some(), "kept view added");
        assert!(shown.find_view(&ViewRef::new("b")).is_some() && shown.find_view(&ViewRef::new("a")).is_none());
        let p = shown.main().focused;
        assert_eq!(shown.pane(p).unwrap().active_view(), Some(&ViewRef::new("b")), "the kept view doesn't take the active tab");
        assert!(shown.validate().is_ok());
        assert!(!book.toggle_keep(&ViewRef::new("logs")));
    }

    #[test]
    fn refusals_are_explained() {
        let mut book = LayoutBook::default();
        let l = layout(&["a"]);
        assert_eq!(book.save_as("  ", &l), Err(BookError::EmptyName));
        let x = book.save_as("x", &l).unwrap();
        assert_eq!(book.save_as("X", &l), Err(BookError::NameTaken("x".into())));
        assert_eq!(book.delete(&x), Err(BookError::DeleteCurrent));
        assert_eq!(book.switch("nope", &l), Err(BookError::Unknown("nope".into())));
        book.switch(DEFAULT_ID, &l).unwrap();
        book.rename(&x, "y").unwrap();
        assert_eq!(names(&book), ["default", "y"]);
        assert_eq!(book.rename(&x, "Default"), Err(BookError::NameTaken("default".into())));
        book.delete(&x).unwrap();
        assert_eq!(names(&book), ["default"]);
        assert_eq!(BookError::DeleteCurrent.to_string(), "switch to another layout before deleting this one");
    }

    #[test]
    fn rename_keeps_the_id() {
        let mut book = LayoutBook::default();
        let l = layout(&["a"]);
        let id = book.save_as("triage", &l).unwrap();
        book.rename(&id, "Incident").unwrap();
        assert_eq!(book.current, id, "what refers to the id (pins, current) still holds");
        assert_eq!(book.name(&id), Some("Incident"));
        assert_eq!(book.layouts[&id].layout, Some(l));
        book.rename(&id, "INCIDENT").unwrap();
        assert_eq!(book.current_name(), "INCIDENT", "its own name in another case is allowed");
    }

    #[test]
    fn the_shape_before_ids_converts() {
        let a = serde_json::to_value(layout(&["a"])).unwrap();
        let b = serde_json::to_value(layout(&["b"])).unwrap();
        let old = serde_json::json!({ "current": "Triage", "layouts": { "Default": a, "focus": b }, "keep": ["logs"] });
        let book = LayoutBook::from_json(&old.to_string()).unwrap();
        assert_eq!(names(&book), ["Default", "focus", "Triage"], "names kept");
        assert_eq!(book.layouts[DEFAULT_ID].layout, Some(layout(&["a"])), "default gets the default id");
        let focus = book.id_named("focus").unwrap();
        assert_eq!(book.layouts[&focus].layout, Some(layout(&["b"])));
        assert_eq!(book.current_name(), "Triage", "the current one, never stored, gets an entry");
        assert_eq!(book.layouts[&book.current].layout, None);
        assert!(book.keep.contains(&ViewRef::new("logs")));
        // Today's shape reads back as is.
        assert_eq!(LayoutBook::from_json(&book.to_json()), Ok(book));
        assert!(LayoutBook::from_json("{oops").is_err());
    }

    #[test]
    fn a_broken_current_is_repaired() {
        let mut book = LayoutBook::default();
        let id = book.save_as("triage", &layout(&["a"])).unwrap();
        book.current = "gone".into();
        let read = LayoutBook::from_json(&book.to_json()).unwrap();
        assert_eq!(read.name(&read.current), Some("gone"), "an entry named after it");
        assert!(read.layouts.contains_key(&id));
        book.current = "TRIAGE".into();
        assert_eq!(LayoutBook::from_json(&book.to_json()).unwrap().current, id, "a current that is a name points at that layout");
        assert_eq!(LayoutBook::from_json("{}").unwrap(), LayoutBook::default());
        assert_eq!(LayoutBook::from_json(r#"{"current": "", "layouts": {}}"#).unwrap(), LayoutBook::default());
    }
}
