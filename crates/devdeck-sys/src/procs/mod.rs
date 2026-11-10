//! The process table (L3): parents and children, so a run's whole tree is found
//! (processes that left its process group included) and its sockets attributed; names
//! and command lines to show; whether a process recorded earlier still runs.

use crate::{SysError, Support};
use std::collections::{BTreeMap, BTreeSet};

mod unix;

#[cfg_attr(target_os = "linux", path = "linux.rs")]
#[cfg_attr(not(target_os = "linux"), path = "unsupported.rs")]
mod imp;

pub const SUPPORT: Support = imp::SUPPORT;

/// One process, as the table has it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Proc {
    pub parent: u32,
    pub session: u32,
    /// When it started, in the OS's units: a later process has a larger number, so a pid
    /// used again is told apart and a parent started no later than its child. Linux:
    /// clock ticks since boot.
    pub start: u64,
    /// It ended and waits to be reaped (a zombie), or is being torn down.
    pub ended: bool,
}

/// `pid` in the table; None when it isn't there or can't be read (another user's, or
/// this OS can't say: [`SUPPORT`]).
pub fn get(pid: u32) -> Option<Proc> {
    imp::get(pid)
}

/// `(pid, parent pid)` of every process that can be read.
pub fn parents() -> Result<Vec<(u32, u32)>, SysError> {
    imp::parents()
}

/// The parent pid of `pid`.
pub fn parent_of(pid: u32) -> Option<u32> {
    get(pid).map(|p| p.parent)
}

/// `(parent pid, start)` of `pid`.
pub fn parent_and_start(pid: u32) -> Option<(u32, u64)> {
    get(pid).map(|p| (p.parent, p.start))
}

/// The process's name (what it set; Linux keeps 15 bytes).
pub fn name(pid: u32) -> Option<String> {
    imp::name(pid)
}

/// Its command line, the arguments joined by spaces; None when it can't be read or is
/// empty (a kernel thread, a zombie).
pub fn command(pid: u32) -> Option<String> {
    imp::command(pid)
}

/// Whether `pid` still runs and is the process that started at `start`.
pub fn alive(pid: u32, start: u64) -> bool {
    get(pid).is_some_and(|p| p.start == start && !p.ended)
}

/// Whether any process has this pid, a zombie included.
pub fn exists(pid: u32) -> bool {
    unix::exists(pid)
}

/// `root` and every process below it, in `pairs` (`(pid, parent)`).
pub fn tree(root: u32, pairs: &[(u32, u32)]) -> BTreeSet<u32> {
    let mut children: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for (pid, parent) in pairs {
        children.entry(*parent).or_default().push(*pid);
    }
    let mut out = BTreeSet::from([root]);
    let mut todo = vec![root];
    while let Some(p) = todo.pop() {
        for c in children.get(&p).into_iter().flatten() {
            if out.insert(*c) {
                todo.push(*c);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tree_follows_children_down() {
        let pairs = [(2, 1), (3, 2), (4, 3), (5, 1), (6, 4)];
        assert_eq!(tree(2, &pairs), BTreeSet::from([2, 3, 4, 6]));
        assert_eq!(tree(5, &pairs), BTreeSet::from([5]));
    }

    #[test]
    fn this_process_exists_and_a_pid_past_the_maximum_doesnt() {
        assert!(exists(std::process::id()));
        assert!(!exists(i32::MAX as u32));
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn says_why_there_is_no_table() {
        assert!(matches!(parents(), Err(SysError::Unsupported(_))));
        assert_eq!(get(std::process::id()), None);
    }
}

/// Against real processes, on an OS with a backend (macOS from step 2b).
#[cfg(all(test, target_os = "linux"))]
mod conformance {
    use super::*;
    use std::process::{Child, Command};
    use std::time::{Duration, Instant};

    fn wait_until(what: &str, f: impl Fn() -> bool) {
        let end = Instant::now() + Duration::from_secs(3);
        while !f() {
            assert!(Instant::now() < end, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// `sleep 5`, killed when dropped.
    struct Sleep(Child);

    impl Sleep {
        fn start() -> Sleep {
            Sleep(Command::new("sleep").arg("5").spawn().expect("sleep runs"))
        }
    }

    impl Drop for Sleep {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    #[test]
    fn this_process_has_a_parent_started_before_it() {
        let me = get(std::process::id()).expect("readable");
        let parent = get(me.parent).expect("the parent");
        assert!(parent.start <= me.start, "a parent started first");
        assert_eq!(parent_and_start(std::process::id()), Some((me.parent, me.start)));
        assert!(!me.ended);
    }

    #[test]
    fn a_child_is_in_the_tree_with_its_name_and_command() {
        let child = Sleep::start();
        let pid = child.0.id();
        let me = std::process::id();
        assert!(tree(me, &parents().unwrap()).contains(&pid));
        assert_eq!(parent_of(pid), Some(me));
        assert_eq!(name(pid).as_deref(), Some("sleep"));
        assert_eq!(command(pid).as_deref(), Some("sleep 5"));
        let start = get(pid).unwrap().start;
        assert!(alive(pid, start));
        assert!(!alive(pid, start + 1), "another process with the same pid");
    }

    #[test]
    fn an_ended_process_exists_until_reaped() {
        let mut child = Command::new("sleep").arg("5").spawn().unwrap();
        let pid = child.id();
        let start = get(pid).unwrap().start;
        child.kill().unwrap();
        wait_until("the zombie", || get(pid).is_some_and(|p| p.ended));
        assert!(exists(pid), "a zombie is still in the table");
        assert!(!alive(pid, start));
        assert_eq!(command(pid), None, "a zombie has no command line");
        child.wait().unwrap();
        assert!(!exists(pid));
        assert_eq!(get(pid), None);
    }
}
