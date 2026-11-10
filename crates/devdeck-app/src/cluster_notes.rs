//! A service's notifications (plan "notifications", step C): what the notify dropdown of
//! a service view subscribes to, and the events of the deploy follow-up
//! ([`crate::cluster_deploys`]) that become toasts.
//!
//! - Subscriptions are per (workspace, env, service): kept by the workspace's cluster,
//!   off at start, kept until devdeck quits (closing the view keeps them).
//! - One notification per (env, service, rollout, kind), whatever caused it: failing and
//!   stuck are one kind, a rollout replaced is no event. A rollout is told apart by a
//!   number it gets when it is spotted (or followed again after a restart).
//! - devdeck's own deploys and redeploys always say how they end (finished, failing or
//!   stuck), ticked or not, and never that they began (the user just did it).
//! - Pods crashing outside a rollout: once per (service, template), from `pod_failure`'s
//!   threshold, until every pod of that template has been ready, its restarts unchanged,
//!   for [`STEADY`] (a pod without a readiness probe turns ready after every restart: that
//!   isn't recovering); checked on the updates that come, no timer. The templates of a
//!   rollout told failing (and of its pods in trouble then) are skipped the same way. A
//!   mark goes once its template is gone (no pod, not the deployment's). A service's pods
//!   are held only while "pods crashing" is ticked.
//! - A rollout followed again after devdeck restarted says nothing of the kind its record
//!   already ended on (a deploy left failing isn't told failing at every launch); one that
//!   finished meanwhile says so once.
//! - Two workspaces on one env notify twice (accepted).

use crate::cluster::{Cluster, PodScope, PodsInterest};
use devdeck_core::cluster::{RolloutState, pod_failure};
use devdeck_core::view_addr::ViewAddr;
use devdeck_ui::{NewToast, ToastKind};
use gpui::Context;
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

/// How long every pod of a template told crashing stays ready, restarts unchanged, before
/// a new crash of it is told again.
pub const STEADY: Duration = Duration::from_secs(600);

type Key = (String, String);
/// A template told crashing: since when every pod of it is ready, and their restarts then.
type Steady = Option<(Instant, u32)>;

/// What a service can notify about, in the dropdown's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Notify {
    /// Another branch deployed (by anyone but devdeck).
    BranchDeployed,
    /// The same branch redeployed (by anyone but devdeck).
    Redeployed,
    /// A rollout finished: every new pod ready.
    Finished,
    /// A rollout failing or stuck.
    Failed,
    /// Pods in trouble outside a rollout.
    Crashing,
}

impl Notify {
    pub const ALL: [Notify; 5] = [Notify::BranchDeployed, Notify::Redeployed, Notify::Finished, Notify::Failed, Notify::Crashing];

    /// Its words in the dropdown.
    pub fn label(self) -> &'static str {
        match self {
            Notify::BranchDeployed => "branch deployed",
            Notify::Redeployed => "redeployed",
            Notify::Finished => "rollout finished",
            Notify::Failed => "deploy failed or stuck",
            Notify::Crashing => "pods crashing",
        }
    }

    /// The kind of toast it makes: its label is drawn in that kind's color.
    pub fn kind(self) -> ToastKind {
        match self {
            Notify::BranchDeployed | Notify::Redeployed => ToastKind::Info,
            Notify::Finished => ToastKind::Ok,
            Notify::Failed | Notify::Crashing => ToastKind::Err,
        }
    }
}

/// An event waiting to be told (it needs the app, which the follow-up doesn't have).
struct Event {
    key: Key,
    what: Notify,
    text: String,
    /// devdeck's own deploy: its outcome is told ticked or not.
    own: bool,
    /// What it marks as told when it is: (rollout, kind) or (template) for a crash.
    mark: Mark,
}

enum Mark {
    Rollout(u64),
    /// A crash of one of the service's pod templates.
    Template(String),
}

#[derive(Default)]
pub(crate) struct NoteState {
    subs: BTreeMap<Key, BTreeSet<Notify>>,
    /// The pods of services whose crashes are watched.
    holding: BTreeMap<Key, PodsInterest>,
    /// Each service's rollout followed now (or last), by number.
    serials: BTreeMap<Key, u64>,
    next: u64,
    /// What was told of each service's current rollout.
    told: BTreeMap<Key, (u64, BTreeSet<Notify>)>,
    /// Templates whose trouble was told.
    crashed: BTreeMap<(Key, String), Steady>,
    /// Rollouts followed again after a restart: the kind their record ended on, not told
    /// again (until their state is another).
    resumed: BTreeMap<Key, Option<Notify>>,
    outbox: Vec<Event>,
}

impl Cluster {
    /// What `name` in `env` notifies about.
    pub fn notify_options(&self, env: &str, name: &str) -> BTreeSet<Notify> {
        self.notes.subs.get(&(env.to_string(), name.to_string())).cloned().unwrap_or_default()
    }

