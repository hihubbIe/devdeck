//! The generic sidebar list (R4.1–R4.3): Pinned then All, both alphabetical, with a
//! filter that hides rows but never reorders them, and a "running only" switch.
//!
//! Items with a `path` (`front-end/apps/shellapp`) make All a tree that keeps their
//! folders: a folder is an item's own row when it is one (a monorepo's root app),
//! otherwise a folder row; both open and close, collapsed by default, and a collapsed
//! row says what's inside (how many, something running, a warning). While filtering
//! (or "running only"), the matches show with their folders open. Pinned stays flat.
//! Items without a path give a flat list.
//!
//! Both sections fold from their header (R4.7). All holds the groups the user made
//! (R4.8), first at each level: an item or a folder put in a group shows there with all
//! it holds, and keeps its real path for opening and selection.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// The groups the user made in a list's All section (R4.8), nested by path.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Groups {
    /// Every group by path (`front-end`, `front-end/web`); a group's parents are in it.
    pub folders: BTreeSet<String>,
    /// Node path → group path. A node is an item (its path, else its key) or a folder of
    /// the tree (its path); it moves with all it holds.
    pub members: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ListItem {
    /// Stable identity (pins and selection refer to it).
    pub key: String,
    pub label: String,
    /// Dim text on the right (a port, a tag, a ready count).
    pub detail: String,
    pub running: bool,
    /// Something in progress on it (a rollout): drawn before its detail.
    pub busy: bool,
    /// Why it can't be used (a missing tool): drawn with ⚠ instead of the state glyph.
    pub warning: Option<String>,
    /// Where it sits in the tree (`/`-separated); `None`: at the top.
    pub path: Option<String>,
    /// Drawn de-emphasized (a library, R6.2).
    pub dim: bool,
    /// Shown on hover instead of the label (what the detail leaves out).
    pub hint: Option<String>,
}

impl ListItem {
    pub fn new(key: impl Into<String>, label: impl Into<String>) -> Self {
        Self { key: key.into(), label: label.into(), detail: String::new(), running: false, busy: false, warning: None, path: None, dim: false, hint: None }
    }

    pub fn hint(mut self, h: Option<String>) -> Self {
        self.hint = h;
        self
    }

    pub fn path(mut self, p: impl Into<String>) -> Self {
        self.path = Some(p.into());
        self
    }

    pub fn dim(mut self, on: bool) -> Self {
        self.dim = on;
        self
    }

    pub fn detail(mut self, d: impl Into<String>) -> Self {
        self.detail = d.into();
        self
    }

    pub fn running(mut self, on: bool) -> Self {
        self.running = on;
        self
    }

    pub fn busy(mut self, on: bool) -> Self {
        self.busy = on;
        self
    }

    pub fn warning(mut self, w: Option<String>) -> Self {
        self.warning = w;
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Pinned,
    All,
}

impl Section {
    /// Its name in the saved folded sections.
    pub fn id(self) -> &'static str {
        match self {
            Section::Pinned => "pinned",
            Section::All => "all",
        }
    }
}

/// What a row with children holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Kids {
    pub open: bool,
    /// Items below it.
    pub count: usize,
    /// Something below it runs.
    pub running: bool,
    /// Something below it has a warning.
    pub warning: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    /// A section's header: whether it's unfolded, and the items it shows (those inside
    /// closed rows included, and while folded).
    Header { section: Section, open: bool, count: usize },
    /// A section with nothing to show (explains why).
    Empty(Section),
    /// Index into the items given to [`rows`], whether it is pinned, its level in the
    /// tree (0 at the top) and, when items sit below it, what they are.
    Item { index: usize, pinned: bool, depth: usize, kids: Option<Kids> },
    /// A folder that holds items without being one, or a group the user made (its
    /// path is then [`group_row_path`]).
    Folder { path: String, name: String, depth: usize, kids: Kids, group: bool },
}

impl Row {
    pub fn is_item(&self) -> bool {
        matches!(self, Row::Item { .. })
    }

    /// Items and folders: what a selection can be on.
    pub fn is_selectable(&self) -> bool {
        matches!(self, Row::Item { .. } | Row::Folder { .. })
    }

    /// Its level in the tree (0 at the top, and for headers).
    pub fn depth(&self) -> usize {
        match self {
            Row::Item { depth, .. } | Row::Folder { depth, .. } => *depth,
            _ => 0,
        }
    }

    /// What it holds, when items sit below it.
    pub fn kids(&self) -> Option<Kids> {
        match self {
            Row::Item { kids, .. } => *kids,
            Row::Folder { kids, .. } => Some(*kids),
            _ => None,
        }
    }

    /// The tree path this row opens and closes, when it has children.
    pub fn open_path<'a>(&'a self, items: &'a [ListItem]) -> Option<&'a str> {
        match self {
            Row::Folder { path, .. } => Some(path),
            Row::Item { index, kids: Some(_), .. } => items[*index].path.as_deref(),
            _ => None,
        }
    }
}

/// The selection key of a folder row (an item's key never ends with `/`).
pub fn folder_key(path: &str) -> String {
    format!("{path}/")
}

/// Starts a group's row path, which no real path can start with.
const GROUP_TAG: char = '\u{1}';
/// Starts the tree segment of a node moved into a group (its real path follows).
const MOVED_TAG: char = '\u{2}';

