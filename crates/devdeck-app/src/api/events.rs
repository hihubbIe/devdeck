//! Events (R13.10): a connection follows topics with `events.subscribe`; devdeck sends it
//! `event` notifications saying what changed, never its content (the client reads that
//! with the methods), numbered from 1. Each connection has a bounded queue
//! ([`super::transport::OUTBOX`] lines, shared with its answers); when it's full the
//! events are dropped and, once there's room, the client is told `events.lagged` with the
//! first one it missed.
//!
//! The models are watched only while some connection follows a topic (R1.3): the
//! [`Hub`] is made with the first subscription and dropped with the last. Changes are
//! gathered and sent [`EVERY`] at most, the last of each thing winning (a run's state, a
//! note's revision). A caller hears about its own workspace only (R2.1), and about
//! output only for runs and its own terminal (the user's shells hold what they typed).

use super::{ApiState, Who};
use crate::runner::RunKey;
use super::dispatch::CallCx;
use devdeck_api::types::{Event, Lagged, Subscribed, Topic, TopicsParams};
use gpui::{App, Context, Entity, Subscription};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::sync::mpsc::SyncSender;
use std::time::Duration;

/// Changes are sent this often at most.
pub const EVERY: Duration = Duration::from_millis(100);

/// One connection's events: numbered, dropped when its queue is full.
pub struct Outbox {
    tx: SyncSender<Vec<u8>>,
    seq: u64,
    /// The first event dropped, while the client hasn't been told.
    lagged: Option<u64>,
}

fn notification(method: &str, params: impl Serialize) -> Vec<u8> {
    let mut line = serde_json::to_vec(&json!({ "jsonrpc": "2.0", "method": method, "params": params })).unwrap_or_default();
    line.push(b'\n');
    line
}

impl Outbox {
    pub fn new(tx: SyncSender<Vec<u8>>) -> Self {
        Self { tx, seq: 0, lagged: None }
    }

    fn push(&mut self, topic: Topic, data: Value) {
        self.seq += 1;
        if let Some(seq) = self.lagged {
            if self.tx.try_send(notification("events.lagged", Lagged { seq })).is_err() {
                return;
            }
            self.lagged = None;
        }
        if self.tx.try_send(notification("event", Event { seq: self.seq, topic, data })).is_err() {
            self.lagged = Some(self.seq);
        }
    }
}

/// A change waiting to be sent: for a workspace's callers (none: everyone's), and for
/// one terminal's only when it's that terminal's output.
pub(super) struct Change {
    workspace: Option<String>,
    terminal: Option<u32>,
    data: Value,
}

/// What the models were last seen as, to tell what changed; and the watching itself.
pub struct Hub {
    _subs: Vec<Subscription>,
    runs: HashMap<String, BTreeMap<RunKey, devdeck_core::runs::RunState>>,
    notes: HashMap<String, BTreeMap<String, u64>>,
    shown: Option<String>,
    edit_mode: bool,
    toast: u64,
}

impl ApiState {
    /// Whether some connection follows `topic`.
    fn follows(&self, topic: Topic) -> bool {
        self.connections.values().any(|c| c.topics.contains(&topic))
    }

