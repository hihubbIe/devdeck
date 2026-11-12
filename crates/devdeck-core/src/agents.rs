//! Agents (R6.14): programs like Claude Code run as sessions devdeck knows. Nothing here
//! knows any agent: what one is, how it starts and how it resumes are `machine.toml`'s
//! ([`AgentKind`]). A session is a record ([`AgentSession`]) that outlives its process:
//! the process runs while a view shows it, or while it works or waits on the user, and
//! a session shown again resumes. Its state comes from its terminal ([`Pulse`]): output
//! means working, a bell means it needs you; and, when the agent reports through its own
//! hooks or status line (`devdeck-ctl report <event>`), from what it says ([`Event`],
//! [`Facts`], [`Limit`]): which of its JSON's fields say what is the config's.

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
    /// Where each fact ([`FACTS`]) is in what it reports: a JSON pointer.
    pub fields: std::collections::BTreeMap<String, String>,
    /// Its usage limits, as it reports them.
    pub limits: Vec<LimitSpec>,
}

/// The facts an agent may report, by name (`fields` in its config).
pub const FACTS: [&str; 11] = ["session", "title", "model", "context", "cost", "added", "removed", "message", "prompt", "asks", "pr"];

/// A usage limit of an agent: where its share used (0–100) and its reset time (seconds
/// since the epoch) are in what it reports.
#[derive(Clone, Debug, PartialEq)]
pub struct LimitSpec {
    pub name: String,
    pub used: String,
    pub resets: Option<String>,
}

/// A usage limit as last reported.
#[derive(Clone, Debug, PartialEq)]
pub struct Limit {
    pub name: String,
    /// Percent used.
    pub used: f32,
    /// When it resets, seconds since the epoch.
    pub resets: Option<u64>,
}

/// What an agent says happened (`devdeck-ctl report <event>`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// Its process started or resumed a conversation (its id may be new).
    Started,
    /// It took a prompt.
    Working,
    /// It waits on you: a permission, a question.
    NeedsYou,
    /// It ended its turn.
    YourTurn,
    /// Its process ends.
    Ended,
    /// Facts only (a status line).
    Status,
}

impl Event {
    pub fn parse(s: &str) -> Option<Event> {
        Some(match s {
            "started" => Event::Started,
            "working" => Event::Working,
            "needs_you" => Event::NeedsYou,
            "your_turn" => Event::YourTurn,
            "ended" => Event::Ended,
            "status" => Event::Status,
            _ => return None,
        })
    }
}

/// What a session's agent said of it last, kept with the session.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Facts {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Percent of its context window used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<f32>,
    /// Its cost so far, in US dollars.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub added: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub removed: Option<u64>,
    /// The start of its last answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Your last prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    /// What it waits on you for (a tool it wants to use).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asks: Option<String>,
    /// Its branch's pull request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr: Option<String>,
    /// Prompts it took.
    #[serde(default)]
    pub prompts: u32,
}

/// Text this long at most is kept of a message or a prompt.
const KEPT: usize = 400;

fn text_of(v: &serde_json::Value) -> Option<String> {
    let s = match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Null => return None,
        other => other.to_string(),
    };
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    Some(match s.char_indices().nth(KEPT) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_string(),
    })
}

