//! The demo's cluster driver: an env lists after [`LOAD_DELAY`], dev36's API server can't
//! be reached, metrics are forbidden on lab (R7.8). Deployments and their pods come from
//! `devdeck_core::demo`, every pod of an env at once; its metrics walk; rollouts are the
//! entity's simulation.

use crate::cluster::{Cluster, ClusterDriver, EnvUpdate, METRICS_TICK};
use devdeck_core::demo::{self, ClusterSpec};
use gpui::{Context, Task};
use std::time::Duration;

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
}
