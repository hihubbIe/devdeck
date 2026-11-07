//! The arguments typed for each command, most recent first (a run tab's arguments
//! menu), in the state folder (`args.json`). Nested by workspace id, app id, command id
//! (app ids hold `/`, so no joined key).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// How many arguments a command keeps.
pub const KEPT: usize = 10;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RecentArgs {
    pub path: PathBuf,
    pub args: BTreeMap<String, BTreeMap<String, BTreeMap<String, Vec<String>>>>,
}

impl RecentArgs {
    /// Reads `args.json` in `state_dir`. Missing or unreadable means none; a broken file
    /// is moved aside (`args.json.bad`) so the next save doesn't destroy it.
    pub fn load(state_dir: &Path) -> RecentArgs {
        let path = state_dir.join("args.json");
        let args = match std::fs::read_to_string(&path) {
            Ok(t) => serde_json::from_str(&t).unwrap_or_else(|_| {
                let _ = std::fs::rename(&path, path.with_extension("json.bad"));
                BTreeMap::new()
            }),
            Err(_) => BTreeMap::new(),
        };
        RecentArgs { path, args }
    }

    /// The arguments typed for a command, most recent first.
    pub fn get(&self, workspace: &str, app: &str, cmd: &str) -> &[String] {
        self.args.get(workspace).and_then(|a| a.get(app)).and_then(|c| c.get(cmd)).map_or(&[], Vec::as_slice)
    }

    /// Puts `args` first (trimmed; none is never kept), keeping [`KEPT`], and saves when
    /// that changed something.
    pub fn remember(&mut self, workspace: &str, app: &str, cmd: &str, args: &str) -> Result<(), String> {
        let args = args.trim();
        if args.is_empty() || self.get(workspace, app, cmd).first().is_some_and(|a| a == args) {
            return Ok(());
        }
        let list = self.args.entry(workspace.into()).or_default().entry(app.into()).or_default().entry(cmd.into()).or_default();
        list.retain(|a| a != args);
        list.insert(0, args.to_string());
        list.truncate(KEPT);
        let text = serde_json::to_string_pretty(&self.args).map_err(|e| e.to_string())?;
        crate::state::write_atomic(&self.path, &text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_most_recent_first_and_a_few_only() {
        let dir = crate::testing::TempDir::new("recent-args");
        let mut m = RecentArgs::load(&dir);
        for a in ["--port 1", "--port 2", " --port 1 ", ""] {
            m.remember("ws", "front-end/web", "pnpm:dev", a).unwrap();
        }
        assert_eq!(m.get("ws", "front-end/web", "pnpm:dev"), ["--port 1", "--port 2"], "moved first, trimmed, none not kept");
        for i in 0..20 {
            m.remember("ws", "front-end/web", "pnpm:dev", &format!("-n {i}")).unwrap();
        }
        let back = RecentArgs::load(&dir);
        assert_eq!(back.get("ws", "front-end/web", "pnpm:dev").len(), KEPT);
        assert_eq!(back.get("ws", "front-end/web", "pnpm:dev")[0], "-n 19", "saved, newest first");
        assert_eq!(back.get("ws", "front-end/web", "pnpm:build"), [] as [&str; 0]);
    }

    #[test]
    fn a_broken_file_is_empty_and_kept_aside() {
        let dir = crate::testing::TempDir::new("recent-args-bad");
        std::fs::write(dir.join("args.json"), "{oops").unwrap();
        assert_eq!(RecentArgs::load(&dir).args, BTreeMap::new());
        assert!(dir.join("args.json.bad").exists());
    }
}
