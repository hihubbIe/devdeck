//! Reading processes from `/proc` (Linux): parents and children, so a run's whole tree
//! can be found (processes that left its process group included) and its sockets
//! attributed. Shared by `process` (stopping) and `ports` (attribution).

use std::collections::{BTreeMap, BTreeSet};

/// `(pid, parent pid)` of every process we can read.
pub fn parents() -> Vec<(u32, u32)> {
    let Ok(dir) = std::fs::read_dir("/proc") else { return Vec::new() };
    dir.filter_map(|e| e.ok()?.file_name().to_str()?.parse::<u32>().ok())
        .filter_map(|pid| Some((pid, parent_of(pid)?)))
        .collect()
}

/// The parent pid of `pid`, from `/proc/<pid>/stat`.
pub fn parent_of(pid: u32) -> Option<u32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    parse_ppid(&stat)
}

/// The ppid field of a `stat` line. The command name is in parentheses and may hold
/// spaces or parentheses itself, so fields are counted after the last `)`.
pub fn parse_ppid(stat: &str) -> Option<u32> {
    stat.rsplit_once(')')?.1.split_whitespace().nth(1)?.parse().ok()
}

/// `root` and every process below it, in `pairs` (`(pid, parent)`).
pub fn tree(root: u32, pairs: &[(u32, u32)]) -> BTreeSet<u32> {
    let mut children: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for (pid, parent) in pairs {
        children.entry(*parent).or_default().push(*pid);
    }
    let mut out = BTreeSet::from([root]);
    let mut todo = vec![root];
    while let Some(p) = todo.pop() {
        for c in children.get(&p).into_iter().flatten() {
            if out.insert(*c) {
                todo.push(*c);
            }
        }
    }
    out
}

/// `root` and every live process below it.
pub fn descendants(root: u32) -> BTreeSet<u32> {
    tree(root, &parents())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ppid_survives_odd_command_names() {
        assert_eq!(parse_ppid("123 (node) S 77 123 123 0"), Some(77));
        assert_eq!(parse_ppid("123 (a) b (c) S 88 1 1"), Some(88));
        assert_eq!(parse_ppid("garbage"), None);
    }

    #[test]
    fn a_tree_follows_children_down() {
        let pairs = [(2, 1), (3, 2), (4, 3), (5, 1), (6, 4)];
        assert_eq!(tree(2, &pairs), BTreeSet::from([2, 3, 4, 6]));
        assert_eq!(tree(5, &pairs), BTreeSet::from([5]));
    }

    #[test]
    fn this_test_process_is_below_its_parent() {
        let me = std::process::id();
        let parent = parent_of(me).expect("readable");
        assert!(descendants(parent).contains(&me));
    }
}
