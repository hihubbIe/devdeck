//! Which Node an app runs with (R6.8). The installs come from the machine config's
//! `node_dirs` (nvm's folder without it), newest first. An app's `.nvmrc` (searched from
//! its folder up to its workspace folder) names a version (`20`, `v20.11.1`, `lts/iron`,
//! using `~/.nvm/alias` for aliases); without one, its `engines.node` range keeps the
//! default Node (the one first on the PATH) when it fits, else takes the newest install
//! that does.

use super::Facts;
use super::semver::satisfies;
use crate::environment::which;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Install {
    /// `22.23.3`.
    pub version: String,
    /// Its `bin` folder.
    pub bin: PathBuf,
}

/// The Node installs found on this machine.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Machine {
    /// Newest first.
    pub installs: Vec<Install>,
    /// Where nvm keeps aliases (`~/.nvm/alias`), when it exists.
    pub aliases: Option<PathBuf>,
    /// The version of the Node first on the PATH, when it's one of `installs`.
    pub default: Option<String>,
}

impl Machine {
    /// The installs in `node_dirs` (nvm's folder when empty), and which one `path` runs.
    pub fn load(node_dirs: &[PathBuf], home: Option<&Path>, path: &[PathBuf]) -> Machine {
        let aliases = home.map(|h| h.join(".nvm/alias")).filter(|a| a.is_dir());
        let dirs = match node_dirs.is_empty() {
            true => home.map(|h| vec![h.join(".nvm/versions/node")]).unwrap_or_default(),
            false => node_dirs.to_vec(),
        };
        Machine::new(installs(&dirs), aliases, path)
    }

    pub(super) fn new(installs: Vec<Install>, aliases: Option<PathBuf>, path: &[PathBuf]) -> Machine {
        let first = which("node", path).and_then(|n| n.parent().and_then(|b| std::fs::canonicalize(b).ok()));
        let default = first.and_then(|bin| installs.iter().find(|n| std::fs::canonicalize(&n.bin).is_ok_and(|b| b == bin)).map(|n| n.version.clone()));
        Machine { installs, aliases, default }
    }

    /// The install an app runs with, or why there is none (`.nvmrc wants node 20, which
    /// isn't installed`). `Ok(None)`: the default Node (the app asks for no version, or
    /// the default fits its `engines.node`, or its range can't be read, or which one is
    /// the default isn't known and no install fits).
    pub fn choose(&self, facts: &Facts) -> Result<Option<&Install>, String> {
        if let Some(wanted) = facts.nvmrc.as_deref().map(str::trim).filter(|w| !w.is_empty()) {
            return self.resolve(wanted).map(Some).ok_or_else(|| format!(".nvmrc wants node {wanted}, which isn't installed"));
        }
        let Some(range) = facts.engines.as_deref().map(str::trim).filter(|r| !r.is_empty()) else { return Ok(None) };
        // An unreadable range doesn't block: the default Node runs.
        let fits = |v: &str| version_parts(v).and_then(|p| satisfies(&p, range));
        if self.default.as_deref().is_some_and(|d| fits(d) != Some(false)) {
            return Ok(None);
        }
        match self.installs.iter().find(|n| fits(&n.version) == Some(true)) {
            Some(n) => Ok(Some(n)),
            None if self.default.is_none() => Ok(None),
            None => Err(format!("package.json wants node {range}, which isn't installed")),
        }
    }

    /// The newest install `wanted` names (`20`, `v20.11`, `node`), following nvm aliases
    /// (`lts/*` → `lts/iron` → `v20.11.1`) a few levels deep, so a loop ends.
    fn resolve(&self, wanted: &str) -> Option<&Install> {
        let mut wanted = wanted.to_string();
        for _ in 0..8 {
            if wanted == "node" || wanted == "stable" {
                return self.installs.first();
            }
            if let Some(parts) = version_parts(&wanted) {
                return self.installs.iter().find(|n| version_parts(&n.version).is_some_and(|v| v.starts_with(&parts)));
            }
            wanted = self.alias(&wanted)?;
        }
        None
    }

    /// What the nvm alias `name` (`lts/iron`, `lts/*`, `default`) points to: the first
    /// line of its file in the alias folder.
    fn alias(&self, name: &str) -> Option<String> {
        if name.split('/').any(|p| p.is_empty() || p == "." || p == "..") {
            return None;
        }
        let text = std::fs::read_to_string(self.aliases.as_ref()?.join(name)).ok()?;
        text.lines().map(str::trim).find(|l| !l.is_empty()).map(str::to_string)
    }
}

