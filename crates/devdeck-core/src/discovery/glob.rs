//! Workspace globs (`apps/*`, `packages/**`, `!apps/legacy`): pnpm and npm workspaces,
//! Cargo workspace members. Only the folders a pattern needs are read; hidden folders
//! and `node_modules` are never entered.

use super::Cx;
use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

/// How deep `**` goes below where it starts.
const MAX_DEPTH: usize = 12;

#[derive(Debug, Default)]
pub(super) struct Matches {
    /// Matching folders relative to the root, with `/`, alphabetical.
    pub dirs: Vec<String>,
    /// Folders whose children were matched against a pattern (`apps` for `apps/*`,
    /// `packages` for `packages/**`).
    pub listed: Vec<PathBuf>,
}

/// The folders below `root` that `patterns` match; `!pattern` excludes.
pub(super) fn expand(root: &Path, patterns: &[String], cx: &mut Cx) -> Matches {
    let mut walk = Walk { found: BTreeSet::new(), listed: Vec::new(), ancestors: Vec::new(), seen: HashSet::new() };
    let mut excluded = Vec::new();
    for pattern in patterns {
        match pattern.strip_prefix('!') {
            Some(p) => excluded.push(segments(p)),
            None => {
                // The memo is per pattern: its key is the rest of this pattern.
                walk.seen.clear();
                walk.walk(root, "", &segments(pattern), 0, cx)
            }
        }
    }
    let dirs = walk
        .found
        .into_iter()
        .filter(|d| {
            let path: Vec<&str> = d.split('/').collect();
            !excluded.iter().any(|p| path_matches(p, &path))
        })
        .collect();
    Matches { dirs, listed: walk.listed }
}

/// A pattern's segments; repeated `**` are one (`**/**` means `**`, and repeats would
/// make walking and matching combinatorial).
fn segments(pattern: &str) -> Vec<&str> {
    let mut segs: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty() && *s != ".").collect();
    segs.dedup_by(|a, b| *a == "**" && *b == "**");
    segs
}

struct Walk {
    found: BTreeSet<String>,
    listed: Vec<PathBuf>,
    /// Real paths of the folders `**` is inside, to stop at symlink loops.
    ancestors: Vec<PathBuf>,
    /// Folders already walked with the same rest of a pattern: several `**` can reach a
    /// folder in many ways, and each needs walking once.
    seen: HashSet<(PathBuf, usize)>,
}

impl Walk {
    fn walk(&mut self, dir: &Path, rel: &str, segs: &[&str], depth: usize, cx: &mut Cx) {
        if !self.seen.insert((dir.to_path_buf(), segs.len())) {
            return;
        }
        let Some((seg, rest)) = segs.split_first() else {
            if !rel.is_empty() {
                self.found.insert(rel.to_string());
            }
            return;
        };
        let join = |name: &str| if rel.is_empty() { name.to_string() } else { format!("{rel}/{name}") };
        if *seg == "**" {
            if depth == 0 {
                self.listed.push(dir.to_path_buf());
            }
            self.walk(dir, rel, rest, depth, cx);
            if depth >= MAX_DEPTH {
                return;
            }
            let Ok(real) = fs::canonicalize(dir) else { return };
            if self.ancestors.contains(&real) {
                cx.problem(dir, "symlink loop");
                return;
            }
            self.ancestors.push(real);
            for name in children(dir, cx) {
                self.walk(&dir.join(&name), &join(&name), segs, depth + 1, cx);
            }
            self.ancestors.pop();
        } else if !seg.contains(['*', '?']) {
            let child = dir.join(seg);
            if !super::skipped(seg) && *seg != ".." && child.is_dir() {
                self.walk(&child, &join(seg), rest, depth, cx);
            }
        } else {
            // Inside `**` every folder is read anyway; watching them all would be too many.
            if depth == 0 {
                self.listed.push(dir.to_path_buf());
            }
            for name in children(dir, cx) {
                if segment_matches(seg, &name) {
                    self.walk(&dir.join(&name), &join(&name), rest, depth, cx);
                }
            }
        }
    }
}

/// Child folders that can hold packages; a missing folder has none.
fn children(dir: &Path, cx: &mut Cx) -> Vec<String> {
    if !dir.is_dir() {
        return Vec::new();
    }
    cx.listing(dir).map(|l| l.folders().map(String::from).collect()).unwrap_or_default()
}

/// One path segment against a pattern segment with `*` and `?`.
fn segment_matches(pattern: &str, name: &str) -> bool {
    let (p, n): (Vec<char>, Vec<char>) = (pattern.chars().collect(), name.chars().collect());
    // Classic wildcard matching, backtracking to the last `*`.
    let (mut i, mut j, mut star, mut mark) = (0, 0, None, 0);
    while j < n.len() {
        if i < p.len() && (p[i] == '?' || p[i] == n[j]) {
            i += 1;
            j += 1;
        } else if i < p.len() && p[i] == '*' {
            star = Some(i);
            mark = j;
            i += 1;
        } else if let Some(s) = star {
            i = s + 1;
            mark += 1;
            j = mark;
        } else {
            return false;
        }
    }
    p[i..].iter().all(|c| *c == '*')
}

/// A path (as segments) against a pattern (as segments) with `**`.
fn path_matches(pattern: &[&str], path: &[&str]) -> bool {
    match pattern.split_first() {
        None => path.is_empty(),
        Some((&"**", rest)) => (0..=path.len()).any(|k| path_matches(rest, &path[k..])),
        Some((seg, rest)) => path.split_first().is_some_and(|(p, prest)| segment_matches(seg, p) && path_matches(rest, prest)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_match_with_wildcards() {
        assert!(segment_matches("*", "app"));
        assert!(segment_matches("app-*", "app-web"));
        assert!(!segment_matches("app-*", "lib-web"));
        assert!(segment_matches("a?c", "abc"));
        assert!(segment_matches("*-*", "a-b-c"));
        assert!(!segment_matches("abc", "abcd"));
    }

    #[test]
    fn repeated_double_stars_are_one() {
        assert_eq!(segments("**/**/a/**/**/**"), ["**", "a", "**"]);
        let many = ["**"; 40].join("/") + "/x";
        let segs = segments(&many);
        assert_eq!(segs, ["**", "x"]);
        let path: Vec<&str> = (0..12).map(|_| "d").collect();
        assert!(!path_matches(&["**", "a", "**", "b", "**", "c"], &path), "fails fast, no blow-up");
    }

    #[test]
    fn paths_match_with_double_stars() {
        assert!(path_matches(&["apps", "legacy"], &["apps", "legacy"]));
        assert!(path_matches(&["**", "test"], &["packages", "a", "test"]));
        assert!(path_matches(&["packages", "**"], &["packages", "a", "b"]));
        assert!(!path_matches(&["apps", "*"], &["apps", "a", "b"]));
    }
}
