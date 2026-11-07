//! The window layout model: windows hold a tree of splits whose leaves are panes, and
//! panes hold tabs. Each tab is a [`ViewRef`], the address of a view.
//!
//! Every operation keeps the invariants checked by [`Layout::validate`]:
//! - every pane appears in exactly one window tree, exactly once;
//! - splits have at least two children, positive weights summing to 1, and never a
//!   direct child split on the same axis (nested same-axis splits are flattened);
//! - a view is open in at most one tab across all windows;
//! - only the main window may hold an empty pane, and only when it is its only pane;
//! - a detached window's root is a single pane;
//! - each window's focused and maximized panes belong to it;
//! - each window keeps at least one expanded (not collapsed) pane.

mod geometry;
#[cfg(test)]
mod tests;

pub use geometry::{Direction, Rect};

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// Bumped whenever the serialized shape changes.
pub const LAYOUT_VERSION: u32 = 1;
/// Smallest weight a split child can be resized down to.
pub const MIN_WEIGHT: f32 = 0.05;

macro_rules! id_type {
    ($name:ident) => {
        #[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        pub struct $name(pub u64);
    };
}
id_type!(PaneId);
id_type!(SplitId);
id_type!(WindowId);

/// The main window always has this id.
pub const MAIN_WINDOW: WindowId = WindowId(0);

/// The address of a view, for example `logs/dev35/entityservice`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ViewRef(pub String);

impl ViewRef {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ViewRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// `Row` lays children side by side, `Column` stacks them.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Axis {
    Row,
    Column,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

impl Edge {
    pub fn axis(self) -> Axis {
        match self {
            Edge::Left | Edge::Right => Axis::Row,
            Edge::Top | Edge::Bottom => Axis::Column,
        }
    }
    /// Whether the new pane goes before the target in its split.
    pub fn is_before(self) -> bool {
        matches!(self, Edge::Left | Edge::Top)
    }
}

/// Where a dragged tab or pane lands.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Drop {
    /// Into the pane's tab strip.
    Into(PaneId),
    /// Next to the pane, splitting it.
    Edge(PaneId, Edge),
}

