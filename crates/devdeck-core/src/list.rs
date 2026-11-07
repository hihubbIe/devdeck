//! The generic sidebar list (R4.1–R4.3): Pinned then All, both alphabetical, with a
//! filter that hides rows but never reorders them, and a "running only" switch.
//!
//! Items with a `path` (`front-end/apps/shellapp`) make All a tree that keeps their
//! folders: a folder is an item's own row when it is one (a monorepo's root app),
//! otherwise a folder row; both open and close, collapsed by default, and a collapsed
//! row says what's inside (how many, something running, a warning). While filtering
//! (or "running only"), the matches show with their folders open. Pinned stays flat.
//! Items without a path give a flat list.

use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq)]
pub struct ListItem {
    /// Stable identity (pins and selection refer to it).
    pub key: String,
    pub label: String,
    /// Dim text on the right (a port, a tag, a ready count).
    pub detail: String,
    pub running: bool,
    /// Why it can't be used (a missing tool): drawn with ⚠ instead of the state glyph.
    pub warning: Option<String>,
    /// Where it sits in the tree (`/`-separated); `None`: at the top.
    pub path: Option<String>,
    /// Drawn de-emphasized (a library, R6.2).
    pub dim: bool,
}

impl ListItem {
    pub fn new(key: impl Into<String>, label: impl Into<String>) -> Self {
        Self { key: key.into(), label: label.into(), detail: String::new(), running: false, warning: None, path: None, dim: false }
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
    Header(Section),
    /// A section with nothing to show (explains why).
    Empty(Section),
    /// Index into the items given to [`rows`], whether it is pinned, its level in the
    /// tree (0 at the top) and, when items sit below it, what they are.
    Item { index: usize, pinned: bool, depth: usize, kids: Option<Kids> },
    /// A folder that holds items without being one.
    Folder { path: String, name: String, depth: usize, kids: Kids },
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

/// The rows to draw: Pinned header, pinned items (flat), All header, the other items as
/// a tree. `open` holds the paths of open folders and open items.
pub fn rows(items: &[ListItem], pins: &BTreeSet<String>, filter: &str, running_only: bool, open: &BTreeSet<String>) -> Vec<Row> {
    let shown = |i: usize| keeps(&items[i], filter, running_only);
    let mut pinned: Vec<usize> = (0..items.len()).filter(|&i| pins.contains(&items[i].key) && shown(i)).collect();
    pinned.sort_by(|&a, &b| order(&items[a], &items[b]));
    let mut out = vec![Row::Header(Section::Pinned)];
    if pinned.is_empty() {
        out.push(Row::Empty(Section::Pinned));
    }
    out.extend(pinned.into_iter().map(|index| Row::Item { index, pinned: true, depth: 0, kids: None }));
    out.push(Row::Header(Section::All));
    let rest: Vec<usize> = (0..items.len()).filter(|&i| !pins.contains(&items[i].key)).collect();
    let narrowing = !filter.trim().is_empty() || running_only;
    let tree = Tree::new(items, &rest);
    let before = out.len();
    tree.emit(&[], 0, &shown, narrowing, open, &mut out);
    if out.len() == before {
        out.push(Row::Empty(Section::All));
    }
    out
}

/// The All section's items by tree path: each node is an item, a folder, or both.
struct Tree<'a> {
    items: &'a [ListItem],
    /// Node path (segments) → the item at it.
    nodes: BTreeMap<Vec<String>, Option<usize>>,
}

impl<'a> Tree<'a> {
    fn new(items: &'a [ListItem], idx: &[usize]) -> Self {
        let mut nodes: BTreeMap<Vec<String>, Option<usize>> = BTreeMap::new();
        for &i in idx {
            // Without a path an item is a leaf at the top, whatever its key holds.
            let segs: Vec<String> = match &items[i].path {
                Some(p) => p.split('/').filter(|s| !s.is_empty()).map(String::from).collect(),
                None => vec![format!("\0{}", items[i].key)],
            };
            for k in 1..segs.len() {
                nodes.entry(segs[..k].to_vec()).or_insert(None);
            }
            nodes.insert(segs, Some(i));
        }
        Self { items, nodes }
    }

