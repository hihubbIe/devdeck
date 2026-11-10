//! The demo's cluster driver: an env lists after [`LOAD_DELAY`], dev36's API server can't
//! be reached, metrics are forbidden on lab (R7.8). Deployments and their pods come from
//! `devdeck_core::demo`, every pod of an env at once; its metrics walk; rollouts are the
//! entity's follow-up of what this driver sends, as from a watch: one replica of the new
//! version per [`ROLLOUT_STEP`] (a redeploy: new pods on the same tag); a tag containing
//! `broken` can't be pulled and keeps failing. Logs are made up, one line per stream every [`LOGS_TICK`]; a read back finds
//! older made-up lines.

use crate::cluster::{Cluster, ClusterDriver, EnvUpdate, ImageChange, LogFrom, LogLine, LogStream, LogUpdate, METRICS_TICK, ROLLOUT_STEP, Restart};
use devdeck_core::cluster::{Deployment, Image, PodPhase, Progress};
use devdeck_core::demo::{self, ClusterSpec};
use devdeck_core::runs::log_level;
use gpui::{AsyncApp, Context, Task, WeakEntity};
use std::time::Duration;

/// The fake feed writes one line per stream this often.
pub const LOGS_TICK: Duration = Duration::from_millis(500);
/// Lines a fake stream has to show at once (its tail).
const FAKE_BACKLOG: u32 = 3;
/// Lines a fake read back finds before a stream's first.
const FAKE_EARLIER: u32 = 2000;

/// How long the fake API takes to list an env.
pub const LOAD_DELAY: Duration = Duration::from_millis(300);
/// The fake env whose API server can't be reached.
pub const UNREACHABLE_ENV: &str = "dev36";
/// The fake env where metrics are forbidden (R7.8).
pub const NO_METRICS_ENV: &str = "lab";

pub struct FakeCluster {
    pub spec: ClusterSpec,
}

