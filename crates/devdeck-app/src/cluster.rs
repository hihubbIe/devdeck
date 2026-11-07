//! The cluster of one workspace (R7), as views see it: deployments per env, pods,
//! metrics, rollouts and devdeck's deploy history. M1 fakes it here (loading delays, a
//! failing env, rollouts on a timer); M3 fills the same state from kube-rs.
//!
//! Metrics are sampled only while someone holds a [`Sampling`] handle (a visible service
//! view); rollouts tick on their own timer while one runs, in any workspace.

use devdeck_core::cluster::{
    DeployRecord, Deployment, Env, Pod, PodPhase, Remote, Rollout, RolloutSim, Usage, check_write, set_image_command, walk,
};
use devdeck_core::demo::{self, ClusterSpec};
use gpui::{AppContext, Context, Task};
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::Duration;

/// How long the fake API takes to list an env.
pub const LOAD_DELAY: Duration = Duration::from_millis(300);
/// One replica rolls per step.
pub const ROLLOUT_STEP: Duration = Duration::from_secs(1);
/// Metrics refresh while sampled.
pub const METRICS_TICK: Duration = Duration::from_secs(2);
/// The fake env whose API server can't be reached.
pub const UNREACHABLE_ENV: &str = "dev36";
/// The fake env where metrics are forbidden (R7.8).
pub const NO_METRICS_ENV: &str = "lab";

