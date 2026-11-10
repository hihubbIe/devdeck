//! Files (U5, U6, L14): locks, opening without following a link, who owns a file, and
//! where an open file really is.

use crate::{SysError, Support};
use std::fs::{File, Metadata, OpenOptions};
use std::path::{Path, PathBuf};

mod unix;

#[cfg_attr(target_os = "linux", path = "linux.rs")]
#[cfg_attr(not(target_os = "linux"), path = "unsupported.rs")]
mod imp;

pub const OPEN_PATH_SUPPORT: Support = imp::OPEN_PATH_SUPPORT;

/// Takes the file's exclusive lock without waiting; an error when another holds it. The
/// lock goes with the file (and with the process).
pub fn try_lock(file: &File) -> std::io::Result<()> {
    unix::try_lock(file)
}

/// Adds "never follow a link at the end of the path" to `options`, and "don't block"
/// (a FIFO) when asked.
pub fn no_follow(options: &mut OpenOptions, nonblocking: bool) -> &mut OpenOptions {
    unix::no_follow(options, nonblocking)
}

/// Whether this process's user owns the file.
pub fn is_mine(meta: &Metadata) -> bool {
    unix::is_mine(meta)
}

/// Whether the error is a loop of links.
pub fn is_loop(e: &std::io::Error) -> bool {
    unix::is_loop(e)
}

/// Where an open file is now, links resolved: the file opened is the one checked.
pub fn open_path(file: &File) -> Result<PathBuf, SysError> {
    imp::open_path(file)
}

/// Makes a FIFO (tests: a file that would block a read).
pub fn make_fifo(path: &Path) -> std::io::Result<()> {
    unix::make_fifo(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    #[test]
    fn a_second_lock_is_refused() {
        let dir = TempDir::new("fs-lock");
        let path = dir.join("l");
        let first = File::create(&path).unwrap();
        try_lock(&first).unwrap();
        let second = File::open(&path).unwrap();
        assert!(try_lock(&second).is_err());
        drop(first);
        try_lock(&second).unwrap();
    }

    #[test]
    fn a_link_is_not_followed() {
        let dir = TempDir::new("fs-nofollow");
        std::fs::write(dir.join("f"), "x").unwrap();
        std::os::unix::fs::symlink(dir.join("f"), dir.join("link")).unwrap();
        assert!(no_follow(OpenOptions::new().read(true), false).open(dir.join("f")).is_ok());
        let e = no_follow(OpenOptions::new().read(true), false).open(dir.join("link")).unwrap_err();
        assert!(is_loop(&e), "{e}");
    }

    #[test]
    fn a_fifo_opens_without_blocking() {
        let dir = TempDir::new("fs-fifo");
        let fifo = dir.join("p");
        make_fifo(&fifo).unwrap();
        let file = no_follow(OpenOptions::new().read(true), true).open(&fifo).unwrap();
        assert!(!file.metadata().unwrap().is_file());
    }

    #[test]
    fn my_files_are_mine() {
        let dir = TempDir::new("fs-mine");
        assert!(is_mine(&std::fs::metadata(dir.path()).unwrap()));
        assert!(!is_mine(&std::fs::metadata("/").unwrap()) || rustix::process::getuid().is_root());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn an_open_file_says_where_it_is() {
        let dir = TempDir::new("fs-path");
        std::fs::write(dir.join("f"), "x").unwrap();
        std::os::unix::fs::symlink(dir.join("f"), dir.join("link")).unwrap();
        let file = File::open(dir.join("link")).unwrap();
        assert_eq!(open_path(&file).unwrap(), dir.join("f").canonicalize().unwrap());
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn an_open_file_path_says_why_there_is_none() {
        let dir = TempDir::new("fs-path");
        std::fs::write(dir.join("f"), "x").unwrap();
        assert!(matches!(open_path(&File::open(dir.join("f")).unwrap()), Err(SysError::Unsupported(_))));
    }
}
