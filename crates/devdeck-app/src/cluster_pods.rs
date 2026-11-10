//! A cluster's pods and metrics (R7.1, R7.2): who wants which pods, the watches that feed
//! them, the pods grouped by deployment, and metrics read while someone looks.
//!
//! Pods are watched only for whoever holds a [`PodsInterest`]: a service or pod view for
//! its service, the Cluster list's **watch** toggle for its whole env (user, 2026-11-08).
//! The env-wide watch, when held, replaces the services' own watches once it has listed.
//! Dropping the last handle of a key sends the key on a channel the cluster listens on
//! (no timer), and the watch stops with its pods.
//!
//! Pods are grouped by deployment (their owner) once per update; usage is joined at read
//! time from a per-env map, filled while a [`Sampling`] handle of the env lives (a visible
//! service or pod view), at the driver's pace, never two reads at once. An env whose
//! metrics are refused (403, 404) isn't read again until its target changes.

use crate::cluster::Cluster;
use devdeck_core::cluster::{Deployment, Pod, Progress, Remote, Usage, walk};
use gpui::{Context, Task};
use std::collections::{BTreeMap, BTreeSet};
use std::rc::{Rc, Weak};

/// What a pod watch covers.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PodScope {
    /// Every pod of the env (the list's watch toggle).
    Env,
    /// One deployment's pods (an open service or pod view).
    Service(String),
}

type PodKey = (String, PodScope);

/// What a pod watch hears.
pub use devdeck_kube::PodUpdate;

/// What a metrics read gives.
#[derive(Clone, Debug, PartialEq)]
pub enum MetricsRead {
    Usage(BTreeMap<String, Usage>),
    /// Not allowed or not there (403, 404): not read again until the target changes.
    Refused(String),
    /// Failed this time.
    Failed(String),
}