    /// Queues a change, keyed so a later one of the same thing replaces it.
    fn change(&mut self, topic: Topic, key: String, change: Change, cx: &mut Context<Self>) {
        if !self.follows(topic) {
            return;
        }
        self.pending.insert((topic, key), change);
        if self.flushing.is_none() {
            self.flushing = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(EVERY).await;
                let _ = this.update(cx, |s, cx| s.flush(cx));
            }));
        }
    }

    /// Sends what's queued to the connections that follow it and may hear it. A caller
    /// whose shell ended follows nothing any more (an idle one wouldn't call again to be
    /// told), and the models stop being watched with the last follower.
    fn flush(&mut self, cx: &mut Context<Self>) {
        self.flushing = None;
        let alive = self.alive;
        let mut ended = false;
        for c in self.connections.values_mut() {
            if !c.topics.is_empty() && !alive(&c.who, cx) {
                c.topics.clear();
                ended = true;
            }
        }
        if ended {
            self.watch(cx);
        }
        for ((topic, _), change) in std::mem::take(&mut self.pending) {
            for c in self.connections.values_mut() {
                let Some(out) = c.events.as_mut() else { continue };
                if c.topics.contains(&topic) && hears(&c.who, &change) {
                    out.push(topic, change.data.clone());
                }
            }
        }
    }

    /// Follows (or stops following) `topics` on connection `conn`; the topics it follows
    /// now. The models are watched while anyone follows anything.
    pub fn follow(&mut self, conn: u64, topics: &[Topic], on: bool, cx: &mut Context<Self>) -> Result<Vec<Topic>, devdeck_api::ApiError> {
        if topics.contains(&Topic::Unknown) {
            return Err(devdeck_api::ApiError::invalid("topics: one isn't known to this devdeck"));
        }
        let c = self.connections.get_mut(&conn).ok_or_else(|| devdeck_api::ApiError::invalid("events need a connection of their own"))?;
        if c.events.is_none() {
            return Err(devdeck_api::ApiError::invalid("this connection takes no events"));
        }
        for t in topics {
            if on {
                c.topics.insert(*t);
            } else {
                c.topics.remove(t);
            }
        }
        let now = c.topics.iter().copied().collect();
        self.watch(cx);
        Ok(now)
    }

    /// Makes or drops the hub as anyone follows anything or nobody does.
    pub(super) fn watch(&mut self, cx: &mut Context<Self>) {
        let wanted = self.connections.values().any(|c| !c.topics.is_empty());
        if !wanted {
            self.hub = None;
            self.pending.clear();
            self.flushing = None;
        } else if self.hub.is_none() {
            self.hub = Some(hub(cx));
        }
    }

    fn runner_changed(&mut self, ws: &str, runner: &Entity<crate::runner::Runner>, cx: &mut Context<Self>) {
        let now: BTreeMap<RunKey, devdeck_core::runs::RunState> = runner.read(cx).runs().map(|(k, r)| (k.clone(), r.state)).collect();
        let Some(hub) = self.hub.as_mut() else { return };
        let before = hub.runs.insert(ws.to_string(), now.clone()).unwrap_or_default();
        for (key, state) in now {
            if before.get(&key) == Some(&state) {
                continue;
            }
            let (s, code, _) = super::handlers::run_state(state);
            let (topic, data) = match key.terminal_number() {
                Some(n) => (Topic::Terminals, json!({ "n": n, "state": s, "exit_code": code })),
                None => (Topic::Runs, json!({ "app": key.app, "cmd": key.cmd, "state": s, "exit_code": code })),
            };
            self.change(topic, format!("{ws}/{}/{}", key.app, key.cmd), Change { workspace: Some(ws.into()), terminal: None, data }, cx);
        }
    }

    fn output(&mut self, ws: &str, key: &RunKey, cx: &mut Context<Self>) {
        let (terminal, data) = match key.terminal_number() {
            Some(n) => (Some(n), json!({ "n": n })),
            None => (None, json!({ "app": key.app, "cmd": key.cmd })),
        };
        self.change(Topic::Output, format!("{ws}/{}/{}", key.app, key.cmd), Change { workspace: Some(ws.into()), terminal, data }, cx);
    }

    fn notes_changed(&mut self, ws: &str, notes: &Entity<crate::notes::Notes>, cx: &mut Context<Self>) {
        let n = notes.read(cx);
        let now: BTreeMap<String, u64> = n.all().iter().map(|m| (m.id.clone(), n.revision(&m.id))).collect();
        let Some(hub) = self.hub.as_mut() else { return };
        let before = hub.notes.insert(ws.to_string(), now.clone()).unwrap_or_default();
        let mut changes = Vec::new();
        for (id, rev) in &now {
            if before.get(id) != Some(rev) {
                changes.push((id.clone(), json!({ "id": id, "revision": rev })));
            }
        }
        for id in before.keys().filter(|id| !now.contains_key(*id)) {
            changes.push((id.clone(), json!({ "id": id, "deleted": true })));
        }
        for (id, data) in changes {
            self.change(Topic::Notes, format!("{ws}/{id}"), Change { workspace: Some(ws.into()), terminal: None, data }, cx);
        }
    }

    fn shell_changed(&mut self, cx: &mut Context<Self>) {
        let Some(shell) = crate::shell::try_shell(cx) else { return };
        let shown = shell.read(cx).workspaces.get(shell.read(cx).active).map(|w| w.id.clone());
        let Some(hub) = self.hub.as_mut() else { return };
        let before = std::mem::replace(&mut hub.shown, shown.clone());
        if before == shown {
            return;
        }
        for (ws, on) in [(before, false), (shown, true)] {
            if let Some(ws) = ws {
                self.change(Topic::Workspace, ws.clone(), Change { workspace: Some(ws), terminal: None, data: json!({ "shown": on }) }, cx);
            }
        }
    }

    fn edit_mode_changed(&mut self, cx: &mut Context<Self>) {
        let on = crate::edit_mode::is_on(cx);
        let Some(hub) = self.hub.as_mut() else { return };
        if std::mem::replace(&mut hub.edit_mode, on) != on {
            self.change(Topic::EditMode, String::new(), Change { workspace: None, terminal: None, data: json!({ "on": on }) }, cx);
        }
    }

    fn toasts_changed(&mut self, toasts: &Entity<devdeck_ui::overlay::Toasts>, cx: &mut Context<Self>) {
        let Some(hub) = self.hub.as_mut() else { return };
        let since = hub.toast;
        let new: Vec<(u64, devdeck_ui::ToastKind)> = toasts.read(cx).queue.history.iter().filter(|t| t.id > since).map(|t| (t.id, t.kind)).collect();
        if let Some((last, _)) = new.last() {
            hub.toast = *last;
        }
        for (id, kind) in new {
            let kind = format!("{kind:?}").to_lowercase();
            self.change(Topic::Toasts, id.to_string(), Change { workspace: None, terminal: None, data: json!({ "id": id, "kind": kind }) }, cx);
        }
    }
}

