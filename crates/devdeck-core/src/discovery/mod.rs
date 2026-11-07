//! Finding apps in a workspace's folders (R6.1, R6.2, R6.3). Builder B of M2 step 2
//! implements this folder (one file per source next to this one); the interface is
//! fixed here.
//!
//! Each child folder of a workspace folder is a project. Sources, each giving commands
//! with ids `source:name`:
//! - `package.json` scripts (`npm:`, `pnpm:` or `yarn:` from the lockfile or the
//!   `packageManager` field); pnpm and npm workspaces expand into one app per package
//!   (never looking inside `node_modules`);
//! - `Taskfile.yml`/`.yaml` tasks (`task:`), Makefile targets (`make:`), docker compose
//!   files (`compose:up`, `compose:down`), `.vscode/launch.json` configurations
//!   (`vscode:`, turned into a command line), Go main packages (`go:run` for the root,
//!   `go:<cmd>` for `cmd/<cmd>`, plus `go:build` and `go:test`), Cargo (`cargo:run`,
//!   `cargo:build`, `cargo:test`).
//!
//! Precedence: when a Taskfile, Makefile or launch.json runs the Go main, the bare
//! `go:run` is hidden (it lacks their arguments and variables). Tiers come from
//! [`Tier::of`](crate::apps::Tier::of) with the source's other names as siblings.
//! Libraries (a package with no runnable script, a Go module without a main) are listed
//! with `library: true`. Tools: each command names the programs it needs (`pnpm`,
//! `node`, `task`, `go`…).
//!
//! `App::node` comes from [`nvmrc`](crate::environment::nvmrc) (already implemented).
//!
//! App ids are the folder relative to the workspace folder (`front-end/mastertadaapp`);
//! the same id found under two workspace folders is reported and the second skipped.
//! Unreadable folders, broken manifests and symlink loops are problems, not panics.
//!
//! Details:
//! - A folder is an app when it has a manifest (`package.json`, `go.mod`, `Cargo.toml`,
//!   a Taskfile, a Makefile, a compose file), even a broken one, or a launch.json that
//!   gives commands. Hidden folders and `node_modules` are never projects or packages.
//! - The kind is the first source found in this order: go, cargo, the Node package
//!   manager, task, make, docker, vscode.
//! - Precedence applies to every other source's shell text (package.json scripts too),
//!   and to `go:<cmd>` as well: `task run` running `go run ./cmd/api` hides `go:api`.
//! - An app is a library when it has a package (package.json, Go module, Cargo) and
//!   nothing runnable: no primary command, no Go main, no Cargo binary. Folders with
//!   only a Taskfile, a Makefile or compose files are never libraries.
//! - Watched besides the projects and packages: the folders monorepo globs list
//!   (`apps` for `apps/*`, so new packages are noticed), `.vscode` and Go's `cmd`.

mod cargo;
mod compose;
mod glob;
mod go;
mod make;
mod node;
mod taskfile;
mod vscode;
#[cfg(test)]
mod tests;

use crate::apps::{App, Command, EnvApply, Tier};
use crate::environment::nvmrc;
use serde::de::DeserializeOwned;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Found {
    /// Alphabetical by id.
    pub apps: Vec<App>,
    /// Shown to the user (a broken `package.json`, a duplicate id…).
    pub problems: Vec<String>,
    /// Folders to watch (non-recursively) so new projects and changed manifests are
    /// noticed: the workspace folders, each project, each monorepo package folder, and
    /// the folders globs, `.vscode` and Go's `cmd` read. Sorted, no repeats.
    pub watch: Vec<PathBuf>,
}

/// Every app in `folders` (expanded paths).
pub fn discover(folders: &[PathBuf]) -> Found {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    scan(folders, home.as_deref())
}

/// [`discover`] with the home folder given (`~` in `App::folder` and problems).
fn scan(folders: &[PathBuf], home: Option<&Path>) -> Found {
    let mut cx = Cx { home, found: Found::default(), owners: BTreeMap::new() };
    for ws in folders {
        cx.workspace_folder(ws);
    }
    let mut found = cx.found;
    found.apps.sort_by(|a, b| a.id.cmp(&b.id));
    found.watch.sort();
    found.watch.dedup();
    found
}

