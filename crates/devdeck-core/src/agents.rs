//! Agents (R6.14): programs like Claude Code run as sessions devdeck knows. Nothing here
//! knows any agent: what one is, how it starts and how it resumes are `machine.toml`'s
//! ([`AgentKind`]). A session is a record ([`AgentSession`]) that outlives its process:
//! the process runs while a view shows it, or while it works or waits on the user, and
//! a session shown again resumes. Its state comes from its terminal alone ([`Pulse`]):
//! output means working, a bell means it needs you.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Output this recent: the agent is working.
pub const ACTIVE: Duration = Duration::from_millis(1500);

/// How long a session off screen and quiet runs before it stops, unless its agent says.
pub const IDLE_STOP: Duration = Duration::from_secs(60);

/// An agent of the machine, from `machine.toml`'s `[agents.<id>]`.
#[derive(Clone, Debug, PartialEq)]
pub struct AgentKind {
    pub id: String,
    /// As menus and the list say it ("Claude Code"); the id when not given.
    pub name: String,
    /// The program.
    pub command: String,
    /// Before the others, at every start.
    pub args: Vec<String>,
    /// A new session's, `{id}` standing for its id.
    pub start: Vec<String>,
    /// A session's that ran before.
    pub resume: Vec<String>,
    /// Off screen and quiet this long, it stops ([`IDLE_STOP`] when not given).
    pub idle_stop: Duration,
}

impl AgentKind {
    /// The shell line that runs session `id`: new, or resumed. Every word is quoted.
    pub fn line(&self, id: &str, resume: bool) -> String {
        let own = if resume { &self.resume } else { &self.start };
        let words = std::iter::once(self.command.clone()).chain(self.args.iter().chain(own).map(|a| a.replace("{id}", id)));
        let words: Vec<String> = words.map(|w| quote(&w)).collect();
        format!("exec {}", words.join(" "))
    }
}

/// `s` as one word for a POSIX shell.
pub fn quote(s: &str) -> String {
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "-_./=:@%+,".contains(c)) {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// A session: what devdeck keeps of it while its process comes and goes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentSession {
    /// Its id, a UUID devdeck made, given to the agent as `{id}`.
    pub id: String,
    /// Its agent's id.
    pub agent: String,
    /// Where it runs.
    pub dir: PathBuf,
    /// Its terminal's number in the workspace (the same for every process it gets).
    pub n: u32,
    /// The user's name for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The title its agent gave its terminal last.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Seconds since the epoch.
    pub created: u64,
    /// When it last showed output, seconds since the epoch.
    pub active: u64,
    /// Its agent ran it once: it resumes from now on.
    #[serde(default)]
    pub started: bool,
}

impl AgentSession {
    pub fn new(id: String, agent: &str, dir: PathBuf, n: u32, now: u64) -> Self {
        Self { id, agent: agent.into(), dir, n, name: None, title: None, created: now, active: now, started: false }
    }

    /// As the list and its tab say it: the user's name, the agent's title, else the
    /// agent and the folder.
    pub fn label(&self, agent_name: &str) -> String {
        if let Some(n) = self.name.as_deref().filter(|n| !n.trim().is_empty()) {
            return n.trim().to_string();
        }
        if let Some(t) = self.title.as_deref().map(clean_title).filter(|t| !t.is_empty()) {
            return t;
        }
        let folder = self.dir.file_name().map_or_else(|| self.dir.display().to_string(), |f| f.to_string_lossy().into_owned());
        format!("{agent_name} · {folder}")
    }
}

/// A terminal title as a name: without the marks and spinners agents put before it.
pub fn clean_title(title: &str) -> String {
    title.trim_start_matches(|c: char| !c.is_alphanumeric()).trim().to_string()
}

/// The sessions of a workspace, as saved.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentSessions {
    #[serde(default)]
    pub sessions: Vec<AgentSession>,
}

