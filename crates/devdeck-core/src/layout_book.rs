//! Named layouts of one workspace (R3.10): save the arrangement under a name, switch
//! between names, and keep some views in every layout. The live layout is the
//! workspace's `layout.json`; the book holds the others.

use crate::layout::{Layout, ViewRef};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const DEFAULT_NAME: &str = "default";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LayoutBook {
    /// The name the live layout is saved under when switching away.
    pub current: String,
    /// Every name, alphabetical (the current one's entry may be stale: the live
    /// layout is the truth for it).
    pub layouts: BTreeMap<String, Layout>,
    /// Views that follow into every layout.
    pub keep: BTreeSet<ViewRef>,
}

impl Default for LayoutBook {
    fn default() -> Self {
        Self { current: DEFAULT_NAME.into(), layouts: BTreeMap::new(), keep: BTreeSet::new() }
    }
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
            BookError::Unknown(n) => write!(f, "no layout called {n}"),
            BookError::DeleteCurrent => write!(f, "switch to another layout before deleting this one"),
        }
    }
}

/// Trims a name; refuses empty ones. Names keep their case; comparisons ignore it.
pub fn clean_name(name: &str) -> Result<String, BookError> {
    let n = name.trim();
    if n.is_empty() { Err(BookError::EmptyName) } else { Ok(n.to_string()) }
}

impl LayoutBook {
    fn find(&self, name: &str) -> Option<&String> {
        self.layouts.keys().chain(std::iter::once(&self.current)).find(|k| k.eq_ignore_ascii_case(name))
    }

    /// Every name, alphabetical, the current one included.
    pub fn names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.layouts.keys().cloned().collect();
        if !v.contains(&self.current) {
            v.push(self.current.clone());
        }
        v.sort_by_key(|n| n.to_lowercase());
        v
    }

    /// Saves the live layout under a new name and makes it current.
    pub fn save_as(&mut self, name: &str, live: &Layout) -> Result<(), BookError> {
        let name = clean_name(name)?;
        if let Some(existing) = self.find(&name) {
            return Err(BookError::NameTaken(existing.clone()));
        }
        self.layouts.insert(self.current.clone(), live.clone());
        self.layouts.insert(name.clone(), live.clone());
        self.current = name;
        Ok(())
    }

    /// Switches to `name`: stores the live layout under the current name and returns
    /// the layout to show, with kept views it lacks added to its main focused pane.
    pub fn switch(&mut self, name: &str, live: &Layout) -> Result<Layout, BookError> {
        let target_name = self.find(name).cloned().ok_or_else(|| BookError::Unknown(name.into()))?;
        self.layouts.insert(self.current.clone(), live.clone());
        let mut target = self.layouts.get(&target_name).cloned().unwrap_or_else(|| live.clone());
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
        self.current = target_name;
        Ok(target)
    }

    pub fn rename(&mut self, from: &str, to: &str) -> Result<(), BookError> {
        let from = self.find(from).cloned().ok_or_else(|| BookError::Unknown(from.into()))?;
        let to = clean_name(to)?;
        if let Some(existing) = self.find(&to)
            && !existing.eq_ignore_ascii_case(&from)
        {
            return Err(BookError::NameTaken(existing.clone()));
        }
        if let Some(l) = self.layouts.remove(&from) {
            self.layouts.insert(to.clone(), l);
        }
        if self.current == from {
            self.current = to;
        }
        Ok(())
    }

    pub fn delete(&mut self, name: &str) -> Result<(), BookError> {
        let name = self.find(name).cloned().ok_or_else(|| BookError::Unknown(name.into()))?;
        if name == self.current {
            return Err(BookError::DeleteCurrent);
        }
        self.layouts.remove(&name);
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

    #[test]
    fn save_switch_and_back() {
        let mut book = LayoutBook::default();
        let a = layout(&["a", "b"]);
        book.save_as("Triage", &a).unwrap();
        assert_eq!(book.current, "Triage");
        assert_eq!(book.names(), ["default", "Triage"]);
        let changed = layout(&["c"]);
        let shown = book.switch("default", &changed).unwrap();
        assert_eq!(shown, a, "default was saved when Triage was created");
        assert_eq!(book.layouts["Triage"], changed, "the live layout is stored under the current name");
        let back = book.switch("triage", &shown).unwrap();
        assert_eq!(back, changed, "names ignore case");
    }

    #[test]
    fn kept_views_follow_into_every_layout() {
        let mut book = LayoutBook::default();
        book.save_as("one", &layout(&["a"])).unwrap();
        // The live layout changed to b, then is saved as two: "one" is stored as b too
        // (the live layout always wins for the current name).
        book.save_as("two", &layout(&["b"])).unwrap();
        let shown = book.switch("default", &layout(&["b"])).unwrap();
        assert_eq!(shown, layout(&["a"]));
        assert!(book.toggle_keep(&ViewRef::new("logs")));
        let live = layout(&["a", "logs"]);
        let shown = book.switch("two", &live).unwrap();
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
        book.save_as("x", &l).unwrap();
        assert_eq!(book.save_as("X", &l), Err(BookError::NameTaken("x".into())));
        assert_eq!(book.delete("x"), Err(BookError::DeleteCurrent));
        assert_eq!(book.switch("nope", &l), Err(BookError::Unknown("nope".into())));
        book.switch("default", &l).unwrap();
        book.rename("x", "y").unwrap();
        assert_eq!(book.names(), ["default", "y"]);
        assert_eq!(book.rename("y", "Default"), Err(BookError::NameTaken("default".into())));
        book.delete("y").unwrap();
        assert_eq!(book.names(), ["default"]);
        assert_eq!(BookError::DeleteCurrent.to_string(), "switch to another layout before deleting this one");
    }
}