/// The row path of group `group` (`front-end/web`): it opens through the same set as
/// folders, and never meets a real path.
pub fn group_row_path(group: &str) -> String {
    format!("{GROUP_TAG}{group}")
}

/// The group a row path is of, when it's a group's.
pub fn group_of_row(path: &str) -> Option<&str> {
    path.strip_prefix(GROUP_TAG)
}

/// The node path of an item, what groups hold: its tree path, else its key.
pub fn node_path(item: &ListItem) -> &str {
    item.path.as_deref().unwrap_or(&item.key)
}

/// Case-insensitive subsequence match: `ensvc` matches `entityservice`.
pub fn matches(label: &str, filter: &str) -> bool {
    let mut chars = label.chars().flat_map(char::to_lowercase);
    filter
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .all(|f| chars.any(|c| c == f))
}

/// Whether the filter and the "running only" switch keep `item` (the rows also show the
/// folders and items above it, to keep the structure).
pub fn keeps(item: &ListItem, filter: &str, running_only: bool) -> bool {
    matches(&item.label, filter) && (!running_only || item.running)
}

fn order(a: &ListItem, b: &ListItem) -> std::cmp::Ordering {
    a.label.to_lowercase().cmp(&b.label.to_lowercase()).then_with(|| a.key.cmp(&b.key))
}

/// What shapes the rows besides the items and the filter.
#[derive(Clone, Copy)]
pub struct Shape<'a> {
    /// Paths of open folders, open items and open groups ([`group_row_path`]).
    pub open: &'a BTreeSet<String>,
    pub groups: &'a Groups,
    /// Sections folded (by [`Section::id`]).
    pub folded: &'a BTreeSet<String>,
    /// Folder rows' names by path, where a folder's last segment isn't its name (the
    /// Notes list's projects); sorted by that name.
    pub names: &'a BTreeMap<String, String>,
    /// Folders before items at each level, after the groups (the Notes list: projects,
    /// then the notes without one).
    pub folders_first: bool,
}

/// The rows to draw: Pinned header, pinned items (flat), All header, the groups and
/// the other items as a tree. While filtering (or "running only") folded sections and
/// closed rows show their matches.
pub fn rows(items: &[ListItem], pins: &BTreeSet<String>, filter: &str, running_only: bool, shape: Shape) -> Vec<Row> {
    let shown = |i: usize| keeps(&items[i], filter, running_only);
    let narrowing = !filter.trim().is_empty() || running_only;
    let open = |s: Section| narrowing || !shape.folded.contains(s.id());
    let mut pinned: Vec<usize> = (0..items.len()).filter(|&i| pins.contains(&items[i].key) && shown(i)).collect();
    pinned.sort_by(|&a, &b| order(&items[a], &items[b]));
    let mut out = vec![Row::Header { section: Section::Pinned, open: open(Section::Pinned), count: pinned.len() }];
    if open(Section::Pinned) {
        if pinned.is_empty() {
            out.push(Row::Empty(Section::Pinned));
        }
        out.extend(pinned.into_iter().map(|index| Row::Item { index, pinned: true, depth: 0, kids: None }));
    }
    let rest: Vec<usize> = (0..items.len()).filter(|&i| !pins.contains(&items[i].key)).collect();
    let tree = Tree::new(items, &rest, shape);
    out.push(Row::Header { section: Section::All, open: open(Section::All), count: tree.count(&shown) });
    if open(Section::All) {
        let before = out.len();
        tree.emit(&[], 0, &shown, narrowing, shape.open, &mut out);
        if out.len() == before {
            out.push(Row::Empty(Section::All));
        }
    }
    out
}

/// One place of the All tree: an item, a folder, both, or a group.
struct Node {
    item: Option<usize>,
    /// What it opens by and is selected by: its real path (where it is shown may be in
    /// a group), or a group's [`group_row_path`].
    path: String,
    name: String,
    group: bool,
}

/// The All section's items and groups by place (tree segments).
struct Tree<'a> {
    items: &'a [ListItem],
    nodes: BTreeMap<Vec<String>, Node>,
    folders_first: bool,
}

impl<'a> Tree<'a> {
    fn new(items: &'a [ListItem], idx: &[usize], shape: Shape) -> Self {
        let groups = shape.groups;
        let mut nodes: BTreeMap<Vec<String>, Node> = BTreeMap::new();
        // Groups, with their parents.
        let place_of_group = |g: &str| -> Vec<String> { g.split('/').map(|s| format!("{GROUP_TAG}{s}")).collect() };
        for g in &groups.folders {
            let segs: Vec<&str> = g.split('/').collect();
            for k in 1..=segs.len() {
                let path = segs[..k].join("/");
                nodes.entry(place_of_group(&path)).or_insert_with(|| Node { item: None, path: group_row_path(&path), name: segs[k - 1].to_string(), group: true });
            }
        }
        for &i in idx {
            // Without a path an item is a leaf at the top, whatever its key holds.
            let (real, flat): (Vec<String>, bool) = match &items[i].path {
                Some(p) => (p.split('/').filter(|s| !s.is_empty()).map(String::from).collect(), false),
                None => (vec![items[i].key.clone()], true),
            };
            if real.is_empty() {
                continue;
            }
            let anchor = groups.anchor(&real).map(|(k, g)| (k, g.to_string()));
            // Each place from the top down, with what it opens by and its name.
            let mut places: Vec<(Vec<String>, String, String)> = Vec::new();
            let (mut place, from) = match &anchor {
                Some((k, g)) => {
                    let moved = real[..*k].join("/");
                    let mut p = if g == TOP { Vec::new() } else { place_of_group(g) };
                    p.push(format!("{MOVED_TAG}{moved}"));
                    places.push((p.clone(), moved.clone(), moved));
                    (p, *k)
                }
                None => (Vec::new(), 0),
            };
            for k in from..real.len() {
                place.push(if flat { format!("\0{}", real[k]) } else { real[k].clone() });
                places.push((place.clone(), real[..=k].join("/"), real[k].clone()));
            }
            let last = places.len() - 1;
            for (n, (place, path, name)) in places.into_iter().enumerate() {
                let name = shape.names.get(&path).cloned().unwrap_or(name);
                let node = nodes.entry(place).or_insert_with(|| Node { item: None, path, name, group: false });
                if n == last {
                    node.item = Some(i);
                }
            }
        }
        Self { items, nodes, folders_first: shape.folders_first }
    }

