//! Runs of app commands (R6.7): state, exit status and output lines. M1 feeds them
//! from a fake runner; M2 from real processes in a pseudo-terminal.

use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunState {
    Running,
    /// Stop requested, not exited yet.
    Stopping,
    /// Ended on its own with this exit code.
    Exited(i32),
    /// Stopped by the user.
    Stopped,
}

impl RunState {
    /// Counts as running for indicators and the quit guard (stopping included).
    pub fn is_running(self) -> bool {
        matches!(self, RunState::Running | RunState::Stopping)
    }

    /// The exit code of a failed run.
    pub fn failed(self) -> Option<i32> {
        match self {
            RunState::Exited(code) if code != 0 => Some(code),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Error,
    Warn,
    Info,
    Debug,
    Plain,
}

/// The level a line announces (`ERR`, `error:`, `WARN`…), `Plain` when it says none.
pub fn level_of(line: &str) -> Level {
    let head: String = line.chars().take(48).collect::<String>().to_ascii_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| head.split(|c: char| !c.is_ascii_alphanumeric()).any(|t| t == *w));
    if has(&["err", "error", "fatal", "panic"]) {
        Level::Error
    } else if has(&["wrn", "warn", "warning"]) {
        Level::Warn
    } else if has(&["dbg", "debug", "trace"]) {
        Level::Debug
    } else if has(&["inf", "info"]) {
        Level::Info
    } else {
        Level::Plain
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub text: String,
    pub level: Level,
}

/// Output kept for a run: the last `cap` lines, and the index of the longest one (so a
/// list can size its horizontal scroll without measuring every line).
#[derive(Clone, Debug)]
pub struct Output {
    pub lines: VecDeque<Line>,
    cap: usize,
    /// Lines dropped from the front since the start.
    pub dropped: usize,
}

impl Output {
    pub fn new(cap: usize) -> Self {
        Self { lines: VecDeque::new(), cap: cap.max(1), dropped: 0 }
    }

    pub fn push(&mut self, text: impl Into<String>) {
        let text = text.into();
        let level = level_of(&text);
        if self.lines.len() == self.cap {
            self.lines.pop_front();
            self.dropped += 1;
        }
        self.lines.push_back(Line { text, level });
    }

    pub fn longest(&self) -> usize {
        self.lines.iter().enumerate().max_by_key(|(_, l)| l.text.chars().count()).map_or(0, |(i, _)| i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_from_text() {
        assert_eq!(level_of("08:41 ERR rabbitmq publish failed"), Level::Error);
        assert_eq!(level_of("error: cannot find module"), Level::Error);
        assert_eq!(level_of("[WARN] slow query"), Level::Warn);
        assert_eq!(level_of("DBG cache hit"), Level::Debug);
        assert_eq!(level_of("INFO listening on :5800"), Level::Info);
        assert_eq!(level_of("compiled successfully"), Level::Plain);
        assert_eq!(level_of("terrible"), Level::Plain, "whole words only");
    }

    #[test]
    fn output_keeps_the_last_lines() {
        let mut o = Output::new(3);
        for i in 0..5 {
            o.push(format!("line {i}"));
        }
        assert_eq!(o.lines.len(), 3);
        assert_eq!(o.dropped, 2);
        assert_eq!(o.lines[0].text, "line 2");
    }

    #[test]
    fn states() {
        assert_eq!(RunState::Exited(1).failed(), Some(1));
        assert_eq!(RunState::Exited(0).failed(), None);
        assert_eq!(RunState::Stopped.failed(), None);
        assert!(RunState::Running.is_running());
    }
}
