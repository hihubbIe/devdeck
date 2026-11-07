//! Temporary folders for tests, so no test leaves anything behind. Each folder sits in
//! one folder per test process (`devdeck-tests/<pid>/`) and is removed when its
//! [`TempDir`] is dropped. Folders of test processes that ended without cleaning up (an
//! abort, a killed run) are removed by the next test process.

use std::path::{Path, PathBuf};
use std::sync::Once;
use std::sync::atomic::{AtomicU64, Ordering};

/// An empty folder for one test, removed when dropped.
#[derive(Debug)]
pub struct TempDir(PathBuf);

impl TempDir {
    /// A new empty folder; `tag` only names it.
    pub fn new(tag: &str) -> TempDir {
        static SWEPT: Once = Once::new();
        static N: AtomicU64 = AtomicU64::new(0);
        SWEPT.call_once(sweep);
        let tag: String = tag.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
        let dir = base().join(std::process::id().to_string()).join(format!("{tag}-{}", N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
        TempDir(dir)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl std::ops::Deref for TempDir {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for TempDir {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
        // The process's folder goes with its last test folder (fails while others remain).
        if let Some(parent) = self.0.parent() {
            let _ = std::fs::remove_dir(parent);
        }
    }
}

fn base() -> PathBuf {
    std::env::temp_dir().join("devdeck-tests")
}

/// Removes the folders of test processes that no longer run.
fn sweep() {
    let Ok(entries) = std::fs::read_dir(base()) else { return };
    for e in entries.flatten() {
        let gone = e.file_name().to_str().and_then(|n| n.parse::<u32>().ok()).is_some_and(|pid| !Path::new(&format!("/proc/{pid}")).exists());
        if gone {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

/// A folder tree for discovery tests, removed when dropped: files are written with their
/// folders, then the tree is scanned as one workspace folder.
#[cfg(test)]
pub(crate) struct Tree(pub TempDir);

#[cfg(test)]
impl Tree {
    pub fn new(name: &str) -> Tree {
        Tree(TempDir::new(name))
    }

    pub fn file(&self, rel: &str, text: &str) -> &Tree {
        let path = self.0.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
        self
    }

    pub fn dir(&self, rel: &str) -> &Tree {
        std::fs::create_dir_all(self.0.join(rel)).unwrap();
        self
    }

    pub fn path(&self, rel: &str) -> PathBuf {
        self.0.join(rel)
    }

    pub fn scan(&self) -> crate::discovery::Found {
        crate::discovery::scan(&[self.0.to_path_buf()], None)
    }
}

/// The app `id` a scan found.
#[cfg(test)]
pub(crate) fn app<'a>(found: &'a crate::discovery::Found, id: &str) -> &'a crate::apps::App {
    found.apps.iter().find(|a| a.id == id).unwrap_or_else(|| panic!("no app {id} in {:?}", ids(found)))
}

/// The ids of the apps a scan found.
#[cfg(test)]
pub(crate) fn ids(found: &crate::discovery::Found) -> Vec<&str> {
    found.apps.iter().map(|a| a.id.as_str()).collect()
}

/// `id → line` of every command, in order.
#[cfg(test)]
pub(crate) fn lines(app: &crate::apps::App) -> Vec<(&str, &str)> {
    app.commands.iter().map(|c| (c.id.as_str(), c.line.as_str())).collect()
}

#[cfg(test)]
pub(crate) fn tier(app: &crate::apps::App, id: &str) -> crate::apps::Tier {
    app.command(id).unwrap_or_else(|| panic!("no command {id} in {:?}", lines(app))).tier
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_is_removed_when_dropped() {
        let d = TempDir::new("drop");
        std::fs::write(d.join("f"), "x").unwrap();
        let path = d.to_path_buf();
        assert!(path.is_dir());
        drop(d);
        assert!(!path.exists());
    }

    #[test]
    fn folders_of_ended_processes_are_swept() {
        // A pid above the kernel's maximum never runs.
        let stale = base().join("4194305");
        std::fs::create_dir_all(stale.join("x")).unwrap();
        sweep();
        assert!(!stale.exists());
    }
}
