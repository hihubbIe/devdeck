//! What one workspace offers its views: the catalog, the env runs and the cluster list
//! use, the runner and the cluster. Views get it through [`ViewCx`](crate::views::ViewCx).
//!
//! A workspace comes from the demo or from its config file. A config workspace's catalog
//! is discovery plus the config's overrides, with the machine's tools; it's rebuilt and
//! swapped in when the tools are found, when folders change and when the config changes
//! (the runner swaps too; runs of an app that disappeared keep running). A rescan reads
//! the folders off the UI thread, and swaps only what changed. The first scan, at start,
//! is on the UI thread: the views restored with the window need their apps.
//!
//! A config workspace with `[cluster]` has a cluster read with kube-rs; a changed config
//! changes it in place (open views keep it), and its kubeconfigs folder is watched on its
//! own route ([`Services::kubeconfigs`]).

use crate::cluster::{Cluster, new_cluster};
use crate::fake_cluster::FakeCluster;
use crate::kube_cluster::KubeCluster;
use crate::environment::Environment;
use crate::port_scan::{PortHolder, Ports};
use crate::runner::{RunKey, Runner, fake_runner};
use devdeck_core::catalog::Catalog;
use devdeck_core::config::{WorkspaceConfig, apply_overrides};
use gpui::{App, AppContext, Context, Entity, EventEmitter, Task};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

pub struct Services {
    pub workspace: String,
    pub catalog: Rc<Catalog>,
    /// The workspace env (title bar `env ▾`), `None` without envs.
    pub env: Option<String>,
    pub runner: Entity<Runner>,
    /// `None` without `[cluster]` (no Cluster activity, R2.3).
    pub cluster: Option<Entity<Cluster>>,
    /// A config workspace's cluster and its driver, kept while `[cluster]` is gone: the
    /// one cluster of the workspace, which views keep, refilled when it comes back.
    kube: Option<(Entity<Cluster>, Rc<KubeCluster>)>,
    /// The config it comes from; `None` for the demo.
    pub config: Option<WorkspaceConfig>,
    /// What discovery and the config overrides reported, shown on request.
    pub problems: Vec<String>,
    /// Folders to watch for new projects and changed manifests.
    pub watch: Vec<PathBuf>,
    /// The ports runs listen on, scanned in the background, and the ports they used.
    pub ports: Entity<Ports>,
    /// The rescan under way; a new one replaces it.
    scan: Option<Task<()>>,
    /// Why each app that can't be used can't (`Tools::unusable`), by app id: checked
    /// once per catalog, not on every render.
    pub unusable: BTreeMap<String, String>,
}

/// Why each app of `catalog` that can't be used can't, by app id.
fn unusable(catalog: &Catalog) -> BTreeMap<String, String> {
    catalog.apps.iter().filter_map(|a| Some((a.id.clone(), catalog.tools.unusable(a)?))).collect()
}

/// A rescan finished (the folders to watch may have changed).
pub struct Rescanned;

impl EventEmitter<Rescanned> for Services {}

/// A port someone listens on: a run, a process a port view shows, or a demo holder.
#[derive(Clone, Debug, PartialEq)]
pub struct PortInfo {
    pub port: u16,
    pub holder: String,
    pub command: String,
    /// The run listening, when it's one of ours.
    pub run: Option<RunKey>,
    /// Another process listening, when we can see it (stopped after a confirmation).
    pub pid: Option<u32>,
}

impl Services {
    /// The demo services of a workspace: fake runner (with its autostart runs) and,
    /// when the catalog has one, the fake cluster.
    pub fn demo(workspace: &str, env: Option<String>, cx: &mut App) -> Entity<Services> {
        let catalog = Rc::new(devdeck_core::demo::catalog(workspace));
        let env = catalog.pick_env(env);
        let runner = fake_runner(workspace, catalog.clone(), env.clone(), cx);
        let clock = Some(devdeck_core::demo::NOW_S);
        let cluster = catalog.cluster.clone().map(|spec| new_cluster(catalog.envs.clone(), Rc::new(FakeCluster { spec }), clock, cx));
        let ports = Ports::new(workspace, runner.clone(), None, cx);
        cx.new(|cx| {
            cx.observe(&runner, |_, _, cx| cx.notify()).detach();
            cx.observe(&ports, |_, _, cx| cx.notify()).detach();
            if let Some(c) = &cluster {
                cx.observe(c, |_, _, cx| cx.notify()).detach();
            }
            let unusable = unusable(&catalog);
            Services { workspace: workspace.into(), catalog, env, runner, cluster, kube: None, config: None, problems: Vec::new(), watch: Vec::new(), ports, scan: None, unusable }
        })
    }

