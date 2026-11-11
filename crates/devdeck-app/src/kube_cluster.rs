//! A configured workspace's cluster driver (M3): its envs are contexts of the kubeconfigs
//! in a folder (`[cluster]` in the workspace file), watched with kube-rs
//! ([`devdeck_kube`]). The kubeconfigs are read off the UI thread when the cluster starts,
//! when its config changes ([`KubeCluster::set_config`]) and when a file in the folder
//! changes: only envs whose target changed (another context, folder or file content, a
//! rotated token) reconnect.
//!
//! Deployments, pods (for whoever wants them) and metrics are read, logs streamed (and
//! read back once), the main image set (deploys) or the deployment redeployed, and whether the account may is asked
//! first.

use crate::cluster::{Cluster, ClusterDriver, EnvUpdate, ImageChange, LogFrom, LogStream, MetricsRead, PodScope, Restart};
use devdeck_core::config::ClusterConfig;
use devdeck_kube::{Kubeconfigs, Target};
use gpui::{Context, Task};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

pub struct KubeCluster {
    config: RefCell<ClusterConfig>,
    /// `None` until the folder was read.
    kubeconfigs: RefCell<Option<Kubeconfigs>>,
    /// The target each watched env was started with.
    targets: RefCell<BTreeMap<String, Target>>,
}

impl KubeCluster {
    pub fn new(config: ClusterConfig) -> Self {
        Self { config: RefCell::new(config), kubeconfigs: RefCell::new(None), targets: RefCell::new(BTreeMap::new()) }
    }

    /// The `[cluster]` section changed: takes effect at the cluster's next
    /// [`Cluster::reload`], which reconnects only the envs whose target changed.
    pub fn set_config(&self, config: ClusterConfig) {
        *self.config.borrow_mut() = config;
    }

    fn target(&self, env: &str) -> Option<Result<Target, String>> {
        let config = self.config.borrow();
        Some(self.kubeconfigs.borrow().as_ref()?.target(env, config.context(env)))
    }
}

impl ClusterDriver for KubeCluster {
    fn start(self: Rc<Self>, cx: &mut Context<Cluster>) -> Option<Task<()>> {
        let folder = self.config.borrow().kubeconfigs.clone();
        Some(cx.spawn(async move |this, cx| {
            let (kubeconfigs, problems) = cx.background_executor().spawn(async move { Kubeconfigs::load(&folder) }).await;
            let _ = this.update(cx, |c, cx| {
                // A context in two files matters only to an env that uses it.
                let mut problems = problems;
                problems.extend(c.envs.iter().filter_map(|e| kubeconfigs.duplicate(self.config.borrow().context(&e.name))));
                *self.kubeconfigs.borrow_mut() = Some(kubeconfigs);
                c.set_problems(problems, cx);
                let changed: Vec<String> = c
                    .wanted()
                    .iter()
                    .filter(|env| {
                        let now = self.target(env).and_then(Result::ok);
                        now.is_none() || self.targets.borrow().get(*env) != now.as_ref()
                    })
                    .cloned()
                    .collect();
                c.restart(changed, cx);
            });
        }))
    }

    fn watch(&self, env: &str, cx: &mut Context<Cluster>) -> Option<Task<()>> {
        let env = env.to_string();
        let target = match self.target(&env)? {
            Ok(t) => t,
            Err(problem) => {
                self.targets.borrow_mut().remove(&env);
                return Some(cx.spawn(async move |this, cx| {
                    let _ = this.update(cx, |c, cx| c.apply(&env, EnvUpdate::Problem(Some(problem)), cx));
                }));
            }
        };
        let (updates, handle) = devdeck_kube::watch(&target);
        self.targets.borrow_mut().insert(env.clone(), target);
        Some(cx.spawn(async move |this, cx| {
            // Dropping the task drops the handle, which ends the watch.
            let _handle = handle;
            while let Ok(update) = updates.recv().await {
                if this.update(cx, |c, cx| c.apply(&env, update, cx)).is_err() {
                    return;
                }
            }
        }))
    }

    fn namespace(&self, env: &str) -> String {
        self.targets.borrow().get(env).map_or_else(|| env.to_string(), |t| t.namespace.clone())
    }

    fn writes(&self) -> Result<(), String> {
        Ok(())
    }

    #[allow(clippy::disallowed_methods)] // Reached only from Cluster::deploy, confirmed.
    fn set_image(&self, env: &str, name: &str, change: ImageChange, cx: &mut Context<Cluster>) -> Option<Task<()>> {
        let target = self.target(env).unwrap_or_else(|| Err("the kubeconfigs are still being read".into()));
        Some(answer(env, name, target.map(|t| devdeck_kube::set_image(&t, name, &change)), cx))
    }

