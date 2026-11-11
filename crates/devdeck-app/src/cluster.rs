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
//! Pods and metrics: [`crate::cluster_pods`]; deploys, their follow-up and history:
//! [`crate::cluster_deploys`]; a service's notifications: [`crate::cluster_notes`].

use devdeck_core::cluster::{Deployment, Env, Remote, by_me, field_manager, restart_command, set_image_command};

/// Why nothing can be deployed before the short name is set.
pub const NO_SHORT_NAME: &str = "set your short name in Settings first";
pub use devdeck_kube::{ImageChange, Restart};
pub use crate::cluster_deploys::{Access, WriteState};
pub use crate::cluster_pods::{MetricsRead, PodScope, PodUpdate, PodsInterest, Sampling};
pub use devdeck_kube::{EnvUpdate, LogFrom, LogLine, LogRefusal, LogUpdate};
use gpui::{AppContext, Context, Task};
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use std::time::Duration;

pub use crate::fake_cluster::{LOAD_DELAY, NO_METRICS_ENV, UNREACHABLE_ENV};

/// The fake rolls one replica per step.
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
    /// Sets a deployment's main image (R7.3); the answer goes to [`Cluster::write_done`].
    /// `None`: this driver can't deploy.
    fn set_image(&self, _env: &str, _name: &str, _change: ImageChange, _cx: &mut Context<Cluster>) -> Option<Task<()>> {
        None
    }
    /// Redeploys a deployment (`kubectl rollout restart`, R7.3, R7.5); the answer goes to
    /// [`Cluster::write_done`]. `None`: this driver can't deploy.
    fn restart(&self, _env: &str, _name: &str, _restart: Restart, _cx: &mut Context<Cluster>) -> Option<Task<()>> {
        None
    }
    /// A short name of its own, used when Settings has none (the demo's): who its deploys
    /// are made by.
    fn short_name(&self) -> Option<String> {
        None
    }
    /// Asks whether deployments may be changed in `env`; the answer goes to
    /// [`Cluster::access_known`]. `None`: not asked (the write decides).
    fn can_deploy(&self, _env: &str, _cx: &mut Context<Cluster>) -> Option<Task<()>> {
        None
    }
    /// Streams `container`'s logs of `pod` in `env` from `from`, followed, or its run
    /// before the last restart read once (`previous`). Dropping the stream stops it.
    fn stream_logs(&self, _env: &str, _pod: &str, _container: &str, _from: LogFrom, _previous: bool, _cx: &mut Context<Cluster>) -> Result<LogStream, String> {
        Err("this cluster has no logs".into())
    }
    /// Reads everything the node serves of `container`'s current run in `pod` once, then
    /// [`LogUpdate::Ended`] (or why not). Dropping the stream stops it.
    fn read_logs(&self, _env: &str, _pod: &str, _container: &str, _tail: Option<u32>, _cx: &mut Context<Cluster>) -> Result<LogStream, String> {
        Err("this cluster can't read logs back".into())
    }
}

/// One container's logs as they come (R7.5); what keeps the stream going goes with it.
pub struct LogStream {
    pub updates: async_channel::Receiver<LogUpdate>,
    pub keep: Box<dyn std::any::Any>,
}

