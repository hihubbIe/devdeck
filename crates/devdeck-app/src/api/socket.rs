//! The API's socket (R13.3): `$XDG_RUNTIME_DIR/devdeck/<name>.sock`, the folder 0700 and
//! ours, the socket 0600. A name is guarded by its own lock (`<name>.lock`), so a second
//! devdeck never takes over a socket a first one serves; held by us, a stale socket from
//! a devdeck that's gone is removed.

use rustix::fs::{FlockOperation, flock};
use std::fs::{DirBuilder, File, OpenOptions};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};

/// The longest socket path (`sun_path` holds 108 bytes, its end included).
const MAX_PATH: usize = 107;

/// The socket's name for a state folder: `main` for the default one, `demo` for the
/// demo's, else one made from the folder (measure runs, a second devdeck), and which of
/// the three it is.
pub fn name(state_dir: &Path, default_state_dir: &Path) -> (String, &'static str) {
    if state_dir == default_state_dir {
        ("main".into(), "main")
    } else if state_dir == default_state_dir.join("demo") {
        ("demo".into(), "demo")
    } else {
        // FNV-1a: stable across runs and Rust versions.
        let hash = state_dir.as_os_str().as_encoded_bytes().iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x100_0000_01b3));
        (format!("s-{hash:016x}"), "other")
    }
}

/// The socket, bound, and the lock that keeps it ours (dropped with it).
pub struct Bound {
    pub listener: UnixListener,
    pub path: PathBuf,
    _lock: File,
}

impl Bound {
    /// Removes the socket and its lock file (devdeck quitting): nothing is left in the
    /// runtime folder, whatever the state folder's name.
    pub fn remove(self) {
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_file(self.path.with_extension("lock"));
    }
}

/// Makes or checks the folder, takes the name's lock, binds the socket.
pub fn bind(runtime: &Path, name: &str) -> Result<Bound, String> {
    let dir = runtime.join("devdeck");
    folder(&dir)?;
    let path = dir.join(format!("{name}.sock"));
    if path.as_os_str().len() > MAX_PATH {
        return Err(format!("{}: too long for a socket", path.display()));
    }
    let lock = OpenOptions::new().create(true).truncate(false).write(true).mode(0o600).custom_flags(libc::O_NOFOLLOW).open(dir.join(format!("{name}.lock"))).map_err(|e| format!("{}: {e}", dir.display()))?;
    if flock(&lock, FlockOperation::NonBlockingLockExclusive).is_err() {
        return Err(format!("another devdeck serves {}", path.display()));
    }
    match std::fs::remove_file(&path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(format!("{}: {e}", path.display())),
        _ => {}
    }
    let listener = UnixListener::bind(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(Bound { listener, path, _lock: lock })
}

/// The folder: made 0700 if missing; refused if it's a link, not a folder, someone
/// else's, or open to others.
fn folder(dir: &Path) -> Result<(), String> {
    match std::fs::symlink_metadata(dir) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            DirBuilder::new().mode(0o700).create(dir).map_err(|e| format!("{}: {e}", dir.display()))
        }
        Err(e) => Err(format!("{}: {e}", dir.display())),
        Ok(m) if !m.is_dir() => Err(format!("{}: not a folder", dir.display())),
        Ok(m) if m.uid() != rustix::process::getuid().as_raw() => Err(format!("{}: not yours", dir.display())),
        Ok(m) if m.mode() & 0o777 != 0o700 => Err(format!("{}: open to others (mode {:o}, wants 700)", dir.display(), m.mode() & 0o777)),
        Ok(_) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use devdeck_core::testing::TempDir;

    #[test]
    fn names_follow_the_state_folder() {
        let def = Path::new("/home/u/.local/state/devdeck");
        assert_eq!(name(def, def), ("main".into(), "main"));
        assert_eq!(name(&def.join("demo"), def), ("demo".into(), "demo"));
        let (n, kind) = name(Path::new("/tmp/measure"), def);
        assert_eq!(kind, "other");
        assert_eq!(n, name(Path::new("/tmp/measure"), def).0, "stable");
        assert_ne!(n, name(Path::new("/tmp/other"), def).0);
    }

    #[test]
    fn a_folder_open_to_others_or_a_link_is_refused() {
        let rt = TempDir::new("api-sock");
        let dir = rt.join("devdeck");
        DirBuilder::new().mode(0o755).create(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(bind(&rt, "t").err().unwrap().contains("open to others"));
        std::fs::remove_dir(&dir).unwrap();
        let elsewhere = rt.join("elsewhere");
        DirBuilder::new().mode(0o700).create(&elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, &dir).unwrap();
        assert!(bind(&rt, "t").err().unwrap().contains("not a folder"));
    }

    #[test]
    fn a_name_is_served_once_and_a_stale_socket_goes() {
        let rt = TempDir::new("api-sock");
        let first = bind(&rt, "t").unwrap();
        let mode = std::fs::metadata(rt.join("devdeck")).unwrap().mode() & 0o777;
        assert_eq!(mode, 0o700);
        assert_eq!(std::fs::metadata(&first.path).unwrap().mode() & 0o777, 0o600);
        assert!(bind(&rt, "t").err().unwrap().contains("another devdeck"), "the lock is held");
        let path = first.path.clone();
        drop(first);
        assert!(path.exists(), "a socket left behind");
        let again = bind(&rt, "t").expect("taken over once the lock is free");
        assert!(std::os::unix::net::UnixStream::connect(&again.path).is_ok());
        again.remove();
        assert_eq!(std::fs::read_dir(rt.join("devdeck")).unwrap().count(), 0, "nothing left");
    }
}
