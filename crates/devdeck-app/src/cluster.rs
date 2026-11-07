//! The cluster of one workspace (R7), as views see it: deployments per env, pods,
//! metrics, rollouts and devdeck's deploy history. A [`ClusterDriver`] feeds it: the
//! fake one for the demo ([`crate::fake_cluster`]), kube-rs for a configured workspace
//! ([`crate::kube_cluster`]).
//!
//! An env is watched from the first time something asks for it ([`Cluster::want`]) until
//! the cluster goes or its config changes: an idle watch costs nothing, and lists show at
//! once when one comes back (user, 2026-11-07). A lost connection keeps the last list,
//! with the problem next to it.
//!
//! Pods and metrics: [`crate::cluster_pods`]. Rollouts tick on their own timer while one
//! runs, in any workspace; they're simulated (the fake's) until real writes (M3 step 5).

use devdeck_core::cluster::{DeployRecord, Deployment, Env, PodPhase, Remote, Rollout, RolloutSim, check_write, set_image_command};
pub use crate::cluster_pods::{MetricsRead, PodScope, PodUpdate, PodsInterest, Sampling};
pub use devdeck_kube::EnvUpdate;
use gpui::{AppContext, Context, Task};
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use std::time::Duration;

pub use crate::fake_cluster::{LOAD_DELAY, NO_METRICS_ENV, UNREACHABLE_ENV};

/// One replica rolls per step.
pub const ROLLOUT_STEP: Duration = Duration::from_secs(1);
/// The fake's metrics pace.
pub const METRICS_TICK: Duration = Duration::from_secs(2);

/// What feeds a [`Cluster`].
pub trait ClusterDriver {
    /// Prepares what watches need (the kubeconfigs), then restarts the watches that
    /// changed ([`Cluster::restart`]). Called when the cluster starts and when its
    /// config or kubeconfigs change; the cluster keeps the task, so a newer start drops an
    /// older one still running.
    fn start(self: Rc<Self>, _cx: &mut Context<Cluster>) -> Option<Task<()>> {
        None
    }
    /// Starts feeding `env` into the cluster ([`Cluster::apply`]); dropping the task stops
    /// it. `None`: not yet (the driver restarts it once [`ClusterDriver::start`] is done).
    fn watch(&self, env: &str, cx: &mut Context<Cluster>) -> Option<Task<()>>;
    /// The namespace `env`'s resources are in.
    fn namespace(&self, env: &str) -> String;
    /// Why deploys can't be made, if they can't.
    fn writes(&self) -> Result<(), String>;
    /// Why `env`'s metrics can't be read, if they can't (R7.8).
    fn metrics(&self, _env: &str) -> Result<(), String> {
        Ok(())
    }
    /// Starts feeding `scope`'s pods in `env` ([`Cluster::apply_pods`]), filtered by
    /// `selector` when given; dropping the task stops it. `None`: this driver doesn't
    /// watch pods (the fake loads them with the deployments).
    fn watch_pods(&self, _env: &str, _scope: PodScope, _selector: Option<&str>, _cx: &mut Context<Cluster>) -> Option<Task<()>> {
        None
    }
    /// Reads `env`'s metrics once ([`Cluster::apply_metrics`]).
    fn sample(&self, _env: &str, _cx: &mut Context<Cluster>) -> Option<Task<()>> {
        None
    }
    /// How often metrics are read while someone looks.
    fn metrics_every(&self) -> Duration {
        Duration::from_secs(15)
    }
    /// Why logs aren't shown, if they aren't.
    fn logs(&self) -> Result<(), String> {
        Ok(())
    }
}

/// An env as last heard of.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EnvState {
    /// By name; `None` until a first list came.
    pub deployments: Option<BTreeMap<String, Deployment>>,
    /// What's wrong now; a list kept from before stays shown with it.
    pub problem: Option<String>,
}

type Key = (String, String);

pub struct Cluster {
    /// The workspace's envs, alphabetical.
    pub envs: Vec<Env>,
    pub(crate) driver: Rc<dyn ClusterDriver>,
    states: BTreeMap<String, EnvState>,
    /// The envs something asked for, watched from then on.
    wanted: BTreeSet<String>,
    /// Every env is wanted, those added later too (the matrix).
    all: bool,
    watches: BTreeMap<String, Task<()>>,
    /// The driver's [`ClusterDriver::start`] under way.
    loading: Option<Task<()>>,
    /// What reading the cluster's config found wrong (a broken kubeconfig), shown with
    /// the workspace's problems.
    pub problems: Vec<String>,
    pub(crate) pod: crate::cluster_pods::PodState,
    rollouts: BTreeMap<Key, RolloutSim>,
    history: BTreeMap<Key, Vec<DeployRecord>>,
    rolling: Option<Task<()>>,
    /// Metrics reads started (tests check sampling stops).
    pub metric_ticks: usize,
    /// A fixed clock (epoch seconds) that rollouts advance: the demo's, so snapshots
    /// don't change. `None`: the system's.
    clock: Option<u64>,
}