/// Whether `who` may hear `change`: its own workspace's, and output of a terminal only
/// when it's its own.
fn hears(who: &Who, change: &Change) -> bool {
    let Who::Terminal { workspace, n, .. } = who else { return false };
    change.workspace.as_ref().is_none_or(|w| w == workspace) && change.terminal.is_none_or(|t| t == *n)
}

/// Watches every workspace's runner, deck and notes, the shell, edit mode, the settings
/// and the toasts; what they are now is the baseline (no events for it).
fn hub(cx: &mut Context<ApiState>) -> Hub {
    let mut subs = Vec::new();
    let mut runs = HashMap::new();
    let mut notes = HashMap::new();
    let mut shown = None;
    if let Some(shell) = crate::shell::try_shell(cx) {
        let s = shell.read(cx);
        shown = s.workspaces.get(s.active).map(|w| w.id.clone());
        let list: Vec<_> = s
            .workspaces
            .iter()
            .map(|w| {
                let services = w.services.read(cx);
                (w.id.clone(), w.deck.clone(), services.runner.clone(), services.notes.clone())
            })
            .collect();
        for (ws, deck, runner, n) in list {
            runs.insert(ws.clone(), runner.read(cx).runs().map(|(k, r)| (k.clone(), r.state)).collect());
            let nn = n.read(cx);
            notes.insert(ws.clone(), nn.all().iter().map(|m| (m.id.clone(), nn.revision(&m.id))).collect());
            let id = ws.clone();
            subs.push(cx.observe(&runner, move |this, r, cx| this.runner_changed(&id, &r, cx)));
            let id = ws.clone();
            subs.push(cx.subscribe(&runner, move |this, _, e: &crate::runner::RunnerEvent, cx| {
                if let crate::runner::RunnerEvent::Output { key } = e {
                    this.output(&id, key, cx);
                }
            }));
            let id = ws.clone();
            subs.push(cx.observe(&n, move |this, n, cx| this.notes_changed(&id, &n, cx)));
            let id = ws.clone();
            subs.push(cx.observe(&deck, move |this, _, cx| {
                this.change(Topic::Layout, id.clone(), Change { workspace: Some(id.clone()), terminal: None, data: json!({}) }, cx);
            }));
        }
        subs.push(cx.observe(&shell, |this, _, cx| this.shell_changed(cx)));
    }
    let edit = crate::edit_mode::edit_mode(cx);
    subs.push(cx.observe(&edit, |this, _, cx| this.edit_mode_changed(cx)));
    let settings = crate::app_settings::settings(cx);
    subs.push(cx.observe(&settings, |this, _, cx| {
        this.change(Topic::Settings, String::new(), Change { workspace: None, terminal: None, data: json!({}) }, cx);
    }));
    let toasts = devdeck_ui::toasts(cx);
    let toast = toasts.read(cx).queue.history.back().map_or(0, |t| t.id);
    subs.push(cx.observe(&toasts, |this, t, cx| this.toasts_changed(&t, cx)));
    Hub { _subs: subs, runs, notes, shown, edit_mode: crate::edit_mode::is_on(cx), toast }
}