fn number_of(v: &serde_json::Value) -> Option<f64> {
    match v {
        serde_json::Value::Number(n) => n.as_f64(),
        serde_json::Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// What a report says, read with the agent's `fields` and `limits`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Said {
    /// The agent's own id for the conversation, when reported.
    pub session: Option<String>,
    pub title: Option<String>,
    pub facts: Facts,
    pub limits: Vec<Limit>,
}

impl AgentKind {
    /// Reads a report's JSON: each configured fact found, each limit found.
    pub fn read(&self, data: &serde_json::Value) -> Said {
        let at = |fact: &str| self.fields.get(fact).and_then(|p| data.pointer(p)).filter(|v| !v.is_null());
        let text = |fact: &str| at(fact).and_then(text_of);
        let number = |fact: &str| at(fact).and_then(number_of);
        let facts = Facts {
            model: text("model"),
            context: number("context").map(|n| n as f32),
            cost: number("cost"),
            added: number("added").map(|n| n.max(0.) as u64),
            removed: number("removed").map(|n| n.max(0.) as u64),
            message: text("message"),
            prompt: text("prompt"),
            asks: text("asks"),
            pr: text("pr"),
            prompts: 0,
        };
        let limits = self
            .limits
            .iter()
            .filter_map(|l| {
                let used = data.pointer(&l.used).and_then(number_of)? as f32;
                let resets = l.resets.as_ref().and_then(|p| data.pointer(p)).and_then(number_of).map(|n| n.max(0.) as u64);
                Some(Limit { name: l.name.clone(), used, resets })
            })
            .collect();
        Said { session: text("session"), title: text("title"), facts, limits }
    }
}

impl Facts {
    /// Takes what a report said; what it didn't say stays.
    pub fn merge(&mut self, new: Facts) {
        macro_rules! keep {
            ($($f:ident),*) => {$(if new.$f.is_some() { self.$f = new.$f; })*};
        }
        keep!(model, context, cost, added, removed, message, prompt, asks, pr);
    }

    /// The line an agent's status line shows: its model, its context, its cost.
    pub fn line(&self) -> String {
        let mut parts = Vec::new();
        if let Some(m) = &self.model {
            parts.push(m.clone());
        }
        if let Some(c) = self.context {
            parts.push(format!("context {c:.0}%"));
        }
        if let Some(c) = self.cost {
            parts.push(format!("${c:.2}"));
        }
        parts.join(" · ")
    }
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
    /// What its agent reported of it last.
    #[serde(default)]
    pub facts: Facts,
    /// Its project: the workspace app whose folder holds its folder (the deepest), found
    /// when it's made; `None`: in no app's folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    /// The conversation it resumes when its agent moved to another one (`/clear`,
    /// `/resume`): the agent's id for it; `None`: its own id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation: Option<String>,
}

impl AgentSession {
    pub fn new(id: String, agent: &str, dir: PathBuf, n: u32, now: u64) -> Self {
        Self { id, agent: agent.into(), dir, n, name: None, title: None, created: now, active: now, started: false, facts: Facts::default(), project: None, conversation: None }
    }

    /// The agent's id for the conversation it resumes.
    pub fn conversation(&self) -> &str {
        self.conversation.as_deref().unwrap_or(&self.id)
    }

    /// As the list and its tab say it: the user's name, the agent's title, else the
    /// agent and its terminal's number (its project names it already).
    pub fn label(&self, agent_name: &str) -> String {
        if let Some(n) = self.name.as_deref().filter(|n| !n.trim().is_empty()) {
            return n.trim().to_string();
        }
        if let Some(t) = self.title.as_deref().map(clean_title).filter(|t| !t.is_empty()) {
            return t;
        }
        format!("{agent_name} #{}", self.n)
    }
}

/// Whether `id` can be a conversation's id on an agent's command line: letters, digits,
/// `-` and `_`, not starting with `-`, 128 at most. What an agent reports is checked with
/// it: anything else could be read as an option (`--resume --dangerously…`).
pub fn is_conversation_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && !id.starts_with('-') && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// The project of a folder: the app (id, folder) whose folder holds it, the deepest one;
/// folders compared part by part (`app` doesn't hold `app-2`).
pub fn project_of<'a>(dir: &std::path::Path, apps: impl IntoIterator<Item = (&'a str, &'a std::path::Path)>) -> Option<String> {
    apps.into_iter().filter(|(_, d)| dir.starts_with(d)).max_by_key(|(_, d)| d.components().count()).map(|(id, _)| id.to_string())
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

/// Minutes of activity kept per session ([`History`]).
pub const HISTORY_MINUTES: usize = 60;

/// How busy a session was, minute by minute, the last [`HISTORY_MINUTES`]: seconds it
/// worked in each, the current minute last.
#[derive(Clone, Debug, PartialEq)]
pub struct History {
    /// Seconds worked per minute, oldest first.
    pub minutes: std::collections::VecDeque<f32>,
    /// The minute the last one is (minutes since the clock's start).
    pub last: u64,
}

impl Default for History {
    fn default() -> Self {
        Self { minutes: std::iter::repeat_n(0.0, HISTORY_MINUTES).collect(), last: 0 }
    }
}

impl History {
    /// Moves to minute `now` (older minutes scroll out), then adds `worked` seconds to it.
    pub fn add(&mut self, now: u64, worked: f32) {
        let gap = now.saturating_sub(self.last).min(HISTORY_MINUTES as u64);
        for _ in 0..gap {
            self.minutes.pop_front();
            self.minutes.push_back(0.0);
        }
        self.last = self.last.max(now);
        if let Some(m) = self.minutes.back_mut() {
            *m = (*m + worked).min(60.0);
        }
    }

    /// Each minute's share of work, 0 to 1, oldest first.
    pub fn shares(&self) -> Vec<f32> {
        self.minutes.iter().map(|s| s / 60.0).collect()
    }

    /// Seconds worked over the whole history.
    pub fn total(&self) -> f32 {
        self.minutes.iter().sum()
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
    /// What its agent reported last, when it reports: it wins over the terminal's signs.
    pub reported: Option<Event>,
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

    /// What its agent says happened. Returns whether its state changed.
    pub fn report(&mut self, event: Event, now: Instant) -> bool {
        let before = self.status(true, now);
        match event {
            Event::Working => {
                self.reported = Some(Event::Working);
                self.bell = false;
                self.done = false;
            }
            Event::NeedsYou => self.reported = Some(Event::NeedsYou),
            Event::YourTurn => {
                self.reported = Some(Event::YourTurn);
                self.bell = false;
                if self.hidden_since.is_some() {
                    self.done = true;
                }
            }
            Event::Started | Event::Ended => {
                self.reported = None;
                self.bell = false;
            }
            Event::Status => {}
        }
        // A report is a sign of life too.
        self.output = Some(now);
        self.working = false;
        before != self.status(true, now)
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

    /// Working: as its agent reported, else output lately.
    pub fn working(&self, now: Instant) -> bool {
        match self.reported {
            Some(Event::Working) => true,
            Some(_) => false,
            None => self.output.is_some_and(|o| now.saturating_duration_since(o) < ACTIVE),
        }
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
            _ if self.bell || self.reported == Some(Event::NeedsYou) => Status::NeedsYou,
            _ if self.working(now) => Status::Working,
            _ if self.done => Status::Done,
            _ => Status::Ready,
        }
    }

    /// Whether its process should stop now: off screen, not waiting on you, and quiet
    /// for `idle` since its last output or since it went off screen.
    pub fn stop_due(&self, now: Instant, idle: Duration) -> bool {
        let Some(hidden) = self.hidden_since else { return false };
        if self.bell || matches!(self.reported, Some(Event::Working | Event::NeedsYou)) {
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
            fields: [("model", "/model/display_name"), ("context", "/context_window/used_percentage"), ("cost", "/cost/total_cost_usd"), ("message", "/last_assistant_message"), ("asks", "/tool_name"), ("title", "/session_name")]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            limits: vec![LimitSpec { name: "5 hours".into(), used: "/rate_limits/five_hour/used_percentage".into(), resets: Some("/rate_limits/five_hour/resets_at".into()) }],
        }
    }

    #[test]
    fn a_report_is_read_with_the_agents_fields_and_merged() {
        let k = claude();
        let status = serde_json::json!({
            "model": { "display_name": "Opus" },
            "context_window": { "used_percentage": 42.5 },
            "cost": { "total_cost_usd": 1.2345 },
            "session_name": "Fix the build",
            "rate_limits": { "five_hour": { "used_percentage": 71, "resets_at": 1762900000 } }
        });
        let said = k.read(&status);
        assert_eq!(said.title.as_deref(), Some("Fix the build"));
        assert_eq!(said.facts.model.as_deref(), Some("Opus"));
        assert_eq!(said.facts.context, Some(42.5));
        assert_eq!(said.limits, [Limit { name: "5 hours".into(), used: 71.0, resets: Some(1762900000) }]);
        let mut facts = Facts::default();
        facts.merge(said.facts);
        assert_eq!(facts.line(), "Opus · context 42% · $1.23");
        // A hook says less: what it doesn't say stays.
        let stop = serde_json::json!({ "last_assistant_message": "Done: the build passes.", "context_window": null });
        facts.merge(k.read(&stop).facts);
        assert_eq!((facts.message.as_deref(), facts.model.as_deref(), facts.context), (Some("Done: the build passes."), Some("Opus"), Some(42.5)));
        assert_eq!(k.read(&serde_json::json!({ "last_assistant_message": "x".repeat(500) })).facts.message.map(|m| m.chars().count()), Some(401), "cut");
    }

    #[test]
    fn reports_win_over_the_terminals_signs() {
        let t0 = Instant::now();
        let idle = Duration::from_secs(60);
        let mut p = Pulse::default();
        p.shown(false, t0);
        assert!(p.report(Event::Working, t0));
        assert_eq!(p.status(true, t0 + idle * 5), Status::Working, "quiet but working: a long tool");
        assert!(!p.stop_due(t0 + idle * 5, idle), "never stopped while working");
        p.report(Event::NeedsYou, t0 + idle);
        assert_eq!(p.status(true, t0 + idle * 5), Status::NeedsYou);
        assert!(!p.stop_due(t0 + idle * 5, idle));
        p.report(Event::YourTurn, t0 + idle * 2);
        assert_eq!(p.status(true, t0 + idle * 2), Status::Done, "off screen: done until seen");
        assert!(p.stop_due(t0 + idle * 3, idle), "its turn over, quiet: it stops");
        p.shown(true, t0 + idle * 3);
        assert_eq!(p.status(true, t0 + idle * 3), Status::Ready);
        assert_eq!(Event::parse("needs_you"), Some(Event::NeedsYou));
        assert_eq!(Event::parse("bogus"), None);
    }

    #[test]
    fn history_keeps_the_last_hour_minute_by_minute() {
        let mut h = History::default();
        h.add(10, 30.0);
        h.add(10, 45.0);
        assert_eq!(h.minutes.back(), Some(&60.0), "a minute holds 60 s at most");
        h.add(12, 6.0);
        let shares = h.shares();
        assert_eq!(shares.len(), HISTORY_MINUTES);
        assert_eq!(&shares[HISTORY_MINUTES - 3..], [1.0, 0.0, 0.1]);
        assert_eq!(h.total(), 66.0);
        h.add(500, 0.0);
        assert_eq!(h.total(), 0.0, "an hour later, all gone");
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
        assert_eq!(s.label("Claude Code"), "Claude Code #3");
        s.title = Some("✳ Fix the build".into());
        assert_eq!(s.label("Claude Code"), "Fix the build");
        s.title = Some("✳ ".into());
        assert_eq!(s.label("Claude Code"), "Claude Code #3", "an empty title isn't a name");
        s.name = Some(" mine ".into());
        assert_eq!(s.label("Claude Code"), "mine");
    }

    #[test]
    fn a_folders_project_is_the_deepest_app_holding_it() {
        use std::path::Path;
        let apps = [("front-end", Path::new("/r/front-end")), ("front-end/web", Path::new("/r/front-end/apps/web")), ("api", Path::new("/r/api"))];
        assert_eq!(project_of(Path::new("/r/front-end/apps/web/src"), apps).as_deref(), Some("front-end/web"));
        assert_eq!(project_of(Path::new("/r/front-end/libs"), apps).as_deref(), Some("front-end"));
        assert_eq!(project_of(Path::new("/r/api"), apps).as_deref(), Some("api"));
        assert_eq!(project_of(Path::new("/r/api-2"), apps), None, "part by part");
        assert_eq!(project_of(Path::new("/tmp"), apps), None);
    }

    #[test]
    fn only_a_plain_id_can_be_a_conversation() {
        assert!(is_conversation_id("6f1c2e7a-0d4b-4c59-9a1e-2b7f3c8d9e01"));
        assert!(is_conversation_id("my_session-2"));
        for bad in ["", "--dangerously-skip-permissions", "-x", "a b", "a;rm -rf /", "a/b", "é", &"a".repeat(129)] {
            assert!(!is_conversation_id(bad), "{bad:?}");
        }
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