impl Cluster {
    pub fn new(envs: Vec<Env>, driver: Rc<dyn ClusterDriver>, clock: Option<u64>, cx: &mut Context<Self>) -> Self {
        let loading = driver.clone().start(cx);
        Self {
            envs,
            driver,
            states: BTreeMap::new(),
            wanted: BTreeSet::new(),
            all: false,
            watches: BTreeMap::new(),
            loading,
            problems: Vec::new(),
            pod: Default::default(),
            rollouts: BTreeMap::new(),
            history: BTreeMap::new(),
            rolling: None,
            metric_ticks: 0,
            clock,
        }
    }

    /// Another driver: every env still wanted is watched again, its list kept meanwhile.
    pub fn set_driver(&mut self, envs: Vec<Env>, driver: Rc<dyn ClusterDriver>, cx: &mut Context<Self>) {
        self.set_envs(envs, cx);
        // Pods are the driver's: a new one reads them again.
        self.forget_pods(&|_| false);
        self.watches.clear();
        self.driver = driver;
        self.problems.clear();
        self.loading = self.driver.clone().start(cx);
        self.restart(self.wanted.clone(), cx);
        self.reinstate_pods(cx);
        cx.notify();
    }

    /// The workspace's envs changed: what's gone is forgotten and no longer watched, what
    /// stays keeps its watch (a protected flag changes nothing else), a new env is watched
    /// when every env is wanted.
    pub fn set_envs(&mut self, envs: Vec<Env>, cx: &mut Context<Self>) {
        let names: BTreeSet<String> = envs.iter().map(|e| e.name.clone()).collect();
        self.states.retain(|e, _| names.contains(e));
        self.wanted.retain(|e| names.contains(e));
        self.watches.retain(|e, _| names.contains(e));
        self.forget_pods(&|e| names.contains(e));
        self.envs = envs;
        if self.all {
            self.want_all(cx);
        }
        self.reinstate_pods(cx);
        cx.notify();
    }

