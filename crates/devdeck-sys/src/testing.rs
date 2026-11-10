//! Temporary folders for this crate's tests, in the same per-process folder as
//! `devdeck_core::testing` (`devdeck-tests/<pid>/`, which core's tests sweep when a test
//! process ended without cleaning up), removed when dropped.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(tag: &str) -> TempDir {
        static N: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join("devdeck-tests").join(std::process::id().to_string()).join(format!("sys-{tag}-{}", N.fetch_add(1, Ordering::Relaxed)));
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

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
        if let Some(parent) = self.0.parent() {
            let _ = std::fs::remove_dir(parent);
        }
    }
}