/// One discovery pass: what was found so far.
struct Cx<'a> {
    home: Option<&'a Path>,
    found: Found,
    /// The workspace folder each app id came from, to report duplicates.
    owners: BTreeMap<String, PathBuf>,
}

impl Cx<'_> {
    fn problem(&mut self, path: &Path, what: impl std::fmt::Display) {
        let at = display(path, self.home);
        self.found.problems.push(format!("{at}: {what}"));
    }

    fn watch(&mut self, dir: &Path) {
        self.found.watch.push(dir.to_path_buf());
    }

    /// A file's text; `None` when it doesn't exist (quietly) or can't be read (a problem).
    fn read(&mut self, path: &Path) -> Option<String> {
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
    fn listing(&mut self, dir: &Path) -> Option<Listing> {
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
            self.project(ws, &dir, name);
        }
    }

    /// A project folder and, for a monorepo root, its packages.
    fn project(&mut self, ws: &Path, dir: &Path, id: &str) {
        self.watch(dir);
        let Some(listing) = self.listing(dir) else { return };
        let node = node::scan(dir, &listing, None, self);
        let monorepo = node.as_ref().filter(|n| !n.workspaces.is_empty()).map(|n| (n.manager, n.workspaces.clone()));
        self.app(ws, dir, id, &listing, node);
        let Some((manager, patterns)) = monorepo else { return };
        let matches = glob::expand(dir, &patterns, self);
        for listed in &matches.listed {
            self.watch(listed);
        }
        for rel in matches.dirs {
            let package = dir.join(&rel);
            if !package.join("package.json").is_file() {
                continue;
            }
            self.watch(&package);
            let Some(listing) = self.listing(&package) else { continue };
            let node = node::scan(&package, &listing, Some(manager), self);
            self.app(ws, &package, &format!("{id}/{rel}"), &listing, node);
        }
    }

    /// The app at `dir` from every source, when there is one.
    fn app(&mut self, ws: &Path, dir: &Path, id: &str, listing: &Listing, node: Option<node::Node>) {
        let parts: Vec<Part> = [
            go::scan(dir, listing, self),
            cargo::scan(dir, listing, self),
            node.map(|n| n.part),
            taskfile::scan(dir, listing, self),
            make::scan(dir, listing, self),
            compose::scan(dir, listing, self),
            vscode::scan(dir, listing, self),
        ]
        .into_iter()
        .flatten()
        .collect();
        let Some(kind) = parts.first().map(|p| p.kind) else { return };
        if let Some(first) = self.owners.get(id) {
            let first = display(first, self.home);
            self.problem(dir, format!("`{id}` is also in {first}; skipped"));
            return;
        }
        self.owners.insert(id.to_string(), ws.to_path_buf());

        let hidden: BTreeSet<String> = parts
            .iter()
            .filter(|p| p.kind != go::KIND)
            .flat_map(|p| &p.runs)
            .flat_map(|run| go::targets(run))
            .collect();
        let judged: Vec<bool> = parts.iter().filter_map(|p| p.runnable).collect();
        let mut commands: Vec<Command> = Vec::new();
        for part in parts {
            for mut c in part.commands {
                if commands.iter().any(|o| o.id == c.id) {
                    continue;
                }
                if part.kind == go::KIND && hidden.contains(&c.id) {
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
            node: nvmrc(dir, ws),
            commands,
            env: EnvApply::None,
            urls: Vec::new(),
        });
    }
}

/// What one source found in a folder.
struct Part {
    /// `go`, `pnpm`, `task`…: the app's kind when this is its main source.
    kind: &'static str,
    commands: Vec<Command>,
    /// The shell text its commands run, to see which of them run a Go main.
    runs: Vec<String>,
    /// For a package (package.json, Go module, Cargo): whether it has something to
    /// run. `None` when the source doesn't say (or its manifest is broken).
    runnable: Option<bool>,
}

