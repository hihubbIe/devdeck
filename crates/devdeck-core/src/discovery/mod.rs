//! Finding apps in a workspace's folders (R6.1, R6.2, R6.3): the walk. What a folder
//! holds is the toolchains' business ([`crate::toolchains`]); this module only knows
//! folders.
//!
//! Each child folder of a workspace folder is a project when a toolchain finds
//! something in it. A folder that isn't one is looked into for projects, down to
//! [`NESTED_DEPTH`] levels under the workspace folder (`vhult/graph`); a project's own
//! inside is its toolchains' (a monorepo root's packages, found with the globs its
//! toolchain gives and scanned by every toolchain with the root's facts).
//!
//! An app is assembled from every toolchain's part: the first part's kind (the order of
//! `toolchains::ALL`), the commands of all of them (a command a toolchain says another
//! already runs is hidden), the facts each keeps, then completed by every toolchain.
//! An app is a library when a toolchain judges its package and none has something to
//! run nor a primary command. Tiers come from [`Tier::of`] with the source's other
//! names as siblings.
//!
//! App ids are the folder relative to the workspace folder (`front-end/mastertadaapp`);
//! the same id found under two workspace folders is reported and the second skipped.
//! Unreadable folders, broken manifests and symlink loops are problems, not panics.
//! An app's own `.gitlab-ci.yml` gives the image it's deployed as (`variables:
//! IMAGE_NAME`); that file isn't devdeck's, so a broken one is ignored silently.
//! Watched: the workspace folders, every folder looked into, each project and package,
//! and what toolchains ask for (glob folders, `.vscode`, Go's `cmd`, `node_modules`).

pub(crate) mod glob;
#[cfg(test)]
mod tests;

use crate::apps::{App, Command, EnvApply, Tier};
use crate::toolchains::{ALL, Facts, Listing, Part, Toolchain};
use std::any::{Any, TypeId};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub(crate) use crate::toolchains::skipped;
use crate::toolchains::reason;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Found {
    /// Alphabetical by id.
    pub apps: Vec<App>,
    /// Shown to the user (a broken `package.json`, a duplicate id…).
    pub problems: Vec<String>,
    /// Folders to watch (non-recursively) so new projects and changed manifests are
    /// noticed. Sorted, no repeats; some may not exist yet.
    pub watch: Vec<PathBuf>,
}

/// How many levels under a workspace folder projects are looked for: `a` is level 1,
/// `a/b/c` level 3.
pub const NESTED_DEPTH: usize = 3;

/// Every app in `folders` (expanded paths).
pub fn discover(folders: &[PathBuf]) -> Found {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    scan(folders, home.as_deref())
}

/// [`discover`] with the home folder given (`~` in `App::folder` and problems).
pub(crate) fn scan(folders: &[PathBuf], home: Option<&Path>) -> Found {
    let mut cx = Cx { home, found: Found::default(), owners: BTreeMap::new(), workspace: PathBuf::new(), memos: HashMap::new() };
    for ws in folders {
        cx.workspace = ws.clone();
        cx.workspace_folder(ws);
    }
    let mut found = cx.found;
    found.apps.sort_by(|a, b| a.id.cmp(&b.id));
    found.watch.sort();
    found.watch.dedup();
    found
}

/// One discovery pass: what was found so far. Toolchains read through it.
pub(crate) struct Cx<'a> {
    home: Option<&'a Path>,
    found: Found,
    /// The workspace folder each app id came from, to report duplicates.
    owners: BTreeMap<String, PathBuf>,
    /// The workspace folder being walked.
    workspace: PathBuf,
    /// What toolchains computed for a folder during this pass, by type and folder.
    memos: HashMap<(TypeId, PathBuf), Box<dyn Any>>,
}

/// A project a toolchain found: what its toolchains keep about it, and the package
/// globs of a monorepo root, by the toolchain that gave them.
struct Project {
    facts: Facts,
    packages: Vec<(&'static dyn Toolchain, Vec<String>)>,
}

impl Cx<'_> {
    pub(crate) fn problem(&mut self, path: &Path, what: impl std::fmt::Display) {
        let at = display(path, self.home);
        self.found.problems.push(format!("{at}: {what}"));
    }

    pub(crate) fn watch(&mut self, dir: &Path) {
        self.found.watch.push(dir.to_path_buf());
    }

    /// The workspace folder being walked.
    pub(crate) fn workspace(&self) -> &Path {
        &self.workspace
    }

