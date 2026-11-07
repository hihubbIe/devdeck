//! Workspace configuration (R10.1, R1.5, R6.4, R6.6). Builder A of M2 step 2 implements
//! this file; the interface is fixed here.
//!
//! Files, all TOML, in the config folder (`~/.config/devdeck`):
//! - `workspaces/<id>.toml`, one per workspace, shareable (no machine paths):
//!   ```toml
//!   name = "tadaweb"
//!   folders = ["~/Documents/repos"]   # each child folder is a project; monorepos expand
//!   envs = ["dev33", "dev35", "dls"]  # optional
//!   protected = ["dls"]               # writes need an unlock (R7.6)
//!   default_env = "dev35"
//!
//!   [apps.entityservice]              # key: the app id (its folder, relative)
//!   name = "entity"                   # display name; the id never changes
//!   hidden = false
//!   env = { argument = "-s {env}" }   # or { variable = "DEV_ENV" }, or "none"
//!   urls = ["http://localhost:5720/health"]
//!   [apps.entityservice.commands."task:run"]
//!   line = "go run main.go serve -c configs -s {env}"
//!   ports = [5720]
//!   tier = "primary"
//!   ```
//! - `settings.toml`, `[machine]` table, per machine (never shared):
//!   ```toml
//!   [machine]
//!   node_dirs = ["~/.nvm/versions/node"]  # where `.nvmrc` versions are looked up
//!   path = ["~/go/bin"]                   # added in front of the login shell's PATH
//!   ```
//!
//! Rules: `~` expands to the home folder; unknown keys are errors (a typo in an
//! override must not be silently ignored); a broken file is reported with its path and
//! line and the other files still load; overrides naming an app that wasn't found are
//! warnings.

use crate::apps::{App, EnvApply, Tier};
use crate::catalog::Env;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The folder holding workspace files, inside the config folder.
pub const WORKSPACES_DIR: &str = "workspaces";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WorkspaceConfig {
    /// The file name without `.toml`.
    pub id: String,
    pub name: String,
    /// Expanded (no `~`).
    pub folders: Vec<PathBuf>,
    /// Alphabetical.
    pub envs: Vec<Env>,
    pub default_env: Option<String>,
    /// By app id.
    pub apps: BTreeMap<String, AppOverride>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AppOverride {
    pub name: Option<String>,
    pub hidden: bool,
    pub env: Option<EnvApply>,
    pub urls: Vec<String>,
    /// By command id (`task:run`); an id that discovery didn't find adds a command.
    pub commands: BTreeMap<String, CommandOverride>,
}

/// Changes to a found command, or a new one: a command id discovery didn't find needs a
/// `line` (an error otherwise) and gets the tier of its name unless one is given.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CommandOverride {
    pub line: Option<String>,
    pub ports: Option<Vec<u16>>,
    pub tier: Option<Tier>,
    /// Programs it needs, for a new command (found commands keep theirs).
    pub tools: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MachineConfig {
    /// Folders holding one folder per Node version (`v22.23.3/bin/node`).
    pub node_dirs: Vec<PathBuf>,
    /// Added in front of the login shell's PATH.
    pub path: Vec<PathBuf>,
}

/// A problem in a config file, shown to the user.
#[derive(Clone, Debug, PartialEq)]
pub struct ConfigError {
    pub file: PathBuf,
    pub line: Option<usize>,
    pub message: String,
}

/// Every `workspaces/*.toml` in `config_dir`, alphabetical by id, with the problems of
/// the files that couldn't be read (those are left out).
pub fn load_workspaces(config_dir: &Path) -> (Vec<WorkspaceConfig>, Vec<ConfigError>) {
    let _ = config_dir;
    todo!("M2 step 2, builder A")
}

/// The `[machine]` table of `settings.toml` (defaults when missing).
pub fn load_machine(config_dir: &Path) -> (MachineConfig, Vec<ConfigError>) {
    let _ = config_dir;
    todo!("M2 step 2, builder A")
}

/// Discovered apps with the workspace's overrides applied: names, hidden apps removed,
/// env bindings, URLs, command lines, ports and tiers changed or added. Returns
/// warnings (shown like discovery problems, no file line) for overrides that name no
/// discovered app and for added commands without a line.
pub fn apply_overrides(apps: Vec<App>, overrides: &BTreeMap<String, AppOverride>) -> (Vec<App>, Vec<String>) {
    let _ = (apps, overrides);
    todo!("M2 step 2, builder A")
}