impl ClusterDriver for FakeCluster {
    fn watch(&self, env: &str, cx: &mut Context<Cluster>) -> Option<Task<()>> {
        let env = env.to_string();
        let services = self.spec.services.clone();
        Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(LOAD_DELAY).await;
            let _ = this.update(cx, |c, cx| {
                if env == UNREACHABLE_ENV {
                    let problem = format!("cannot reach the {env} API server: connection refused");
                    return c.apply(&env, EnvUpdate::Problem(Some(problem)), cx);
                }
                let deployments: Vec<_> = services.iter().filter_map(|s| demo::deployment(&env, s)).collect();
                let pods = deployments.iter().flat_map(|d| demo::pods(&env, d)).collect();
                c.apply(&env, EnvUpdate::Deployments(deployments), cx);
                c.set_env_pods(&env, pods, cx);
            });
        }))
    }

    fn namespace(&self, env: &str) -> String {
        env.to_string()
    }

    fn writes(&self) -> Result<(), String> {
        Ok(())
    }

    fn metrics(&self, env: &str) -> Result<(), String> {
        if env == NO_METRICS_ENV { Err("metrics forbidden: your role can't read metrics.k8s.io".into()) } else { Ok(()) }
    }

    fn sample(&self, env: &str, cx: &mut Context<Cluster>) -> Option<Task<()>> {
        let env = env.to_string();
        Some(cx.spawn(async move |this, cx| {
            let _ = this.update(cx, |c, cx| c.walk_usage(&env, cx));
        }))
    }

    fn metrics_every(&self) -> Duration {
        METRICS_TICK
    }

    fn can_deploy(&self, env: &str, cx: &mut Context<Cluster>) -> Option<Task<()>> {
        let env = env.to_string();
        Some(cx.spawn(async move |this, cx| {
            let _ = this.update(cx, |c, cx| c.access_known(&env, Ok(true), cx));
        }))
    }

    fn set_image(&self, env: &str, name: &str, change: ImageChange, cx: &mut Context<Cluster>) -> Option<Task<()>> {
        let (env, name) = (env.to_string(), name.to_string());
        Some(cx.spawn(async move |this, cx| {
            let Ok(Some(old)) = this.update(cx, |c, _| c.deployment(&env, &name).cloned()) else { return };
            if old.containers.get(change.index).is_none_or(|c| c.name != change.container || c.image.raw != change.from) {
                let _ = this.update(cx, |c, cx| c.write_done(&env, &name, Err(format!("{name} changed since you confirmed")), cx));
                return;
            }
            let mut new = old.clone();
            new.containers[change.index].image = Image::parse(&change.to);
            let broken = change.to.contains("broken").then(|| change.to.clone());
            roll(&this, cx, &env, &name, old, new, broken).await;
        }))
    }

    /// A redeploy: a new template on the same tag, rolled as a branch change is.
    fn restart(&self, env: &str, name: &str, restart: Restart, cx: &mut Context<Cluster>) -> Option<Task<()>> {
        let (env, name) = (env.to_string(), name.to_string());
        Some(cx.spawn(async move |this, cx| {
            let Ok(Some(old)) = this.update(cx, |c, _| c.deployment(&env, &name).cloned()) else { return };
            let moved = !restart.annotated && old.generation != restart.generation;
            if moved || old.containers.get(restart.index).is_none_or(|c| c.name != restart.container || c.image.raw != restart.image) {
                let _ = this.update(cx, |c, cx| c.write_done(&env, &name, Err(format!("{name} changed since you confirmed")), cx));
                return;
            }
            let mut new = old.clone();
            new.restarted_at = Some(restart.at.clone());
            new.template_annotated = true;
            roll(&this, cx, &env, &name, old, new, None).await;
        }))
    }

    fn stream_logs(&self, _env: &str, pod: &str, _container: &str, from: LogFrom, previous: bool, cx: &mut Context<Cluster>) -> Result<LogStream, String> {
        let (tx, updates) = async_channel::unbounded();
        // `<deployment>-<hash>-<id>`.
        let service = pod.rsplitn(3, '-').nth(2).unwrap_or(pod).to_string();
        let seed = demo::hash(pod);
        let tick = LOGS_TICK.as_nanos() as i64;
        let (mut at, backlog) = match from {
            LogFrom::Tail(n) => (demo::NOW_S as i64 * 1_000_000_000 - tick * i64::from(n.min(FAKE_BACKLOG)), n.min(FAKE_BACKLOG)),
            LogFrom::Since { at, .. } => (at, 0),
        };
        let mut n = seed;
        let mut line = move || {
            n += 1;
            at += tick;
            let text = fake_line(&service, n);
            LogLine { at: Some(at), level: log_level(&text), text }
        };
        let task = cx.background_executor().spawn({
            let executor = cx.background_executor().clone();
            async move {
                let first: Vec<LogLine> = (0..if previous { FAKE_BACKLOG } else { backlog }).map(|_| line()).collect();
                if !first.is_empty() && tx.send(LogUpdate::Lines(first)).await.is_err() {
                    return;
                }
                if previous {
                    let _ = tx.send(LogUpdate::Ended).await;
                    return;
                }
                loop {
                    executor.timer(LOGS_TICK).await;
                    if tx.send(LogUpdate::Lines(vec![line()])).await.is_err() {
                        return;
                    }
                }
            }
        });
        Ok(LogStream { updates, keep: Box::new(task) })
    }

    /// [`FAKE_EARLIER`] made-up lines older than a stream's first, then that stream's
    /// first lines (what's shown lines up with them), then the end.
    fn read_logs(&self, _env: &str, pod: &str, _container: &str, _tail: Option<u32>, _cx: &mut Context<Cluster>) -> Result<LogStream, String> {
        let (tx, updates) = async_channel::unbounded();
        let service = pod.rsplitn(3, '-').nth(2).unwrap_or(pod).to_string();
        let seed = demo::hash(pod);
        let tick = LOGS_TICK.as_nanos() as i64;
        // A stream's first line is the one after `seed`, at `now - tick * (backlog - 1)`.
        let first = demo::NOW_S as i64 * 1_000_000_000 - tick * i64::from(FAKE_BACKLOG - 1);
        let lines: Vec<LogLine> = (0..FAKE_EARLIER + FAKE_BACKLOG)
            .map(|i| {
                let back = i64::from(FAKE_EARLIER) - i64::from(i);
                let n = seed.wrapping_add(1).wrapping_add_signed(-back);
                let text = fake_line(&service, n);
                LogLine { at: Some(first - tick * back), level: log_level(&text), text }
            })
            .collect();
        let _ = tx.try_send(LogUpdate::Lines(lines));
        let _ = tx.try_send(LogUpdate::Ended);
        Ok(LogStream { updates, keep: Box::new(()) })
    }
}

