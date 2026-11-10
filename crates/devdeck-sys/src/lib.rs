//! The only code that touches the OS (R11.6, `docs/PLATFORMS.md` §4). Every other crate
//! asks it in devdeck's terms ("the socket's folder", "this process's memory"), never
//! for `/proc` or a syscall. Split by capability; each module picks one backend from the
//! target being built (`linux.rs`, `macos.rs`, code both share, or `unsupported.rs`).
//! A function that can't work returns [`SysError::Unsupported`], never an empty answer
//! that reads as "nothing found".

pub mod dirs;
pub mod fs;
pub mod memory;
pub mod net;
pub mod signals;
pub mod time;

#[cfg(test)]
mod scan;
#[cfg(test)]
mod testing;

use std::fmt;

/// Why an OS call gave no answer.
#[derive(Debug)]
pub enum SysError {
    /// This OS can't do it (yet), and why, for the user.
    Unsupported(&'static str),
    Io(std::io::Error),
}

impl fmt::Display for SysError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SysError::Unsupported(why) => f.write_str(why),
            SysError::Io(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for SysError {}

impl From<std::io::Error> for SysError {
    fn from(e: std::io::Error) -> Self {
        SysError::Io(e)
    }
}

/// What devdeck needs from the OS, asked through [`support`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Capability {
    /// A folder for the control API's socket (L6).
    Runtime,
    /// devdeck's own memory (L8).
    Memory,
    /// The path of an open file (L14).
    OpenPath,
}

/// Whether this OS does a capability: an action that needs one it doesn't is shown
/// disabled with the reason (R5.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Support {
    Yes,
    Partial(&'static str),
    No(&'static str),
}

/// Fixed by the target for the capabilities so far; those only known at run time come
/// with their steps, computed once and kept.
pub fn support(capability: Capability) -> Support {
    match capability {
        Capability::Runtime => dirs::SUPPORT,
        Capability::Memory => memory::SUPPORT,
        Capability::OpenPath => fs::OPEN_PATH_SUPPORT,
    }
}