    /// Ticks or unticks `what` for `name` in `env`; "pods crashing" holds its pods.
    pub fn set_notify(&mut self, env: &str, name: &str, what: Notify, on: bool, cx: &mut Context<Self>) {
        let key = (env.to_string(), name.to_string());
        let set = self.notes.subs.entry(key.clone()).or_default();
        if on {
            set.insert(what);
        } else {
            set.remove(&what);
        }
        if set.is_empty() {
            self.notes.subs.remove(&key);
        }
        let crashes = self.notes.subs.get(&key).is_some_and(|s| s.contains(&Notify::Crashing));
        if crashes && !self.notes.holding.contains_key(&key) {
            let interest = self.pods_interest(env, PodScope::Service(name.to_string()), cx);
            self.notes.holding.insert(key, interest);
        } else if !crashes {
            self.notes.holding.remove(&key);
        }
        // Pods already known may be crashing now.
        let now = cx.background_executor().now();
        self.watch_crashes(env, now);
        self.tell(cx);
        cx.notify();
    }

    /// Every subscription: env, service and what it notifies about, alphabetical.
    pub fn subscriptions(&self) -> Vec<(String, String, BTreeSet<Notify>)> {
        self.notes.subs.iter().map(|((e, n), s)| (e.clone(), n.clone(), s.clone())).collect()
    }

    /// Turns every subscription off.
    pub fn notify_none(&mut self, cx: &mut Context<Self>) {
        self.notes.subs.clear();
        self.notes.holding.clear();
        cx.notify();
    }

    /// Whose pods are held for their crashes (tests).
    pub fn crash_watches(&self) -> usize {
        self.notes.holding.len()
    }

    /// Templates marked as told crashing (tests).
    pub fn crash_marks(&self) -> usize {
        self.notes.crashed.len()
    }

    /// A new rollout of `key` was spotted: on the watch now (`resumed` None), or followed
    /// again after a restart with the outcome its record had. It gets a number; one on the
    /// watch now, anyone's but devdeck's, says it began.
    pub(crate) fn rollout_began(&mut self, key: &Key, resumed: Option<&str>) {
        self.notes.next += 1;
        let serial = self.notes.next;
        self.notes.serials.insert(key.clone(), serial);
        self.notes.told.remove(key);
        if let Some(outcome) = resumed {
            let ended = (outcome.starts_with("failing") || outcome.starts_with("stuck")).then_some(Notify::Failed);
            self.notes.resumed.insert(key.clone(), ended);
            return;
        }
        self.notes.resumed.remove(key);
        let Some(r) = self.rollout(&key.0, &key.1) else { return };
        if r.by == "devdeck" {
            return;
        }
        let (what, text) = if r.redeploy() {
            (Notify::Redeployed, format!("{} redeployed by {}", r.to, r.by))
        } else {
            (Notify::BranchDeployed, format!("{} deployed by {} (was {})", r.to, r.by, r.from))
        };
        self.notes.outbox.push(Event { key: key.clone(), what, text, own: false, mark: Mark::Rollout(serial) });
    }

    /// `key`'s rollout changed state: finished, failing or stuck are events.
    pub(crate) fn rollout_moved(&mut self, key: &Key) {
        let Some(r) = self.rollout(&key.0, &key.1).cloned() else { return };
        let serial = self.notes.serials.get(key).copied().unwrap_or(0);
        let what = if r.redeploy() { format!("redeploy of {}", r.to) } else { format!("rollout to {}", r.to) };
        let (kind, text) = match &r.state {
            RolloutState::Done if r.redeploy() => (Notify::Finished, format!("redeployed {}: every pod ready", r.to)),
            RolloutState::Done => (Notify::Finished, format!("now running {} (was {})", r.to, r.from)),
            RolloutState::Failing { ready, of, reason } => (Notify::Failed, format!("{what} failing ({ready}/{of} ready): {reason}")),
            RolloutState::Stuck { reason } => (Notify::Failed, format!("{what} stuck: {reason}")),
            RolloutState::Rolling { .. } | RolloutState::Replaced { .. } => return,
        };
        // Followed again after a restart: what its record ended on was said before.
        if let Some(ended) = self.notes.resumed.get(key) {
            if *ended == Some(kind) {
                return;
            }
            self.notes.resumed.remove(key);
        }
        let own = r.by == "devdeck";
        self.notes.outbox.push(Event { key: key.clone(), what: kind, text, own, mark: Mark::Rollout(serial) });
    }