/// Answers devdeck's write of `name` with `new`'s generation (its spec as patched; the
/// generation, revision and template moved here), then rolls it as a watch would show it:
/// one replica of the new version per [`ROLLOUT_STEP`]; a `broken` image can't be pulled
/// and keeps failing.
async fn roll(this: &WeakEntity<Cluster>, cx: &mut AsyncApp, env: &str, name: &str, old: Deployment, mut new: Deployment, broken: Option<String>) {
    new.generation += 1;
    new.observed = new.generation;
    new.revision += 1;
    let settled = Deployment { ready: new.replicas, updated: new.replicas, ..new.clone() };
    let new_pods = demo::pods(env, &settled);
    new.template = new_pods.first().map(|p| p.template.clone());
    let old_pods = demo::pods(env, &old);
    let generation = new.generation;
    let _ = this.update(cx, |c, cx| c.write_done(env, name, Ok(generation), cx));
    let replicas = new.replicas;
    for step in 0..=replicas {
        let done = step == replicas && broken.is_none();
        let mut d = new.clone();
        d.progress = if done { Progress::Settled } else { Progress::Rolling };
        d.updated = if broken.is_some() { 1 } else { step };
        let mut pods: Vec<_> = if broken.is_some() { old_pods.clone() } else { new_pods.iter().take(step as usize).cloned().chain(old_pods.iter().skip(step as usize).cloned()).collect() };
        if let Some(image) = &broken {
            let mut stuck = new_pods[0].clone();
            stuck.name = format!("{}-pull", stuck.name);
            stuck.phase = PodPhase::Waiting("ImagePullBackOff".into());
            stuck.ready = false;
            stuck.restarts = 0;
            stuck.started = None;
            stuck.last_termination = None;
            if let Some(c) = stuck.containers.first_mut() {
                c.ready = false;
                c.restarts = 0;
                c.state = "waiting: ImagePullBackOff".into();
                c.message = Some(format!("rpc error: manifest for {image} not found: manifest unknown"));
                c.last_termination = None;
            }
            pods.push(stuck);
        }
        d.ready = pods.iter().filter(|p| p.ready).count() as u32;
        let alive = this.update(cx, |c, cx| {
            if done {
                d.since = Some(c.now_s());
            }
            let mut list: Vec<Deployment> = c.peek(env).ready().map(|m| m.values().cloned().collect()).unwrap_or_default();
            list.retain(|x| x.name != name);
            list.push(d);
            c.apply(env, EnvUpdate::Deployments(list), cx);
            c.replace_pods(env, name, pods, cx);
        });
        if alive.is_err() || step == replicas || broken.is_some() {
            return;
        }
        cx.background_executor().timer(ROLLOUT_STEP).await;
    }
}

/// A made-up line of `service`'s logs.
fn fake_line(service: &str, n: u64) -> String {
    match n % 7 {
        0 => format!("WRN slow query entities.find {}ms", 400 + n % 500),
        1 => format!("DBG cache hit entity:{n:08x}"),
        2 if n.is_multiple_of(3) => "ERR rabbitmq publish failed: channel closed".into(),
        3 => format!("INF POST /v1/{service} 201 {}ms", 10 + n % 40),
        _ => format!("INF GET /v1/{service}/{n} 200 {}ms", 5 + n % 30),
    }
}
