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

use crate::apps::App;
use std::path::PathBuf;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Found {
    /// Alphabetical by id.
    pub apps: Vec<App>,
    /// Shown to the user (a broken `package.json`, a duplicate id…).
    pub problems: Vec<String>,
    /// Folders to watch (non-recursively) so new projects and changed manifests are
    /// noticed: the workspace folders, each project, each monorepo package folder.
    pub watch: Vec<PathBuf>,
}

/// Every app in `folders` (expanded paths).
pub fn discover(folders: &[PathBuf]) -> Found {
    let _ = folders;
    todo!("M2 step 2, builder B")
}