    /// A workspace from its config: its apps found in its folders, real runs, its cluster
    /// when it has one. Tools come from `environment` and follow it.
    /// `state_dir` holds what runs leave behind (started process groups, ports seen,
    /// arguments used);
    /// `None` keeps nothing (tests).
    pub fn from_config(config: WorkspaceConfig, env: Option<String>, environment: Entity<Environment>, state_dir: Option<PathBuf>, cx: &mut App) -> Entity<Services> {
        let tools = environment.read(cx).tools();
        let (catalog, problems, watch) = discover(&config, tools);
        let catalog = Rc::new(catalog);
        let env = catalog.pick_env(env);
        let log = state_dir.as_deref().map(devdeck_core::process::GroupLog::new);
        let driver = Rc::new(crate::pty_driver::PtyDriver { env: environment.clone(), log });
        let recent = state_dir.as_deref().map(devdeck_core::recent_args::RecentArgs::load);
        let runner = cx.new(|_| {
            let runner = Runner::new(config.id.clone(), catalog.clone(), driver);
            match recent {
                Some(r) => runner.with_recent_args(r),
                None => runner,
            }
        });
        let ports = Ports::new(&config.id, runner.clone(), state_dir, cx);
        let kube = config.cluster.clone().map(|c| {
            let driver = Rc::new(KubeCluster::new(c));
            (new_cluster(config.envs.clone(), driver.clone(), None, cx), driver)
        });
        let cluster = kube.as_ref().map(|(c, _)| c.clone());
        cx.new(|cx| {
            cx.observe(&runner, |_, _, cx| cx.notify()).detach();
            cx.observe(&ports, |_, _, cx| cx.notify()).detach();
            if let Some(c) = &cluster {
                cx.observe(c, |_, _, cx| cx.notify()).detach();
            }
            cx.observe(&environment, |this: &mut Services, e, cx| {
                let tools = e.read(cx).tools();
                this.set_catalog(Catalog { tools, ..(*this.catalog).clone() }, cx);
            })
            .detach();
            let unusable = unusable(&catalog);
            Services { workspace: config.id.clone(), catalog, env, runner, cluster, kube, config: Some(config), problems, watch, ports, scan: None, unusable }
        })
    }

    /// Finds the apps again, with `config` when it changed (a config workspace only), in
    /// the background. Emits [`Rescanned`] once done.
    pub fn rescan(&mut self, config: Option<WorkspaceConfig>, cx: &mut Context<Self>) {
        if let Some(c) = config {
            self.set_cluster(&c, cx);
            self.config = Some(c);
        }
        let Some(config) = self.config.clone() else { return };
        let tools = self.catalog.tools.clone();
        self.scan = Some(cx.spawn(async move |this, cx| {
            let found = cx.background_executor().spawn(async move { discover(&config, tools) }).await;
            let _ = this.update(cx, |s, cx| s.found(found, cx));
        }));
    }

    fn found(&mut self, (mut catalog, problems, watch): (Catalog, Vec<String>, Vec<PathBuf>), cx: &mut Context<Self>) {
        self.scan = None;
        // The tools may have been found meanwhile.
        catalog.tools = self.catalog.tools.clone();
        if self.problems != problems || self.watch != watch {
            self.problems = problems;
            self.watch = watch;
            cx.notify();
        }
        self.set_catalog(catalog, cx);
        cx.emit(Rescanned);
    }

    /// The cluster follows a changed config, in place: envs change without touching the
    /// others' watches, a changed `[cluster]` reconnects only the envs whose target changed,
    /// and a removed one empties the cluster (no Cluster activity; views keep it, and it
    /// fills again when `[cluster]` comes back).
    fn set_cluster(&mut self, config: &WorkspaceConfig, cx: &mut Context<Self>) {
        let old = self.config.as_ref();
        let Some(k) = config.cluster.clone() else {
            if let Some(c) = self.cluster.take() {
                c.update(cx, |c, cx| c.clear(cx));
                cx.notify();
            }
            return;
        };
        let back = self.cluster.is_none();
        match &self.kube {
            Some((c, driver)) => {
                let c = c.clone();
                if back || old.is_none_or(|o| o.envs != config.envs) {
                    c.update(cx, |c, cx| c.set_envs(config.envs.clone(), cx));
                }
                if back || old.is_none_or(|o| o.cluster.as_ref() != Some(&k)) {
                    driver.set_config(k);
                    c.update(cx, |c, cx| c.reload(cx));
                }
                self.cluster = Some(c);
            }
            None => {
                let driver = Rc::new(KubeCluster::new(k));
                let c = new_cluster(config.envs.clone(), driver.clone(), None, cx);
                cx.observe(&c, |_, _, cx| cx.notify()).detach();
                self.kube = Some((c.clone(), driver));
                self.cluster = Some(c);
            }
        }
        cx.notify();
    }