/// The version an `.nvmrc` at `dir` or above (up to `stop`, included) asks for, trimmed
/// (`20`, `v20.11.1`, `lts/iron`).
pub fn nvmrc(dir: &Path, stop: &Path) -> Option<String> {
    let mut at = Some(dir);
    while let Some(d) = at {
        // A file with only comments or blank lines asks for nothing: keep looking up.
        if let Ok(text) = std::fs::read_to_string(d.join(".nvmrc"))
            && let Some(v) = text.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with('#'))
        {
            return Some(v.to_string());
        }
        if d == stop {
            break;
        }
        at = d.parent();
    }
    None
}

/// The Node installs in `dirs` (`v22.23.3/bin/node`), newest first. A version found in
/// two folders keeps the first.
pub(super) fn installs(dirs: &[PathBuf]) -> Vec<Install> {
    let mut found: Vec<(Vec<u64>, Install)> = Vec::new();
    for entry in dirs.iter().filter_map(|d| std::fs::read_dir(d).ok()).flatten().flatten() {
        let name = entry.file_name();
        let Some(version) = name.to_str().and_then(|n| n.strip_prefix('v')) else { continue };
        let Some(parts) = version_parts(version).filter(|p| p.len() == 3) else { continue };
        let bin = entry.path().join("bin");
        if found.iter().any(|(p, _)| *p == parts) || which("node", std::slice::from_ref(&bin)).is_none() {
            continue;
        }
        found.push((parts, Install { version: version.to_string(), bin }));
    }
    found.sort_by(|a, b| b.0.cmp(&a.0));
    found.into_iter().map(|(_, n)| n).collect()
}

