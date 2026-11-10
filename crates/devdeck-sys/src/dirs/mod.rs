//! devdeck's folders (L6, L7; R10.1): config, state, what the user writes (notes), and
//! the control API's socket folder. Each takes a lookup of the environment, so tests
//! never touch the real one; `DEVDECK_*_DIR` comes first on every OS.

use crate::{SysError, Support};
use std::path::PathBuf;

// The XDG base folders and their fallbacks under `~`: Linux, and macOS until its own
// folders come (macOS step 2a).
mod xdg;

#[cfg_attr(target_os = "linux", path = "linux.rs")]
#[cfg_attr(not(target_os = "linux"), path = "unsupported.rs")]
mod imp;

pub const SUPPORT: Support = imp::SUPPORT;

/// The longest socket path: `sun_path` holds 108 bytes on Linux, 104 on macOS and the
/// BSDs, its closing NUL included.
#[cfg(target_os = "linux")]
pub const SOCKET_PATH_MAX: usize = 107;
#[cfg(not(target_os = "linux"))]
pub const SOCKET_PATH_MAX: usize = 103;

/// The config folder: `DEVDECK_CONFIG_DIR`, else the OS's.
pub fn config(env: impl Fn(&str) -> Option<String>) -> PathBuf {
    set(&env, "DEVDECK_CONFIG_DIR").unwrap_or_else(|| xdg::config(&env))
}

/// The state folder (pins, layouts, history): `DEVDECK_STATE_DIR`, else the OS's.
pub fn state(env: impl Fn(&str) -> Option<String>) -> PathBuf {
    set(&env, "DEVDECK_STATE_DIR").unwrap_or_else(|| xdg::state(&env))
}

/// What the user writes in devdeck (notes): `DEVDECK_DATA_DIR`, else the OS's.
pub fn data(env: impl Fn(&str) -> Option<String>) -> PathBuf {
    set(&env, "DEVDECK_DATA_DIR").unwrap_or_else(|| xdg::data(&env))
}

/// The folder of the control API's sockets, `devdeck` included, or why there is none.
pub fn runtime(env: impl Fn(&str) -> Option<String>) -> Result<PathBuf, SysError> {
    imp::runtime(&env)
}

/// [`runtime`] of this process's environment.
pub fn default_runtime() -> Result<PathBuf, SysError> {
    runtime(real)
}

/// A variable of this process's environment, for the lookups above.
pub fn real(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

fn set(env: &impl Fn(&str) -> Option<String>, name: &str) -> Option<PathBuf> {
    env(name).filter(|s| !s.is_empty()).map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |k| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())
    }

    #[test]
    fn config_precedence() {
        assert_eq!(config(env(&[("DEVDECK_CONFIG_DIR", "/a"), ("XDG_CONFIG_HOME", "/b"), ("HOME", "/h")])), PathBuf::from("/a"));
        assert_eq!(config(env(&[("XDG_CONFIG_HOME", "/b"), ("HOME", "/h")])), PathBuf::from("/b/devdeck"));
        assert_eq!(config(env(&[("XDG_CONFIG_HOME", ""), ("HOME", "/h")])), PathBuf::from("/h/.config/devdeck"));
    }

    #[test]
    fn state_precedence() {
        assert_eq!(state(env(&[("DEVDECK_STATE_DIR", "/s"), ("XDG_STATE_HOME", "/x")])), PathBuf::from("/s"));
        assert_eq!(state(env(&[("XDG_STATE_HOME", "/x"), ("HOME", "/h")])), PathBuf::from("/x/devdeck"));
        assert_eq!(state(env(&[("HOME", "/h")])), PathBuf::from("/h/.local/state/devdeck"));
    }

    #[test]
    fn data_precedence() {
        assert_eq!(data(env(&[("DEVDECK_DATA_DIR", "/d"), ("XDG_DATA_HOME", "/x")])), PathBuf::from("/d"));
        assert_eq!(data(env(&[("XDG_DATA_HOME", "/x"), ("HOME", "/h")])), PathBuf::from("/x/devdeck"));
        assert_eq!(data(env(&[("HOME", "/h")])), PathBuf::from("/h/.local/share/devdeck"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn runtime_is_the_session_runtime_folder() {
        assert_eq!(runtime(env(&[("XDG_RUNTIME_DIR", "/r")])).unwrap(), PathBuf::from("/r/devdeck"));
        let why = |e: &[(&str, &str)]| runtime(env(e)).unwrap_err().to_string();
        assert!(why(&[]).contains("XDG_RUNTIME_DIR"), "{}", why(&[]));
        assert!(why(&[("XDG_RUNTIME_DIR", "")]).contains("XDG_RUNTIME_DIR"), "an empty one counts as unset");
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn runtime_says_why_there_is_none() {
        assert!(matches!(runtime(env(&[("XDG_RUNTIME_DIR", "/r")])), Err(SysError::Unsupported(_))));
    }
}
