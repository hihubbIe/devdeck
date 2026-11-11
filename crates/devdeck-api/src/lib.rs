//! devdeck's control API (R13): everything devdeck shows and does, by ids, on a local
//! socket. This crate is the protocol only (messages, the method catalog, the types the
//! methods take and give, a client); it knows nothing of the app, and nothing of agents:
//! the CLI and the MCP server are clients like any other.
//!
//! The protocol only grows (R13.2): methods and fields are added, never removed or
//! changed. So every enum a client reads has an `Unknown` fallback, results never refuse
//! fields they don't know, and parameters do (a typo answers `invalid`).

pub mod client;
pub mod methods;
pub mod rpc;
pub mod types;

pub use client::{Client, ClientError, default_socket};
pub use methods::{Method, MethodInfo, Visit, catalog, visit_all};
pub use rpc::{ApiError, ErrorKind};
pub use types::{Caller, Class, Policy, Untrusted, View};

/// The protocol's level; a method says from which level it exists. 2: run.start,
/// run.stop, run.restart; a run says who started it. 3: reading output, the screen,
/// notes and files. 5: `editmode.get` and the `edit_mode` topic removed, with edit mode
/// itself (R13.2's exception: a dropped feature takes its methods and topics with it); a
/// subscription naming it is refused, as for any unknown topic. 6: layouts are pulses
/// (the user's name for them): `layout.*` became `pulse.*`, the `layout` topic `pulse`,
/// every `layout` field `pulse`, and the `layouts` list `pulses` (R13.2).
pub const API_LEVEL: u32 = 6;

/// Set in a devdeck shell: the socket of the devdeck that started it.
pub const SOCKET_ENV: &str = "DEVDECK_SOCKET";
/// Set in a devdeck shell: `<workspace>/<n>`, the terminal it runs in. Grants nothing:
/// devdeck finds its callers from their processes.
pub const TERMINAL_ENV: &str = "DEVDECK_TERMINAL";

/// The longest message either side reads, in bytes, its end of line included.
pub const MAX_MESSAGE: usize = 1 << 20;

#[cfg(test)]
pub(crate) mod testing {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A folder removed with everything in it when dropped (short: sockets' paths are).
    pub struct Dir(pub PathBuf);

    impl Dir {
        pub fn new() -> Self {
            static N: AtomicU32 = AtomicU32::new(0);
            let p = std::env::temp_dir().join(format!("dda-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