impl AgentSessions {
    pub fn get(&self, id: &str) -> Option<&AgentSession> {
        self.sessions.iter().find(|s| s.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut AgentSession> {
        self.sessions.iter_mut().find(|s| s.id == id)
    }

    pub fn remove(&mut self, id: &str) -> Option<AgentSession> {
        let i = self.sessions.iter().position(|s| s.id == id)?;
        Some(self.sessions.remove(i))
    }

    /// The highest terminal number a session holds.
    pub fn max_n(&self) -> Option<u32> {
        self.sessions.iter().map(|s| s.n).max()
    }
}

/// What a session is doing, as its list row and tab show it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// No process.
    Off,
    /// Its process runs and waits for you, seen.
    Ready,
    /// Output lately.
    Working,
    /// It rang: a question, a permission.
    NeedsYou,
    /// It worked while off screen and went quiet: not seen yet.
    Done,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Status::Off => "off",
            Status::Ready => "ready",
            Status::Working => "working",
            Status::NeedsYou => "needs you",
            Status::Done => "done",
        }
    }
}

/// A running session's signs of life, read from its terminal.
#[derive(Clone, Debug, Default)]
pub struct Pulse {
    /// Its last output.
    pub output: Option<Instant>,
    /// It rang since the last key.
    pub bell: bool,
    /// It worked off screen and went quiet since it was last seen.
    pub done: bool,
    /// It was working at the last [`Pulse::tick`].
    working: bool,
    /// When it went off screen (`None`: shown).
    pub hidden_since: Option<Instant>,
}

impl Pulse {
    pub fn output(&mut self, now: Instant) {
        self.output = Some(now);
    }

    /// A key typed: whatever it asked is answered.
    pub fn input(&mut self) {
        self.bell = false;
        self.done = false;
    }

    pub fn shown(&mut self, shown: bool, now: Instant) {
        match (shown, self.hidden_since) {
            (true, _) => {
                self.hidden_since = None;
                self.done = false;
            }
            (false, None) => self.hidden_since = Some(now),
            (false, Some(_)) => {}
        }
    }

    pub fn working(&self, now: Instant) -> bool {
        self.output.is_some_and(|o| now.saturating_duration_since(o) < ACTIVE)
    }

    /// Follows the clock: a session that stopped working off screen is done. Returns
    /// whether what it shows changed.
    pub fn tick(&mut self, now: Instant) -> bool {
        let working = self.working(now);
        let mut changed = working != self.working;
        if self.working && !working && self.hidden_since.is_some() && !self.done {
            self.done = true;
            changed = true;
        }
        self.working = working;
        changed
    }

    pub fn status(&self, running: bool, now: Instant) -> Status {
        match () {
            _ if !running => Status::Off,
            _ if self.bell => Status::NeedsYou,
            _ if self.working(now) => Status::Working,
            _ if self.done => Status::Done,
            _ => Status::Ready,
        }
    }

