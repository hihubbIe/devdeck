//! devdeck's own memory, for its stats and measurements (L8, R11.4).

use crate::{SysError, Support};

#[cfg_attr(target_os = "linux", path = "linux.rs")]
#[cfg_attr(not(target_os = "linux"), path = "unsupported.rs")]
mod imp;

pub const SUPPORT: Support = imp::SUPPORT;

/// This process's memory in kB: resident, proportional, and its own (private) pages.
#[derive(Debug, Default, Clone, Copy)]
pub struct Memory {
    pub rss: u64,
    pub pss: u64,
    pub uss: u64,
}

pub fn footprint() -> Result<Memory, SysError> {
    imp::footprint()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn this_process_has_memory() {
        let m = footprint().unwrap();
        assert!(m.rss > 0 && m.uss > 0 && m.uss <= m.rss, "{m:?}");
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn says_why_there_is_none() {
        assert!(matches!(footprint(), Err(SysError::Unsupported(_))));
    }
}
