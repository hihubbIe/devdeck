//! A file some view shows (R12.1), followed on disk: the app's watcher watches the
//! closest folder of it that exists (its own, unless it was removed) and tells it when
//! something there changes. One per path and workspace, shared by its views, dropped
//! with the last one ([`crate::services::Services::file`]).

use gpui::{Context, EventEmitter};
use std::path::{Path, PathBuf};

pub struct OpenFile {
    pub path: PathBuf,
    /// Counts the changes seen on disk; a view reads again when it differs from its own.
    pub generation: u64,
    /// The folder watched for it, as last told to the watcher.
    watched: Option<PathBuf>,
}

/// The folder to watch changed (the file's folder went away or came back).
pub struct Moved;

impl EventEmitter<Moved> for OpenFile {}

impl OpenFile {
    pub fn new(path: PathBuf) -> Self {
        let mut f = Self { path, generation: 0, watched: None };
        f.watched = f.watch();
        f
    }

    /// The folder to watch: the file's, or its closest ancestor that exists.
    pub fn watch(&self) -> Option<PathBuf> {
        self.path.ancestors().skip(1).find(|d| d.is_dir()).map(Path::to_path_buf)
    }

    /// Whether a change at `p` can change the file: the file, or a folder on its way.
    pub fn concerns(&self, p: &Path) -> bool {
        self.path == p || self.path.starts_with(p)
    }

    /// Something at a path it [`concerns`](Self::concerns) changed. The watcher hears of
    /// it only when the folder to watch changed (not for every line a log gets).
    pub fn changed(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        let watch = self.watch();
        if watch != self.watched {
            self.watched = watch;
            cx.emit(Moved);
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watches_the_closest_folder_and_concerns_its_way() {
        let dir = devdeck_core::testing::TempDir::new("open-file");
        let f = OpenFile::new(dir.join("a/b/notes.md"));
        assert_eq!(f.watch().as_deref(), Some(dir.path()), "a and b don't exist yet");
        std::fs::create_dir_all(dir.join("a/b")).unwrap();
        assert_eq!(f.watch(), Some(dir.join("a/b")));
        assert!(f.concerns(&dir.join("a/b/notes.md")));
        assert!(f.concerns(&dir.join("a")), "its folder removed or renamed");
        assert!(!f.concerns(&dir.join("a/b/other.md")));
    }
}
