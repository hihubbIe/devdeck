//! A configured workspace's cluster driver (M3): its envs are contexts of the kubeconfigs
//! in a folder (`[cluster]` in the workspace file), watched with kube-rs
//! ([`devdeck_kube`]). The kubeconfigs are read off the UI thread when the cluster starts,
//! when its config changes ([`KubeCluster::set_config`]) and when a file in the folder
//! changes: only envs whose target changed (another context, folder or file content, a
//! rotated token) reconnect.
//!
//! Step 1 reads deployments only: deploys, pods and metrics say why they aren't there.

use crate::cluster::{Cluster, ClusterDriver, EnvUpdate};
use devdeck_core::config::ClusterConfig;
use devdeck_kube::{Kubeconfigs, Target};
use gpui::{Context, Task};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

/// What the kube driver can't do yet.
pub const NOT_YET: &str = "not read yet";
pub const NO_DEPLOYS: &str = "deploys come in a later step";

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
        Err(NO_DEPLOYS.into())
    }

    fn pods(&self) -> Result<(), String> {
        Err(NOT_YET.into())
    }

    fn metrics(&self, _env: &str) -> Result<(), String> {
        Err(NOT_YET.into())
    }
}