    /// Pods crashing outside a rollout, for the services that watch them in `env`, at `now`
    /// (the app's clock). A template told crashing may be told again once every pod of it
    /// has been ready, restarts unchanged, for [`STEADY`]; a mark whose template is gone is
    /// dropped.
    pub(crate) fn watch_crashes(&mut self, env: &str, now: Instant) {
        let keys: Vec<Key> = self.notes.subs.iter().filter(|((e, _), s)| e == env && s.contains(&Notify::Crashing)).map(|(k, _)| k.clone()).collect();
        let marks: Vec<((Key, String), Steady)> = self.notes.crashed.iter().filter(|(((e, _), _), _)| e == env).map(|(k, v)| (k.clone(), *v)).collect();
        for ((key, template), steady) in marks {
            let pods = self.pods(&key.0, &key.1);
            let of: Vec<_> = pods.iter().filter(|p| p.template == template).collect();
            let current = self.deployment(&key.0, &key.1).and_then(|d| d.template.clone());
            let mark = (key, template);
            if of.is_empty() {
                // Gone: no pod of it, and the deployment runs another.
                if current.is_some_and(|c| c != mark.1) {
                    self.notes.crashed.remove(&mark);
                }
                continue;
            }
            let restarts: u32 = of.iter().map(|p| p.restarts).sum();
            let ready = of.iter().all(|p| p.ready);
            let next = match steady {
                // Steady long enough: may be told again (now, if it crashes).
                Some((since, _)) if now.saturating_duration_since(since) >= STEADY => {
                    self.notes.crashed.remove(&mark);
                    continue;
                }
                Some((since, r)) if ready && r == restarts => Some((since, r)),
                _ if ready => Some((now, restarts)),
                _ => None,
            };
            self.notes.crashed.insert(mark, next);
        }
        for key in keys {
            // During a rollout, its own events say it.
            if self.rollout(&key.0, &key.1).is_some_and(|r| !r.finished()) {
                continue;
            }
            let Some(image) = self.deployment(&key.0, &key.1).map(|d| d.main().image.raw.clone()) else { continue };
            let mut found: BTreeMap<String, String> = BTreeMap::new();
            for p in self.pods(&key.0, &key.1) {
                if let Some(why) = pod_failure(p, &image) {
                    found.entry(p.template.clone()).or_insert_with(|| format!("pod {}: {why}", p.name));
                }
            }
            for (template, text) in found {
                let told = self.notes.crashed.contains_key(&(key.clone(), template.clone()));
                let waiting = self.notes.outbox.iter().any(|e| e.key == key && matches!(&e.mark, Mark::Template(t) if *t == template));
                if !told && !waiting {
                    self.notes.outbox.push(Event { key: key.clone(), what: Notify::Crashing, text, own: false, mark: Mark::Template(template) });
                }
            }
        }
    }

    /// Tells the events waiting: each once, when ticked (or devdeck's own outcome), as a
    /// toast of the cluster's workspace with Open and, for a failure, Logs.
    pub(crate) fn tell(&mut self, cx: &mut Context<Self>) {
        for e in std::mem::take(&mut self.notes.outbox) {
            let ticked = self.notes.subs.get(&e.key).is_some_and(|s| s.contains(&e.what));
            if !ticked && !(e.own && matches!(e.what, Notify::Finished | Notify::Failed)) {
                continue;
            }
            match &e.mark {
                Mark::Rollout(serial) => {
                    let told = self.notes.told.entry(e.key.clone()).or_insert_with(|| (*serial, BTreeSet::new()));
                    if told.0 != *serial {
                        *told = (*serial, BTreeSet::new());
                    }
                    if !told.1.insert(e.what) {
                        continue;
                    }
                    // Its pods' trouble was said with it: its template, and those of the pods
                    // in trouble now (the template may be unknown or stale).
                    if e.what == Notify::Failed {
                        let mut templates: BTreeSet<String> = BTreeSet::new();
                        if let Some(d) = self.deployment(&e.key.0, &e.key.1) {
                            templates.extend(d.template.clone());
                            let image = d.main().image.raw.clone();
                            templates.extend(self.pods(&e.key.0, &e.key.1).iter().filter(|p| pod_failure(p, &image).is_some()).map(|p| p.template.clone()));
                        }
                        for t in templates {
                            self.notes.crashed.insert((e.key.clone(), t), None);
                        }
                    }
                }
                Mark::Template(t) => {
                    if self.notes.crashed.insert((e.key.clone(), t.clone()), None).is_some() {
                        continue;
                    }
                }
            }
            self.toast(&e, cx);
        }
    }

    fn toast(&self, e: &Event, cx: &mut Context<Self>) {
        let (env, name) = (&e.key.0, &e.key.1);
        let mut toast = NewToast::new(e.what.kind(), e.text.clone()).title(format!("{name} on {env}")).source(self.workspace.clone().map(Into::into));
        if let Some(ws) = &self.workspace {
            let svc = ViewAddr::Svc { env: env.clone(), name: name.clone() }.to_ref();
            toast = toast.note(crate::notifications::reveal_action(ws, &svc, cx));
            if matches!(e.what, Notify::Failed | Notify::Crashing) {
                let logs = ViewAddr::Logs { env: env.clone(), name: name.clone() }.to_ref();
                toast = toast.note(crate::notifications::open_action("Logs", ws, &logs));
            }
        }
        devdeck_ui::toasts(cx).update(cx, |t, cx| t.push_new(toast, cx));
    }
}