impl Drop {
    pub fn pane(self) -> PaneId {
        match self {
            Drop::Into(p) | Drop::Edge(p, _) => p,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Node {
    Split(Split),
    Pane(PaneId),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Split {
    pub id: SplitId,
    pub axis: Axis,
    pub children: Vec<Child>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Child {
    pub node: Node,
    /// Fraction of the split, all children summing to 1.
    pub weight: f32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Pane {
    pub tabs: Vec<ViewRef>,
    pub active: usize,
    pub collapsed: bool,
}

impl Pane {
    pub fn active_view(&self) -> Option<&ViewRef> {
        self.tabs.get(self.active)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Window {
    pub id: WindowId,
    pub root: Node,
    pub focused: PaneId,
    pub maximized: Option<PaneId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LayoutError {
    UnknownPane(PaneId),
    UnknownWindow(WindowId),
    UnknownSplit(SplitId),
    TabOutOfRange,
    /// The main window must keep one pane.
    MainWindowLastPane,
    /// Detached windows hold a single pane, so they cannot be split.
    DetachedWindowIsSinglePane,
    /// The operation needs a detached window but got the main one.
    NotDetached,
    /// A window can't be attached onto itself.
    SameWindow,
    /// Each window keeps one expanded pane.
    LastExpandedPane,
    /// Detaching would just recreate the same window.
    AlreadyAlone,
    UnsupportedVersion(u32),
    Invalid(String),
}

impl fmt::Display for LayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for LayoutError {}

pub type Result<T> = std::result::Result<T, LayoutError>;

/// What closing a window led to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CloseOutcome {
    /// The main window closed: the app quits.
    Quit,
    /// A detached window closed together with its tabs.
    Closed(Vec<ViewRef>),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    pub version: u32,
    /// The main window first, then detached windows in creation order.
    pub windows: Vec<Window>,
    pub panes: BTreeMap<PaneId, Pane>,
    next_id: u64,
}

impl Default for Layout {
    fn default() -> Self {
        Self::new()
    }
}

impl Layout {
    /// A main window with one empty pane.
    pub fn new() -> Self {
        let pane = PaneId(1);
        let mut panes = BTreeMap::new();
        panes.insert(pane, Pane::default());
        Self {
            version: LAYOUT_VERSION,
            windows: vec![Window {
                id: MAIN_WINDOW,
                root: Node::Pane(pane),
                focused: pane,
                maximized: None,
            }],
            panes,
            next_id: 2,
        }
    }

    fn alloc(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn new_pane(&mut self, tabs: Vec<ViewRef>) -> PaneId {
        let id = PaneId(self.alloc());
        self.panes.insert(id, Pane { tabs, active: 0, collapsed: false });
        id
    }

    // ---------------------------------------------------------------- queries

    pub fn main(&self) -> &Window {
        &self.windows[0]
    }

    pub fn window(&self, id: WindowId) -> Result<&Window> {
        self.windows.iter().find(|w| w.id == id).ok_or(LayoutError::UnknownWindow(id))
    }

    fn window_mut(&mut self, id: WindowId) -> Result<&mut Window> {
        self.windows.iter_mut().find(|w| w.id == id).ok_or(LayoutError::UnknownWindow(id))
    }

    pub fn pane(&self, id: PaneId) -> Result<&Pane> {
        self.panes.get(&id).ok_or(LayoutError::UnknownPane(id))
    }

    fn pane_mut(&mut self, id: PaneId) -> Result<&mut Pane> {
        self.panes.get_mut(&id).ok_or(LayoutError::UnknownPane(id))
    }

    /// Panes of a window in reading order (depth first).
    pub fn panes_in(&self, window: WindowId) -> Result<Vec<PaneId>> {
        let mut out = Vec::new();
        collect_panes(&self.window(window)?.root, &mut out);
        Ok(out)
    }

    pub fn window_of(&self, pane: PaneId) -> Result<WindowId> {
        for w in &self.windows {
            if contains_pane(&w.root, pane) {
                return Ok(w.id);
            }
        }
        Err(LayoutError::UnknownPane(pane))
    }

    /// Where a view is open, if it is.
    pub fn find_view(&self, view: &ViewRef) -> Option<(PaneId, usize)> {
        self.panes
            .iter()
            .find_map(|(id, p)| p.tabs.iter().position(|t| t == view).map(|i| (*id, i)))
    }

    /// Every open view, in no particular order.
    pub fn views(&self) -> BTreeSet<ViewRef> {
        self.panes.values().flat_map(|p| p.tabs.iter().cloned()).collect()
    }

    /// The split directly containing `pane`, with the pane's index and the split axis.
    pub fn parent_of(&self, pane: PaneId) -> Option<(SplitId, usize, Axis)> {
        fn walk(node: &Node, pane: PaneId) -> Option<(SplitId, usize, Axis)> {
            let Node::Split(s) = node else { return None };
            if let Some(i) = s.children.iter().position(|c| matches!(c.node, Node::Pane(p) if p == pane)) {
                return Some((s.id, i, s.axis));
            }
            s.children.iter().find_map(|c| walk(&c.node, pane))
        }
        self.windows.iter().find_map(|w| walk(&w.root, pane))
    }

    /// Grows (positive `delta`) or shrinks the pane along its parent split.
    pub fn grow(&mut self, pane: PaneId, delta: f32) -> Result<()> {
        let Some((split, idx, _)) = self.parent_of(pane) else { return Ok(()) };
        let n = self.split(split)?.children.len();
        if idx + 1 < n { self.resize(split, idx, delta) } else { self.resize(split, idx - 1, -delta) }
    }

    pub fn is_detached(&self, window: WindowId) -> bool {
        window != MAIN_WINDOW
    }

    pub fn split(&self, id: SplitId) -> Result<&Split> {
        self.windows
            .iter()
            .find_map(|w| find_split(&w.root, id))
            .ok_or(LayoutError::UnknownSplit(id))
    }

    // ---------------------------------------------------------------- tabs

    /// Opens `view` in `target`, or activates it where it already is.
    /// Returns the pane that shows it.
    pub fn open(&mut self, view: ViewRef, target: PaneId) -> Result<PaneId> {
        if let Some((pane, idx)) = self.find_view(&view) {
            self.activate(pane, idx)?;
            return Ok(pane);
        }
        let p = self.pane_mut(target)?;
        p.tabs.push(view);
        p.active = p.tabs.len() - 1;
        p.collapsed = false;
        self.focus(target)?;
        self.repair();
        Ok(target)
    }

    pub fn activate(&mut self, pane: PaneId, idx: usize) -> Result<()> {
        let p = self.pane_mut(pane)?;
        if idx >= p.tabs.len() {
            return Err(LayoutError::TabOutOfRange);
        }
        p.active = idx;
        p.collapsed = false;
        self.focus(pane)
    }

    /// Moves the active tab by `delta`, wrapping around.
    pub fn cycle_tab(&mut self, pane: PaneId, delta: isize) -> Result<()> {
        let p = self.pane_mut(pane)?;
        let n = p.tabs.len() as isize;
        if n == 0 {
            return Ok(());
        }
        p.active = (p.active as isize + delta).rem_euclid(n) as usize;
        Ok(())
    }

    pub fn close_tab(&mut self, pane: PaneId, idx: usize) -> Result<ViewRef> {
        let view = self.take_tab(pane, idx)?;
        self.remove_pane_if_empty(pane)?;
        self.repair();
        Ok(view)
    }

    /// Closes the tab showing `view`, wherever it is.
    pub fn close_view(&mut self, view: &ViewRef) -> Result<()> {
        if let Some((pane, idx)) = self.find_view(view) {
            self.close_tab(pane, idx)?;
        }
        Ok(())
    }

    fn take_tab(&mut self, pane: PaneId, idx: usize) -> Result<ViewRef> {
        let p = self.pane_mut(pane)?;
        if idx >= p.tabs.len() {
            return Err(LayoutError::TabOutOfRange);
        }
        let view = p.tabs.remove(idx);
        if p.active > idx || p.active >= p.tabs.len() {
            p.active = p.active.saturating_sub(1);
        }
        Ok(view)
    }

    /// Moves one tab into a pane or onto a pane's edge.
    pub fn move_tab(&mut self, from: PaneId, idx: usize, drop: Drop) -> Result<()> {
        let drop = self.land_in_empty(drop)?;
        let target = drop.pane();
        let target_window = self.window_of(target)?;
        let tabs = self.pane(from)?.tabs.len();
        if idx >= tabs {
            return Err(LayoutError::TabOutOfRange);
        }
        if let Drop::Edge(_, _) = drop {
            if self.is_detached(target_window) {
                return Err(LayoutError::DetachedWindowIsSinglePane);
            }
            if from == target && tabs == 1 {
                return Ok(());
            }
        }
        if drop == Drop::Into(from) {
            return Ok(());
        }
        let view = self.take_tab(from, idx)?;
        let shown = match drop {
            Drop::Into(t) => {
                let p = self.pane_mut(t)?;
                p.tabs.push(view);
                p.active = p.tabs.len() - 1;
                p.collapsed = false;
                t
            }
            Drop::Edge(t, edge) => {
                let np = self.new_pane(vec![view]);
                let sid = SplitId(self.alloc());
                let w = self.window_mut(target_window)?;
                insert_beside(&mut w.root, t, np, edge, sid);
                np
            }
        };
        self.remove_pane_if_empty(from)?;
        self.focus(shown)?;
        self.repair();
        Ok(())
    }

    /// Moves a whole pane (all its tabs) into another pane or beside it.
    pub fn move_pane(&mut self, pane: PaneId, drop: Drop) -> Result<()> {
        let drop = self.land_in_empty(drop)?;
        let target = drop.pane();
        if pane == target {
            return Ok(());
        }
        let from_window = self.window_of(pane)?;
        let target_window = self.window_of(target)?;
        if matches!(drop, Drop::Edge(..)) && self.is_detached(target_window) {
            return Err(LayoutError::DetachedWindowIsSinglePane);
        }
        if from_window == MAIN_WINDOW && self.panes_in(MAIN_WINDOW)?.len() == 1 {
            return Err(LayoutError::MainWindowLastPane);
        }
        match drop {
            Drop::Into(t) => {
                let tabs = std::mem::take(&mut self.pane_mut(pane)?.tabs);
                let p = self.pane_mut(t)?;
                if !tabs.is_empty() {
                    p.active = p.tabs.len();
                }
                p.tabs.extend(tabs);
                p.collapsed = false;
                self.remove_pane_if_empty(pane)?;
                self.focus(t)?;
            }
            Drop::Edge(t, edge) => {
                self.detach_node(from_window, pane)?;
                let sid = SplitId(self.alloc());
                let w = self.window_mut(target_window)?;
                insert_beside(&mut w.root, t, pane, edge, sid);
                self.pane_mut(pane)?.collapsed = false;
                self.focus(pane)?;
            }
        }
        self.repair();
        Ok(())
    }

    /// An empty pane (the main window's placeholder) is filled, never split.
    fn land_in_empty(&self, drop: Drop) -> Result<Drop> {
        Ok(match drop {
            Drop::Edge(t, _) if self.pane(t)?.tabs.is_empty() => Drop::Into(t),
            other => other,
        })
    }

    /// Removes a pane's node from its window tree without deleting the pane.
    /// A detached window losing its only pane is removed.
    fn detach_node(&mut self, window: WindowId, pane: PaneId) -> Result<()> {
        if self.is_detached(window) {
            self.windows.retain(|w| w.id != window);
            return Ok(());
        }
        let w = self.window_mut(window)?;
        if matches!(w.root, Node::Pane(p) if p == pane) {
            return Err(LayoutError::MainWindowLastPane);
        }
        remove_pane_node(&mut w.root, pane);
        Ok(())
    }

    /// Drops an empty pane, unless it is the main window's last one.
    fn remove_pane_if_empty(&mut self, pane: PaneId) -> Result<()> {
        if !self.pane(pane)?.tabs.is_empty() {
            return Ok(());
        }
        let window = self.window_of(pane)?;
        if window == MAIN_WINDOW {
            if self.panes_in(MAIN_WINDOW)?.len() == 1 {
                let p = self.pane_mut(pane)?;
                p.active = 0;
                p.collapsed = false;
                return Ok(());
            }
            remove_pane_node(&mut self.window_mut(window)?.root, pane);
        } else {
            self.windows.retain(|w| w.id != window);
        }
        self.panes.remove(&pane);
        Ok(())
    }

    // ---------------------------------------------------------------- pane state

    pub fn focus(&mut self, pane: PaneId) -> Result<()> {
        let window = self.window_of(pane)?;
        self.window_mut(window)?.focused = pane;
        Ok(())
    }

    pub fn toggle_collapse(&mut self, pane: PaneId) -> Result<()> {
        let window = self.window_of(pane)?;
        let collapsed = self.pane(pane)?.collapsed;
        if !collapsed {
            let expanded = self
                .panes_in(window)?
                .into_iter()
                .filter(|p| !self.panes[p].collapsed)
                .count();
            if expanded <= 1 {
                return Err(LayoutError::LastExpandedPane);
            }
            let w = self.window_mut(window)?;
            if w.maximized == Some(pane) {
                w.maximized = None;
            }
        }
        self.pane_mut(pane)?.collapsed = !collapsed;
        self.repair();
        Ok(())
    }

    pub fn toggle_maximize(&mut self, pane: PaneId) -> Result<()> {
        let window = self.window_of(pane)?;
        let w = self.window_mut(window)?;
        w.maximized = if w.maximized == Some(pane) { None } else { Some(pane) };
        self.pane_mut(pane)?.collapsed = false;
        Ok(())
    }

    /// Moves the boundary after child `index` of a split by `delta` (a weight).
    pub fn resize(&mut self, split: SplitId, index: usize, delta: f32) -> Result<()> {
        let s = self
            .windows
            .iter_mut()
            .find_map(|w| find_split_mut(&mut w.root, split))
            .ok_or(LayoutError::UnknownSplit(split))?;
        if index + 1 >= s.children.len() || !delta.is_finite() {
            return Err(LayoutError::TabOutOfRange);
        }
        let a = s.children[index].weight;
        let b = s.children[index + 1].weight;
        let (lo, hi) = (MIN_WEIGHT - a, b - MIN_WEIGHT);
        if lo <= hi {
            let d = delta.clamp(lo, hi);
            s.children[index].weight = a + d;
            s.children[index + 1].weight = b - d;
        }
        normalize(s);
        Ok(())
    }

    /// Sets the boundary after child `index` to `fraction` of the split (0..1).
    pub fn set_boundary(&mut self, split: SplitId, index: usize, fraction: f32) -> Result<()> {
        let s = self.split(split)?;
        if index + 1 >= s.children.len() {
            return Err(LayoutError::TabOutOfRange);
        }
        let current: f32 = s.children[..=index].iter().map(|c| c.weight).sum();
        self.resize(split, index, fraction - current)
    }

    // ---------------------------------------------------------------- windows

    /// Moves a tab into a new window of its own. Returns the new window.
    pub fn detach(&mut self, pane: PaneId, idx: usize) -> Result<WindowId> {
        let window = self.window_of(pane)?;
        let p = self.pane(pane)?;
        if idx >= p.tabs.len() {
            return Err(LayoutError::TabOutOfRange);
        }
        if self.is_detached(window) && p.tabs.len() == 1 {
            return Err(LayoutError::AlreadyAlone);
        }
        let view = self.take_tab(pane, idx)?;
        let np = self.new_pane(vec![view]);
        let wid = WindowId(self.alloc());
        self.windows.push(Window { id: wid, root: Node::Pane(np), focused: np, maximized: None });
        self.remove_pane_if_empty(pane)?;
        self.repair();
        Ok(wid)
    }

    /// Moves a detached window's tabs back into another window, then removes it.
    pub fn attach(&mut self, window: WindowId, drop: Drop) -> Result<()> {
        if !self.is_detached(window) {
            return Err(LayoutError::NotDetached);
        }
        if self.window_of(drop.pane())? == window {
            return Err(LayoutError::SameWindow);
        }
        let Node::Pane(pane) = self.window(window)?.root else {
            return Err(LayoutError::Invalid("detached window root is not a pane".into()));
        };
        self.move_pane(pane, drop)
    }

    /// Closing the main window quits; closing a detached one closes its tabs too.
    pub fn close_window(&mut self, window: WindowId) -> Result<CloseOutcome> {
        if window == MAIN_WINDOW {
            return Ok(CloseOutcome::Quit);
        }
        let Node::Pane(pane) = self.window(window)?.root else {
            return Err(LayoutError::Invalid("detached window root is not a pane".into()));
        };
        self.windows.retain(|w| w.id != window);
        let views = self.panes.remove(&pane).map(|p| p.tabs).unwrap_or_default();
        self.repair();
        Ok(CloseOutcome::Closed(views))
    }

    /// Sends a detached window's tabs back into the main window's focused pane.
    pub fn reattach(&mut self, window: WindowId) -> Result<PaneId> {
        let target = self.main().focused;
        self.attach(window, Drop::Into(target))?;
        Ok(target)
    }

    // ---------------------------------------------------------------- geometry

    /// Pane rectangles of a window for a given size, honouring weights, collapsed
    /// panes (fixed `strip` size along their split's axis) and maximize.
    pub fn pane_rects(&self, window: WindowId, size: (f32, f32), strip: f32) -> Result<Vec<(PaneId, Rect)>> {
        let w = self.window(window)?;
        let full = Rect { x: 0.0, y: 0.0, w: size.0, h: size.1 };
        if let Some(m) = w.maximized {
            return Ok(vec![(m, full)]);
        }
        let mut out = Vec::new();
        geometry::rects(&w.root, full, strip, &self.panes, &mut out);
        Ok(out)
    }

    /// The pane next to `pane` in `dir`, using rectangles for `size`.
    pub fn neighbor(&self, pane: PaneId, dir: Direction, size: (f32, f32), strip: f32) -> Result<Option<PaneId>> {
        let window = self.window_of(pane)?;
        let rects = self.pane_rects(window, size, strip)?;
        Ok(geometry::neighbor(&rects, pane, dir))
    }

    // ---------------------------------------------------------------- persistence

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("layout serializes")
    }

    pub fn from_json(s: &str) -> Result<Self> {
        let layout: Layout = serde_json::from_str(s).map_err(|e| LayoutError::Invalid(e.to_string()))?;
        if layout.version != LAYOUT_VERSION {
            return Err(LayoutError::UnsupportedVersion(layout.version));
        }
        layout.validate()?;
        Ok(layout)
    }

    // ---------------------------------------------------------------- invariants

    /// Restores derived invariants after a structural change: flattens and normalizes
    /// splits, fixes focus and maximize, and keeps one expanded pane per window.
    fn repair(&mut self) {
        for w in &mut self.windows {
            tidy(&mut w.root);
            let mut ids = Vec::new();
            collect_panes(&w.root, &mut ids);
            if !ids.contains(&w.focused) {
                w.focused = ids[0];
            }
            if w.maximized.is_some_and(|m| !ids.contains(&m)) {
                w.maximized = None;
            }
            if ids.iter().all(|p| self.panes[p].collapsed) {
                let keep = w.focused;
                if let Some(p) = self.panes.get_mut(&keep) {
                    p.collapsed = false;
                }
            }
        }
    }

    pub fn validate(&self) -> Result<()> {
        let bad = |m: String| Err(LayoutError::Invalid(m));
        if self.windows.first().map(|w| w.id) != Some(MAIN_WINDOW) {
            return bad("main window must be first".into());
        }
        let mut seen = BTreeSet::new();
        let mut splits = BTreeSet::new();
        let mut views = BTreeSet::new();
        for w in &self.windows {
            let mut ids = Vec::new();
            collect_panes(&w.root, &mut ids);
            for p in &ids {
                if !seen.insert(*p) {
                    return bad(format!("pane {p:?} appears twice"));
                }
                let Some(pane) = self.panes.get(p) else {
                    return bad(format!("pane {p:?} missing from map"));
                };
                if pane.tabs.is_empty() {
                    if pane.active != 0 {
                        return bad(format!("empty pane {p:?} has active {}", pane.active));
                    }
                    if w.id != MAIN_WINDOW || ids.len() != 1 {
                        return bad(format!("empty pane {p:?} outside a lone main pane"));
                    }
                } else if pane.active >= pane.tabs.len() {
                    return bad(format!("pane {p:?} active out of range"));
                }
                for t in &pane.tabs {
                    if !views.insert(t.clone()) {
                        return bad(format!("view {t} open twice"));
                    }
                }
            }
            if w.id != MAIN_WINDOW && !matches!(w.root, Node::Pane(_)) {
                return bad(format!("detached window {:?} root is not a pane", w.id));
            }
            if !ids.contains(&w.focused) {
                return bad(format!("window {:?} focus outside it", w.id));
            }
            if w.maximized.is_some_and(|m| !ids.contains(&m)) {
                return bad(format!("window {:?} maximized outside it", w.id));
            }
            if ids.iter().all(|p| self.panes[p].collapsed) {
                return bad(format!("window {:?} has no expanded pane", w.id));
            }
            validate_node(&w.root, None, &mut splits)?;
        }
        if seen.len() != self.panes.len() {
            return bad("orphan panes in map".into());
        }
        Ok(())
    }
}

// -------------------------------------------------------------------- tree helpers

fn collect_panes(node: &Node, out: &mut Vec<PaneId>) {
    match node {
        Node::Pane(p) => out.push(*p),
        Node::Split(s) => s.children.iter().for_each(|c| collect_panes(&c.node, out)),
    }
}

fn contains_pane(node: &Node, pane: PaneId) -> bool {
    match node {
        Node::Pane(p) => *p == pane,
        Node::Split(s) => s.children.iter().any(|c| contains_pane(&c.node, pane)),
    }
}

fn find_split(node: &Node, id: SplitId) -> Option<&Split> {
    match node {
        Node::Pane(_) => None,
        Node::Split(s) if s.id == id => Some(s),
        Node::Split(s) => s.children.iter().find_map(|c| find_split(&c.node, id)),
    }
}

fn find_split_mut(node: &mut Node, id: SplitId) -> Option<&mut Split> {
    let Node::Split(s) = node else { return None };
    if s.id == id {
        return Some(s);
    }
    s.children.iter_mut().find_map(|c| find_split_mut(&mut c.node, id))
}

/// Removes the leaf for `pane`; parents are tidied later by [`tidy`].
fn remove_pane_node(node: &mut Node, pane: PaneId) -> bool {
    let Node::Split(s) = node else { return false };
    if let Some(i) = s.children.iter().position(|c| matches!(c.node, Node::Pane(p) if p == pane)) {
        s.children.remove(i);
        return true;
    }
    s.children.iter_mut().any(|c| remove_pane_node(&mut c.node, pane))
}

/// Puts `new` next to `target`, reusing the parent split when its axis matches.
fn insert_beside(node: &mut Node, target: PaneId, new: PaneId, edge: Edge, sid: SplitId) -> bool {
    let axis = edge.axis();
    match node {
        Node::Pane(p) if *p == target => {
            let (a, b) = if edge.is_before() { (new, target) } else { (target, new) };
            *node = Node::Split(Split {
                id: sid,
                axis,
                children: vec![
                    Child { node: Node::Pane(a), weight: 0.5 },
                    Child { node: Node::Pane(b), weight: 0.5 },
                ],
            });
            true
        }
        Node::Pane(_) => false,
        Node::Split(s) => {
            if s.axis == axis
                && let Some(i) = s.children.iter().position(|c| matches!(c.node, Node::Pane(p) if p == target))
            {
                let half = s.children[i].weight / 2.0;
                s.children[i].weight = half;
                let at = if edge.is_before() { i } else { i + 1 };
                s.children.insert(at, Child { node: Node::Pane(new), weight: half });
                return true;
            }
            s.children.iter_mut().any(|c| insert_beside(&mut c.node, target, new, edge, sid))
        }
    }
}

/// Simplifies one-child splits, flattens same-axis nesting and normalizes weights.
fn tidy(node: &mut Node) {
    let Node::Split(s) = node else { return };
    for c in &mut s.children {
        tidy(&mut c.node);
    }
    let axis = s.axis;
    let mut flat = Vec::with_capacity(s.children.len());
    for c in s.children.drain(..) {
        match c.node {
            Node::Split(inner) if inner.axis == axis => {
                let total: f32 = inner.children.iter().map(|x| x.weight).sum();
                for ic in inner.children {
                    flat.push(Child { weight: c.weight * ic.weight / total, node: ic.node });
                }
            }
            other => flat.push(Child { node: other, weight: c.weight }),
        }
    }
    s.children = flat;
    if s.children.len() == 1 {
        let only = s.children.pop().expect("one child").node;
        *node = only;
        return;
    }
    normalize(s);
}

fn normalize(s: &mut Split) {
    for c in &mut s.children {
        if !c.weight.is_finite() || c.weight <= 0.0 {
            c.weight = MIN_WEIGHT;
        }
    }
    let total: f32 = s.children.iter().map(|c| c.weight).sum();
    for c in &mut s.children {
        c.weight /= total;
    }
}

fn validate_node(node: &Node, parent: Option<Axis>, splits: &mut BTreeSet<SplitId>) -> Result<()> {
    let Node::Split(s) = node else { return Ok(()) };
    let bad = |m: String| Err(LayoutError::Invalid(m));
    if !splits.insert(s.id) {
        return bad(format!("split {:?} appears twice", s.id));
    }
    if s.children.len() < 2 {
        return bad(format!("split {:?} has {} children", s.id, s.children.len()));
    }
    if parent == Some(s.axis) {
        return bad(format!("split {:?} nested on the same axis", s.id));
    }
    let total: f32 = s.children.iter().map(|c| c.weight).sum();
    if (total - 1.0).abs() > 1e-3 || s.children.iter().any(|c| !c.weight.is_finite() || c.weight <= 0.0) {
        return bad(format!("split {:?} weights {total}", s.id));
    }
    for c in &s.children {
        validate_node(&c.node, Some(s.axis), splits)?;
    }
    Ok(())
}