/// Held by whoever wants a scope's pods; the last one dropped stops their watch.
#[derive(Clone)]
pub struct PodsInterest(#[allow(dead_code)] Rc<Keep>);

struct Keep {
    key: PodKey,
    dropped: async_channel::Sender<PodKey>,
}

impl Drop for Keep {
    fn drop(&mut self) {
        let _ = self.dropped.try_send(self.key.clone());
    }
}

/// Held by whoever wants an env's live metrics; dropping it stops the reading.
pub struct Sampling(#[allow(dead_code)] Rc<()>);

/// Pods of one watch, as last heard of.
#[derive(Default)]
struct Source {
    pods: Option<Vec<Pod>>,
    problem: Option<String>,
    /// Its watch stopped (the toggle off): shown until the services' own watches list.
    stale: bool,
}

#[derive(Default)]
pub(crate) struct PodState {
    interests: BTreeMap<PodKey, Weak<Keep>>,
    /// The driver's pod watches, with the selector each was started with.
    watches: BTreeMap<PodKey, (String, Task<()>)>,
    sources: BTreeMap<PodKey, Source>,
    /// Pods by env and deployment, from the sources in use.
    grouped: BTreeMap<String, BTreeMap<String, Vec<Pod>>>,
    dropped: Option<async_channel::Sender<PodKey>>,
    listen: Option<Task<()>>,
    usage: BTreeMap<String, BTreeMap<String, Usage>>,
    samplers: BTreeMap<String, Rc<()>>,
    sampling: BTreeMap<String, Task<()>>,
    reading: BTreeMap<String, Task<()>>,
    metrics_problems: BTreeMap<String, String>,
    refused: BTreeSet<String>,
    seed: u64,
}

impl Cluster {
    /// A handle on `scope`'s pods in `env`; they're watched while one lives.
    pub fn pods_interest(&mut self, env: &str, scope: PodScope, cx: &mut Context<Self>) -> PodsInterest {
        self.want(env, cx);
        let key = (env.to_string(), scope);
        if let Some(keep) = self.pod.interests.get(&key).and_then(Weak::upgrade) {
            return PodsInterest(keep);
        }
        let dropped = self.dropped_sender(cx);
        let keep = Rc::new(Keep { key: key.clone(), dropped });
        self.pod.interests.insert(key, Rc::downgrade(&keep));
        self.refresh_pods(env, cx);
        PodsInterest(keep)
    }

    /// Live pod interests (the leak run checks they go back to zero).
    pub fn pod_interests(&self) -> usize {
        self.pod.interests.values().filter(|w| w.strong_count() > 0).count()
    }

    /// The pod watches running (tests).
    pub fn pod_watches(&self) -> Vec<(String, PodScope)> {
        self.pod.watches.keys().cloned().collect()
    }

    fn dropped_sender(&mut self, cx: &mut Context<Self>) -> async_channel::Sender<PodKey> {
        if let Some(tx) = &self.pod.dropped {
            return tx.clone();
        }
        let (tx, rx) = async_channel::unbounded::<PodKey>();
        self.pod.listen = Some(cx.spawn(async move |this, cx| {
            while let Ok(key) = rx.recv().await {
                if this.update(cx, |c, cx| c.interest_dropped(key, cx)).is_err() {
                    return;
                }
            }
        }));
        self.pod.dropped = Some(tx.clone());
        tx
    }

    fn interest_dropped(&mut self, key: PodKey, cx: &mut Context<Self>) {
        // Taken again meanwhile: still wanted.
        if self.pod.interests.get(&key).is_some_and(|w| w.strong_count() > 0) {
            return;
        }
        self.pod.interests.remove(&key);
        self.refresh_pods(&key.0, cx);
    }

    /// Keeps `env`'s pod watches in line with the interests: the env-wide watch when held
    /// (the services' own ones until it has listed), else one per held service whose
    /// deployment is known, restarted when its selector changed. A source goes with its
    /// watch.
    pub(crate) fn refresh_pods(&mut self, env: &str, cx: &mut Context<Self>) {
        let held = |k: &PodKey| k.0 == env && self.pod.interests.get(k).is_some_and(|w| w.strong_count() > 0);
        let env_key = (env.to_string(), PodScope::Env);
        let env_held = held(&env_key);
        let env_listed = self.pod.sources.get(&env_key).is_some_and(|s| s.pods.is_some());
        let mut want: BTreeMap<PodKey, String> = BTreeMap::new();
        if env_held {
            want.insert(env_key.clone(), String::new());
        }
        if !(env_held && env_listed) {
            for key in self.pod.interests.keys().filter(|k| held(k)) {
                if let (_, PodScope::Service(name)) = key
                    && let Some(d) = self.deployment(env, name)
                {
                    want.insert(key.clone(), d.selector.clone());
                }
            }
        }
        let stopped: Vec<PodKey> = self.pod.watches.iter().filter(|(k, (sel, _))| k.0 == env && want.get(*k) != Some(sel)).map(|(k, _)| k.clone()).collect();
        let services_held = want.keys().any(|k| k.1 != PodScope::Env);
        for key in stopped {
            self.pod.watches.remove(&key);
            if want.contains_key(&key) {
                continue;
            }
            // The env-wide pods stay shown until the services' own watches have listed.
            match self.pod.sources.get_mut(&key) {
                Some(source) if key.1 == PodScope::Env && services_held && source.pods.is_some() => source.stale = true,
                _ => {
                    self.pod.sources.remove(&key);
                }
            }
        }
        let driver = self.driver.clone();
        for (key, selector) in want {
            if self.pod.watches.contains_key(&key) {
                continue;
            }
            let filter = (key.1 != PodScope::Env).then_some(selector.as_str());
            if let Some(task) = driver.watch_pods(env, key.1.clone(), filter, cx) {
                self.pod.watches.insert(key.clone(), (selector, task));
                self.pod.sources.entry(key).or_default().stale = false;
            }
        }
        self.regroup(env, cx);
    }

    /// Metrics refused before are tried again (a reload: permissions may have changed).
    pub(crate) fn retry_metrics(&mut self) {
        self.pod.refused.clear();
    }

    /// Restarts `env`'s pod watches (its target changed) and reads its metrics again.
    pub(crate) fn restart_pods(&mut self, env: &str, cx: &mut Context<Self>) {
        self.pod.watches.retain(|k, _| k.0 != env);
        self.pod.refused.remove(env);
        self.pod.metrics_problems.remove(env);
        self.refresh_pods(env, cx);
    }

    /// Forgets what isn't in `envs` (or everything), keeping the interests: their holders
    /// still live, and get their pods again when the env comes back.
    pub(crate) fn forget_pods(&mut self, keep: &dyn Fn(&str) -> bool) {
        let p = &mut self.pod;
        p.watches.retain(|k, _| keep(&k.0));
        p.sources.retain(|k, _| keep(&k.0));
        p.grouped.retain(|e, _| keep(e));
        p.usage.retain(|e, _| keep(e));
        p.sampling.retain(|e, _| keep(e));
        p.reading.retain(|e, _| keep(e));
        p.metrics_problems.retain(|e, _| keep(e));
        p.refused.retain(|e| keep(e));
    }

    /// Wants the envs of the interests still held and refreshes their pods (after a config
    /// change or the cluster coming back).
    pub(crate) fn reinstate_pods(&mut self, cx: &mut Context<Self>) {
        let envs: BTreeSet<String> = self.pod.interests.iter().filter(|(_, w)| w.strong_count() > 0).map(|(k, _)| k.0.clone()).collect();
        for env in envs {
            if self.env(&env).is_some() {
                self.want(&env, cx);
                self.refresh_pods(&env, cx);
            }
        }
        // Views still sampling an env get their reading back.
        let sampled: Vec<String> = self.pod.samplers.iter().filter(|(e, r)| Rc::strong_count(r) > 1 && !self.pod.sampling.contains_key(*e)).map(|(e, _)| e.clone()).collect();
        for env in sampled {
            if self.env(&env).is_some() {
                self.start_sampling(&env, cx);
            }
        }
    }

    /// What a pod watch heard.
    pub fn apply_pods(&mut self, env: &str, scope: PodScope, update: PodUpdate, cx: &mut Context<Self>) {
        let key = (env.to_string(), scope);
        let Some(source) = self.pod.sources.get_mut(&key) else { return };
        let first = source.pods.is_none();
        let listed = match update {
            PodUpdate::Pods(pods) => {
                source.pods = Some(pods);
                true
            }
            PodUpdate::Problem(p) => {
                if source.problem == p {
                    return;
                }
                source.problem = p;
                cx.notify();
                false
            }
        };
        if listed && first && key.1 != PodScope::Env {
            self.drop_stale_env_pods(env);
        }
        if first && key.1 == PodScope::Env && self.pod.sources.get(&key).is_some_and(|s| s.pods.is_some()) {
            // The env-wide watch listed: the services' own watches can go.
            self.refresh_pods(env, cx);
        } else {
            self.regroup(env, cx);
        }
    }

    /// Drops the stale env-wide pods once every held service's own watch has listed.
    fn drop_stale_env_pods(&mut self, env: &str) {
        let env_key = (env.to_string(), PodScope::Env);
        if !self.pod.sources.get(&env_key).is_some_and(|s| s.stale) {
            return;
        }
        let pending = self.pod.sources.iter().any(|((e, scope), s)| e == env && *scope != PodScope::Env && s.pods.is_none());
        if !pending {
            self.pod.sources.remove(&env_key);
        }
    }

    /// Every pod of `env` as the driver read them at once (the fake loads them so).
    pub fn set_env_pods(&mut self, env: &str, pods: Vec<Pod>, cx: &mut Context<Self>) {
        let source = self.pod.sources.entry((env.to_string(), PodScope::Env)).or_default();
        source.pods = Some(pods);
        self.regroup(env, cx);
    }

    /// Replaces a deployment's pods in `env`'s env-wide source (the fake's rollouts).
    pub(crate) fn replace_pods(&mut self, env: &str, name: &str, pods: Vec<Pod>, cx: &mut Context<Self>) {
        let Some(list) = self.pod.sources.get_mut(&(env.to_string(), PodScope::Env)).and_then(|s| s.pods.as_mut()) else { return };
        list.retain(|p| p.owner.as_deref() != Some(name));
        list.extend(pods);
        list.sort_by(|a, b| a.name.cmp(&b.name));
        self.regroup(env, cx);
    }

    /// Groups `env`'s pods by deployment from the sources in use (the env-wide one once
    /// listed, else the services'); renders only when that changed.
    fn regroup(&mut self, env: &str, cx: &mut Context<Self>) {
        let env_key = (env.to_string(), PodScope::Env);
        let mut grouped: BTreeMap<String, Vec<Pod>> = BTreeMap::new();
        match self.pod.sources.get(&env_key).and_then(|s| s.pods.as_ref()) {
            Some(pods) => {
                for p in pods {
                    if let Some(owner) = &p.owner {
                        grouped.entry(owner.clone()).or_default().push(p.clone());
                    }
                }
            }
            None => {
                for ((e, scope), source) in &self.pod.sources {
                    if let (true, PodScope::Service(name), Some(pods)) = (e == env, scope, &source.pods) {
                        // A selector can catch another deployment's pods: the owner decides.
                        grouped.insert(name.clone(), pods.iter().filter(|p| p.owner.as_deref() == Some(name)).cloned().collect());
                    }
                }
            }
        }
        if self.pod.grouped.get(env) != Some(&grouped) {
            self.pod.grouped.insert(env.to_string(), grouped);
            // Deploys follow their pods.
            self.reassess(env, cx);
            cx.notify();
        }
    }

    /// A deployment's pods as known now (none when not read).
    pub fn pods(&self, env: &str, name: &str) -> &[Pod] {
        self.pod.grouped.get(env).and_then(|g| g.get(name)).map_or(&[], Vec::as_slice)
    }

    /// A deployment's pods: loading, the problem when there are none to show, or the pods.
    pub fn pods_state(&self, env: &str, name: &str) -> Remote<&[Pod]> {
        if let Some(pods) = self.pod.grouped.get(env).and_then(|g| g.get(name)) {
            return Remote::Ready(pods);
        }
        let source = self.pod.sources.get(&(env.to_string(), PodScope::Env)).or_else(|| self.pod.sources.get(&(env.to_string(), PodScope::Service(name.to_string()))));
        match source {
            Some(Source { pods: Some(_), .. }) => Remote::Ready(&[]),
            Some(Source { pods: None, problem: Some(p), .. }) => Remote::Error(p.clone()),
            _ => Remote::Loading,
        }
    }

    /// What's wrong with `env`'s pods now (any source).
    pub fn pods_problem(&self, env: &str) -> Option<&str> {
        self.pod.sources.iter().filter(|((e, _), _)| e == env).find_map(|(_, s)| s.problem.as_deref())
    }

    pub fn find_pod(&self, env: &str, pod: &str) -> Option<(&str, &Pod)> {
        self.pod.grouped.get(env)?.iter().find_map(|(n, pods)| pods.iter().find(|p| p.name == pod).map(|p| (n.as_str(), p)))
    }

    /// The deployment a pod belongs to, from its name (`<deployment>-<hash>-<id>`): the
    /// longest known deployment named so; its owner, once the pod is known.
    pub fn deployment_of_pod(&self, env: &str, pod: &str) -> Option<String> {
        if let Some((name, _)) = self.find_pod(env, pod) {
            return Some(name.to_string());
        }
        let names = self.peek(env).ready().map(|m| m.keys().cloned().collect::<Vec<_>>()).unwrap_or_default();
        names
            .into_iter()
            .filter(|d| pod.strip_prefix(d.as_str()).and_then(|rest| rest.strip_prefix('-')).is_some_and(|rest| rest.split('-').count() == 2))
            .max_by_key(String::len)
    }

    /// When the current version of `d` started running: the earliest start among its
    /// running pods of the newest template, when its pods are known; else the
    /// deployment's own (step 1). `None` while it isn't settled.
    pub fn since(&self, env: &str, d: &Deployment) -> Option<u64> {
        if d.progress != Progress::Settled {
            return None;
        }
        let running: Vec<&Pod> = self.pods(env, &d.name).iter().filter(|p| p.phase == devdeck_core::cluster::PodPhase::Running && p.started.is_some()).collect();
        let Some(newest) = running.iter().max_by_key(|p| p.started) else { return d.since };
        running.iter().filter(|p| p.template == newest.template).filter_map(|p| p.started).min()
    }

    /// A pod's CPU and memory, when read.
    pub fn usage(&self, env: &str, pod: &str) -> Option<Usage> {
        self.pod.usage.get(env)?.get(pod).copied()
    }

    /// Whether `env`'s metrics were read at least once (a pod missing from them has none).
    pub fn usage_read(&self, env: &str) -> bool {
        self.pod.usage.contains_key(env)
    }

    /// Why `env`'s metrics aren't shown, if they aren't (R7.8).
    pub fn metrics_allowed(&self, env: &str) -> Result<(), String> {
        self.driver.metrics(env)?;
        match self.pod.metrics_problems.get(env) {
            Some(p) => Err(p.clone()),
            None => Ok(()),
        }
    }

    /// Live metrics of `env` while the returned handle lives: read at once, then at the
    /// driver's pace.
    pub fn sample(&mut self, env: &str, cx: &mut Context<Self>) -> Sampling {
        let rc = self.pod.samplers.entry(env.to_string()).or_insert_with(|| Rc::new(())).clone();
        if self.pod.sampling.contains_key(env) {
            // Held again while the loop waits: read now, not at its next turn.
            if Rc::strong_count(&rc) == 2 {
                self.read_metrics(env, cx);
            }
        } else {
            self.start_sampling(env, cx);
        }
        Sampling(rc)
    }

    /// The reading loop of `env`: at once, then at the driver's pace, until no handle
    /// is left.
    fn start_sampling(&mut self, env: &str, cx: &mut Context<Self>) {
        {
            let every = self.driver.metrics_every();
            let env_name = env.to_string();
            let task = cx.spawn(async move |this, cx| {
                loop {
                    let more = this
                        .update(cx, |c, cx| {
                            if c.pod.samplers.get(&env_name).is_none_or(|r| Rc::strong_count(r) <= 1) {
                                c.pod.sampling.remove(&env_name);
                                return false;
                            }
                            c.read_metrics(&env_name, cx);
                            true
                        })
                        .unwrap_or(false);
                    if !more {
                        return;
                    }
                    cx.background_executor().timer(every).await;
                }
            });
            self.pod.sampling.insert(env.to_string(), task);
        }
    }

    fn read_metrics(&mut self, env: &str, cx: &mut Context<Self>) {
        if self.pod.reading.contains_key(env) || self.pod.refused.contains(env) {
            return;
        }
        let driver = self.driver.clone();
        if let Some(task) = driver.sample(env, cx) {
            self.metric_ticks += 1;
            self.pod.reading.insert(env.to_string(), task);
        }
    }

    /// What a metrics read gave.
    pub fn apply_metrics(&mut self, env: &str, read: MetricsRead, cx: &mut Context<Self>) {
        self.pod.reading.remove(env);
        let problem = match read {
            MetricsRead::Usage(usage) => {
                if self.pod.usage.get(env) != Some(&usage) {
                    self.pod.usage.insert(env.to_string(), usage);
                    cx.notify();
                }
                None
            }
            MetricsRead::Refused(why) => {
                self.pod.refused.insert(env.to_string());
                Some(why)
            }
            MetricsRead::Failed(why) => Some(why),
        };
        if self.pod.metrics_problems.get(env) != problem.as_ref() {
            match problem {
                Some(p) => self.pod.metrics_problems.insert(env.to_string(), p),
                None => self.pod.metrics_problems.remove(env),
            };
            cx.notify();
        }
    }

    /// Moves the fake's numbers a step (its "read").
    pub(crate) fn walk_usage(&mut self, env: &str, cx: &mut Context<Self>) {
        let mut seed = self.pod.seed.wrapping_add(42);
        let ready: Vec<String> = self.pod.grouped.get(env).into_iter().flat_map(|g| g.values().flatten()).filter(|p| p.ready).map(|p| p.name.clone()).collect();
        let usage = self.pod.usage.entry(env.to_string()).or_default();
        for name in ready {
            let u = usage.entry(name.clone()).or_insert_with(|| devdeck_core::demo::usage(&name));
            u.cpu_m = walk(u.cpu_m, &mut seed, 5, 480);
            u.mem_mi = walk(u.mem_mi, &mut seed, 60, 500);
        }
        self.pod.seed = seed;
        self.pod.reading.remove(env);
        cx.notify();
    }

    /// Live sampling handles, every env (the leak run checks they go back to zero).
    pub fn samplers(&self) -> usize {
        self.pod.samplers.values().map(|r| Rc::strong_count(r) - 1).sum()
    }

    pub fn is_sampling(&self) -> bool {
        !self.pod.sampling.is_empty()
    }
}
