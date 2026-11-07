//! Self-measurement used by `scripts/measure.sh` (R11.4).
//!
//! - `DEVDECK_STATS=<seconds>` prints a stats line on that interval.
//! - `DEVDECK_STRESS=<cycles>` runs layout cycles to look for leaks, then quits.

use std::sync::atomic::{AtomicU64, Ordering};

/// Incremented by every window render; must stay flat while the app is idle.
pub static RENDERS: AtomicU64 = AtomicU64::new(0);

pub fn count_render() {
    RENDERS.fetch_add(1, Ordering::Relaxed);
}

pub fn renders() -> u64 {
    RENDERS.load(Ordering::Relaxed)
}

/// Memory of this process in kB, from `/proc/self/smaps_rollup`.
#[derive(Debug, Default, Clone, Copy)]
pub struct Memory {
    pub rss: u64,
    pub pss: u64,
    pub uss: u64,
}

pub fn memory() -> Memory {
    let Ok(text) = std::fs::read_to_string("/proc/self/smaps_rollup") else {
        return Memory::default();
    };
    parse_smaps_rollup(&text)
}

pub fn parse_smaps_rollup(text: &str) -> Memory {
    let field = |name: &str| -> u64 {
        text.lines()
            .find(|l| l.starts_with(name))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    };
    Memory {
        rss: field("Rss:"),
        pss: field("Pss:"),
        uss: field("Private_Clean:") + field("Private_Dirty:"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_smaps_rollup() {
        let text = "55a0-7ffd ---p 00000000 00:00 0 [rollup]\nRss:              120000 kB\nPss:               90000 kB\nPss_Anon:          60000 kB\nShared_Clean:      20000 kB\nPrivate_Clean:      5000 kB\nPrivate_Dirty:     70000 kB\n";
        let m = parse_smaps_rollup(text);
        assert_eq!((m.rss, m.pss, m.uss), (120000, 90000, 75000));
    }

    #[test]
    fn missing_fields_are_zero() {
        let m = parse_smaps_rollup("nothing here");
        assert_eq!((m.rss, m.pss, m.uss), (0, 0, 0));
    }
}
