//! Languages and tools devdeck knows (R6.1, R6.5, R6.8), one module each behind the
//! [`Toolchain`] trait. Discovery, the watcher and the tool check only go through the
//! trait and [`ALL`]; nothing outside a toolchain's module knows its files, programs or
//! rules.
//!
//! Adding one: a module here, a line in [`ALL`] (its place decides an app's kind when
//! several toolchains find something in a folder), and, when it keeps something, a
//! field in [`Facts`] (about an app) or [`Machine`] (about this machine, with its
//! machine config key). The compiler points at each place.

pub mod cargo;
mod compose;
mod go;
mod make;
pub mod node;
mod project;
pub mod shell;
mod taskfile;
mod vscode;

pub(crate) use crate::discovery::Cx;
pub(crate) use crate::discovery::glob;
pub(crate) use project::{Listing, Part, quote, reason, skipped, yaml};

use crate::apps::{App, Command};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// A language or tool that finds commands in project folders and knows what they need
/// to run. Every method but `sources`, `reads` and `scan` has a default doing nothing.
pub(crate) trait Toolchain: Sync {
    /// The command sources it owns (the part of a command id before `:`: `pnpm`, `go`).
    fn sources(&self) -> &'static [&'static str];

    /// Whether it reads files of this name in a project folder: a change to one can
    /// change what it finds.
    fn reads(&self, name: &str) -> bool;

    /// Whether a change at `path` (in a watched folder) concerns it otherwise (Node: a
    /// program installed into `node_modules/.bin`).
    fn concerns(&self, _path: &Path) -> bool {
        false
    }

    /// Folders it keeps to itself, never projects (`node_modules`).
    fn skips(&self, _name: &str) -> bool {
        false
    }

    /// What it finds in a project folder. `parent` is the facts of the monorepo root
    /// when `dir` is one of its packages.
    fn scan(&self, dir: &Path, listing: &Listing, parent: Option<&Facts>, cx: &mut Cx) -> Option<Part>;

    /// For a folder its monorepo globs matched: whether it's a package.
    fn is_package(&self, _listing: &Listing) -> bool {
        false
    }

    /// Completes an app's facts whatever found the app (Node: an `.nvmrc` above a
    /// Taskfile project).
    fn complete(&self, _facts: &mut Facts, _dir: &Path, _cx: &mut Cx) {}

    /// Its commands other toolchains already run (`runs`: their shell text), which it
    /// hides (Go: `go:run` when a Taskfile runs the main).
    fn covered(&self, _runs: &[String]) -> BTreeSet<String> {
        BTreeSet::new()
    }

    /// Whether `program` comes with it (Node: `node`, `npm`, `pnpm`…): a command needing
    /// it is refused when [`front`](Toolchain::front) fails for the app.
    fn provides(&self, _program: &str) -> bool {
        false
    }

    /// Folders to put first on the PATH of an app's runs (Node: the app's Node), or why
    /// it can't serve this app.
    fn front(&self, _app: &App, _machine: &Machine) -> Result<Vec<PathBuf>, String> {
        Ok(Vec::new())
    }

    /// Why its command can't run although its programs are there (Node: the project's
    /// install isn't done), with `path` the run's PATH.
    fn installed(&self, _app: &App, _cmd: &Command, _path: &[PathBuf]) -> Option<String> {
        None
    }
}

/// Every toolchain, in the order that decides an app's kind.
pub(crate) static ALL: [&dyn Toolchain; 7] =
    [&go::Go, &cargo::Cargo, &node::Node, &taskfile::GoTask, &make::Make, &compose::Compose, &vscode::VsCode];

/// The toolchain owning a command (`None` for one only the config defines).
pub(crate) fn of(cmd: &Command) -> Option<&'static dyn Toolchain> {
    ALL.iter().copied().find(|t| t.sources().contains(&cmd.source()))
}

/// Whether a toolchain reads files of this name.
pub fn reads(name: &str) -> bool {
    ALL.iter().any(|t| t.reads(name))
}

