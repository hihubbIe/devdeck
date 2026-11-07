//! What one workspace offers its views: the catalog, the env runs and the cluster list
//! use, the runner and the cluster. Views get it through [`ViewCx`](crate::views::ViewCx).

use crate::cluster::{Cluster, new_cluster};
use crate::runner::{RunKey, Runner, fake_runner};
use devdeck_core::demo::Catalog;
use gpui::{App, AppContext, Context, Entity};
use std::rc::Rc;

pub struct Services {
    pub workspace: String,
    /// The workspace's display name (toasts).
    pub name: String,
    pub catalog: Rc<Catalog>,
    /// The workspace env (title bar `env ▾`), `None` without envs.
    pub env: Option<String>,
    pub runner: Entity<Runner>,
    pub cluster: Option<Entity<Cluster>>,
}

/// A port someone listens on: a configured holder or a running command.
#[derive(Clone, Debug, PartialEq)]
pub struct PortInfo {
    pub port: u16,
    pub holder: String,
    pub command: String,
    pub run: Option<RunKey>,
}

impl Services {
    /// The demo services of a workspace: fake runner (with its autostart runs) and,
    /// when the catalog has one, the fake cluster.
    pub fn demo(workspace: &str, name: &str, env: Option<String>, cx: &mut App) -> Entity<Services> {
        let catalog = Rc::new(devdeck_core::demo::catalog(workspace));
        let env = catalog.cluster.as_ref().map(|c| match env {
            Some(e) if c.envs.iter().any(|x| x.name == e) => e,
            _ => c.default_env.clone(),
        });
        let runner = fake_runner(workspace, catalog.clone(), env.clone(), cx);
        let cluster = catalog.cluster.clone().map(|spec| new_cluster(spec, cx));
        cx.new(|cx| {
            cx.observe(&runner, |_, _, cx| cx.notify()).detach();
            if let Some(c) = &cluster {
                cx.observe(c, |_, _, cx| cx.notify()).detach();
            }
            Services { workspace: workspace.into(), name: name.into(), catalog, env, runner, cluster }
        })
    }

    pub fn set_env(&mut self, env: &str, cx: &mut Context<Self>) {
        let known = self.catalog.cluster.as_ref().is_some_and(|c| c.envs.iter().any(|e| e.name == env));
        if known && self.env.as_deref() != Some(env) {
            self.env = Some(env.into());
            cx.notify();
        }
    }

    pub fn protected_env(&self) -> bool {
        let env = self.env.as_deref();
        self.catalog.cluster.as_ref().is_some_and(|c| c.envs.iter().any(|e| Some(e.name.as_str()) == env && e.protected))
    }

    /// Ports in use, alphabetical by number.
    pub fn ports(&self, cx: &App) -> Vec<PortInfo> {
        let mut out: Vec<PortInfo> = self
            .catalog
            .ports
            .iter()
            .map(|p| PortInfo { port: p.port, holder: p.holder.clone(), command: p.command.clone(), run: None })
            .collect();
        let runner = self.runner.read(cx);
        for (key, port) in runner.running_ports() {
            let line = self.catalog.app(&key.app).and_then(|a| a.command(&key.cmd)).map(|c| c.line.clone()).unwrap_or_default();
            out.push(PortInfo { port, holder: key.title(), command: line, run: Some(key) });
        }
        out.sort_by_key(|p| p.port);
        out
    }
}