    fn children(&self, parent: &[String]) -> Vec<&Vec<String>> {
        let mut kids: Vec<&Vec<String>> = self.nodes.keys().filter(|k| k.len() == parent.len() + 1 && k.starts_with(parent)).collect();
        kids.sort_by_key(|k| (self.name(k).to_lowercase(), k.last().cloned()));
        kids
    }

    fn name(&self, node: &[String]) -> String {
        match self.nodes.get(node).copied().flatten() {
            Some(i) => self.items[i].label.clone(),
            None => node.last().cloned().unwrap_or_default(),
        }
    }

    /// The items below `node` (not itself) that `shown` keeps.
    fn below(&self, node: &[String], shown: &dyn Fn(usize) -> bool) -> Vec<usize> {
        self.nodes.iter().filter(|(k, _)| k.len() > node.len() && k.starts_with(node)).filter_map(|(_, i)| *i).filter(|&i| shown(i)).collect()
    }

    fn emit(&self, parent: &[String], depth: usize, shown: &dyn Fn(usize) -> bool, narrowing: bool, open: &BTreeSet<String>, out: &mut Vec<Row>) {
        for node in self.children(parent) {
            let item = self.nodes[node];
            let below = self.below(node, shown);
            if !item.is_some_and(shown) && below.is_empty() {
                continue;
            }
            let path = node.join("/");
            let kids = (!below.is_empty()).then(|| Kids {
                open: narrowing || open.contains(&path),
                count: below.len(),
                running: below.iter().any(|&i| self.items[i].running),
                warning: below.iter().any(|&i| self.items[i].warning.is_some()),
            });
            match item {
                Some(index) => out.push(Row::Item { index, pinned: false, depth, kids }),
                None => out.push(Row::Folder { path: path.clone(), name: self.name(node), depth, kids: kids.unwrap_or_default() }),
            }
            if kids.is_some_and(|k| k.open) {
                self.emit(node, depth + 1, shown, narrowing, open, out);
            }
        }
    }
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

/// The items a section shows, those inside closed rows included (its header's count).
pub fn count(rows: &[Row], section: Section) -> usize {
    let mut in_section = false;
    let mut n = 0;
    for row in rows {
        match row {
            Row::Header(s) => in_section = *s == section,
            Row::Item { depth: 0, kids, .. } if in_section => n += 1 + kids.map_or(0, |k| k.count),
            Row::Folder { depth: 0, kids, .. } if in_section => n += kids.count,
            _ => {}
        }
    }
    n
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
                Row::Header(s) => format!("[{s:?}]"),
                Row::Empty(s) => format!("({s:?} empty)"),
                Row::Item { index, depth, kids, .. } => format!("{}{}{}", "  ".repeat(*depth), items[*index].label, mark(kids)),
                Row::Folder { name, depth, kids, .. } => format!("{}{name}/{}", "  ".repeat(*depth), mark(&Some(*kids))),
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

    #[test]
    fn alphabetical_sections_ignore_case() {
        let it = items();
        let pins = BTreeSet::from(["w".to_string(), "e".to_string()]);
        let r = rows(&it, &pins, "", false, &none());
        assert_eq!(labels(&it, &r), ["[Pinned]", "entityservice", "vesselservice", "[All]", "authservice", "Boardservice"]);
    }

    #[test]
    fn empty_sections_say_so() {
        let it = items();
        let r = rows(&it, &BTreeSet::new(), "zzz", false, &none());
        assert_eq!(labels(&it, &r), ["[Pinned]", "(Pinned empty)", "[All]", "(All empty)"]);
    }

    #[test]
    fn filter_hides_without_reordering_and_running_only() {
        let it = items();
        assert!(matches("entityservice", "ensvc"));
        assert!(matches("entityservice", "Entity Service"));
        assert!(!matches("entityservice", "svx"));
        let r = rows(&it, &BTreeSet::new(), "serv", false, &none());
        assert_eq!(labels(&it, &r), ["[Pinned]", "(Pinned empty)", "[All]", "authservice", "Boardservice", "entityservice", "vesselservice"]);
        let r = rows(&it, &BTreeSet::new(), "", true, &none());
        assert_eq!(labels(&it, &r), ["[Pinned]", "(Pinned empty)", "[All]", "Boardservice", "entityservice"]);
    }

    #[test]
    fn selection_steps_over_headers_and_clamps() {
        let it = items();
        let pins = BTreeSet::from(["e".to_string()]);
        let r = rows(&it, &pins, "", false, &none());
        let first = step(&r, None, 1).unwrap();
        assert_eq!(r[first], Row::Item { index: 2, pinned: true, depth: 0, kids: None });
        let second = step(&r, Some(first), 1).unwrap();
        assert!(matches!(r[second], Row::Item { pinned: false, .. }), "skips the All header");
        assert_eq!(step(&r, Some(first), -5), Some(first), "clamped at the top");
        let last = step(&r, Some(first), 99).unwrap();
        assert_eq!(last, r.len() - 1);
        assert_eq!(step(&rows(&it, &pins, "zzz", false, &none()), None, 1), None);
    }

    #[test]
    fn pins_of_unknown_keys_are_not_shown() {
        let it = items();
        let pins = BTreeSet::from(["gone".to_string()]);
        let r = rows(&it, &pins, "", false, &none());
        assert_eq!(r[1], Row::Empty(Section::Pinned));
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
        let r = rows(&it, &none(), "", false, &none());
        assert_eq!(labels(&it, &r), ["[Pinned]", "(Pinned empty)", "[All]", "api", "fe ▸4*!"]);
        let open = BTreeSet::from(["fe".to_string(), "fe/packages".to_string()]);
        let r = rows(&it, &none(), "", false, &open);
        assert_eq!(
            labels(&it, &r),
            ["[Pinned]", "(Pinned empty)", "[All]", "api", "fe ▾4*!", "  apps/ ▸2*", "  packages/ ▾2!", "    ui/ ▸2!"],
            "folders keep the structure; what's inside shows on the collapsed row"
        );
    }

    #[test]
    fn filtering_opens_the_way_to_the_matches() {
        let it = tree_items();
        let r = rows(&it, &none(), "card", false, &none());
        assert_eq!(labels(&it, &r), ["[Pinned]", "(Pinned empty)", "[All]", "fe ▾1", "  packages/ ▾1", "    ui/ ▾1", "      card"]);
        let r = rows(&it, &none(), "", true, &none());
        assert_eq!(labels(&it, &r), ["[Pinned]", "(Pinned empty)", "[All]", "fe ▾1*", "  apps/ ▾1*", "    shell"], "running only");
    }

    #[test]
    fn a_pinned_item_leaves_the_tree_and_its_children_stay() {
        let it = tree_items();
        let pins = BTreeSet::from(["fe".to_string()]);
        let r = rows(&it, &pins, "", false, &none());
        assert_eq!(labels(&it, &r), ["[Pinned]", "fe", "[All]", "api", "fe/ ▸4*!"], "the monorepo's packages under a folder row");
    }

    #[test]
    fn folders_are_selectable_and_open_by_their_path() {
        let it = tree_items();
        let r = rows(&it, &none(), "", false, &BTreeSet::from(["fe".to_string()]));
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
        let r = rows(&it, &pins, "", false, &none());
        assert_eq!((count(&r, Section::Pinned), count(&r, Section::All)), (1, 5));
        let r = rows(&it, &pins, "", false, &BTreeSet::from(["fe".to_string(), "fe/apps".to_string()]));
        assert_eq!(count(&r, Section::All), 5, "the same open");
    }

    #[test]
    fn pin_toggles() {
        let mut pins = BTreeSet::new();
        assert!(toggle_pin(&mut pins, "a"));
        assert!(!toggle_pin(&mut pins, "a"));
        assert!(pins.is_empty());
    }
}
