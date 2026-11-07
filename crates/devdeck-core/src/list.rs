//! The generic sidebar list (R4.1–R4.3): Pinned then All, both alphabetical, with a
//! filter that hides rows but never reorders them, and a "running only" switch.

use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq)]
pub struct ListItem {
    /// Stable identity (pins and selection refer to it).
    pub key: String,
    pub label: String,
    /// Dim text on the right (a port, a tag, a ready count).
    pub detail: String,
    pub running: bool,
}

impl ListItem {
    pub fn new(key: impl Into<String>, label: impl Into<String>) -> Self {
        Self { key: key.into(), label: label.into(), detail: String::new(), running: false }
    }

    pub fn detail(mut self, d: impl Into<String>) -> Self {
        self.detail = d.into();
        self
    }

    pub fn running(mut self, on: bool) -> Self {
        self.running = on;
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Pinned,
    All,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    Header(Section),
    /// A section with nothing to show (explains why).
    Empty(Section),
    /// Index into the items given to [`rows`], and whether it is pinned.
    Item { index: usize, pinned: bool },
}

impl Row {
    pub fn is_item(&self) -> bool {
        matches!(self, Row::Item { .. })
    }
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

fn order(a: &ListItem, b: &ListItem) -> std::cmp::Ordering {
    a.label.to_lowercase().cmp(&b.label.to_lowercase()).then_with(|| a.key.cmp(&b.key))
}

/// The rows to draw: Pinned header, pinned items, All header, the other items.
pub fn rows(items: &[ListItem], pins: &BTreeSet<String>, filter: &str, running_only: bool) -> Vec<Row> {
    let mut idx: Vec<usize> = (0..items.len())
        .filter(|&i| matches(&items[i].label, filter) && (!running_only || items[i].running))
        .collect();
    idx.sort_by(|&a, &b| order(&items[a], &items[b]));
    let (pinned, rest): (Vec<usize>, Vec<usize>) = idx.into_iter().partition(|&i| pins.contains(&items[i].key));
    let mut out = Vec::with_capacity(pinned.len() + rest.len() + 4);
    for (section, list, is_pinned) in [(Section::Pinned, pinned, true), (Section::All, rest, false)] {
        out.push(Row::Header(section));
        if list.is_empty() {
            out.push(Row::Empty(section));
        }
        out.extend(list.into_iter().map(|index| Row::Item { index, pinned: is_pinned }));
    }
    out
}

/// Moves a selection among item rows by `delta`, clamped; `None` picks the first item.
pub fn step(rows: &[Row], from: Option<usize>, delta: isize) -> Option<usize> {
    let items: Vec<usize> = rows.iter().enumerate().filter(|(_, r)| r.is_item()).map(|(i, _)| i).collect();
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
                Row::Header(s) => format!("[{s:?}]"),
                Row::Empty(s) => format!("({s:?} empty)"),
                Row::Item { index, .. } => items[*index].label.clone(),
            })
            .collect()
    }

    #[test]
    fn alphabetical_sections_ignore_case() {
        let it = items();
        let pins = BTreeSet::from(["w".to_string(), "e".to_string()]);
        let r = rows(&it, &pins, "", false);
        assert_eq!(labels(&it, &r), ["[Pinned]", "entityservice", "vesselservice", "[All]", "authservice", "Boardservice"]);
    }

    #[test]
    fn empty_sections_say_so() {
        let it = items();
        let r = rows(&it, &BTreeSet::new(), "zzz", false);
        assert_eq!(labels(&it, &r), ["[Pinned]", "(Pinned empty)", "[All]", "(All empty)"]);
    }

    #[test]
    fn filter_hides_without_reordering_and_running_only() {
        let it = items();
        assert!(matches("entityservice", "ensvc"));
        assert!(matches("entityservice", "Entity Service"));
        assert!(!matches("entityservice", "svx"));
        let r = rows(&it, &BTreeSet::new(), "serv", false);
        assert_eq!(labels(&it, &r), ["[Pinned]", "(Pinned empty)", "[All]", "authservice", "Boardservice", "entityservice", "vesselservice"]);
        let r = rows(&it, &BTreeSet::new(), "", true);
        assert_eq!(labels(&it, &r), ["[Pinned]", "(Pinned empty)", "[All]", "Boardservice", "entityservice"]);
    }

    #[test]
    fn selection_steps_over_headers_and_clamps() {
        let it = items();
        let pins = BTreeSet::from(["e".to_string()]);
        let r = rows(&it, &pins, "", false);
        let first = step(&r, None, 1).unwrap();
        assert_eq!(r[first], Row::Item { index: 2, pinned: true });
        let second = step(&r, Some(first), 1).unwrap();
        assert!(matches!(r[second], Row::Item { pinned: false, .. }), "skips the All header");
        assert_eq!(step(&r, Some(first), -5), Some(first), "clamped at the top");
        let last = step(&r, Some(first), 99).unwrap();
        assert_eq!(last, r.len() - 1);
        assert_eq!(step(&rows(&it, &pins, "zzz", false), None, 1), None);
    }

    #[test]
    fn pin_toggles() {
        let mut pins = BTreeSet::new();
        assert!(toggle_pin(&mut pins, "a"));
        assert!(!toggle_pin(&mut pins, "a"));
        assert!(pins.is_empty());
    }
}