impl Part {
    fn new(kind: &'static str) -> Part {
        Part { kind, commands: Vec::new(), runs: Vec::new(), runnable: None }
    }

    /// Adds `source:name`, tiered among `siblings`.
    fn add(&mut self, source: &str, name: &str, line: String, siblings: &[&str], tools: &[&str]) {
        self.commands.push(Command {
            id: format!("{source}:{name}"),
            line,
            tier: Tier::of(name, siblings),
            tools: tools.iter().map(|t| t.to_string()).collect(),
            ports: Vec::new(),
        });
    }
}

/// A folder's entries, read once so sources only look names up.
struct Listing(BTreeMap<String, Entry>);

enum Entry {
    File,
    Dir,
    /// A link that can't be followed (a loop…), with the reason.
    Broken(String),
}

impl Listing {
    fn read(dir: &Path) -> io::Result<Listing> {
        let mut entries = BTreeMap::new();
        for e in fs::read_dir(dir)? {
            let Ok(e) = e else { continue };
            let Ok(name) = e.file_name().into_string() else { continue };
            let entry = match e.file_type() {
                Ok(t) if t.is_dir() => Entry::Dir,
                Ok(t) if t.is_symlink() => match fs::metadata(e.path()) {
                    Ok(m) if m.is_dir() => Entry::Dir,
                    // A dangling link reads as missing later.
                    Ok(_) => Entry::File,
                    Err(e) if e.kind() == io::ErrorKind::NotFound => Entry::File,
                    Err(e) => Entry::Broken(reason(&e)),
                },
                Ok(_) => Entry::File,
                Err(e) => Entry::Broken(reason(&e)),
            };
            entries.insert(name, entry);
        }
        Ok(Listing(entries))
    }

    fn file(&self, name: &str) -> bool {
        matches!(self.0.get(name), Some(Entry::File))
    }

    fn dir(&self, name: &str) -> bool {
        matches!(self.0.get(name), Some(Entry::Dir))
    }

    /// File names, alphabetical.
    fn files(&self) -> impl Iterator<Item = &str> {
        self.0.iter().filter(|(_, e)| matches!(e, Entry::File)).map(|(n, _)| n.as_str())
    }

    /// Child folders that can be projects or packages: not hidden, not `node_modules`.
    fn folders(&self) -> impl Iterator<Item = &str> {
        self.0.iter().filter(|(n, e)| matches!(e, Entry::Dir) && !skipped(n)).map(|(n, _)| n.as_str())
    }

    /// Entries that can't be followed, except hidden ones.
    fn broken(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0.iter().filter(|(n, _)| !skipped(n)).filter_map(|(n, e)| match e {
            Entry::Broken(why) => Some((n.as_str(), why.as_str())),
            _ => None,
        })
    }
}

/// Folders never looked into.
fn skipped(name: &str) -> bool {
    name.starts_with('.') || name == "node_modules"
}

/// An IO error as shown.
fn reason(e: &io::Error) -> String {
    if rustix::io::Errno::from_io_error(e) == Some(rustix::io::Errno::LOOP) {
        "symlink loop".into()
    } else {
        e.to_string()
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

/// `s` as one shell word: unchanged when safe, else single-quoted.
fn quote(s: &str) -> String {
    let safe = !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "_-./:=@%+,".contains(c));
    if safe { s.to_string() } else { format!("'{}'", s.replace('\'', r"'\''")) }
}

/// A YAML document; empty gives the default. Duplicate keys are tolerated (the last
/// wins), as the tools reading these files do.
fn yaml<T: DeserializeOwned + Default>(text: &str) -> Result<T, String> {
    let options = serde_saphyr::options! { duplicate_keys: serde_saphyr::DuplicateKeyPolicy::LastWins };
    match serde_saphyr::from_str_with_options::<Option<T>>(text, options) {
        Ok(v) => Ok(v.unwrap_or_default()),
        Err(e) => Err(e.to_string().lines().next().unwrap_or_default().trim_start_matches("error: ").to_string()),
    }
}