    fn children(&self, parent: &[String]) -> Vec<&Vec<String>> {
        let mut kids: Vec<&Vec<String>> = self.nodes.keys().filter(|k| k.len() == parent.len() + 1 && k.starts_with(parent)).collect();
        // Groups first, then (when asked) folders, then by name.
        kids.sort_by_key(|k| {
            let node = &self.nodes[*k];
            (!node.group, self.folders_first && node.item.is_some(), self.name(k).to_lowercase(), k.last().cloned())
        });
        kids
    }

    fn name(&self, place: &[String]) -> String {
        let node = &self.nodes[place];
        match node.item {
            Some(i) => self.items[i].label.clone(),
            None => node.name.clone(),
        }
    }

    /// The items below `place` (not itself) that `shown` keeps.
    fn below(&self, place: &[String], shown: &dyn Fn(usize) -> bool) -> Vec<usize> {
        self.nodes.iter().filter(|(k, _)| k.len() > place.len() && k.starts_with(place)).filter_map(|(_, n)| n.item).filter(|&i| shown(i)).collect()
    }

    /// The items the section shows, those inside closed rows included: each top row's
    /// below, plus itself when it's an item.
    fn count(&self, shown: &dyn Fn(usize) -> bool) -> usize {
        self.children(&[])
            .into_iter()
            .map(|place| {
                let below = self.below(place, shown).len();
                let own = self.nodes[place].item.is_some_and(|i| shown(i) || below > 0);
                below + usize::from(own)
            })
            .sum()
    }

    fn emit(&self, parent: &[String], depth: usize, shown: &dyn Fn(usize) -> bool, narrowing: bool, open: &BTreeSet<String>, out: &mut Vec<Row>) {
        for place in self.children(parent) {
            let node = &self.nodes[place];
            let below = self.below(place, shown);
            // A group shows even empty, except while narrowing.
            let keep = if node.group { !narrowing || !below.is_empty() } else { node.item.is_some_and(shown) || !below.is_empty() };
            if !keep {
                continue;
            }
            let kids = (node.group || !below.is_empty()).then(|| Kids {
                open: narrowing || open.contains(&node.path),
                count: below.len(),
                running: below.iter().any(|&i| self.items[i].running),
                warning: below.iter().any(|&i| self.items[i].warning.is_some()),
            });
            match node.item {
                Some(index) => out.push(Row::Item { index, pinned: false, depth, kids }),
                None => out.push(Row::Folder { path: node.path.clone(), name: self.name(place), depth, kids: kids.unwrap_or_default(), group: node.group }),
            }
            if kids.is_some_and(|k| k.open) {
                self.emit(place, depth + 1, shown, narrowing, open, out);
            }
        }
    }
}

/// The group of a node taken back to the top while what holds it is in a group.
pub const TOP: &str = "";

impl Groups {
    /// Where the node at `real` (path segments) shows: the size of the node that was put
    /// somewhere (it, or the deepest node above it) and its group ([`TOP`]: the top).
    /// `None`: in its own place.
    fn anchor(&self, real: &[String]) -> Option<(usize, &str)> {
        (1..=real.len()).rev().find_map(|k| match self.members.get(&real[..k].join("/")).map(String::as_str) {
            Some(g) if self.folders.contains(g) => Some((k, g)),
            // At the top only while something above it is in a group.
            Some(TOP) if self.anchor(&real[..k - 1]).is_some_and(|(_, g)| g != TOP) => Some((k, TOP)),
            _ => None,
        })
    }

    /// The group a node (an item's [`node_path`], a folder's path) shows in, itself or
    /// with what holds it; `None`: not in a group.
    pub fn shown_in(&self, node: &str) -> Option<&str> {
        let real: Vec<String> = node.split('/').filter(|s| !s.is_empty()).map(String::from).collect();
        self.anchor(&real).map(|(_, g)| g).filter(|g| *g != TOP)
    }

    fn join(parent: Option<&str>, name: &str) -> String {
        parent.map_or_else(|| name.to_string(), |p| format!("{p}/{name}"))
    }

    fn parent(path: &str) -> Option<&str> {
        path.rsplit_once('/').map(|(p, _)| p)
    }