    /// What `make` gives for `key`, computed once per pass.
    pub(crate) fn memo<T: 'static>(&mut self, key: &Path, make: impl FnOnce() -> T) -> &T {
        let value = self.memos.entry((TypeId::of::<T>(), key.to_path_buf())).or_insert_with(|| Box::new(make()));
        value.downcast_ref().expect("stored by its type")
    }

    /// A file's text; `None` when it doesn't exist (quietly) or can't be read (a problem).
    pub(crate) fn read(&mut self, path: &Path) -> Option<String> {
        match fs::read_to_string(path) {
            Ok(text) => Some(text),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => {
                self.problem(path, reason(&e));
                None
            }
        }
    }

    /// A folder's entries; entries that can't be followed are problems.
    pub(crate) fn listing(&mut self, dir: &Path) -> Option<Listing> {
        match Listing::read(dir) {
            Ok(listing) => {
                for (name, why) in listing.broken() {
                    self.problem(&dir.join(name), why);
                }
                Some(listing)
            }
            Err(e) => {
                self.problem(dir, reason(&e));
                None
            }
        }
    }

    fn workspace_folder(&mut self, ws: &Path) {
        self.watch(ws);
        let Some(listing) = self.listing(ws) else { return };
        let real = fs::canonicalize(ws).ok();
        for name in listing.folders() {
            let dir = ws.join(name);
            // A link to the workspace folder or above would list it again as a project.
            if let Some(real) = &real
                && fs::canonicalize(&dir).is_ok_and(|d| real.starts_with(d))
            {
                self.problem(&dir, "symlink loop");
                continue;
            }
            self.project(ws, &dir, name, 1);
        }
    }

    /// A project folder and, for a monorepo root, its packages. A folder that isn't a
    /// project is looked into, `depth` being its level under the workspace folder.
    fn project(&mut self, ws: &Path, dir: &Path, id: &str, depth: usize) {
        self.watch(dir);
        let Some(listing) = self.listing(dir) else { return };
        let Some(project) = self.app(ws, dir, id, &listing, None) else {
            if depth < NESTED_DEPTH {
                let names: Vec<String> = listing.folders().map(String::from).collect();
                for name in names {
                    self.project(ws, &dir.join(&name), &format!("{id}/{name}"), depth + 1);
                }
            }
            return;
        };
        for (toolchain, patterns) in project.packages {
            let matches = glob::expand(dir, &patterns, self);
            for listed in &matches.listed {
                self.watch(listed);
            }
            for rel in matches.dirs {
                let package = dir.join(&rel);
                let Some(listing) = Listing::read(&package).ok().filter(|l| toolchain.is_package(l)) else { continue };
                for (name, why) in listing.broken() {
                    self.problem(&package.join(name), why);
                }
                self.watch(&package);
                self.app(ws, &package, &format!("{id}/{rel}"), &listing, Some(&project.facts));
            }
        }
    }

    /// The app at `dir` from every toolchain, when one finds something (even when the
    /// app is skipped as a duplicate). `parent`: the monorepo root's facts.
    fn app(&mut self, ws: &Path, dir: &Path, id: &str, listing: &Listing, parent: Option<&Facts>) -> Option<Project> {
        let mut parts: Vec<(&'static dyn Toolchain, Part)> = Vec::new();
        for t in ALL {
            if let Some(part) = t.scan(dir, listing, parent, self) {
                parts.push((t, part));
            }
        }
        let kind = parts.first()?.1.kind;
        let mut project = Project { facts: Facts::default(), packages: Vec::new() };
        for (t, part) in &mut parts {
            project.facts.merge(std::mem::take(&mut part.facts));
            if !part.packages.is_empty() {
                project.packages.push((*t, std::mem::take(&mut part.packages)));
            }
        }
        for t in ALL {
            t.complete(&mut project.facts, dir, self);
        }
        if let Some(first) = self.owners.get(id) {
            let first = display(first, self.home);
            self.problem(dir, format!("`{id}` is also in {first}; skipped"));
            return Some(Project { packages: Vec::new(), ..project });
        }
        self.owners.insert(id.to_string(), ws.to_path_buf());

        // Commands a toolchain says the others already run.
        let covered: Vec<BTreeSet<String>> = (0..parts.len())
            .map(|i| {
                let others: Vec<String> = parts.iter().enumerate().filter(|(j, _)| *j != i).flat_map(|(_, (_, p))| p.runs.clone()).collect();
                parts[i].0.covered(&others)
            })
            .collect();
        let judged: Vec<bool> = parts.iter().filter_map(|(_, p)| p.runnable).collect();
        let mut commands: Vec<Command> = Vec::new();
        for ((_, part), covered) in parts.into_iter().zip(covered) {
            for mut c in part.commands {
                if commands.iter().any(|o| o.id == c.id) {
                    continue;
                }
                if covered.contains(&c.id) {
                    c.tier = Tier::Hidden;
                }
                commands.push(c);
            }
        }
        commands.sort_by(|a, b| a.id.cmp(&b.id));
        let library = !judged.is_empty() && !judged.contains(&true) && !commands.iter().any(|c| c.tier == Tier::Primary);
        self.found.apps.push(App {
            id: id.to_string(),
            name: id.rsplit('/').next().unwrap_or(id).to_string(),
            folder: display(dir, self.home),
            dir: dir.to_path_buf(),
            kind: kind.to_string(),
            library,
            facts: project.facts.clone(),
            commands,
            env: EnvApply::None,
            urls: Vec::new(),
            images: ci_image(dir).into_iter().collect(),
        });
        Some(project)
    }
}

/// The `IMAGE_NAME` of the top-level `variables` in `dir/.gitlab-ci.yml`, when it's a
/// plain string (one holding `$` uses variables: left out). Anything else in the file is
/// left alone; unreadable or broken is `None`.
fn ci_image(dir: &Path) -> Option<String> {
    #[derive(Default, serde::Deserialize)]
    struct Ci {
        #[serde(default)]
        variables: BTreeMap<String, Var>,
    }
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum Var {
        Text(String),
        Other(serde::de::IgnoredAny),
    }
    let text = fs::read_to_string(dir.join(".gitlab-ci.yml")).ok()?;
    let ci: Ci = crate::toolchains::yaml(&text).ok()?;
    match ci.variables.get("IMAGE_NAME") {
        Some(Var::Text(name)) if !name.trim().is_empty() && !name.contains('$') => Some(name.trim().to_string()),
        _ => None,
    }
}

/// `path` as shown: `~` for the home folder.
fn display(path: &Path, home: Option<&Path>) -> String {
    if let Some(home) = home.filter(|h| h.parent().is_some())
        && let Ok(rest) = path.strip_prefix(home)
    {
        return if rest.as_os_str().is_empty() { "~".into() } else { format!("~/{}", rest.display()) };
    }
    path.display().to_string()
}
