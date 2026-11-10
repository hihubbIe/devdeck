//! Local time (U8).

#[cfg_attr(unix, path = "unix.rs")]
#[cfg_attr(not(unix), path = "unsupported.rs")]
mod imp;

/// Hours, minutes and seconds, in local time, of milliseconds since the epoch; None when
/// the OS can't say.
pub fn local_hms(ms: u64) -> Option<(u8, u8, u8)> {
    imp::local_hms(ms)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn local_time_follows_tz() {
        // SAFETY: only under nextest (docs/WORKFLOW.md), which runs each test in a process
        // of its own, where no other thread reads the environment meanwhile; set before
        // the first local time call (glibc reads it once).
        unsafe { std::env::set_var("TZ", "XXX-2") };
        // 2026-11-10 10:30:15 UTC, two hours east.
        assert_eq!(local_hms(1_794_306_615_000), Some((12, 30, 15)));
    }
}