    fn leaf(path: &str) -> &str {
        path.rsplit_once('/').map_or(path, |(_, l)| l)
    }

    /// Every group, by path, alphabetical (menus show them all at one level).
    pub fn paths(&self) -> Vec<String> {
        let mut v: Vec<String> = self.folders.iter().cloned().collect();
        v.sort_by_key(|p| p.to_lowercase());
        v
    }

    /// The group `node` was put in.
    pub fn group_of(&self, node: &str) -> Option<&str> {
        self.members.get(node).map(String::as_str)
    }

    /// A trimmed name for a group under `parent`: not empty, no `/` or control
    /// character, no sibling of that name (ignoring case) other than `except`.
    pub fn check_name(&self, parent: Option<&str>, name: &str, except: Option<&str>) -> Result<String, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("a group needs a name".into());
        }
        if name.contains('/') || name.chars().any(char::is_control) {
            return Err("a group's name can't hold / or control characters".into());
        }
        let path = Self::join(parent, name);
        if let Some(taken) = self.folders.iter().find(|f| same(f, &path) && Some(f.as_str()) != except) {
            return Err(format!("a group called {} exists here", Self::leaf(taken)));
        }
        Ok(name.to_string())
    }

    /// Makes a group under `parent` (or at the top); returns its path.
    pub fn create(&mut self, parent: Option<&str>, name: &str) -> Result<String, String> {
        if let Some(p) = parent
            && !self.folders.contains(p)
        {
            return Err(format!("no group {p}"));
        }
        let name = self.check_name(parent, name, None)?;
        let path = Self::join(parent, &name);
        self.folders.insert(path.clone());
        Ok(path)
    }

    /// Moves group `from`, what it holds and its open state to `to`, merging with a
    /// group already there.
    fn reroot(&mut self, from: &str, to: &str, open: &mut BTreeSet<String>) {
        let moved = |p: &str| -> Option<String> {
            if p == from {
                Some(to.to_string())
            } else {
                p.strip_prefix(from).and_then(|r| r.strip_prefix('/')).map(|r| format!("{to}/{r}"))
            }
        };
        self.folders = std::mem::take(&mut self.folders).into_iter().map(|f| moved(&f).unwrap_or(f)).collect();
        for g in self.members.values_mut() {
            if let Some(m) = moved(g) {
                *g = m;
            }
        }
        *open = std::mem::take(open)
            .into_iter()
            .map(|o| match group_of_row(&o).and_then(moved) {
                Some(m) => group_row_path(&m),
                None => o,
            })
            .collect();
    }

    /// Renames a group (what it holds follows); returns its new path.
    pub fn rename(&mut self, path: &str, name: &str, open: &mut BTreeSet<String>) -> Result<String, String> {
        if !self.folders.contains(path) {
            return Err(format!("no group {path}"));
        }
        let parent = Self::parent(path);
        let name = self.check_name(parent, name, Some(path))?;
        let to = Self::join(parent, &name);
        self.reroot(path, &to, open);
        Ok(to)
    }

    /// Moves a group into `parent` (or to the top); returns its new path. Refused into
    /// itself or a group inside it.
    pub fn move_group(&mut self, path: &str, parent: Option<&str>, open: &mut BTreeSet<String>) -> Result<String, String> {
        if !self.folders.contains(path) {
            return Err(format!("no group {path}"));
        }
        if let Some(p) = parent {
            if p == path || p.starts_with(&format!("{path}/")) {
                return Err("a group can't go inside itself".into());
            }
            if !self.folders.contains(p) {
                return Err(format!("no group {p}"));
            }
        }
        if Self::parent(path) == parent {
            return Ok(path.to_string());
        }
        let name = self.check_name(parent, Self::leaf(path), None)?;
        let to = Self::join(parent, &name);
        self.reroot(path, &to, open);
        Ok(to)
    }

    /// Deletes a group: what it holds (groups and nodes) moves to its parent, groups of
    /// the same name there (ignoring case) merge. Nothing is lost. Returns where its
    /// groups went.
    pub fn delete(&mut self, path: &str, open: &mut BTreeSet<String>) -> Vec<(String, String)> {
        if !self.folders.contains(path) {
            return Vec::new();
        }
        let parent = Self::parent(path).map(str::to_string);
        let subgroups: Vec<String> = self.folders.iter().filter(|f| Self::parent(f) == Some(path)).cloned().collect();
        let mut moves = Vec::new();
        for g in subgroups {
            let to = Self::join(parent.as_deref(), Self::leaf(&g));
            let to = self.folders.iter().find(|f| *f != &g && same(f, &to)).cloned().unwrap_or(to);
            self.reroot(&g, &to, open);
            moves.push((g, to));
        }
        self.folders.remove(path);
        open.remove(&group_row_path(path));
        let held: Vec<String> = self.members.iter().filter(|(_, g)| *g == path).map(|(n, _)| n.clone()).collect();
        for node in held {
            self.assign(&node, parent.as_deref());
        }
        moves
    }

    /// Puts `node` (an item's [`node_path`] or a folder's path) in `group`, or back
    /// where it came from with `None`.
    pub fn assign(&mut self, node: &str, group: Option<&str>) {
        self.members.remove(node);
        match group.filter(|g| self.folders.contains(*g)) {
            Some(g) => {
                self.members.insert(node.to_string(), g.to_string());
            }
            // Something holding it is in a group: marked to show at the top.
            None if self.shown_in(node).is_some() => {
                self.members.insert(node.to_string(), TOP.to_string());
            }
            None => {}
        }
    }
}