/// Held by whoever wants live metrics; dropping it stops the sampling.
pub struct Sampling(#[allow(dead_code)] Rc<()>);

type Key = (String, String);

pub struct Cluster {
    pub spec: ClusterSpec,
    deployments: BTreeMap<String, Remote<BTreeMap<String, Deployment>>>,
    pods: BTreeMap<Key, Vec<Pod>>,
    rollouts: BTreeMap<Key, RolloutSim>,
    history: BTreeMap<Key, Vec<DeployRecord>>,
    seed: u64,
    samplers: Rc<()>,
    sampling: Option<Task<()>>,
    rolling: Option<Task<()>>,
    loads: Vec<Task<()>>,
    /// Metrics ticks done (tests check sampling stops).
    pub metric_ticks: usize,
    /// Fake clock for history entries, seconds.
    now_s: u64,
}

impl Cluster {
    pub fn new(spec: ClusterSpec) -> Self {
        Self {
            spec,
            deployments: BTreeMap::new(),
            pods: BTreeMap::new(),
            rollouts: BTreeMap::new(),
            history: BTreeMap::new(),
            seed: 42,
            samplers: Rc::new(()),
            sampling: None,
            rolling: None,
            loads: Vec::new(),
            metric_ticks: 0,
            now_s: 1_762_500_000,
        }
    }

    pub fn env(&self, name: &str) -> Option<&Env> {
        self.spec.envs.iter().find(|e| e.name == name)
    }

    /// The deployments of an env, starting a load the first time.
    pub fn deployments(&mut self, env: &str, cx: &mut Context<Self>) -> Remote<Vec<Deployment>> {
        if !self.deployments.contains_key(env) && self.env(env).is_some() {
            self.deployments.insert(env.into(), Remote::Loading);
            let env = env.to_string();
            self.loads.push(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(LOAD_DELAY).await;
                let _ = this.update(cx, |c, cx| c.loaded(&env, cx));
            }));
        }
        self.peek(env)
    }

    /// The deployments as known now, without starting a load.
    pub fn peek(&self, env: &str) -> Remote<Vec<Deployment>> {
        match self.deployments.get(env) {
            None | Some(Remote::Loading) => Remote::Loading,
            Some(Remote::Error(e)) => Remote::Error(e.clone()),
            Some(Remote::Ready(m)) => Remote::Ready(m.values().cloned().collect()),
        }
    }

    fn loaded(&mut self, env: &str, cx: &mut Context<Self>) {
        let state = if env == UNREACHABLE_ENV {
            Remote::Error(format!("cannot reach the {env} API server: connection refused"))
        } else {
            let map: BTreeMap<String, Deployment> =
                self.spec.services.iter().filter_map(|s| demo::deployment(env, s)).map(|d| (d.name.clone(), d)).collect();
            for d in map.values() {
                self.pods.insert((env.into(), d.name.clone()), demo::pods(env, d));
            }
            Remote::Ready(map)
        };
        self.deployments.insert(env.into(), state);
        cx.notify();
    }

    pub fn deployment(&self, env: &str, name: &str) -> Option<&Deployment> {
        match self.deployments.get(env) {
            Some(Remote::Ready(m)) => m.get(name),
            _ => None,
        }
    }

    pub fn pods(&self, env: &str, name: &str) -> &[Pod] {
        self.pods.get(&(env.to_string(), name.to_string())).map_or(&[], |p| p.as_slice())
    }

    pub fn find_pod(&self, env: &str, pod: &str) -> Option<(&str, &Pod)> {
        self.pods.iter().filter(|((e, _), _)| e == env).find_map(|((_, n), pods)| pods.iter().find(|p| p.name == pod).map(|p| (n.as_str(), p)))
    }

    pub fn rollout(&self, env: &str, name: &str) -> Option<&Rollout> {
        self.rollouts.get(&(env.to_string(), name.to_string())).map(|r| &r.state)
    }

    pub fn history(&self, env: &str, name: &str) -> &[DeployRecord] {
        self.history.get(&(env.to_string(), name.to_string())).map_or(&[], |h| h.as_slice())
    }

    pub fn metrics_allowed(&self, env: &str) -> Result<(), String> {
        if env == NO_METRICS_ENV {
            Err("metrics forbidden: your role can't read metrics.k8s.io".into())
        } else {
            Ok(())
        }
    }

    /// Tags of `name` on every loaded env plus devdeck's history, newest history first,
    /// without the current one (tag editor suggestions).
    pub fn tag_suggestions(&self, env: &str, name: &str) -> Vec<(String, String)> {
        let current = self.deployment(env, name).map(|d| d.tag.clone());
        let mut out: Vec<(String, String)> = Vec::new();
        for (e, state) in &self.deployments {
            if let Remote::Ready(m) = state
                && let Some(d) = m.get(name)
                && e != env
            {
                out.push((d.tag.clone(), format!("on {e}")));
            }
        }
        for r in self.history(env, name).iter().rev() {
            out.push((r.from.clone(), "previous".into()));
        }
        let mut seen = std::collections::BTreeSet::new();
        out.retain(|(t, _)| Some(t) != current.as_ref() && seen.insert(t.clone()));
        out
    }

    /// The command a tag change would run.
    pub fn command_for(&self, env: &str, name: &str, tag: &str) -> Option<String> {
        self.deployment(env, name).map(|d| set_image_command(env, d, tag))
    }

    /// Starts a rollout to `tag`. Refuses on a protected env that isn't unlocked, or
    /// while another rollout of the same service runs.
    pub fn set_image(&mut self, env: &str, name: &str, tag: &str, unlocked: bool, cx: &mut Context<Self>) -> Result<(), String> {
        let e = self.env(env).cloned().ok_or_else(|| format!("unknown env {env}"))?;
        check_write(&e, unlocked)?;
        let key = (env.to_string(), name.to_string());
        if self.rollouts.get(&key).is_some_and(|r| !r.finished()) {
            return Err("a rollout is in progress".into());
        }
        let d = self.deployment(env, name).ok_or_else(|| format!("{name} isn't deployed in {env}"))?;
        if d.tag == tag {
            return Err(format!("{name} already runs {tag}"));
        }
        let sim = RolloutSim::start(&d.tag, tag, d.replicas);
        self.rollouts.insert(key, sim);
        self.ensure_rolling(cx);
        cx.notify();
        Ok(())
    }

    /// Goes back to the tag before the last rollout.
    pub fn undo(&mut self, env: &str, name: &str, unlocked: bool, cx: &mut Context<Self>) -> Result<(), String> {
        let target = self.rollout(env, name).and_then(|r| r.undo_target()).map(str::to_string).ok_or("nothing to undo")?;
        if let Some(d) = self.deployment(env, name)
            && d.tag == target
        {
            // A failed rollout never replaced the tag: clear the failure.
            self.rollouts.remove(&(env.to_string(), name.to_string()));
            self.refresh_pods(env, name);
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
        self.now_s += ROLLOUT_STEP.as_secs();
        let keys: Vec<Key> = self.rollouts.iter().filter(|(_, r)| !r.finished()).map(|(k, _)| k.clone()).collect();
        for (env, name) in keys {
            let repo = self.deployment(&env, &name).map(|d| d.repo.clone()).unwrap_or_default();
            let Some(sim) = self.rollouts.get_mut(&(env.clone(), name.clone())) else { continue };
            sim.step(&repo);
            let state = sim.state.clone();
            match &state {
                Rollout::Done { from, to } => {
                    if let Some(Remote::Ready(m)) = self.deployments.get_mut(&env)
                        && let Some(d) = m.get_mut(&name)
                    {
                        d.tag = to.clone();
                        d.ready = d.replicas;
                        d.since_s = 0;
                    }
                    self.record(&env, &name, from, to, "ready");
                }
                Rollout::Failed { from, to, reason } => {
                    let outcome = format!("failed: {}", reason.split(':').next().unwrap_or(reason));
                    self.record(&env, &name, from, to, &outcome);
                }
                Rollout::Progressing { .. } => {}
            }
            self.refresh_pods(&env, &name);
        }
        cx.notify();
        self.rollouts.values().any(|r| !r.finished())
    }

    fn record(&mut self, env: &str, name: &str, from: &str, to: &str, outcome: &str) {
        let at_s = self.now_s;
        self.history.entry((env.into(), name.into())).or_default().push(DeployRecord { at_s, from: from.into(), to: to.into(), outcome: outcome.into() });
    }

    /// Pods follow the deployment and its rollout: new-tag pods appear as it progresses; a
    /// failed one leaves a pod stuck pulling the image.
    fn refresh_pods(&mut self, env: &str, name: &str) {
        let Some(d) = self.deployment(env, name).cloned() else { return };
        let mut pods = demo::pods(env, &d);
        match self.rollout(env, name) {
            Some(Rollout::Progressing { to, updated, .. }) => {
                for p in pods.iter_mut().take(*updated as usize) {
                    p.tag = to.clone();
                    p.age_s = 0;
                }
            }
            Some(Rollout::Failed { to, .. }) => {
                let mut stuck = pods[0].clone();
                stuck.name = format!("{}-pull", stuck.name);
                stuck.tag = to.clone();
                stuck.phase = PodPhase::Waiting("ImagePullBackOff".into());
                stuck.ready = false;
                stuck.restarts = 0;
                stuck.age_s = 0;
                stuck.usage = Usage { cpu_m: 0, mem_mi: 0 };
                pods.push(stuck);
            }
            _ => {}
        }
        self.pods.insert((env.into(), name.into()), pods);
    }

    /// Live metrics while the returned handle lives.
    pub fn sample(&mut self, cx: &mut Context<Self>) -> Sampling {
        let handle = Sampling(self.samplers.clone());
        if self.sampling.is_none() {
            self.sampling = Some(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(METRICS_TICK).await;
                    let more = this
                        .update(cx, |c, cx| {
                            if Rc::strong_count(&c.samplers) <= 1 {
                                c.sampling = None;
                                return false;
                            }
                            c.tick_metrics(cx);
                            true
                        })
                        .unwrap_or(false);
                    if !more {
                        return;
                    }
                }
            }));
        }
        handle
    }

    /// Live sampling handles (the leak run checks they go back to zero).
    pub fn samplers(&self) -> usize {
        Rc::strong_count(&self.samplers) - 1
    }

    pub fn is_sampling(&self) -> bool {
        self.sampling.is_some()
    }

    fn tick_metrics(&mut self, cx: &mut Context<Self>) {
        self.metric_ticks += 1;
        let mut seed = self.seed;
        for ((_, name), pods) in self.pods.iter_mut() {
            let _ = name;
            for p in pods.iter_mut().filter(|p| p.ready) {
                p.usage.cpu_m = walk(p.usage.cpu_m, &mut seed, 5, 480);
                p.usage.mem_mi = walk(p.usage.mem_mi, &mut seed, 60, 500);
            }
        }
        self.seed = seed;
        cx.notify();
    }
}

pub fn new_cluster(spec: ClusterSpec, cx: &mut gpui::App) -> gpui::Entity<Cluster> {
    cx.new(|_| Cluster::new(spec))
}
