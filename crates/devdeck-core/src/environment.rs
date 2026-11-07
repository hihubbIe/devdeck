//! The environment runs get (R6.5, R6.8, R1.4): the login shell's variables, the
//! programs on its PATH, and the Node version each app asks for. Builder C of M2 step 2
//! implements this file; the interface is fixed here.
//!
//! An app started from the desktop doesn't see what `~/.bashrc` sets (nvm, pnpm,
//! `~/go/bin`), so the user's login shell is asked once (`$SHELL -lic env`, with a
//! timeout) and its variables are the base for every run; the machine config's `path`
//! goes in front. Node: `.nvmrc` is searched from the app's folder upward to its
//! workspace folder; versions (`20`, `v20.11.1`, `lts/*`, `lts/iron`, using
//! `~/.nvm/alias` for aliases) resolve against the installs found in the machine
//! config's `node_dirs`; the chosen Node's `bin` goes first on the run's PATH, which
//! also decides whether `pnpm` and `npm` exist for that app.

use crate::apps::{App, Command};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct NodeInstall {
    /// `22.23.3`.
    pub version: String,
    /// Its `bin` folder.
    pub bin: PathBuf,
}

/// What runs need, found once per workspace load.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Toolbox {
    /// The login shell's variables (PATH included, machine `path` in front).
    pub vars: Vec<(String, String)>,
    pub node: Vec<NodeInstall>,
    /// Where nvm keeps aliases (`~/.nvm/alias`), when it exists.
    pub aliases: Option<PathBuf>,
}

impl Toolbox {
    /// Asks the login shell for its environment and finds Node installs.
    pub fn load(machine: &crate::config::MachineConfig) -> Result<Toolbox, String> {
        let _ = machine;
        todo!("M2 step 2, builder C")
    }

    /// The Node install for an app, or why there is none (`.nvmrc wants node 20, which
    /// isn't installed`). `Ok(None)` when the app asks for no version.
    pub fn node_for(&self, app: &App) -> Result<Option<&NodeInstall>, String> {
        let _ = app;
        todo!("M2 step 2, builder C")
    }

    /// The PATH for a run of `app`: its Node first, then the base PATH.
    pub fn path_for(&self, app: &App) -> Vec<PathBuf> {
        let _ = app;
        todo!("M2 step 2, builder C")
    }

    /// Why `cmd` of `app` can't run (`go not found`, the Node reason), if it can't.
    pub fn missing(&self, app: &App, cmd: &Command) -> Option<String> {
        let _ = (app, cmd);
        todo!("M2 step 2, builder C")
    }
}

/// The program `name` in `path`, when it's there and executable.
pub fn which(name: &str, path: &[PathBuf]) -> Option<PathBuf> {
    let _ = (name, path);
    todo!("M2 step 2, builder C")
}

/// The version an `.nvmrc` at `dir` or above (up to `stop`, included) asks for, trimmed
/// (`20`, `v20.11.1`, `lts/iron`). Discovery sets `App::node` with it.
pub fn nvmrc(dir: &Path, stop: &Path) -> Option<String> {
    let mut at = Some(dir);
    while let Some(d) = at {
        if let Ok(text) = std::fs::read_to_string(d.join(".nvmrc")) {
            let v = text.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with('#'))?;
            return Some(v.to_string());
        }
        if d == stop {
            break;
        }
        at = d.parent();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nvmrc_is_found_upward_until_the_workspace_folder() {
        let root = std::env::temp_dir().join(format!("devdeck-nvmrc-{}", std::process::id()));
        let app = root.join("front-end/apps/x");
        std::fs::create_dir_all(&app).unwrap();
        assert_eq!(nvmrc(&app, &root), None);
        std::fs::write(root.join("front-end/.nvmrc"), "# node\n 20 \n").unwrap();
        assert_eq!(nvmrc(&app, &root).as_deref(), Some("20"));
        assert_eq!(nvmrc(&app, &root.join("front-end/apps")), None, "stops at the workspace folder");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