/// An env as last heard of.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EnvState {
    /// By name; `None` until a first list came.
    pub deployments: Option<BTreeMap<String, Deployment>>,
    /// What's wrong now; a list kept from before stays shown with it.
    pub problem: Option<String>,
}



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
    pub(crate) dep: crate::cluster_deploys::DeployState,
    pub(crate) notes: crate::cluster_notes::NoteState,
    /// Metrics reads started (tests check sampling stops).
    pub metric_ticks: usize,
    /// A fixed clock (epoch seconds) that rollouts advance: the demo's, so snapshots
    /// don't change. `None`: the system's.
    clock: Option<u64>,
    /// Log streams' generations: the whole cluster's (a new driver, a reload) and each
    /// env's (its target changed).
    log_gens: (u64, BTreeMap<String, u64>),
    /// The workspace it is the cluster of (its id): its notifications are that
    /// workspace's, whichever shows.
    pub workspace: Option<String>,
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
            dep: Default::default(),
            notes: Default::default(),
            metric_ticks: 0,
            clock,
            log_gens: (0, BTreeMap::new()),
            workspace: None,
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
        self.log_gens.0 += 1;
        self.forget_versions(None);
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
        self.log_gens.0 += 1;
        cx.notify();
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
                self.forget_versions(Some(&env));
                *self.log_gens.1.entry(env).or_default() += 1;
                cx.notify();
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
                    self.saw_deployments(env, cx);
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
            let source = self.workspace.clone().map(Into::into);
            devdeck_ui::toasts(cx).update(cx, |t, cx| t.push_in(source, devdeck_ui::ToastKind::Warn, text, vec![], cx));
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

    /// The fixed clock, when there is one (the demo's, tests').
    pub(crate) fn clock(&self) -> Option<u64> {
        self.clock
    }

    /// Now, in epoch seconds (ages are computed against it).
    pub fn now_s(&self) -> u64 {
        self.clock.unwrap_or_else(|| std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()))
    }

    /// Streams a container's logs (see [`ClusterDriver::stream_logs`]).
    pub fn stream_logs(&mut self, env: &str, pod: &str, container: &str, from: LogFrom, previous: bool, cx: &mut Context<Self>) -> Result<LogStream, String> {
        let driver = self.driver.clone();
        driver.stream_logs(env, pod, container, from, previous, cx)
    }

    /// Reads a container's logs back once (see [`ClusterDriver::read_logs`]).
    pub fn read_logs(&mut self, env: &str, pod: &str, container: &str, tail: Option<u32>, cx: &mut Context<Self>) -> Result<LogStream, String> {
        let driver = self.driver.clone();
        driver.read_logs(env, pod, container, tail, cx)
    }

    /// Changes when `env`'s log streams must start again: another driver, its target
    /// changed, or a reload (permissions may have changed).
    pub fn log_generation(&self, env: &str) -> (u64, u64) {
        (self.log_gens.0, self.log_gens.1.get(env).copied().unwrap_or(0))
    }

    /// Why deploys can't be made now, if they can't: the driver can't, edit mode is off
    /// (R5.6), or nobody is named to make them (the short name, Settings). Every write to
    /// the cluster goes through it.
    pub fn writes(&self, cx: &gpui::App) -> Result<(), String> {
        self.driver.writes()?;
        if let Some(why) = crate::edit_mode::refusal(cx) {
            return Err(why);
        }
        self.field_manager(cx).map(|_| ())
    }

    /// Who deploys from here: the short name in Settings, else the driver's own (the demo's).
    pub fn short_name(&self, cx: &gpui::App) -> Option<String> {
        crate::app_settings::short_name(cx).or_else(|| self.driver.short_name())
    }

    /// The field manager this window's writes carry ("<short name> via Kugelblitz"), or why
    /// there is none.
    pub fn field_manager(&self, cx: &gpui::App) -> Result<String, String> {
        self.short_name(cx).map(|n| field_manager(&n)).ok_or_else(|| NO_SHORT_NAME.to_string())
    }

    /// Whether a deploy made by `by` is the user's own (see [`by_me`]).
    pub fn mine(&self, by: &str, cx: &gpui::App) -> bool {
        by_me(by, self.short_name(cx).as_deref())
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

    /// The command a redeploy is the equivalent of.
    pub fn restart_command_for(&self, env: &str, name: &str) -> Option<String> {
        self.deployment(env, name).map(|d| restart_command(&self.driver.namespace(env), d))
    }
}

pub fn new_cluster(envs: Vec<Env>, driver: Rc<dyn ClusterDriver>, clock: Option<u64>, cx: &mut gpui::App) -> gpui::Entity<Cluster> {
    cx.new(|cx| Cluster::new(envs, driver, clock, cx))
}