/// Whether a change at `path`, in a watched folder, can change what's found: a file a
/// toolchain reads, a folder (a new project or package, a rename), or what a toolchain
/// watches otherwise.
pub fn concerns(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    ALL.iter().any(|t| t.concerns(path)) || (!in_skipped(path) && (reads(name) || path.is_dir()))
}

/// Inside a folder a toolchain keeps to itself (`node_modules/x`).
fn in_skipped(path: &Path) -> bool {
    path.parent().and_then(Path::file_name).and_then(|n| n.to_str()).is_some_and(|n| ALL.iter().any(|t| t.skips(n)))
}

/// What toolchains keep about one app: one field per toolchain that keeps anything.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Facts {
    pub node: Option<node::Facts>,
}

impl Facts {
    /// Takes what `other` has and this doesn't.
    pub fn merge(&mut self, other: Facts) {
        let Facts { node } = other;
        if self.node.is_none() {
            self.node = node;
        }
    }
}

/// What toolchains found on this machine: one field per toolchain that looks.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Machine {
    pub node: node::Machine,
}

impl Machine {
    /// From the machine config, the home folder and the PATH runs get.
    pub fn load(config: &crate::config::MachineConfig, home: Option<&Path>, path: &[PathBuf]) -> Machine {
        Machine { node: node::Machine::load(&config.node_dirs, home, path) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::Tree;

    #[test]
    fn every_command_found_has_its_toolchain_and_every_manifest_is_read() {
        let mut seen = BTreeSet::new();
        for t in ALL {
            for s in t.sources() {
                assert!(seen.insert(*s), "{s} owned twice");
            }
        }
        let t = Tree::new("contract");
        let manifests = [
            ("a/package.json", r#"{"scripts": {"dev": "vite"}}"#),
            ("a/pnpm-lock.yaml", ""),
            ("b/go.mod", "module b\n"),
            ("b/main.go", "package main\nfunc main() {}\n"),
            ("c/Cargo.toml", "[package]\nname = \"c\"\n"),
            ("c/src/main.rs", "fn main() {}\n"),
            ("d/Taskfile.yml", "version: '3'\ntasks:\n  run:\n    cmds: [echo]\n"),
            ("e/Makefile", "run:\n\ttrue\n"),
            ("f/compose.yaml", "services: {}\n"),
            ("g/.vscode/launch.json", r#"{"configurations": [{"name": "x", "type": "node", "request": "launch", "program": "${workspaceFolder}/x.js"}]}"#),
        ];
        for (path, text) in manifests {
            t.file(path, text);
            let name = path.rsplit('/').next().unwrap();
            assert!(reads(name) || name == "main.rs", "{name} is read by a toolchain");
        }
        let found = t.scan();
        assert_eq!(found.apps.len(), 7, "{:?}", found.apps.iter().map(|a| &a.id).collect::<Vec<_>>());
        for app in &found.apps {
            for cmd in &app.commands {
                assert!(of(cmd).is_some(), "{}: {} has no toolchain", app.id, cmd.id);
            }
        }
    }

    #[test]
    fn changes_inside_kept_folders_dont_concern_discovery() {
        let t = Tree::new("concerns");
        t.dir("web/node_modules/.bin").dir("web/node_modules/vite");
        assert!(concerns(&t.path("web/node_modules/.bin")), "the programs folder appears");
        assert!(concerns(&t.path("web/node_modules/.bin/vite")), "a program is installed or removed");
        assert!(!concerns(&t.path("web/node_modules/vite")), "a package's own folder");
        assert!(concerns(&t.path("web/package.json")) && concerns(&t.path("web/node_modules")));
        assert!(!concerns(&t.path("web/debug.log")));
    }

    #[test]
    fn reads_names_the_files_discovery_reads() {
        for name in ["package.json", "pnpm-workspace.yaml", "Taskfile.yml", "GNUmakefile", "compose.yaml", "docker-compose.dev.yml", "launch.json", "main.go", ".nvmrc"] {
            assert!(reads(name), "{name}");
        }
        for name in ["tsconfig.tsbuildinfo", "debug.log", ".env", "main_test.go", "docker-compose.override.yml", "README.md"] {
            assert!(!reads(name), "{name}");
        }
    }
}