    /// No envs any more (the workspace lost its `[cluster]`): nothing is watched or loaded.
    /// Views keep the entity: the cluster coming back fills it again.
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.envs.clear();
        self.states.clear();
        self.wanted.clear();
        self.watches.clear();
        self.loading = None;
        self.forget_pods(&|_| false);
        self.problems.clear();
        cx.notify();
    }

    /// Reads the driver's config again (it changed, or a kubeconfig did); watches whose
    /// target changed restart.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.retry_metrics();
        self.loading = self.driver.clone().start(cx);
    }

    /// Watches every env, now and as they're added.
    pub fn want_all(&mut self, cx: &mut Context<Self>) {
        self.all = true;
        let envs: Vec<String> = self.envs.iter().map(|e| e.name.clone()).collect();
        for e in envs {
            self.want(&e, cx);
        }
    }

    pub fn env(&self, name: &str) -> Option<&Env> {
        self.envs.iter().find(|e| e.name == name)
    }

    /// Starts watching `env` the first time it's asked for.
    pub fn want(&mut self, env: &str, cx: &mut Context<Self>) {
        if self.env(env).is_none() || !self.wanted.insert(env.to_string()) {
            return;
        }
        self.states.entry(env.to_string()).or_default();
        self.start_watch(env, cx);
    }

    /// Watches `envs` again (those that are wanted), keeping what they showed.
    pub fn restart(&mut self, envs: impl IntoIterator<Item = String>, cx: &mut Context<Self>) {
        for env in envs {
            if self.wanted.contains(&env) {
                self.watches.remove(&env);
                self.start_watch(&env, cx);
                self.restart_pods(&env, cx);
            }
        }
    }

    pub fn wanted(&self) -> &BTreeSet<String> {
        &self.wanted
    }

    fn start_watch(&mut self, env: &str, cx: &mut Context<Self>) {
        let driver = self.driver.clone();
        if let Some(task) = driver.watch(env, cx) {
            self.watches.insert(env.to_string(), task);
        }
    }

    /// What a watch heard. Nothing renders when nothing changed.
    pub fn apply(&mut self, env: &str, update: EnvUpdate, cx: &mut Context<Self>) {
        let Some(state) = self.states.get_mut(env) else { return };
        let changed = match update {
            EnvUpdate::Deployments(list) => {
                let map: BTreeMap<String, Deployment> = list.into_iter().map(|d| (d.name.clone(), d)).collect();
                let changed = state.deployments.as_ref() != Some(&map);
                state.deployments = Some(map);
                if changed {
                    // A service's watch waits for its deployment's selector.
                    self.refresh_pods(env, cx);
                }
                changed
            }
            EnvUpdate::Problem(p) => {
                let changed = state.problem != p;
                state.problem = p;
                changed
            }
        };
        if changed {
            cx.notify();
        }
    }

    /// What reading the driver's config found wrong; new problems are said once (a toast),
    /// and stay listed with the workspace's problems.
    pub fn set_problems(&mut self, problems: Vec<String>, cx: &mut Context<Self>) {
        if self.problems == problems {
            return;
        }
        let new: Vec<&String> = problems.iter().filter(|p| !self.problems.contains(p)).collect();
        let text = match new.as_slice() {
            [] => None,
            [one] => Some(one.to_string()),
            more => Some(format!("{} problems reading the kubeconfigs: see Problems", more.len())),
        };
        if let Some(text) = text {
            devdeck_ui::toasts(cx).update(cx, |t, cx| t.push(devdeck_ui::ToastKind::Warn, text, vec![], cx));
        }
        self.problems = problems;
        cx.notify();
    }

    pub fn state(&self, env: &str) -> Option<&EnvState> {
        self.states.get(env)
    }

    /// The deployments as known now: loading, the problem when there's no list, or the
    /// list (see [`Cluster::problem`] for a problem next to it).
    pub fn peek(&self, env: &str) -> Remote<&BTreeMap<String, Deployment>> {
        match self.states.get(env) {
            Some(EnvState { deployments: Some(m), .. }) => Remote::Ready(m),
            Some(EnvState { deployments: None, problem: Some(p) }) => Remote::Error(p.clone()),
            _ => Remote::Loading,
        }
    }

    /// What's wrong with `env` now, list or not.
    pub fn problem(&self, env: &str) -> Option<&str> {
        self.states.get(env)?.problem.as_deref()
    }

    pub fn deployment(&self, env: &str, name: &str) -> Option<&Deployment> {
        self.states.get(env)?.deployments.as_ref()?.get(name)
    }

    /// Every deployment name of every env heard of, alphabetical (the matrix's rows).
    pub fn names(&self) -> Vec<&str> {
        let names: BTreeSet<&str> = self.states.values().filter_map(|s| s.deployments.as_ref()).flat_map(|m| m.keys().map(String::as_str)).collect();
        names.into_iter().collect()
    }

    /// Now, in epoch seconds (ages are computed against it).
    pub fn now_s(&self) -> u64 {
        self.clock.unwrap_or_else(|| std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()))
    }

    pub fn rollout(&self, env: &str, name: &str) -> Option<&Rollout> {
        self.rollouts.get(&(env.to_string(), name.to_string())).map(|r| &r.state)
    }

    pub fn history(&self, env: &str, name: &str) -> &[DeployRecord] {
        self.history.get(&(env.to_string(), name.to_string())).map_or(&[], |h| h.as_slice())
    }

    /// Why logs aren't shown, if they aren't.
    pub fn logs(&self) -> Result<(), String> {
        self.driver.logs()
    }

    /// Why deploys can't be made here, if they can't.
    pub fn writes(&self) -> Result<(), String> {
        self.driver.writes()
    }

    /// Tags of `name` on every loaded env plus devdeck's history, newest history first,
    /// without the current one (tag editor suggestions).
    pub fn tag_suggestions(&self, env: &str, name: &str) -> Vec<(String, String)> {
        let current = self.deployment(env, name).map(Deployment::tag);
        let mut out: Vec<(String, String)> = Vec::new();
        for (e, state) in &self.states {
            if let Some(d) = state.deployments.as_ref().and_then(|m| m.get(name))
                && e != env
            {
                out.push((d.tag(), format!("on {e}")));
            }
        }
        for r in self.history(env, name).iter().rev() {
            out.push((r.from.clone(), "previous".into()));
        }
        let mut seen = BTreeSet::new();
        out.retain(|(t, _)| Some(t) != current.as_ref() && seen.insert(t.clone()));
        out
    }

    /// The command a tag change would run.
    pub fn command_for(&self, env: &str, name: &str, tag: &str) -> Option<String> {
        self.deployment(env, name).map(|d| set_image_command(&self.driver.namespace(env), d, tag))
    }

    /// Starts a rollout to `tag`. Refuses where writes aren't possible, on a protected env
    /// that isn't unlocked, or while another rollout of the same service runs.
    pub fn set_image(&mut self, env: &str, name: &str, tag: &str, unlocked: bool, cx: &mut Context<Self>) -> Result<(), String> {
        self.driver.writes()?;
        let e = self.env(env).cloned().ok_or_else(|| format!("unknown env {env}"))?;
        check_write(&e, unlocked)?;
        let key = (env.to_string(), name.to_string());
        if self.rollouts.get(&key).is_some_and(|r| !r.finished()) {
            return Err("a rollout is in progress".into());
        }
        let d = self.deployment(env, name).ok_or_else(|| format!("{name} isn't deployed in {env}"))?;
        if d.tag() == tag {
            return Err(format!("{name} already runs {tag}"));
        }
        let sim = RolloutSim::start(&d.tag(), tag, d.replicas);
        self.rollouts.insert(key, sim);
        self.ensure_rolling(cx);
        cx.notify();
        Ok(())
    }

    /// Goes back to the tag before the last rollout.
    pub fn undo(&mut self, env: &str, name: &str, unlocked: bool, cx: &mut Context<Self>) -> Result<(), String> {
        let target = self.rollout(env, name).and_then(|r| r.undo_target()).map(str::to_string).ok_or("nothing to undo")?;
        if self.deployment(env, name).is_some_and(|d| d.tag() == target) {
            // A failed rollout never replaced the tag: clear the failure.
            self.rollouts.remove(&(env.to_string(), name.to_string()));
            self.roll_pods(env, name, cx);
            cx.notify();
            return Ok(());
        }
        self.set_image(env, name, &target, unlocked, cx)
    }

    fn ensure_rolling(&mut self, cx: &mut Context<Self>) {
        if self.rolling.is_some() {
            return;
        }
        self.rolling = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(ROLLOUT_STEP).await;
                let more = this.update(cx, |c, cx| c.step_rollouts(cx)).unwrap_or(false);
                if !more {
                    let _ = this.update(cx, |c, _| c.rolling = None);
                    return;
                }
            }
        }));
    }

    /// Advances every running rollout one step; returns whether any still runs.
    fn step_rollouts(&mut self, cx: &mut Context<Self>) -> bool {
        if let Some(now) = &mut self.clock {
            *now += ROLLOUT_STEP.as_secs();
        }
        let now = self.now_s();
        let keys: Vec<Key> = self.rollouts.iter().filter(|(_, r)| !r.finished()).map(|(k, _)| k.clone()).collect();
        for (env, name) in keys {
            let repo = self.deployment(&env, &name).map(|d| d.main().image.repo.clone()).unwrap_or_default();
            let Some(sim) = self.rollouts.get_mut(&(env.clone(), name.clone())) else { continue };
            sim.step(&repo);
            let state = sim.state.clone();
            match &state {
                Rollout::Done { from, to } => {
                    if let Some(d) = self.states.get_mut(&env).and_then(|s| s.deployments.as_mut()).and_then(|m| m.get_mut(&name)) {
                        d.main_mut().image.tag = Some(to.clone());
                        d.ready = d.replicas;
                        d.updated = d.replicas;
                        d.since = Some(now);
                    }
                    self.record(&env, &name, from, to, "ready");
                }
                Rollout::Failed { from, to, reason } => {
                    let outcome = format!("failed: {}", reason.split(':').next().unwrap_or(reason));
                    self.record(&env, &name, from, to, &outcome);
                }
                Rollout::Progressing { .. } => {}
            }
            self.roll_pods(&env, &name, cx);
        }
        cx.notify();
        self.rollouts.values().any(|r| !r.finished())
    }

    fn record(&mut self, env: &str, name: &str, from: &str, to: &str, outcome: &str) {
        let at_s = self.now_s();
        self.history.entry((env.into(), name.into())).or_default().push(DeployRecord { at_s, from: from.into(), to: to.into(), outcome: outcome.into() });
    }

    /// Pods follow the deployment and its rollout: new-tag pods appear as it progresses; a
    /// failed one leaves a pod stuck pulling the image.
    fn roll_pods(&mut self, env: &str, name: &str, cx: &mut Context<Self>) {
        let Some(d) = self.deployment(env, name).cloned() else { return };
        let now = self.now_s();
        let mut pods = devdeck_core::demo::pods(env, &d);
        match self.rollout(env, name) {
            Some(Rollout::Progressing { to, updated, .. }) => {
                for p in pods.iter_mut().take(*updated as usize) {
                    p.tag = to.clone();
                    p.started = Some(now);
                }
            }
            Some(Rollout::Failed { to, .. }) => {
                let mut stuck = pods[0].clone();
                stuck.name = format!("{}-pull", stuck.name);
                stuck.tag = to.clone();
                stuck.phase = PodPhase::Waiting("ImagePullBackOff".into());
                stuck.ready = false;
                stuck.restarts = 0;
                stuck.started = None;
                stuck.last_termination = None;
                pods.push(stuck);
            }
            _ => {}
        }
        self.replace_pods(env, name, pods, cx);
    }
}

pub fn new_cluster(envs: Vec<Env>, driver: Rc<dyn ClusterDriver>, clock: Option<u64>, cx: &mut gpui::App) -> gpui::Entity<Cluster> {
    cx.new(|cx| Cluster::new(envs, driver, clock, cx))
}