/// `events.subscribe` / `events.unsubscribe` for the connection of the call.
fn follow_on(cx: &CallCx, topics: &[Topic], on: bool, app: &mut App) -> Result<Subscribed, devdeck_api::ApiError> {
    let state = super::state(app).ok_or_else(|| devdeck_api::ApiError::unavailable("the API is off"))?;
    let conn = cx.conn.ok_or_else(|| devdeck_api::ApiError::invalid("events need a connection"))?;
    state.update(app, |s, cx| s.follow(conn, topics, on, cx)).map(|topics| Subscribed { topics })
}

pub fn subscribe(cx: &CallCx, p: TopicsParams, app: &mut App) -> Result<Subscribed, devdeck_api::ApiError> {
    follow_on(cx, &p.topics, true, app)
}

pub fn unsubscribe(cx: &CallCx, p: TopicsParams, app: &mut App) -> Result<Subscribed, devdeck_api::ApiError> {
    follow_on(cx, &p.topics, false, app)
}

impl ApiState {
    /// Whether the models are being watched (some connection follows a topic).
    pub fn watching(&self) -> bool {
        self.hub.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::sync_channel;

    fn read(rx: &std::sync::mpsc::Receiver<Vec<u8>>) -> Vec<Value> {
        rx.try_iter().map(|l| serde_json::from_slice(&l).unwrap()).collect()
    }

    #[test]
    fn a_full_queue_drops_events_and_says_so_once_there_is_room() {
        let (tx, rx) = sync_channel(2);
        let mut out = Outbox::new(tx);
        for _ in 0..3 {
            out.push(Topic::Layout, json!({}));
        }
        let got = read(&rx);
        assert_eq!(got.iter().map(|m| m["params"]["seq"].as_u64().unwrap()).collect::<Vec<_>>(), [1, 2], "the third was dropped");
        out.push(Topic::Layout, json!({}));
        let got = read(&rx);
        assert_eq!(got[0]["method"], "events.lagged");
        assert_eq!(got[0]["params"]["seq"], 3, "the first one missed");
        assert_eq!((got[1]["method"].as_str(), got[1]["params"]["seq"].as_u64()), (Some("event"), Some(4)));
    }
}

