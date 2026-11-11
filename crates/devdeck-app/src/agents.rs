//! The agent sessions of one workspace (R6.14): their records, kept in the state folder,
//! and their processes. A session's process is one of the runner's terminals, running its
//! agent's line instead of a shell, under the session's own terminal number. It starts
//! when a view shows the session (resuming one that ran before), keeps running off screen
//! while it works or waits on the user, and stops once off screen and quiet for its
//! agent's `idle_stop` ([`devdeck_core::agents::Pulse::stop_due`]).

use crate::runner::{RunKey, Runner, RunnerEvent};
use devdeck_core::agents::{AgentKind, AgentSession, AgentSessions, Pulse, Status};
use devdeck_core::state::{Loaded, StateStore};
use devdeck_ui::{ToastKind, toasts};
use gpui::{App, AppContext, Context, Entity, Subscription, Task};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Duration;

/// How often running sessions are looked at (their state, whether one should stop).
pub const TICK: Duration = Duration::from_millis(500);

pub struct Agents {
    workspace: String,
    store: Option<StateStore>,
    sessions: AgentSessions,
    runner: Entity<Runner>,
    /// Each session's signs of life, by id.
    pulses: BTreeMap<String, Pulse>,
    /// Typed input counted last, by id (a change answers its bell).
    inputs: BTreeMap<String, u64>,
    /// Sessions a view shows now.
    shown: BTreeSet<String>,
    /// Sessions to start once they can (shown, or just made).
    starting: BTreeSet<String>,
    tick: Option<Task<()>>,
    save_failed: bool,
    _subs: Vec<Subscription>,
}

impl Agents {
    pub fn new(workspace: String, runner: Entity<Runner>, store: Option<StateStore>, cx: &mut Context<Self>) -> Self {
        let sessions = match store.as_ref().map(|s| s.load_agents(&workspace)) {
            Some(Loaded::Ok(s)) => s,
            Some(Loaded::Broken(e)) => {
                let text = format!("Could not read the agent sessions: {e}");
                cx.defer(move |cx| {
                    toasts(cx).update(cx, |t, cx| t.push(ToastKind::Warn, text, vec![], cx));
                });
                AgentSessions::default()
            }
            _ => AgentSessions::default(),
        };
        let environment = crate::environment::environment(cx);
        let subs = vec![
            cx.subscribe(&runner, |this: &mut Self, runner, e: &RunnerEvent, cx| {
                if let RunnerEvent::Output { key } = e {
                    this.output(&runner, key, cx);
                }
            }),
            // The tools found: a session waiting for them starts.
            cx.observe(&runner, |this: &mut Self, _, cx| this.sync(cx)),
            cx.observe(&environment, |this: &mut Self, _, cx| {
                this.sync(cx);
                cx.notify();
            }),
        ];
        Self { workspace, store, sessions, runner, pulses: BTreeMap::new(), inputs: BTreeMap::new(), shown: BTreeSet::new(), starting: BTreeSet::new(), tick: None, save_failed: false, _subs: subs }
    }

    /// Every session, as saved.
    pub fn sessions(&self) -> &[AgentSession] {
        &self.sessions.sessions
    }

    pub fn session(&self, id: &str) -> Option<&AgentSession> {
        self.sessions.get(id)
    }

    /// The session whose terminal is `n`.
    pub fn of_terminal(&self, n: u32) -> Option<&AgentSession> {
        self.sessions.sessions.iter().find(|s| s.n == n)
    }

    /// The highest terminal number a session holds (a new terminal takes another).
    pub fn max_terminal(&self) -> Option<u32> {
        self.sessions.max_n()
    }

    /// A session's agent, from the machine's config.
    pub fn kind(&self, session: &AgentSession, cx: &App) -> Option<AgentKind> {
        crate::environment::agent_kinds(cx).into_iter().find(|k| k.id == session.agent)
    }

    /// How the list and its tab name a session.
    pub fn label(&self, id: &str, cx: &App) -> String {
        let Some(s) = self.sessions.get(id) else { return "agent".into() };
        let agent = self.kind(s, cx).map_or_else(|| s.agent.clone(), |k| k.name);
        s.label(&agent)
    }

    pub fn key(&self, id: &str) -> Option<RunKey> {
        self.sessions.get(id).map(|s| RunKey::terminal(s.n))
    }

