//! Reading a project folder, shared by every toolchain: the folder's entries, what a
//! toolchain found in it, and the helpers its manifests need (shell quoting, YAML).

use super::Facts;
use crate::apps::{Command, Tier};
use serde::de::DeserializeOwned;
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

/// What one toolchain found in a folder.
pub(crate) struct Part {
    /// `go`, `pnpm`, `task`…: the app's kind when this is its first part (the order of
    /// [`ALL`](super::ALL)).
    pub kind: &'static str,
    pub commands: Vec<Command>,
    /// The shell text its commands run, for other toolchains' [`covered`](super::Toolchain::covered).
    pub runs: Vec<String>,
    /// For a package (package.json, Go module, Cargo): whether it has something to
    /// run. `None` when the toolchain doesn't say (or its manifest is broken).
    pub runnable: Option<bool>,
    /// What it keeps about the app: its own field of [`Facts`].
    pub facts: Facts,
    /// For a monorepo root: globs of its packages' folders, relative to it.
    pub packages: Vec<String>,
}

impl Part {
    pub fn new(kind: &'static str) -> Part {
        Part { kind, commands: Vec::new(), runs: Vec::new(), runnable: None, facts: Facts::default(), packages: Vec::new() }
    }

    /// Adds `source:name`, tiered among `siblings`.
    pub fn add(&mut self, source: &str, name: &str, line: String, siblings: &[&str], tools: &[&str]) {
        self.commands.push(Command {
            id: format!("{source}:{name}"),
            line,
            tier: Tier::of(name, siblings),
            tools: tools.iter().map(|t| t.to_string()).collect(),
            from_install: Vec::new(),
            setup: false,
            ports: Vec::new(),
        });
    }
}

/// A folder's entries, read once so toolchains only look names up.
pub(crate) struct Listing(BTreeMap<String, Entry>);

enum Entry {
    File,
    Dir,
    /// A link that can't be followed (a loop…), with the reason.
    Broken(String),
}

impl Listing {
    pub fn read(dir: &Path) -> io::Result<Listing> {
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

    pub fn file(&self, name: &str) -> bool {
        matches!(self.0.get(name), Some(Entry::File))
    }

    pub fn dir(&self, name: &str) -> bool {
        matches!(self.0.get(name), Some(Entry::Dir))
    }

    /// File names, alphabetical.
    pub fn files(&self) -> impl Iterator<Item = &str> {
        self.0.iter().filter(|(_, e)| matches!(e, Entry::File)).map(|(n, _)| n.as_str())
    }

    /// Child folders that can be projects or packages: not hidden, not one a toolchain
    /// skips (`node_modules`).
    pub fn folders(&self) -> impl Iterator<Item = &str> {
        self.0.iter().filter(|(n, e)| matches!(e, Entry::Dir) && !skipped(n)).map(|(n, _)| n.as_str())
    }

    /// Entries that can't be followed, except skipped ones.
    pub fn broken(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0.iter().filter(|(n, _)| !skipped(n)).filter_map(|(n, e)| match e {
            Entry::Broken(why) => Some((n.as_str(), why.as_str())),
            _ => None,
        })
    }
}

/// Folders never looked into: hidden ones, and those a toolchain skips.
pub(crate) fn skipped(name: &str) -> bool {
    name.starts_with('.') || super::ALL.iter().any(|t| t.skips(name))
}

/// An IO error as shown.
pub(crate) fn reason(e: &io::Error) -> String {
    if rustix::io::Errno::from_io_error(e) == Some(rustix::io::Errno::LOOP) {
        "symlink loop".into()
    } else {
        e.to_string()
    }
}

/// `s` as one shell word: unchanged when safe, else single-quoted.
pub(crate) fn quote(s: &str) -> String {
    let safe = !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "_-./:=@%+,".contains(c));
    if safe { s.to_string() } else { format!("'{}'", s.replace('\'', r"'\''")) }
}

/// A YAML document; empty gives the default. Duplicate keys are tolerated (the last
/// wins), as the tools reading these files do.
pub(crate) fn yaml<T: DeserializeOwned + Default>(text: &str) -> Result<T, String> {
    let options = serde_saphyr::options! { duplicate_keys: serde_saphyr::DuplicateKeyPolicy::LastWins };
    match serde_saphyr::from_str_with_options::<Option<T>>(text, options) {
        Ok(v) => Ok(v.unwrap_or_default()),
        Err(e) => Err(e.to_string().lines().next().unwrap_or_default().trim_start_matches("error: ").to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_words_are_quoted_when_needed() {
        assert_eq!(quote("start:dev"), "start:dev");
        assert_eq!(quote("./cmd/api"), "./cmd/api");
        assert_eq!(quote("a b"), "'a b'");
        assert_eq!(quote("it's"), r"'it'\''s'");
        assert_eq!(quote(""), "''");
    }

    #[test]
    fn empty_yaml_gives_defaults() {
        #[derive(Debug, Default, PartialEq, serde::Deserialize)]
        struct Doc {
            #[serde(default)]
            packages: Vec<String>,
        }
        assert_eq!(yaml::<Doc>(""), Ok(Doc::default()));
        assert_eq!(yaml::<Doc>("# only a comment\n"), Ok(Doc::default()));
        assert_eq!(yaml::<Doc>("packages: [a]\npackages: [b]\n"), Ok(Doc { packages: vec!["b".into()] }), "duplicates: the last wins");
    }
}
