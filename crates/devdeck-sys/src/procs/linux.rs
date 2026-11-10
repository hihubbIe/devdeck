//! From `/proc`: `/proc/<pid>/stat`, `comm` and `cmdline`.

use super::Proc;
use crate::{SysError, Support};

pub const SUPPORT: Support = Support::Yes;

pub fn get(pid: u32) -> Option<Proc> {
    parse_stat(&std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?)
}

pub fn parents() -> Result<Vec<(u32, u32)>, SysError> {
    let dir = std::fs::read_dir("/proc")?;
    Ok(dir.filter_map(|e| e.ok()?.file_name().to_str()?.parse::<u32>().ok()).filter_map(|pid| Some((pid, get(pid)?.parent))).collect())
}

pub fn name(pid: u32) -> Option<String> {
    Some(std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?.trim_end().to_string())
}

pub fn command(pid: u32) -> Option<String> {
    let args = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    let line = args.split(|&b| b == 0).filter(|a| !a.is_empty()).map(String::from_utf8_lossy).collect::<Vec<_>>().join(" ");
    (!line.is_empty()).then_some(line)
}

/// A `stat` line. The command name is in parentheses and may hold spaces or parentheses
/// itself, so fields are counted after the last `)`: state is field 3, parent 4, session
/// 6, start time 22.
fn parse_stat(text: &str) -> Option<Proc> {
    let mut fields = text.rsplit_once(')')?.1.split_whitespace();
    let state = fields.next()?.chars().next()?;
    let parent = fields.next()?.parse().ok()?;
    let session = fields.nth(1)?.parse().ok()?;
    let start = fields.nth(15)?.parse().ok()?;
    Some(Proc { parent, session, start, ended: matches!(state, 'Z' | 'X') })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stat_fields_survive_odd_command_names() {
        let line = "42 (a) b) S 7 42 40 0 -1 4194560 100 0 0 0 1 2 0 0 20 0 1 0 98765 1000 10";
        assert_eq!(parse_stat(line), Some(Proc { parent: 7, session: 40, start: 98765, ended: false }));
        let odd = "123 (a) b (c) Z 88 1 1 0 -1 0 0 0 0 0 0 0 0 0 20 0 1 0 5 1000 10";
        assert_eq!(parse_stat(odd), Some(Proc { parent: 88, session: 1, start: 5, ended: true }));
        assert_eq!(parse_stat("123 (node) X 77 123 123 0 -1 0 0 0 0 0 0 0 0 0 20 0 1 0 9 1 1").map(|p| p.ended), Some(true));
        assert!(parse_stat("garbage").is_none());
        assert!(parse_stat("1 (short) S 2 3").is_none());
    }
}
