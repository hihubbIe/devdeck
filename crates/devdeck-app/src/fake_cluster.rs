//! The demo's cluster driver: an env lists after [`LOAD_DELAY`], dev36's API server can't
//! be reached, metrics are forbidden on lab (R7.8). Deployments and their pods come from
//! `devdeck_core::demo`, every pod of an env at once; its metrics walk; rollouts are the
//! entity's simulation. Logs are made up, one line per stream every [`LOGS_TICK`].

use crate::cluster::{Cluster, ClusterDriver, EnvUpdate, LogFrom, LogLine, LogStream, LogUpdate, METRICS_TICK};
use devdeck_core::demo::{self, ClusterSpec};
use devdeck_core::runs::log_level;
use gpui::{Context, Task};
use std::time::Duration;

/// The fake feed writes one line per stream this often.
pub const LOGS_TICK: Duration = Duration::from_millis(500);
/// Lines a fake stream has to show at once (its tail).
const FAKE_BACKLOG: u32 = 3;

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