    /// The folder of the cluster's kubeconfigs, watched for changes.
    pub fn kubeconfigs(&self) -> Option<&Path> {
        self.config.as_ref()?.cluster.as_ref().map(|c| c.kubeconfigs.as_path())
    }

    /// What discovery, the overrides and the cluster's kubeconfigs reported.
    pub fn all_problems(&self, cx: &App) -> Vec<String> {
        let mut out = self.problems.clone();
        if let Some(c) = &self.cluster {
            out.extend(c.read(cx).problems.iter().cloned());
        }
        out
    }

    /// Swaps the catalog in, for the views and the runner, when it changed.
    pub fn set_catalog(&mut self, catalog: Catalog, cx: &mut Context<Self>) {
        if *self.catalog == catalog {
            return;
        }
        let catalog = Rc::new(catalog);
        self.unusable = unusable(&catalog);
        self.env = catalog.pick_env(self.env.clone());
        self.catalog = catalog.clone();
        self.runner.update(cx, |r, cx| r.set_catalog(catalog, cx));
        cx.notify();
    }

    pub fn set_env(&mut self, env: &str, cx: &mut Context<Self>) {
        let known = self.catalog.env(env).is_some();
        if known && self.env.as_deref() != Some(env) {
            self.env = Some(env.into());
            cx.notify();
        }
    }

    pub fn protected_env(&self) -> bool {
        let env = self.env.as_deref();
        env.and_then(|e| self.catalog.env(e)).is_some_and(|e| e.protected)
    }

    /// Ports in use, by number: from the last scan (nothing is read here), the demo's
    /// holders, and the ports the demo's runs declare.
    pub fn ports(&self, cx: &App) -> Vec<PortInfo> {
        let mut out: Vec<PortInfo> = self
            .catalog
            .ports
            .iter()
            .map(|p| PortInfo { port: p.port, holder: p.holder.clone(), command: p.command.clone(), run: None, pid: None })
            .collect();
        let runner = self.runner.read(cx);
        let line = |key: &RunKey| self.catalog.app(&key.app).and_then(|a| a.command(&key.cmd)).map(|c| c.line.clone()).unwrap_or_default();
        // Terminals aren't runs: what listens in one shows as any other holder would.
        let real: BTreeSet<RunKey> = runner.pids().into_iter().map(|(k, _)| k).filter(|k| !k.is_terminal()).collect();
        // Runs without a process (the demo's fake ones): the ports their command declares.
        for (key, port) in runner.running_ports() {
            if !real.contains(&key) {
                out.push(PortInfo { port, holder: runner.title(&key), command: line(&key), run: Some(key), pid: None });
            }
        }
        let scan = &self.ports.read(cx).scan;
        for (key, ports) in scan.listening.iter().filter(|(k, _)| real.contains(*k)) {
            for &port in ports {
                out.push(PortInfo { port, holder: runner.title(key), command: line(key), run: Some(key.clone()), pid: None });
            }
        }
        // Ports a port view shows, held by something else.
        for (&port, holder) in &scan.held {
            if out.iter().any(|p| p.port == port) {
                continue;
            }
            out.push(match holder {
                PortHolder::Run { key, title } => PortInfo { port, holder: title.clone(), command: line(key), run: Some(key.clone()), pid: None },
                PortHolder::Process { pid, command } => PortInfo { port, holder: holder.to_string(), command: command.clone(), run: None, pid: Some(*pid) },
                PortHolder::Unknown => PortInfo { port, holder: holder.to_string(), command: String::new(), run: None, pid: None },
            });
        }
        out.sort_by_key(|p| p.port);
        out
    }
}

/// A config workspace's catalog: discovery, then the overrides; and what went wrong, and
/// the folders to watch.
fn discover(config: &WorkspaceConfig, tools: devdeck_core::apps::Tools) -> (Catalog, Vec<String>, Vec<PathBuf>) {
    let found = devdeck_core::discovery::discover(&config.folders);
    let (apps, warnings) = apply_overrides(found.apps, &config.apps);
    let mut problems = found.problems;
    problems.extend(warnings);
    let catalog = Catalog { apps, tools, envs: config.envs.clone(), default_env: config.default_env.clone(), ..Catalog::default() };
    (catalog, problems, found.watch)
}