    #[allow(clippy::disallowed_methods)] // Reached only from Cluster::deploy_part, confirmed.
    fn set_part_image(&self, env: &str, name: &str, change: ImageChange, cx: &mut Context<Cluster>) -> Option<Task<()>> {
        let target = self.target(env).unwrap_or_else(|| Err("the kubeconfigs are still being read".into()));
        Some(answer(env, name, target.map(|t| devdeck_kube::set_init_image(&t, name, &change)), cx))
    }

    #[allow(clippy::disallowed_methods)] // Reached only from Cluster::redeploy, confirmed.
    fn restart(&self, env: &str, name: &str, restart: Restart, cx: &mut Context<Cluster>) -> Option<Task<()>> {
        let target = self.target(env).unwrap_or_else(|| Err("the kubeconfigs are still being read".into()));
        Some(answer(env, name, target.map(|t| devdeck_kube::restart(&t, name, &restart)), cx))
    }

    fn can_deploy(&self, env: &str, cx: &mut Context<Cluster>) -> Option<Task<()>> {
        let target = self.target(env)?.ok()?;
        let rx = devdeck_kube::can(&target, "patch", "apps", "deployments");
        let env = env.to_string();
        Some(cx.spawn(async move |this, cx| {
            let result = rx.recv().await.unwrap_or_else(|_| Err("the check stopped".into()));
            let _ = this.update(cx, |c, cx| c.access_known(&env, result, cx));
        }))
    }

    fn stream_logs(&self, env: &str, pod: &str, container: &str, from: LogFrom, previous: bool, _cx: &mut Context<Cluster>) -> Result<LogStream, String> {
        let target = self.target(env).unwrap_or_else(|| Err("the kubeconfigs are still being read".into()))?;
        let (updates, handle) = devdeck_kube::logs(&target, pod, container, from, previous);
        Ok(LogStream { updates, keep: Box::new(handle) })
    }

    fn read_logs(&self, env: &str, pod: &str, container: &str, tail: Option<u32>, _cx: &mut Context<Cluster>) -> Result<LogStream, String> {
        let target = self.target(env).unwrap_or_else(|| Err("the kubeconfigs are still being read".into()))?;
        let (updates, handle) = devdeck_kube::read_logs(&target, pod, container, tail);
        Ok(LogStream { updates, keep: Box::new(handle) })
    }

    fn watch_pods(&self, env: &str, scope: PodScope, selector: Option<&str>, cx: &mut Context<Cluster>) -> Option<Task<()>> {
        let target = match self.target(env)? {
            Ok(t) => t,
            // The deployments' watch says why.
            Err(_) => return None,
        };
        let (updates, handle) = devdeck_kube::watch_pods(&target, selector);
        let env = env.to_string();
        Some(cx.spawn(async move |this, cx| {
            let _handle = handle;
            while let Ok(update) = updates.recv().await {
                if this.update(cx, |c, cx| c.apply_pods(&env, scope.clone(), update, cx)).is_err() {
                    return;
                }
            }
        }))
    }

    fn sample(&self, env: &str, cx: &mut Context<Cluster>) -> Option<Task<()>> {
        let target = self.target(env)?.ok()?;
        let read = devdeck_kube::metrics(&target);
        let env = env.to_string();
        Some(cx.spawn(async move |this, cx| {
            let result = match read.recv().await {
                Ok(Ok(usage)) => MetricsRead::Usage(usage),
                Ok(Err(devdeck_kube::MetricsError::Refused(why))) => MetricsRead::Refused(why),
                Ok(Err(devdeck_kube::MetricsError::Failed(why))) => MetricsRead::Failed(why),
                Err(_) => MetricsRead::Failed("the read stopped".into()),
            };
            let _ = this.update(cx, |c, cx| c.apply_metrics(&env, result, cx));
        }))
    }
}

/// Waits for a write's answer and gives it to [`Cluster::write_done`].
fn answer(env: &str, name: &str, write: Result<async_channel::Receiver<Result<u64, devdeck_kube::DeployError>>, String>, cx: &mut Context<Cluster>) -> Task<()> {
    let (env, name) = (env.to_string(), name.to_string());
    cx.spawn(async move |this, cx| {
        let result = match write {
            Ok(rx) => match rx.recv().await {
                Ok(Ok(generation)) => Ok(generation),
                Ok(Err(devdeck_kube::DeployError::Forbidden(why) | devdeck_kube::DeployError::Gone(why) | devdeck_kube::DeployError::Changed(why) | devdeck_kube::DeployError::Other(why))) => Err(why),
                Err(_) => Err("the write stopped".into()),
            },
            Err(why) => Err(why),
        };
        let _ = this.update(cx, |c, cx| c.write_done(&env, &name, result, cx));
    })
}
