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

pub use geometry::{Direction, Metrics, Rect};

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
    /// Next to the pane, taking half of it.
    Edge(PaneId, Edge),
    /// Next to a whole split (a column, a row, the window), taking half of it (R3.13).
    Around(SplitId, Edge),
}

impl Drop {
    /// Whether it makes a new pane beside something.
    pub fn splits(self) -> bool {
        !matches!(self, Drop::Into(_))
    }
}

/// A part of a window's tree.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum NodeRef {
    Pane(PaneId),
    Split(SplitId),
}

/// What a drop moves: a tab, a whole pane, or a view not yet in the layout (a sidebar row).
#[derive(Clone, Debug, PartialEq)]
pub enum Moved {
    Tab(PaneId, usize),
    Pane(PaneId),
    View(ViewRef),
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
    /// Last known size in logical pixels (detached windows), restored on reopen.
    /// Positions are never kept: Wayland doesn't let apps place windows.
    #[serde(default)]
    pub size: Option<(f32, f32)>,
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
                size: None,
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

    /// The views a window shows: the active tab of each expanded pane, or of its
    /// maximized pane alone.
    pub fn shown_in(&self, window: WindowId) -> Vec<ViewRef> {
        let Ok(w) = self.window(window) else { return Vec::new() };
        let panes = match w.maximized {
            Some(m) => vec![m],
            None => self.panes_in(window).unwrap_or_default(),
        };
        panes.into_iter().filter_map(|p| self.pane(p).ok().filter(|p| !p.collapsed).and_then(|p| p.active_view().cloned())).collect()
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

    /// The window a split is in.
    pub fn window_of_split(&self, id: SplitId) -> Result<WindowId> {
        self.windows.iter().find(|w| find_split(&w.root, id).is_some()).map(|w| w.id).ok_or(LayoutError::UnknownSplit(id))
    }

    /// The pane a drop is at: its pane, or a split's first pane.
    pub fn anchor(&self, drop: Drop) -> Result<PaneId> {
        match drop {
            Drop::Into(p) | Drop::Edge(p, _) => self.pane(p).map(|_| p),
            Drop::Around(s, _) => {
                let mut ids = Vec::new();
                collect_panes(&Node::Split(self.split(s)?.clone()), &mut ids);
                Ok(ids[0])
            }
        }
    }

    /// The drop at a window's `edge`, beside everything in it (R3.13, R5.1).
    pub fn window_edge(&self, window: WindowId, edge: Edge) -> Result<Drop> {
        Ok(match &self.window(window)?.root {
            Node::Split(s) => Drop::Around(s.id, edge),
            Node::Pane(p) => Drop::Edge(*p, edge),
        })
    }

    /// The window a drop lands in.
    pub fn drop_window(&self, drop: Drop) -> Result<WindowId> {
        match drop {
            Drop::Into(p) | Drop::Edge(p, _) => self.window_of(p),
            Drop::Around(s, _) => self.window_of_split(s),
        }
    }

    /// Puts pane `new` beside `drop`'s target in `window` (Edge or Around), and shows the
    /// whole tree again (a maximized pane would hide the new one).
    fn place_beside(&mut self, window: WindowId, drop: Drop, new: PaneId) -> Result<()> {
        let (target, edge) = match drop {
            Drop::Edge(p, e) => (NodeRef::Pane(p), e),
            Drop::Around(s, e) => (NodeRef::Split(s), e),
            Drop::Into(_) => return Err(LayoutError::Invalid("not a drop beside".into())),
        };
        let collapsed: BTreeSet<PaneId> = self.panes.iter().filter(|(_, p)| p.collapsed).map(|(id, _)| *id).collect();
        let sid = SplitId(self.alloc());
        let w = self.window_mut(window)?;
        let expanded = |n: &Node| !matches!(n, Node::Pane(p) if collapsed.contains(p));
        if !insert_beside(&mut w.root, target, &Node::Pane(new), edge, sid, &expanded) {
            return Err(match target {
                NodeRef::Pane(p) => LayoutError::UnknownPane(p),
                NodeRef::Split(s) => LayoutError::UnknownSplit(s),
            });
        }
        w.maximized = None;
        Ok(())
    }

    // ---------------------------------------------------------------- background

    /// Runs `op` as a change the user didn't make (the control API, R13.1): each window
    /// keeps its focused pane and each pane its active tab, unless `op` took them away;
    /// only `target` may show another tab (what the caller asked to show). All or
    /// nothing: a refused `op` leaves the layout as it was.
    pub fn in_background<T>(&mut self, target: Option<PaneId>, op: impl FnOnce(&mut Layout) -> Result<T>) -> Result<T> {
        let mut next = self.clone();
        let out = op(&mut next)?;
        for w in &self.windows {
            if next.window_of(w.focused) == Ok(w.id) {
                next.window_mut(w.id)?.focused = w.focused;
            }
        }
        for (id, p) in &self.panes {
            let Some(view) = p.active_view() else { continue };
            if Some(*id) == target {
                continue;
            }
            if let Some(q) = next.panes.get_mut(id)
                && let Some(i) = q.tabs.iter().position(|t| t == view)
            {
                q.active = i;
            }
        }
        *self = next;
        Ok(out)
    }

    // ---------------------------------------------------------------- tabs

    /// Opens `view` at `drop` (into a pane or beside something); an open view moves there.
    pub fn open_at(&mut self, view: ViewRef, drop: Drop) -> Result<()> {
        if let Some((p, i)) = self.find_view(&view) {
            return self.move_tab(p, i, drop);
        }
        let drop = self.land_in_empty(drop)?;
        if let Drop::Into(p) = drop {
            return self.open(view, p).map(|_| ());
        }
        // Beside something: a pane of its own, no other pane touched.
        let window = self.drop_window(drop)?;
        if self.is_detached(window) {
            return Err(LayoutError::DetachedWindowIsSinglePane);
        }
        let np = self.new_pane(vec![view]);
        self.place_beside(window, drop, np)?;
        self.focus(np)?;
        self.repair();
        Ok(())
    }

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

    /// Brings `view` on screen (an "Open" from a notification): its tab active, its pane
    /// expanded and focused, and another pane maximized in its window restored. Not open:
    /// it opens in the main window's focused pane. Returns its pane and whether it was
    /// open already.
    pub fn reveal(&mut self, view: ViewRef) -> Result<(PaneId, bool)> {
        let open = self.find_view(&view).is_some();
        let pane = match self.find_view(&view) {
            Some((pane, idx)) => {
                self.activate(pane, idx)?;
                pane
            }
            None => {
                let target = self.main().focused;
                self.open(view, target)?
            }
        };
        let window = self.window_of(pane)?;
        let w = self.window_mut(window)?;
        if w.maximized.is_some_and(|m| m != pane) {
            w.maximized = None;
        }
        Ok((pane, open))
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
        self.remove_pane_if_empty(pane, None)?;
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

    /// Moves tab `idx` of `from` into `to`'s tabs at `at`, counted before it leaves (it
    /// lands before the tab now at `at`; the tab count puts it last): a reorder within a
    /// pane, or a move to a place in another. It becomes the active tab, focused.
    pub fn move_tab_to(&mut self, from: PaneId, idx: usize, to: PaneId, at: usize) -> Result<()> {
        if idx >= self.pane(from)?.tabs.len() || at > self.pane(to)?.tabs.len() {
            return Err(LayoutError::TabOutOfRange);
        }
        if from == to && (at == idx || at == idx + 1) {
            return self.activate(from, idx);
        }
        let receiver = self.receiver(to, true);
        let view = self.take_tab(from, idx)?;
        let at = if from == to && idx < at { at - 1 } else { at };
        let p = self.pane_mut(to)?;
        p.tabs.insert(at, view);
        p.active = at;
        p.collapsed = false;
        self.remove_pane_if_empty(from, receiver)?;
        self.focus(to)?;
        self.repair();
        Ok(())
    }

    /// Moves one tab into a pane or onto a pane's edge.
    pub fn move_tab(&mut self, from: PaneId, idx: usize, drop: Drop) -> Result<()> {
        let drop = self.land_in_empty(drop)?;
        let target_window = self.drop_window(drop)?;
        let tabs = self.pane(from)?.tabs.len();
        if idx >= tabs {
            return Err(LayoutError::TabOutOfRange);
        }
        if drop.splits() {
            if self.is_detached(target_window) {
                return Err(LayoutError::DetachedWindowIsSinglePane);
            }
            if matches!(drop, Drop::Edge(t, _) if t == from) && tabs == 1 {
                return Ok(());
            }
            // Its only tab: the pane itself moves, leaving its place first, so what it
            // lands beside is measured without it.
            if tabs == 1 {
                return self.move_pane(from, drop);
            }
        }
        if drop == Drop::Into(from) {
            return Ok(());
        }
        let receiver = match drop {
            Drop::Into(t) => self.receiver(t, true),
            _ => None,
        };
        let view = self.take_tab(from, idx)?;
        let shown = match drop {
            Drop::Into(t) => {
                let p = self.pane_mut(t)?;
                p.tabs.push(view);
                p.active = p.tabs.len() - 1;
                p.collapsed = false;
                t
            }
            Drop::Edge(..) | Drop::Around(..) => {
                let np = self.new_pane(vec![view]);
                self.place_beside(target_window, drop, np)?;
                np
            }
        };
        self.remove_pane_if_empty(from, receiver)?;
        self.focus(shown)?;
        self.repair();
        Ok(())
    }

    /// Moves a whole pane (all its tabs) into another pane or beside it.
    pub fn move_pane(&mut self, pane: PaneId, drop: Drop) -> Result<()> {
        let drop = self.land_in_empty(drop)?;
        if matches!(drop, Drop::Into(t) | Drop::Edge(t, _) if t == pane) {
            return Ok(());
        }
        let from_window = self.window_of(pane)?;
        let target_window = self.drop_window(drop)?;
        if drop.splits() && self.is_detached(target_window) {
            return Err(LayoutError::DetachedWindowIsSinglePane);
        }
        if from_window == MAIN_WINDOW && self.panes_in(MAIN_WINDOW)?.len() == 1 {
            return Err(LayoutError::MainWindowLastPane);
        }
        match drop {
            Drop::Into(t) => {
                let receiver = self.receiver(t, true);
                let tabs = std::mem::take(&mut self.pane_mut(pane)?.tabs);
                let p = self.pane_mut(t)?;
                if !tabs.is_empty() {
                    p.active = p.tabs.len();
                }
                p.tabs.extend(tabs);
                p.collapsed = false;
                self.remove_pane_if_empty(pane, receiver)?;
                self.focus(t)?;
            }
            Drop::Edge(t, edge) if self.reorder(pane, t, edge)? => {
                self.pane_mut(pane)?.collapsed = false;
                self.focus(pane)?;
            }
            Drop::Edge(..) | Drop::Around(..) => {
                // Beside a pane: that pane takes its space first. Beside a split: the
                // one before it, as when it closes.
                let receiver = match drop {
                    Drop::Edge(t, _) => self.receiver(t, false),
                    _ => None,
                };
                self.detach_node(from_window, pane, receiver)?;
                self.place_beside(target_window, drop, pane)?;
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

    /// Removes a pane's node from its window tree without deleting the pane, its space
    /// going to `receiver`'s side (see [`remove_pane_node`]). A detached window losing
    /// its only pane is removed.
    fn detach_node(&mut self, window: WindowId, pane: PaneId, receiver: Option<Receiver>) -> Result<()> {
        if self.is_detached(window) {
            self.windows.retain(|w| w.id != window);
            return Ok(());
        }
        let collapsed = self.collapsed();
        let w = self.window_mut(window)?;
        if matches!(w.root, Node::Pane(p) if p == pane) {
            return Err(LayoutError::MainWindowLastPane);
        }
        remove_pane_node(&mut w.root, pane, receiver, &|n| !matches!(n, Node::Pane(p) if collapsed.contains(p)));
        Ok(())
    }

    fn collapsed(&self) -> BTreeSet<PaneId> {
        self.panes.iter().filter(|(_, p)| p.collapsed).map(|(id, _)| *id).collect()
    }

    /// The pane another's space goes towards: one it merges into (`opens`: the merge
    /// expands it), or one it lands beside. Read before the move changes it.
    fn receiver(&self, pane: PaneId, opens: bool) -> Option<Receiver> {
        self.pane(pane).ok().map(|p| Receiver { pane, was_collapsed: p.collapsed, opens })
    }

    /// A pane put beside a pane of its own split, along that split: just its place changes,
    /// with its own size. Whether it was that.
    fn reorder(&mut self, pane: PaneId, target: PaneId, edge: Edge) -> Result<bool> {
        let (Some((s1, i, axis)), Some((s2, j, _))) = (self.parent_of(pane), self.parent_of(target)) else { return Ok(false) };
        if s1 != s2 || axis != edge.axis() || pane == target {
            return Ok(false);
        }
        let s = self.windows.iter_mut().find_map(|w| find_split_mut(&mut w.root, s1)).ok_or(LayoutError::UnknownSplit(s1))?;
        let child = s.children.remove(i);
        let j = if j > i { j - 1 } else { j };
        s.children.insert(if edge.is_before() { j } else { j + 1 }, child);
        Ok(true)
    }

    /// Drops an empty pane, unless it is the main window's last one; its space goes to
    /// `receiver`'s side.
    fn remove_pane_if_empty(&mut self, pane: PaneId, receiver: Option<Receiver>) -> Result<()> {
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
            let collapsed = self.collapsed();
            remove_pane_node(&mut self.window_mut(window)?.root, pane, receiver, &|n| !matches!(n, Node::Pane(p) if collapsed.contains(p)));
        } else {
            self.windows.retain(|w| w.id != window);
        }
        self.panes.remove(&pane);
        Ok(())
    }

    // ---------------------------------------------------------------- pane state

    /// Remembers a window's size (logical pixels).
    pub fn set_window_size(&mut self, window: WindowId, size: (f32, f32)) -> Result<()> {
        self.window_mut(window)?.size = Some(size);
        Ok(())
    }

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
        self.windows.push(Window { id: wid, root: Node::Pane(np), focused: np, maximized: None, size: None });
        self.remove_pane_if_empty(pane, None)?;
        self.repair();
        Ok(wid)
    }

    /// Moves a detached window's tabs back into another window, then removes it.
    pub fn attach(&mut self, window: WindowId, drop: Drop) -> Result<()> {
        if !self.is_detached(window) {
            return Err(LayoutError::NotDetached);
        }
        if self.drop_window(drop)? == window {
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

    // ---------------------------------------------------------------- send to (R3.11)

    /// Every pane, numbered for "Send to…": the main window first, then detached
    /// windows in creation order; inside a window, reading order (top to bottom, then
    /// left to right), ignoring maximize.
    pub fn numbered_panes(&self) -> Vec<(PaneId, WindowId)> {
        let mut plain = self.clone();
        for w in &mut plain.windows {
            w.maximized = None;
        }
        let mut windows: Vec<WindowId> = plain.windows.iter().map(|w| w.id).collect();
        windows.sort_by_key(|w| (*w != MAIN_WINDOW, w.0));
        let mut out = Vec::new();
        for w in windows {
            let mut rects = plain.pane_rects(w, (1000.0, 1000.0), Metrics::uniform(0.0)).unwrap_or_default();
            rects.sort_by(|a, b| (a.1.y, a.1.x).partial_cmp(&(b.1.y, b.1.x)).unwrap_or(std::cmp::Ordering::Equal));
            out.extend(rects.into_iter().map(|(p, _)| (p, w)));
        }
        out
    }

    /// Sends a tab into `target`, or into a new window when `None`.
    pub fn send_tab(&mut self, view: &ViewRef, target: Option<PaneId>) -> Result<()> {
        let (pane, idx) = self.find_view(view).ok_or(LayoutError::TabOutOfRange)?;
        match target {
            Some(t) if t == pane => Ok(()),
            Some(t) => self.move_tab(pane, idx, Drop::Into(t)),
            None => self.detach(pane, idx).map(|_| ()),
        }
    }

    /// Sends every tab of a pane into `target`, or into a new window when `None`.
    pub fn send_pane(&mut self, pane: PaneId, target: Option<PaneId>) -> Result<()> {
        match target {
            Some(t) if t == pane => Ok(()),
            Some(t) => self.move_pane(pane, Drop::Into(t)),
            None => {
                let tabs = self.pane(pane)?.tabs.clone();
                let window = self.window_of(pane)?;
                if self.is_detached(window) && self.panes_in(window)?.len() == 1 {
                    return Err(LayoutError::AlreadyAlone);
                }
                if tabs.is_empty() {
                    return Err(LayoutError::TabOutOfRange);
                }
                let new_window = self.detach(pane, 0)?;
                let new_pane = self.window(new_window)?.focused;
                for v in tabs.iter().skip(1) {
                    self.send_tab(v, Some(new_pane))?;
                }
                Ok(())
            }
        }
    }

    // ---------------------------------------------------------------- geometry

    /// Rectangles of a window's panes and splits for a given size, as the app draws them
    /// with `m`: weights, dividers, collapsed panes, maximize (only that pane).
    pub fn node_rects(&self, window: WindowId, size: (f32, f32), m: Metrics) -> Result<Vec<(NodeRef, Rect)>> {
        let w = self.window(window)?;
        let full = Rect { x: 0.0, y: 0.0, w: size.0, h: size.1 };
        if let Some(p) = w.maximized {
            return Ok(vec![(NodeRef::Pane(p), full)]);
        }
        let mut out = Vec::new();
        geometry::rects(&w.root, full, m, &self.panes, &mut out);
        Ok(out)
    }

    /// The panes' rectangles of [`Layout::node_rects`].
    pub fn pane_rects(&self, window: WindowId, size: (f32, f32), m: Metrics) -> Result<Vec<(PaneId, Rect)>> {
        Ok(self
            .node_rects(window, size, m)?
            .into_iter()
            .filter_map(|(n, r)| match n {
                NodeRef::Pane(p) => Some((p, r)),
                NodeRef::Split(_) => None,
            })
            .collect())
    }

    /// The pane next to `pane` in `dir`, using rectangles for `size`.
    pub fn neighbor(&self, pane: PaneId, dir: Direction, size: (f32, f32), m: Metrics) -> Result<Option<PaneId>> {
        let window = self.window_of(pane)?;
        let rects = self.pane_rects(window, size, m)?;
        Ok(geometry::neighbor(&rects, pane, dir))
    }

    /// The drop under `at`, in a window drawn at `size` with `m` (R3.4, R3.13). Over a
    /// pane's body: its centre is `Into`; nearer an edge than a quarter of the body, the
    /// edge band is `Edge(pane)`, and at its very outside one strip per enclosing split
    /// whose side is this pane's own (`Around`, the outermost at the edge). Strips are
    /// `strip` wide but take at most 2/3 of the band. `None` over headers, dividers and
    /// collapsed panes (their tab strips take drops themselves). A detached window and a
    /// maximized pane offer no outer strips; a detached one no edges either.
    pub fn drop_at(&self, window: WindowId, size: (f32, f32), m: Metrics, strip: f32, at: (f32, f32)) -> Result<Option<Drop>> {
        let w = self.window(window)?;
        let found = self.pane_rects(window, size, m)?.into_iter().find(|(_, r)| r.contains(at));
        let Some((pane, r)) = found else { return Ok(None) };
        if self.pane(pane)?.collapsed {
            return Ok(None);
        }
        let body = Rect { x: r.x, y: r.y + m.header, w: r.w, h: r.h - m.header };
        if body.w <= 0.0 || body.h <= 0.0 || !body.contains(at) {
            return Ok(None);
        }
        let sides = [
            (Edge::Left, at.0 - body.x, body.w),
            (Edge::Right, body.right() - at.0, body.w),
            (Edge::Top, at.1 - body.y, body.h),
            (Edge::Bottom, body.bottom() - at.1, body.h),
        ];
        let (edge, d, len) = sides.into_iter().min_by(|a, b| (a.1 / a.2).total_cmp(&(b.1 / b.2))).expect("four sides");
        if d / len >= 0.25 || self.is_detached(window) {
            return Ok(Some(Drop::Into(pane)));
        }
        let outer = if w.maximized.is_some() { Vec::new() } else { self.outer_splits(window, pane, edge)? };
        let total = (outer.len() as f32 * strip).min(len * 0.25 * 2.0 / 3.0);
        if !outer.is_empty() && d < total {
            let each = total / outer.len() as f32;
            let k = ((d / each) as usize).min(outer.len() - 1);
            return Ok(Some(Drop::Around(outer[outer.len() - 1 - k], edge)));
        }
        Ok(Some(Drop::Edge(pane, edge)))
    }

    /// The splits whose `edge` side is `pane`'s own, innermost first: every enclosing split
    /// across the edge's axis, and one along it while `pane` is at its end on that side
    /// (collapsed panes after it don't count). Worked out from the tree, not from rectangles.
    fn outer_splits(&self, window: WindowId, pane: PaneId, edge: Edge) -> Result<Vec<SplitId>> {
        fn path(node: &Node, pane: PaneId, out: &mut Vec<(SplitId, Axis, bool, bool)>, collapsed: &dyn Fn(&Node) -> bool) -> bool {
            let Node::Split(s) = node else { return matches!(node, Node::Pane(p) if *p == pane) };
            for (i, c) in s.children.iter().enumerate() {
                if path(&c.node, pane, out, collapsed) {
                    let first = s.children[..i].iter().all(|c| collapsed(&c.node));
                    let last = s.children[i + 1..].iter().all(|c| collapsed(&c.node));
                    out.push((s.id, s.axis, first, last));
                    return true;
                }
            }
            false
        }
        let collapsed = |n: &Node| matches!(n, Node::Pane(p) if self.panes.get(p).is_some_and(|p| p.collapsed));
        let mut chain = Vec::new();
        path(&self.window(window)?.root, pane, &mut chain, &collapsed);
        let mut out = Vec::new();
        for (id, axis, first, last) in chain {
            if axis == edge.axis() && !(if edge.is_before() { first } else { last }) {
                break;
            }
            out.push(id);
        }
        Ok(out)
    }

    /// Where `what` lands with `drop`: the rectangle of the pane holding it afterwards, in
    /// the drop's window drawn at `size` with `m`. Computed by doing it on a copy, so it
    /// is the real result; `None` when the drop would be refused.
    pub fn preview(&self, what: &Moved, drop: Drop, size: (f32, f32), m: Metrics) -> Option<Rect> {
        let window = self.drop_window(drop).ok()?;
        let view = match what {
            Moved::Tab(p, i) => self.pane(*p).ok()?.tabs.get(*i)?.clone(),
            Moved::Pane(p) => self.pane(*p).ok()?.tabs.first()?.clone(),
            Moved::View(v) => v.clone(),
        };
        let mut l = self.clone();
        match what {
            Moved::Tab(p, i) => l.move_tab(*p, *i, drop),
            Moved::Pane(p) => l.move_pane(*p, drop),
            Moved::View(v) => l.open_at(v.clone(), drop),
        }
        .ok()?;
        let (pane, _) = l.find_view(&view)?;
        if l.window_of(pane).ok()? != window {
            return None;
        }
        l.pane_rects(window, size, m).ok()?.into_iter().find(|(p, _)| *p == pane).map(|(_, r)| r)
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
        let collapsed = self.collapsed();
        for w in &mut self.windows {
            tidy(&mut w.root, &|n| !matches!(n, Node::Pane(p) if collapsed.contains(p)));
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

/// The pane a removed pane's space goes towards (what it merged into, or lands beside).
#[derive(Copy, Clone, Debug)]
struct Receiver {
    pane: PaneId,
    was_collapsed: bool,
    /// The move expands it (a merge into it).
    opens: bool,
}

/// Removes the leaf for `pane`, its weight going whole to one sibling, so every other
/// part keeps its exact size: the sibling holding `receiver` (when it is expanded, or is
/// the receiver a merge opens), else the expanded one before it, else the one after (else
/// any, to keep the weights whole). A collapsed receiver the merge opens takes the weight
/// instead of adding it (its own never drew anything); a collapsed pane gives nothing.
/// Parents are tidied later by [`tidy`].
fn remove_pane_node(node: &mut Node, pane: PaneId, receiver: Option<Receiver>, expanded: &dyn Fn(&Node) -> bool) -> bool {
    let Node::Split(s) = node else { return false };
    if let Some(i) = s.children.iter().position(|c| matches!(c.node, Node::Pane(p) if p == pane)) {
        let gone = s.children.remove(i);
        if !expanded(&gone.node) || s.children.is_empty() {
            return true;
        }
        let opened = |r: &Receiver, c: &Child| r.opens && matches!(c.node, Node::Pane(p) if p == r.pane);
        let holds = receiver.and_then(|r| s.children.iter().position(|c| contains_pane(&c.node, r.pane) && (expanded(&c.node) || opened(&r, c))));
        let before = (0..i).rev().find(|&j| expanded(&s.children[j].node));
        let after = (i..s.children.len()).find(|&j| expanded(&s.children[j].node));
        let to = holds.or(before).or(after).unwrap_or(i.saturating_sub(1).min(s.children.len() - 1));
        let into_collapsed = receiver.is_some_and(|r| r.was_collapsed && opened(&r, &s.children[to]));
        if into_collapsed {
            s.children[to].weight = gone.weight;
        } else {
            s.children[to].weight += gone.weight;
        }
        return true;
    }
    s.children.iter_mut().any(|c| remove_pane_node(&mut c.node, pane, receiver, expanded))
}

impl Node {
    fn is(&self, r: NodeRef) -> bool {
        match (self, r) {
            (Node::Pane(p), NodeRef::Pane(q)) => *p == q,
            (Node::Split(s), NodeRef::Split(id)) => s.id == id,
            _ => false,
        }
    }
}

/// Puts `new` beside `target` on `edge`, taking half of it: a sibling (half the target's
/// weight) when the parent runs along the edge, else in `target`'s place (see
/// [`place_at`]). `expanded` tells collapsed panes apart.
fn insert_beside(node: &mut Node, target: NodeRef, new: &Node, edge: Edge, sid: SplitId, expanded: &dyn Fn(&Node) -> bool) -> bool {
    if node.is(target) {
        place_at(node, new.clone(), edge, sid, expanded);
        return true;
    }
    let Node::Split(s) = node else { return false };
    if let Some(i) = s.children.iter().position(|c| c.node.is(target)) {
        if s.axis == edge.axis() {
            let half = s.children[i].weight / 2.0;
            s.children[i].weight = half;
            let at = if edge.is_before() { i } else { i + 1 };
            s.children.insert(at, Child { node: new.clone(), weight: half });
        } else {
            place_at(&mut s.children[i].node, new.clone(), edge, sid, expanded);
        }
        return true;
    }
    s.children.iter_mut().any(|c| insert_beside(&mut c.node, target, new, edge, sid, expanded))
}

/// Puts `new` at `node`'s place on `edge`: a split running along the edge takes it as
/// its first or last child, weighted as all its expanded children (so it gets half of
/// the expanded space, whatever `tidy` and collapsed panes do); anything else is wrapped
/// with it in a new split, half and half.
fn place_at(node: &mut Node, new: Node, edge: Edge, sid: SplitId, expanded: &dyn Fn(&Node) -> bool) {
    let axis = edge.axis();
    if let Node::Split(s) = node
        && s.axis == axis
    {
        let weight = s.children.iter().filter(|c| expanded(&c.node)).map(|c| c.weight).sum::<f32>().max(MIN_WEIGHT);
        let at = if edge.is_before() { 0 } else { s.children.len() };
        s.children.insert(at, Child { node: new, weight });
        return;
    }
    let old = std::mem::replace(node, Node::Pane(PaneId(0)));
    let (a, b) = if edge.is_before() { (new, old) } else { (old, new) };
    *node = Node::Split(Split { id: sid, axis, children: vec![Child { node: a, weight: 0.5 }, Child { node: b, weight: 0.5 }] });
}

/// Simplifies one-child splits, flattens same-axis nesting and normalizes weights. A
/// flattened split's weight goes to its expanded children (`expanded`): a collapsed pane
/// draws at its fixed size whatever its weight, so a share of it would be lost to the
/// others; it keeps its own proportion, for when it opens again.
fn tidy(node: &mut Node, expanded: &dyn Fn(&Node) -> bool) {
    let Node::Split(s) = node else { return };
    for c in &mut s.children {
        tidy(&mut c.node, expanded);
    }
    let axis = s.axis;
    let mut flat = Vec::with_capacity(s.children.len());
    for c in s.children.drain(..) {
        match c.node {
            Node::Split(inner) if inner.axis == axis => {
                let total: f32 = inner.children.iter().map(|x| x.weight).sum();
                let open: f32 = inner.children.iter().filter(|x| expanded(&x.node)).map(|x| x.weight).sum();
                for ic in inner.children {
                    let of = if open > 0.0 && expanded(&ic.node) { open } else { total };
                    flat.push(Child { weight: c.weight * ic.weight / of, node: ic.node });
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