    /// Whether its process should stop now: off screen, not waiting on you, and quiet
    /// for `idle` since its last output or since it went off screen.
    pub fn stop_due(&self, now: Instant, idle: Duration) -> bool {
        let Some(hidden) = self.hidden_since else { return false };
        if self.bell {
            return false;
        }
        let since = self.output.map_or(hidden, |o| o.max(hidden));
        now.saturating_duration_since(since) >= idle
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claude() -> AgentKind {
        AgentKind {
            id: "claude".into(),
            name: "Claude Code".into(),
            command: "claude".into(),
            args: vec!["--settings".into(), r#"{"preferredNotifChannel": "terminal_bell"}"#.into()],
            start: vec!["--session-id".into(), "{id}".into()],
            resume: vec!["--resume".into(), "{id}".into()],
            idle_stop: IDLE_STOP,
        }
    }

    #[test]
    fn a_line_quotes_every_word_and_puts_the_id_in() {
        let k = claude();
        assert_eq!(k.line("6f1c-2e", false), r#"exec claude --settings '{"preferredNotifChannel": "terminal_bell"}' --session-id 6f1c-2e"#);
        assert_eq!(k.line("6f1c-2e", true), r#"exec claude --settings '{"preferredNotifChannel": "terminal_bell"}' --resume 6f1c-2e"#);
        assert_eq!(quote("it's"), r"'it'\''s'");
        assert_eq!(quote(""), "''");
        assert_eq!(quote("$HOME"), "'$HOME'");
    }

    #[test]
    fn a_session_is_named_by_the_user_then_its_title_then_its_folder() {
        let mut s = AgentSession::new("a".into(), "claude", "/home/me/repos/api".into(), 3, 10);
        assert_eq!(s.label("Claude Code"), "Claude Code · api");
        s.title = Some("✳ Fix the build".into());
        assert_eq!(s.label("Claude Code"), "Fix the build");
        s.title = Some("✳ ".into());
        assert_eq!(s.label("Claude Code"), "Claude Code · api", "an empty title isn't a name");
        s.name = Some(" mine ".into());
        assert_eq!(s.label("Claude Code"), "mine");
    }

    #[test]
    fn records_round_trip() {
        let mut all = AgentSessions::default();
        all.sessions.push(AgentSession::new("a".into(), "claude", "/x".into(), 4, 10));
        all.sessions.push(AgentSession::new("b".into(), "claude", "/y".into(), 9, 11));
        let back: AgentSessions = serde_json::from_str(&serde_json::to_string(&all).unwrap()).unwrap();
        assert_eq!(back, all);
        assert_eq!(all.max_n(), Some(9));
        assert_eq!(all.remove("a").map(|s| s.n), Some(4));
        assert!(all.get("a").is_none());
    }

    #[test]
    fn output_is_working_a_bell_needs_you_until_a_key() {
        let t0 = Instant::now();
        let mut p = Pulse::default();
        assert_eq!(p.status(false, t0), Status::Off);
        assert_eq!(p.status(true, t0), Status::Ready);
        p.output(t0);
        assert_eq!(p.status(true, t0 + Duration::from_millis(500)), Status::Working);
        assert_eq!(p.status(true, t0 + ACTIVE), Status::Ready, "quiet again");
        p.bell = true;
        assert_eq!(p.status(true, t0), Status::NeedsYou, "before working");
        p.input();
        assert_eq!(p.status(true, t0 + ACTIVE), Status::Ready);
    }

    #[test]
    fn working_off_screen_then_quiet_is_done_until_shown() {
        let t0 = Instant::now();
        let mut p = Pulse::default();
        p.shown(false, t0);
        p.output(t0);
        assert!(p.tick(t0), "working now");
        assert!(p.tick(t0 + ACTIVE), "quiet: done");
        assert_eq!(p.status(true, t0 + ACTIVE), Status::Done);
        assert!(!p.tick(t0 + ACTIVE * 2));
        p.shown(true, t0 + ACTIVE * 2);
        assert_eq!(p.status(true, t0 + ACTIVE * 2), Status::Ready, "seen");
        // Shown, working then quiet: nothing to see later.
        p.output(t0 + ACTIVE * 3);
        p.tick(t0 + ACTIVE * 3);
        p.tick(t0 + ACTIVE * 5);
        assert_eq!(p.status(true, t0 + ACTIVE * 5), Status::Ready);
    }

    #[test]
    fn it_stops_off_screen_once_quiet_long_enough_unless_it_waits_on_you() {
        let t0 = Instant::now();
        let idle = Duration::from_secs(60);
        let mut p = Pulse::default();
        assert!(!p.stop_due(t0 + idle * 2, idle), "shown");
        p.output(t0);
        p.shown(false, t0 + Duration::from_secs(10));
        assert!(!p.stop_due(t0 + Duration::from_secs(69), idle), "counted from when it went off screen");
        assert!(p.stop_due(t0 + Duration::from_secs(70), idle));
        p.output(t0 + Duration::from_secs(65));
        assert!(!p.stop_due(t0 + Duration::from_secs(70), idle), "it worked since");
        assert!(p.stop_due(t0 + Duration::from_secs(125), idle));
        p.bell = true;
        assert!(!p.stop_due(t0 + idle * 10, idle), "it waits on you");
        p.shown(true, t0 + idle * 10);
        p.bell = false;
        assert!(!p.stop_due(t0 + idle * 20, idle), "shown again");
    }
}