/// `20`, `v20.11`, `20.11.1` as numbers (one to three parts); `None` for anything else.
fn version_parts(s: &str) -> Option<Vec<u64>> {
    let s = s.strip_prefix('v').unwrap_or(s);
    let parts: Vec<u64> = s
        .split('.')
        .map(|p| if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) { p.parse().ok() } else { None })
        .collect::<Option<_>>()?;
    (1..=3).contains(&parts.len()).then_some(parts)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An empty folder of its own for one test.
    fn tmp(tag: &str) -> crate::testing::TempDir {
        crate::testing::TempDir::new(tag)
    }

    /// A Node install `v<version>` in `dir`.
    fn node(dir: &Path, version: &str) {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(format!("v{version}/bin/node"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn machine(dirs: &[&Path], path: &[PathBuf], aliases: Option<PathBuf>) -> Machine {
        let dirs: Vec<PathBuf> = dirs.iter().map(|d| d.to_path_buf()).collect();
        Machine::new(installs(&dirs), aliases, path)
    }

    fn wants<'a>(m: &'a Machine, v: &str) -> Result<Option<&'a str>, String> {
        m.choose(&Facts { nvmrc: Some(v.into()), ..Facts::default() }).map(|n| n.map(|n| n.version.as_str()))
    }

    #[test]
    fn nvmrc_is_found_upward_until_the_workspace_folder() {
        let root = tmp("nvmrc");
        let app = root.join("front-end/apps/x");
        std::fs::create_dir_all(&app).unwrap();
        assert_eq!(nvmrc(&app, &root), None);
        std::fs::write(root.join("front-end/.nvmrc"), "# node\n 20 \n").unwrap();
        assert_eq!(nvmrc(&app, &root).as_deref(), Some("20"));
        assert_eq!(nvmrc(&app, &root.join("front-end/apps")), None, "stops at the workspace folder");
        std::fs::write(app.join(".nvmrc"), "# only a comment\n\n").unwrap();
        assert_eq!(nvmrc(&app, &root).as_deref(), Some("20"), "an .nvmrc asking for nothing doesn't stop the search");
    }

    #[test]
    fn installs_are_found_newest_first() {
        let d = tmp("installs");
        let (a, b) = (d.join("a"), d.join("b"));
        for v in ["18.20.4", "20.9.0", "20.11.1", "22.1.0"] {
            node(&a, v);
        }
        std::fs::create_dir_all(a.join("v21.0.0/bin")).unwrap();
        std::fs::create_dir_all(a.join("notes/bin")).unwrap();
        node(&b, "20.11.1");
        node(&b, "16.0.0");
        let m = machine(&[&a, &b, &d.join("absent")], &[], None);
        let versions: Vec<&str> = m.installs.iter().map(|n| n.version.as_str()).collect();
        assert_eq!(versions, ["22.1.0", "20.11.1", "20.9.0", "18.20.4", "16.0.0"], "v21 has no node");
        assert_eq!(m.installs[1].bin, a.join("v20.11.1/bin"), "the first folder wins");
    }

    #[test]
    fn versions_resolve_to_the_newest_match() {
        let d = tmp("resolve");
        for v in ["18.20.4", "20.9.0", "20.11.1", "22.1.0"] {
            node(&d, v);
        }
        let m = machine(&[&d], &[], None);
        assert_eq!(m.choose(&Facts::default()), Ok(None), "asks for nothing");
        assert_eq!(wants(&m, "20"), Ok(Some("20.11.1")));
        assert_eq!(wants(&m, "v20"), Ok(Some("20.11.1")));
        assert_eq!(wants(&m, "v20.9.0"), Ok(Some("20.9.0")));
        assert_eq!(wants(&m, "20.9"), Ok(Some("20.9.0")));
        assert_eq!(wants(&m, " 18 "), Ok(Some("18.20.4")));
        assert_eq!(wants(&m, "node"), Ok(Some("22.1.0")));
        assert_eq!(wants(&m, "stable"), Ok(Some("22.1.0")));
        assert_eq!(wants(&m, "19"), Err(".nvmrc wants node 19, which isn't installed".into()));
        assert!(wants(&m, "2").is_err(), "a prefix of numbers, not of text");
        assert!(wants(&m, "20.9.0.1").is_err());
        assert!(wants(&m, "iojs").is_err(), "no alias folder");
    }

    #[test]
    fn nvm_aliases_lead_to_versions() {
        let d = tmp("aliases");
        let dir = d.join("node");
        for v in ["20.11.1", "22.1.0"] {
            node(&dir, v);
        }
        let alias = d.join("alias");
        std::fs::create_dir_all(alias.join("lts")).unwrap();
        for (name, to) in [("lts/*", "lts/iron\n"), ("lts/iron", "v20.11.1\n"), ("lts/hydrogen", "v18.19.0"), ("default", "22"), ("loop", "loop")] {
            std::fs::write(alias.join(name), to).unwrap();
        }
        let m = machine(&[&dir], &[], Some(alias));
        assert_eq!(wants(&m, "lts/*"), Ok(Some("20.11.1")));
        assert_eq!(wants(&m, "lts/iron"), Ok(Some("20.11.1")));
        assert_eq!(wants(&m, "default"), Ok(Some("22.1.0")));
        assert_eq!(wants(&m, "lts/hydrogen"), Err(".nvmrc wants node lts/hydrogen, which isn't installed".into()));
        assert!(wants(&m, "lts/jod").is_err(), "no alias file");
        assert!(wants(&m, "loop").is_err(), "an alias loop ends");
        assert!(wants(&m, "../alias/default").is_err(), "stays in the alias folder");
        assert!(wants(&machine(&[&dir], &[], None), "lts/iron").is_err(), "no alias folder");
    }

    #[test]
    fn engines_keep_the_default_node_when_it_fits() {
        let d = tmp("engines");
        let dir = d.join("node");
        for v in ["20.20.2", "22.23.3", "26.0.0"] {
            node(&dir, v);
        }
        let m = machine(&[&dir], &[dir.join("v20.20.2/bin")], None);
        assert_eq!(m.default.as_deref(), Some("20.20.2"), "the PATH's node is an install");
        let engines = |r: &str| Facts { engines: Some(r.into()), ..Facts::default() };
        let picked = |f: &Facts| m.choose(f).map(|n| n.map(|n| n.version.clone()));
        assert_eq!(picked(&engines(">=18")), Ok(None), "the default fits");
        assert_eq!(picked(&engines(">=22.0.0")), Ok(Some("26.0.0".into())), "the newest that fits");
        assert_eq!(picked(&engines("^22")), Ok(Some("22.23.3".into())));
        assert_eq!(picked(&engines(">=30")), Err("package.json wants node >=30, which isn't installed".into()));
        assert_eq!(picked(&engines("lts/*")), Ok(None), "an unreadable range lets the default run");
        assert_eq!(picked(&Facts { nvmrc: Some("22".into()), ..engines(">=26") }), Ok(Some("22.23.3".into())), ".nvmrc first");
        let unknown = machine(&[&dir], &[], None);
        assert_eq!(unknown.choose(&engines(">=30")), Ok(None), "the default isn't known: let it try");
    }
}