    pub fn running(&self, id: &str, cx: &App) -> bool {
        self.key(id).is_some_and(|k| self.runner.read(cx).state(&k).is_some_and(|s| s.is_running()))
    }

    pub fn status(&self, id: &str, cx: &App) -> Status {
        let now = cx.background_executor().now();
        self.pulses.get(id).cloned().unwrap_or_default().status(self.running(id, cx), now)
    }

    /// A new session of `agent` in `dir` as terminal `n`; it starts at once. Returns its
    /// id.
    pub fn create(&mut self, agent: &str, dir: &Path, n: u32, cx: &mut Context<Self>) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        self.sessions.sessions.push(AgentSession::new(id.clone(), agent, dir.to_path_buf(), n, unix_now()));
        self.starting.insert(id.clone());
        self.save(cx);
        self.sync(cx);
        cx.notify();
        id
    }

    /// The sessions a view shows now: one coming on screen starts (resumes) when it
    /// isn't running; one going off screen starts counting towards its stop.
    pub fn set_shown(&mut self, shown: BTreeSet<String>, cx: &mut Context<Self>) {
        if shown == self.shown {
            return;
        }
        let now = cx.background_executor().now();
        for id in shown.difference(&self.shown) {
            self.pulses.entry(id.clone()).or_default().shown(true, now);
            if !self.running(id, cx) {
                self.starting.insert(id.clone());
            }
        }
        for id in self.shown.difference(&shown) {
            self.pulses.entry(id.clone()).or_default().shown(false, now);
            self.starting.remove(id);
        }
        self.shown = shown;
        self.sync(cx);
        cx.notify();
    }

    pub fn shown(&self) -> &BTreeSet<String> {
        &self.shown
    }

    /// Starts the sessions waiting to, when their agent is known and the tools are.
    fn sync(&mut self, cx: &mut Context<Self>) {
        if self.starting.is_empty() || self.runner.read(cx).tools().pending {
            return;
        }
        let kinds = crate::environment::agent_kinds(cx);
        for id in std::mem::take(&mut self.starting) {
            let Some(s) = self.sessions.get(&id).cloned() else { continue };
            let Some(kind) = kinds.iter().find(|k| k.id == s.agent) else {
                // Its agent left the machine's config: it waits for it to come back.
                self.starting.insert(id);
                continue;
            };
            let key = RunKey::terminal(s.n);
            if self.runner.read(cx).state(&key).is_some_and(|st| st.is_running()) {
                continue;
            }
            let line = kind.line(&s.id, s.started);
            let started = self.runner.update(cx, |r, cx| {
                r.open_program(s.n, &s.dir, &line);
                let _ = r.start(&key, None, cx);
                let ok = r.state(&key).is_some_and(|st| st.is_running());
                // Every later start (⏎ once it ended) resumes it.
                r.open_program(s.n, &s.dir, &kind.line(&s.id, true));
                ok
            });
            let mut pulse = Pulse::default();
            pulse.shown(self.shown.contains(&id), cx.background_executor().now());
            self.pulses.insert(id.clone(), pulse);
            if started && !s.started {
                if let Some(rec) = self.sessions.get_mut(&id) {
                    rec.started = true;
                }
                self.save(cx);
            }
        }
        self.ensure_tick(cx);
        cx.notify();
    }

    /// New output in a terminal: a session's pulse, bell and title.
    fn output(&mut self, runner: &Entity<Runner>, key: &RunKey, cx: &mut Context<Self>) {
        let Some(n) = key.terminal_number() else { return };
        let Some(id) = self.of_terminal(n).map(|s| s.id.clone()) else { return };
        if !runner.read(cx).is_program(key) {
            return;
        }
        let now = cx.background_executor().now();
        let bell = runner.update(cx, |r, _| r.take_bell(key));
        let (title, inputs) = {
            let r = runner.read(cx);
            (r.terminal_title(key), r.run(key).map_or(0, |r| r.inputs))
        };
        let pulse = self.pulses.entry(id.clone()).or_default();
        let before = pulse.status(true, now);
        pulse.output(now);
        if self.inputs.insert(id.clone(), inputs).is_some_and(|i| i != inputs) {
            pulse.input();
        }
        if bell {
            pulse.bell = true;
        }
        let after = pulse.status(true, now);
        let mut save = false;
        if let Some(rec) = self.sessions.get_mut(&id) {
            let clean = title.as_deref().map(devdeck_core::agents::clean_title).filter(|t| !t.is_empty());
            if clean.is_some() && rec.title.as_deref().map(devdeck_core::agents::clean_title) != clean {
                rec.title = clean;
                save = true;
            }
            let now_s = unix_now();
            // Kept to the minute: no save per line of output.
            if now_s.saturating_sub(rec.active) >= 60 {
                rec.active = now_s;
                save = true;
            }
        }
        if save {
            self.save(cx);
        }
        self.ensure_tick(cx);
        if before != after || save {
            cx.notify();
        }
    }

    /// Looks at running sessions every [`TICK`] while any runs.
    fn ensure_tick(&mut self, cx: &mut Context<Self>) {
        if self.tick.is_some() {
            return;
        }
        self.tick = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                let go_on = this.update(cx, |a, cx| a.on_tick(cx)).unwrap_or(false);
                if !go_on {
                    let _ = this.update(cx, |a, _| a.tick = None);
                    return;
                }
            }
        }));
    }

    /// Follows the clock; stops what is due. Returns whether any session still runs.
    fn on_tick(&mut self, cx: &mut Context<Self>) -> bool {
        let now = cx.background_executor().now();
        let kinds = crate::environment::agent_kinds(cx);
        let (mut changed, mut any) = (false, false);
        let mut stop = Vec::new();
        for s in &self.sessions.sessions {
            let key = RunKey::terminal(s.n);
            let running = self.runner.read(cx).state(&key).is_some_and(|st| st.is_running());
            let Some(pulse) = self.pulses.get_mut(&s.id) else { continue };
            if !running {
                continue;
            }
            any = true;
            let inputs = self.runner.read(cx).run(&key).map_or(0, |r| r.inputs);
            if self.inputs.insert(s.id.clone(), inputs).is_some_and(|i| i != inputs) {
                pulse.input();
                changed = true;
            }
            changed |= pulse.tick(now);
            let idle = kinds.iter().find(|k| k.id == s.agent).map_or(devdeck_core::agents::IDLE_STOP, |k| k.idle_stop);
            let state = self.runner.read(cx).state(&key);
            if state == Some(devdeck_core::runs::RunState::Running) && pulse.stop_due(now, idle) {
                stop.push(key);
            }
        }
        for key in stop {
            self.runner.update(cx, |r, cx| r.stop(&key, cx));
            changed = true;
        }
        if changed {
            cx.notify();
        }
        any
    }

    /// Stops a session's process (it resumes when shown again).
    pub fn stop(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(key) = self.key(id) {
            self.starting.remove(id);
            self.runner.update(cx, |r, cx| r.stop(&key, cx));
        }
    }

    /// The user's name for a session (empty: none).
    pub fn rename(&mut self, id: &str, name: &str, cx: &mut Context<Self>) {
        if let Some(s) = self.sessions.get_mut(id) {
            let name = name.trim();
            s.name = (!name.is_empty()).then(|| name.to_string());
            self.save(cx);
            cx.notify();
        }
    }

    /// Removes a session from the list, stopping its process. The agent keeps its
    /// conversation.
    pub fn remove(&mut self, id: &str, cx: &mut Context<Self>) {
        self.stop(id, cx);
        if self.sessions.remove(id).is_some() {
            self.pulses.remove(id);
            self.inputs.remove(id);
            self.save(cx);
            cx.notify();
        }
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else { return };
        let (ws, sessions) = (self.workspace.clone(), self.sessions.clone());
        cx.spawn(async move |this, cx| {
            let result = cx.background_executor().spawn(async move { store.save_agents(&ws, &sessions) }).await;
            let _ = this.update(cx, |a, cx| match result {
                Ok(()) => a.save_failed = false,
                Err(e) if !a.save_failed => {
                    a.save_failed = true;
                    toasts(cx).update(cx, |t, cx| t.push(ToastKind::Err, format!("Could not save the agent sessions: {e}"), vec![], cx));
                }
                Err(_) => {}
            });
        })
        .detach();
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// The workspace's agent sessions, made with its runner.
pub fn new(workspace: &str, runner: Entity<Runner>, store: Option<StateStore>, cx: &mut App) -> Entity<Agents> {
    cx.new(|cx| Agents::new(workspace.into(), runner, store, cx))
}