/// Group names are the same ignoring case (any script, not only ASCII).
fn same(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

/// Moves a selection among item and folder rows by `delta`, clamped; `None` picks the
/// first.
pub fn step(rows: &[Row], from: Option<usize>, delta: isize) -> Option<usize> {
    let items: Vec<usize> = rows.iter().enumerate().filter(|(_, r)| r.is_selectable()).map(|(i, _)| i).collect();
    if items.is_empty() {
        return None;
    }
    let at = from.and_then(|f| items.iter().position(|&i| i == f));
    let next = match at {
        None => 0,
        Some(p) => (p as isize + delta).clamp(0, items.len() as isize - 1) as usize,
    };
    Some(items[next])
}

/// Pins or unpins; returns whether the key is pinned now.
pub fn toggle_pin(pins: &mut BTreeSet<String>, key: &str) -> bool {
    if pins.remove(key) {
        false
    } else {
        pins.insert(key.to_string());
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items() -> Vec<ListItem> {
        vec![
            ListItem::new("w", "vesselservice"),
            ListItem::new("b", "Boardservice").running(true),
            ListItem::new("e", "entityservice").detail(":5800").running(true),
            ListItem::new("a", "authservice"),
        ]
    }

    fn labels(items: &[ListItem], rows: &[Row]) -> Vec<String> {
        rows.iter()
            .map(|r| match r {
                Row::Header { section, open, .. } => format!("[{section:?}{}]", if *open { "" } else { " folded" }),
                Row::Empty(s) => format!("({s:?} empty)"),
                Row::Item { index, depth, kids, .. } => format!("{}{}{}", "  ".repeat(*depth), items[*index].label, mark(kids)),
                Row::Folder { name, depth, kids, group: false, .. } => format!("{}{name}/{}", "  ".repeat(*depth), mark(&Some(*kids))),
                Row::Folder { name, depth, kids, group: true, .. } => format!("{}#{name}{}", "  ".repeat(*depth), mark(&Some(*kids))),
            })
            .collect()
    }

    /// ` ▸3` closed with three below (`*` running, `!` warning), ` ▾` open.
    fn mark(kids: &Option<Kids>) -> String {
        kids.map_or(String::new(), |k| {
            format!(" {}{}{}{}", if k.open { "▾" } else { "▸" }, k.count, if k.running { "*" } else { "" }, if k.warning { "!" } else { "" })
        })
    }

    fn none() -> BTreeSet<String> {
        BTreeSet::new()
    }

    /// The headers' counts: (Pinned, All).
    fn counts(rows: &[Row]) -> (usize, usize) {
        let of = |want: Section| rows.iter().find_map(|r| match r {
            Row::Header { section, count, .. } if *section == want => Some(*count),
            _ => None,
        });
        (of(Section::Pinned).unwrap(), of(Section::All).unwrap())
    }

    fn rows_open(items: &[ListItem], pins: &BTreeSet<String>, filter: &str, running_only: bool, open: &BTreeSet<String>) -> Vec<Row> {
        rows(items, pins, filter, running_only, Shape { open, groups: &Groups::default(), folded: &none(), names: &BTreeMap::new(), folders_first: false })
    }

    #[test]
    fn alphabetical_sections_ignore_case() {
        let it = items();
        let pins = BTreeSet::from(["w".to_string(), "e".to_string()]);
        let r = rows_open(&it, &pins, "", false, &none());
        assert_eq!(labels(&it, &r), ["[Pinned]", "entityservice", "vesselservice", "[All]", "authservice", "Boardservice"]);
    }

    #[test]
    fn empty_sections_say_so() {
        let it = items();
        let r = rows_open(&it, &BTreeSet::new(), "zzz", false, &none());
        assert_eq!(labels(&it, &r), ["[Pinned]", "(Pinned empty)", "[All]", "(All empty)"]);
    }

    #[test]
    fn filter_hides_without_reordering_and_running_only() {
        let it = items();
        assert!(matches("entityservice", "ensvc"));
        assert!(matches("entityservice", "Entity Service"));
        assert!(!matches("entityservice", "svx"));
        let r = rows_open(&it, &BTreeSet::new(), "serv", false, &none());
        assert_eq!(labels(&it, &r), ["[Pinned]", "(Pinned empty)", "[All]", "authservice", "Boardservice", "entityservice", "vesselservice"]);
        let r = rows_open(&it, &BTreeSet::new(), "", true, &none());
        assert_eq!(labels(&it, &r), ["[Pinned]", "(Pinned empty)", "[All]", "Boardservice", "entityservice"]);
    }

    #[test]
    fn selection_steps_over_headers_and_clamps() {
        let it = items();
        let pins = BTreeSet::from(["e".to_string()]);
        let r = rows_open(&it, &pins, "", false, &none());
        let first = step(&r, None, 1).unwrap();
        assert_eq!(r[first], Row::Item { index: 2, pinned: true, depth: 0, kids: None });
        let second = step(&r, Some(first), 1).unwrap();
        assert!(matches!(r[second], Row::Item { pinned: false, .. }), "skips the All header");
        assert_eq!(step(&r, Some(first), -5), Some(first), "clamped at the top");
        let last = step(&r, Some(first), 99).unwrap();
        assert_eq!(last, r.len() - 1);
        assert_eq!(step(&rows_open(&it, &pins, "zzz", false, &none()), None, 1), None);
    }

    #[test]
    fn pins_of_unknown_keys_are_not_shown() {
        let it = items();
        let pins = BTreeSet::from(["gone".to_string()]);
        let r = rows_open(&it, &pins, "", false, &none());
        assert_eq!(r[1], Row::Empty(Section::Pinned));
        assert_eq!(counts(&r).0, 0);
    }

    /// A monorepo `fe` (itself an app) with apps and a folder of packages, and a service.
    fn tree_items() -> Vec<ListItem> {
        vec![
            ListItem::new("fe", "fe").path("fe"),
            ListItem::new("fe/apps/shell", "shell").path("fe/apps/shell").running(true),
            ListItem::new("fe/apps/landing", "landing").path("fe/apps/landing"),
            ListItem::new("fe/packages/ui/button", "button").path("fe/packages/ui/button").warning(Some("pnpm not found".into())),
            ListItem::new("fe/packages/ui/card", "card").path("fe/packages/ui/card"),
            ListItem::new("api", "api").path("api"),
        ]
    }

    #[test]
    fn paths_make_a_tree_collapsed_by_default() {
        let it = tree_items();
        let r = rows_open(&it, &none(), "", false, &none());
        assert_eq!(labels(&it, &r), ["[Pinned]", "(Pinned empty)", "[All]", "api", "fe ▸4*!"]);
        let open = BTreeSet::from(["fe".to_string(), "fe/packages".to_string()]);
        let r = rows_open(&it, &none(), "", false, &open);
        assert_eq!(
            labels(&it, &r),
            ["[Pinned]", "(Pinned empty)", "[All]", "api", "fe ▾4*!", "  apps/ ▸2*", "  packages/ ▾2!", "    ui/ ▸2!"],
            "folders keep the structure; what's inside shows on the collapsed row"
        );
    }

    #[test]
    fn filtering_opens_the_way_to_the_matches() {
        let it = tree_items();
        let r = rows_open(&it, &none(), "card", false, &none());
        assert_eq!(labels(&it, &r), ["[Pinned]", "(Pinned empty)", "[All]", "fe ▾1", "  packages/ ▾1", "    ui/ ▾1", "      card"]);
        let r = rows_open(&it, &none(), "", true, &none());
        assert_eq!(labels(&it, &r), ["[Pinned]", "(Pinned empty)", "[All]", "fe ▾1*", "  apps/ ▾1*", "    shell"], "running only");
    }

    #[test]
    fn a_pinned_item_leaves_the_tree_and_its_children_stay() {
        let it = tree_items();
        let pins = BTreeSet::from(["fe".to_string()]);
        let r = rows_open(&it, &pins, "", false, &none());
        assert_eq!(labels(&it, &r), ["[Pinned]", "fe", "[All]", "api", "fe/ ▸4*!"], "the monorepo's packages under a folder row");
    }

    #[test]
    fn folders_are_selectable_and_open_by_their_path() {
        let it = tree_items();
        let r = rows_open(&it, &none(), "", false, &BTreeSet::from(["fe".to_string()]));
        let apps = r.iter().position(|row| matches!(row, Row::Folder { name, .. } if name == "apps")).unwrap();
        assert_eq!(r[apps].open_path(&it), Some("fe/apps"));
        assert_eq!(step(&r, Some(apps - 1), 1), Some(apps), "folders take the selection");
        assert_eq!(folder_key("fe/apps"), "fe/apps/");
        assert_eq!((r[apps].depth(), r[apps].kids().map(|k| k.count)), (1, Some(2)));
    }

    #[test]
    fn counts_include_what_closed_rows_hold() {
        let it = tree_items();
        let pins = BTreeSet::from(["api".to_string()]);
        let r = rows_open(&it, &pins, "", false, &none());
        assert_eq!(counts(&r), (1, 5));
        let r = rows_open(&it, &pins, "", false, &BTreeSet::from(["fe".to_string(), "fe/apps".to_string()]));
        assert_eq!(counts(&r).1, 5, "the same open");
    }

    #[test]
    fn pin_toggles() {
        let mut pins = BTreeSet::new();
        assert!(toggle_pin(&mut pins, "a"));
        assert!(!toggle_pin(&mut pins, "a"));
        assert!(pins.is_empty());
    }

    fn grouped(open: &[&str], groups: &Groups, folded: &[&str], pins: &[&str], filter: &str) -> (Vec<ListItem>, Vec<Row>) {
        let it = tree_items();
        let open: BTreeSet<String> = open.iter().map(|s| s.to_string()).collect();
        let folded: BTreeSet<String> = folded.iter().map(|s| s.to_string()).collect();
        let pins: BTreeSet<String> = pins.iter().map(|s| s.to_string()).collect();
        let r = rows(&it, &pins, filter, false, Shape { open: &open, groups, folded: &folded, names: &BTreeMap::new(), folders_first: false });
        (it, r)
    }

    #[test]
    fn sections_fold_and_a_filter_shows_their_matches() {
        let g = Groups::default();
        let (it, r) = grouped(&[], &g, &["all", "pinned"], &["api"], "");
        assert_eq!(labels(&it, &r), ["[Pinned folded]", "[All folded]"]);
        assert_eq!(counts(&r), (1, 5), "a folded section still counts");
        let (it, r) = grouped(&[], &g, &["all"], &["api"], "card");
        assert_eq!(labels(&it, &r), ["[Pinned]", "(Pinned empty)", "[All]", "fe ▾1", "  packages/ ▾1", "    ui/ ▾1", "      card"]);
    }

    fn groups(folders: &[&str], members: &[(&str, &str)]) -> Groups {
        Groups { folders: folders.iter().map(|s| s.to_string()).collect(), members: members.iter().map(|(n, g)| (n.to_string(), g.to_string())).collect() }
    }

    #[test]
    fn groups_come_first_and_hold_what_was_moved_with_its_real_paths() {
        let g = groups(&["back", "front", "front/ui"], &[("api", "back"), ("fe/packages", "front/ui")]);
        let (it, r) = grouped(&[], &g, &[], &[], "");
        assert_eq!(labels(&it, &r), ["[Pinned]", "(Pinned empty)", "[All]", "#back ▸1", "#front ▸2!", "fe ▸2*"], "groups first; fe keeps its apps");
        assert_eq!(counts(&r).1, 6, "every item, in groups or not");
        let open = [group_row_path("back"), group_row_path("front"), group_row_path("front/ui"), "fe/packages".into(), "fe".into()];
        let open: Vec<&str> = open.iter().map(String::as_str).collect();
        let (it, r) = grouped(&open, &g, &[], &[], "");
        assert_eq!(
            labels(&it, &r),
            ["[Pinned]", "(Pinned empty)", "[All]", "#back ▾1", "  api", "#front ▾2!", "  #ui ▾2!", "    fe/packages/ ▾2!", "      ui/ ▸2!", "fe ▾2*", "  apps/ ▸2*"]
        );
        let moved = r.iter().find(|row| matches!(row, Row::Folder { name, .. } if name == "fe/packages")).unwrap();
        assert_eq!(moved.open_path(&it), Some("fe/packages"), "opens by its real path");
        let ui = r.iter().find(|row| matches!(row, Row::Folder { name, group: true, .. } if name == "ui")).unwrap();
        assert_eq!(ui.open_path(&it).and_then(group_of_row), Some("front/ui"));
    }

    #[test]
    fn two_folders_of_one_name_stay_apart_in_a_group_and_the_deepest_move_wins() {
        let it = vec![
            ListItem::new("fe/apps/a", "a").path("fe/apps/a"),
            ListItem::new("be/apps/b", "b").path("be/apps/b"),
            ListItem::new("be/apps/c", "c").path("be/apps/c"),
        ];
        let g = groups(&["g", "h"], &[("fe/apps", "g"), ("be/apps", "g"), ("be/apps/c", "h")]);
        let open = BTreeSet::from([group_row_path("g"), group_row_path("h")]);
        let r = rows(&it, &none(), "", false, Shape { open: &open, groups: &g, folded: &none(), names: &BTreeMap::new(), folders_first: false });
        assert_eq!(labels(&it, &r), ["[Pinned]", "(Pinned empty)", "[All]", "#g ▾2", "  be/apps/ ▸1", "  fe/apps/ ▸1", "#h ▾1", "  c"]);
    }

    #[test]
    fn a_member_of_a_gone_group_shows_at_the_top_and_empty_groups_hide_while_filtering() {
        let g = groups(&["empty"], &[("api", "gone")]);
        let (it, r) = grouped(&[], &g, &[], &[], "");
        assert_eq!(labels(&it, &r), ["[Pinned]", "(Pinned empty)", "[All]", "#empty ▸0", "api", "fe ▸4*!"]);
        let (it, r) = grouped(&[], &g, &[], &[], "api");
        assert_eq!(labels(&it, &r), ["[Pinned]", "(Pinned empty)", "[All]", "api"]);
    }

    #[test]
    fn a_pinned_item_keeps_its_group_for_later() {
        let g = groups(&["back"], &[("api", "back")]);
        let (it, r) = grouped(&[], &g, &[], &["api"], "");
        assert_eq!(labels(&it, &r), ["[Pinned]", "api", "[All]", "#back ▸0", "fe ▸4*!"]);
        assert_eq!(g.group_of("api"), Some("back"));
    }

    #[test]
    fn flat_items_move_into_groups_by_key() {
        let it = items();
        let g = groups(&["mine"], &[("e", "mine")]);
        let open = BTreeSet::from([group_row_path("mine")]);
        let r = rows(&it, &none(), "", false, Shape { open: &open, groups: &g, folded: &none(), names: &BTreeMap::new(), folders_first: false });
        assert_eq!(labels(&it, &r), ["[Pinned]", "(Pinned empty)", "[All]", "#mine ▾1*", "  entityservice", "authservice", "Boardservice", "vesselservice"]);
        assert_eq!(node_path(&it[2]), "e");
    }

    #[test]
    fn group_operations_keep_everything_and_explain_refusals() {
        let mut g = Groups::default();
        let mut open = BTreeSet::new();
        assert_eq!(g.create(None, "  front "), Ok("front".into()));
        assert_eq!(g.create(Some("front"), "web"), Ok("front/web".into()));
        assert_eq!(g.create(None, "Front"), Err("a group called front exists here".into()));
        assert_eq!(g.create(None, "a/b"), Err("a group's name can't hold / or control characters".into()));
        assert_eq!(g.create(None, " "), Err("a group needs a name".into()));
        assert_eq!(g.create(Some("nope"), "x"), Err("no group nope".into()));
        g.assign("fe", Some("front/web"));
        g.assign("api", Some("nope"));
        assert_eq!(g.group_of("api"), None, "no group, no move");
        open.insert(group_row_path("front/web"));
        // Rename: what it holds and its open state follow; its own name with another case is fine.
        assert_eq!(g.rename("front", "FE", &mut open), Ok("FE".into()));
        assert_eq!(g.group_of("fe"), Some("FE/web"));
        assert!(open.contains(&group_row_path("FE/web")));
        assert_eq!(g.rename("FE", "fe", &mut open), Ok("fe".into()));
        // Move: never into itself.
        g.create(None, "back").unwrap();
        assert_eq!(g.move_group("fe", Some("fe/web"), &mut open), Err("a group can't go inside itself".into()));
        assert_eq!(g.move_group("fe/web", Some("back"), &mut open), Ok("back/web".into()));
        assert_eq!(g.group_of("fe"), Some("back/web"));
        assert_eq!(g.move_group("back/web", None, &mut open), Ok("web".into()));
        // Delete: what it holds moves up; groups of one name merge.
        g.move_group("web", Some("back"), &mut open).unwrap();
        g.create(None, "web").unwrap();
        g.assign("api", Some("web"));
        g.delete("back", &mut open);
        assert_eq!(g.paths(), ["fe", "web"]);
        assert_eq!((g.group_of("fe"), g.group_of("api")), (Some("web"), Some("web")), "merged into the top web");
        g.delete("web", &mut open);
        assert_eq!((g.group_of("fe"), g.group_of("api")), (None, None), "back at the top");
        assert!(!open.iter().any(|o| o.contains("web")));
        g.assign("api", Some("fe"));
        g.assign("api", None);
        assert!(g.members.is_empty());
    }

    #[test]
    fn a_node_inside_a_moved_folder_shows_in_its_group_and_can_go_back_to_the_top() {
        let mut g = groups(&["design"], &[("fe/packages", "design")]);
        assert_eq!(g.shown_in("fe/packages/ui/card"), Some("design"), "with what holds it");
        assert_eq!(g.shown_in("fe/apps/shell"), None);
        g.assign("fe/packages/ui/card", None);
        assert_eq!(g.group_of("fe/packages/ui/card"), Some(TOP), "marked: something above it is in a group");
        assert_eq!(g.shown_in("fe/packages/ui/card"), None);
        let open = [group_row_path("design"), "fe/packages".into(), "fe/packages/ui".into()];
        let open: Vec<&str> = open.iter().map(String::as_str).collect();
        let (it, r) = grouped(&open, &g, &[], &[], "");
        assert_eq!(labels(&it, &r), ["[Pinned]", "(Pinned empty)", "[All]", "#design ▾1!", "  fe/packages/ ▾1!", "    ui/ ▾1!", "      button", "api", "card", "fe ▸2*"]);
        // Its folder leaves the group: the mark is stale and it's back in its place.
        g.assign("fe/packages", None);
        assert_eq!(g.shown_in("fe/packages/ui/card"), None);
        let (it, r) = grouped(&["fe", "fe/packages", "fe/packages/ui"], &g, &[], &[], "");
        assert!(labels(&it, &r).contains(&"      card".to_string()), "inside its folder again");
    }

    #[test]
    fn deleting_a_group_merges_its_groups_with_names_differing_in_case() {
        let mut g = groups(&["back", "back/Web", "web", "Été"], &[("api", "back/Web")]);
        let mut open = BTreeSet::new();
        assert_eq!(g.delete("back", &mut open), [("back/Web".to_string(), "web".to_string())]);
        assert_eq!(g.paths(), ["web", "Été"]);
        assert_eq!(g.group_of("api"), Some("web"));
        assert_eq!(g.create(None, "été"), Err("a group called Été exists here".into()), "case ignored beyond ASCII");
    }

    #[test]
    fn folders_can_be_named_and_come_first() {
        let it = vec![
            ListItem::new("n1", "alpha"),
            ListItem::new("n2", "todo").path("p1/n2"),
            ListItem::new("n3", "bugs").path("p2/n3"),
            ListItem::new("n4", "zeta"),
        ];
        let names = BTreeMap::from([("p1".to_string(), "Zed".to_string()), ("p2".to_string(), "web".to_string())]);
        let open = BTreeSet::from(["p1".to_string()]);
        let (groups, folded) = (Groups::default(), none());
        let shape = |folders_first| Shape { open: &open, groups: &groups, folded: &folded, names: &names, folders_first };
        let r = rows(&it, &none(), "", false, shape(true));
        assert_eq!(labels(&it, &r), ["[Pinned]", "(Pinned empty)", "[All]", "web/ ▸1", "Zed/ ▾1", "  todo", "alpha", "zeta"]);
        let r = rows(&it, &none(), "", false, shape(false));
        assert_eq!(labels(&it, &r), ["[Pinned]", "(Pinned empty)", "[All]", "alpha", "web/ ▸1", "Zed/ ▾1", "  todo", "zeta"]);
    }
}
