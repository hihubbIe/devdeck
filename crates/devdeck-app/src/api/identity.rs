//! Who connects (R13.4), found from processes, never from what the caller says.
//!
//! The OS gives the peer's user, checked, and pins the very process that connected
//! (`devdeck_sys::peer`): a pid used again can't pass for it. While it is pinned, the
//! chain of its ancestors is read from the process table, each started no later than
//! its child (otherwise the pid in between was used again, and the chain stops there).
//! The UI thread then matches the chain against the shells devdeck runs
//! ([`super::dispatch`]). Anything failing here makes an outside caller, never a
//! terminal.

use std::os::unix::net::UnixStream;

/// Ancestors read at most (a deeper chain is cut: its top can't be a devdeck shell).
const MAX_CHAIN: usize = 64;

/// A connected process, as far as it could be told.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Peer {
    /// None: it couldn't be pinned (it's then outside, whatever its ancestors).
    pub pid: Option<u32>,
    /// The process, then its parent, and so on, each with its start time.
    pub chain: Vec<(u32, u64)>,
    /// Its name, untrusted.
    pub program: String,
}

impl Peer {
    fn unpinned() -> Self {
        Self { pid: None, chain: Vec::new(), program: "?".into() }
    }
}

/// The peer of `stream`; an error when it's another user (the connection is refused).
pub fn peer(stream: &UnixStream) -> Result<Peer, String> {
    let Some(pinned) = devdeck_sys::peer::of(stream).map_err(|e| e.to_string())? else { return Ok(Peer::unpinned()) };
    let pid = pinned.pid;
    let chain = chain(pid, devdeck_sys::procs::parent_and_start);
    let program = devdeck_sys::procs::name(pid).unwrap_or_else(|| "?".into());
    // Read while it lived (still pinned): what was read is its own.
    if pinned.ended() || chain.is_empty() {
        return Ok(Peer::unpinned());
    }
    Ok(Peer { pid: Some(pid), chain, program })
}

/// `pid` and its ancestors with their start times, from `stat` (`(parent, start)`),
/// while each parent started no later than its child.
pub fn chain(pid: u32, stat: impl Fn(u32) -> Option<(u32, u64)>) -> Vec<(u32, u64)> {
    let mut out = Vec::new();
    let Some((mut parent, mut start)) = stat(pid) else { return out };
    out.push((pid, start));
    while parent > 1 && out.len() < MAX_CHAIN {
        let Some((grand, parent_start)) = stat(parent) else { break };
        if parent_start > start {
            break;
        }
        out.push((parent, parent_start));
        (parent, start) = (grand, parent_start);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn procs(list: &[(u32, u32, u64)]) -> impl Fn(u32) -> Option<(u32, u64)> {
        let m: HashMap<u32, (u32, u64)> = list.iter().map(|(p, pp, s)| (*p, (*pp, *s))).collect();
        move |p| m.get(&p).copied()
    }

    #[test]
    fn a_chain_goes_up_to_init() {
        let stat = procs(&[(40, 30, 400), (30, 20, 300), (20, 1, 200)]);
        assert_eq!(chain(40, stat), vec![(40, 400), (30, 300), (20, 200)]);
    }

    #[test]
    fn a_chain_stops_at_a_pid_used_again() {
        // 30 died and a later process took its number: it started after 40.
        let stat = procs(&[(40, 30, 400), (30, 20, 500), (20, 1, 200)]);
        assert_eq!(chain(40, stat), vec![(40, 400)]);
        assert_eq!(chain(99, procs(&[])), Vec::<(u32, u64)>::new(), "a process gone");
    }

    /// Linux only until macOS step 2e: a Mac doesn't pin its callers yet.
    #[cfg(target_os = "linux")]
    #[test]
    fn this_process_is_pinned_through_a_real_socket() {
        let (a, b) = UnixStream::pair().unwrap();
        let p = peer(&a).unwrap();
        assert_eq!(p.pid, Some(std::process::id()));
        assert_eq!(p.chain.first().map(|c| c.0), Some(std::process::id()));
        assert!(p.chain.len() >= 2);
        drop(b);
    }
}
