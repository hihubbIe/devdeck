use super::Memory;
use crate::{SysError, Support};

pub const SUPPORT: Support = Support::Yes;

/// From `/proc/self/smaps_rollup`.
pub fn footprint() -> Result<Memory, SysError> {
    Ok(parse_smaps_rollup(&std::fs::read_to_string("/proc/self/smaps_rollup")?))
}

fn parse_smaps_rollup(text: &str) -> Memory {
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
