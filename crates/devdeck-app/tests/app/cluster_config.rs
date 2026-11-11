//! M3 step 1: a configured workspace's cluster (R2.3, R7.1, R7.8). The entity over a
//! scripted driver (loading, a list, a problem kept over the list, recovery, config
//! changes), and the kube driver's config side: kubeconfigs folder, missing contexts,
//! problems, refused writes. Nothing here reaches a cluster.

use devdeck_app::cluster::{Cluster, ClusterDriver, EnvUpdate, new_cluster};
use devdeck_app::shell::{Shell, WorkspaceSpec};
use devdeck_app::sidebar::Activity;
use devdeck_app::views::service_view::ServiceView;
use devdeck_core::cluster::{Container, Deployment, Env, Image, Progress, Remote, Resources};
use devdeck_core::config::{ClusterConfig, WorkspaceConfig};
use devdeck_core::view_addr::ViewAddr;
use gpui::{Context, Entity, Task, TestAppContext};
use std::cell::Cell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// Feeds nothing by itself: the test applies what a watch would hear.
struct Script;

impl ClusterDriver for Script {
    fn watch(&self, _env: &str, _cx: &mut Context<Cluster>) -> Option<Task<()>> {
        None
    }
    fn namespace(&self, env: &str) -> String {
        format!("ns-{env}")
    }
    fn writes(&self) -> Result<(), String> {
        Err("no".into())
    }
}

fn env(name: &str) -> Env {
    Env { name: name.into(), protected: false }
}

fn dep(name: &str, tag: &str) -> Deployment {
    let image = Image::parse(&format!("registry/{name}:{tag}"));
    let c = Container { name: name.into(), image, ports: vec![80], resources: Resources::default(), config_maps: Vec::new(), pull_policy: None };
    Deployment { name: name.into(), containers: vec![c], replicas: 1, ready: 1, updated: 1, progress: Progress::Settled, selector: format!("app={name}"), since: Some(100), ..Default::default() }
}

fn scripted(cx: &mut TestAppContext, envs: &[&str]) -> Entity<Cluster> {
    let envs = envs.iter().map(|e| env(e)).collect();
    cx.update(|cx| new_cluster(envs, Rc::new(Script), Some(1000), cx))
}

fn apply(c: &Entity<Cluster>, cx: &mut TestAppContext, env: &str, u: EnvUpdate) {
    cx.update(|cx| c.update(cx, |c, cx| c.apply(env, u, cx)));
}

#[gpui::test]
fn a_list_survives_a_lost_connection_and_the_problem_clears_on_recovery(cx: &mut TestAppContext) {
    let c = scripted(cx, &["dev35", "hades"]);
    let notified = Rc::new(Cell::new(0));
    let n = notified.clone();
    let _sub = cx.update(|cx| cx.observe(&c, move |_, _| n.set(n.get() + 1)));
    cx.update(|cx| c.update(cx, |c, cx| c.want("dev35", cx)));
    assert!(cx.read(|cx| matches!(c.read(cx).peek("dev35"), Remote::Loading)), "loading until a first list");
    apply(&c, cx, "dev35", EnvUpdate::Deployments(vec![dep("b", "main"), dep("a", "v1")]));
    assert!(cx.read(|cx| matches!(c.read(cx).peek("dev35"), Remote::Ready(m) if m.len() == 2)));
    let before = notified.get();
    apply(&c, cx, "dev35", EnvUpdate::Deployments(vec![dep("a", "v1"), dep("b", "main")]));
    assert_eq!(notified.get(), before, "the same list renders nothing");

    apply(&c, cx, "dev35", EnvUpdate::Problem(Some("can't reach km011gcp (dev35): connection refused".into())));
    cx.read(|cx| {
        let c = c.read(cx);
        assert!(matches!(c.peek("dev35"), Remote::Ready(_)), "the list is kept");
        assert_eq!(c.problem("dev35"), Some("can't reach km011gcp (dev35): connection refused"));
    });
    apply(&c, cx, "dev35", EnvUpdate::Problem(None));
    assert_eq!(cx.read(|cx| c.read(cx).problem("dev35").map(str::to_string)), None);

    // A first watch failing: no list, the problem.
    cx.update(|cx| c.update(cx, |c, cx| c.want("hades", cx)));
    apply(&c, cx, "hades", EnvUpdate::Problem(Some("your account can't list deployments in hades".into())));
    assert!(cx.read(|cx| matches!(c.read(cx).peek("hades"), Remote::Error(_))));
    // An env nobody asked for hears nothing.
    apply(&c, cx, "lab", EnvUpdate::Deployments(vec![dep("x", "y")]));
    assert!(cx.read(|cx| c.read(cx).state("lab").is_none()));
}

#[gpui::test]
fn the_matrix_rows_are_every_deployment_of_every_env(cx: &mut TestAppContext) {
    let c = scripted(cx, &["dev35", "hades"]);
    cx.update(|cx| {
        c.update(cx, |c, cx| {
            c.want("dev35", cx);
            c.want("hades", cx);
        })
    });
    apply(&c, cx, "dev35", EnvUpdate::Deployments(vec![dep("graphservice", "main"), dep("apikeyservice", "v2")]));
    apply(&c, cx, "hades", EnvUpdate::Deployments(vec![dep("graphservice", "lan-000"), dep("vesselservice", "fix")]));
    assert_eq!(cx.read(|cx| c.read(cx).names().into_iter().map(str::to_string).collect::<Vec<_>>()), ["apikeyservice", "graphservice", "vesselservice"]);
    let suggestions = cx.read(|cx| c.read(cx).tag_suggestions("dev35", "graphservice"));
    assert_eq!(suggestions, [("lan-000".to_string(), "on hades".to_string())]);
    assert_eq!(
        cx.read(|cx| c.read(cx).command_for("hades", "graphservice", "main")),
        Some("kubectl -n ns-hades set image deployment/graphservice graphservice=registry/graphservice:main".into()),
        "the env's namespace, not its name"
    );
}

#[gpui::test]
fn a_changed_config_forgets_what_went_and_an_emptied_cluster_watches_nothing(cx: &mut TestAppContext) {
    let c = scripted(cx, &["dev35", "dls"]);
    cx.update(|cx| {
        c.update(cx, |c, cx| {
            c.want("dev35", cx);
            c.want("dls", cx);
        })
    });
    apply(&c, cx, "dls", EnvUpdate::Deployments(vec![dep("a", "v1")]));
    cx.update(|cx| c.update(cx, |c, cx| c.set_driver(vec![env("dev35")], Rc::new(Script), cx)));
    cx.read(|cx| {
        let c = c.read(cx);
        assert!(c.state("dls").is_none() && !c.wanted().contains("dls"));
        assert!(c.wanted().contains("dev35"), "still watched");
    });
    cx.update(|cx| c.update(cx, |c, cx| c.clear(cx)));
    assert!(cx.read(|cx| c.read(cx).envs.is_empty() && c.read(cx).wanted().is_empty()));
}

/// Counts the watches it starts; its start reports `problem` after `delay`.
struct Counting {
    watches: Rc<Cell<usize>>,
    problem: &'static str,
    delay: std::time::Duration,
}

impl ClusterDriver for Counting {
    fn start(self: Rc<Self>, cx: &mut Context<Cluster>) -> Option<Task<()>> {
        Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(self.delay).await;
            let _ = this.update(cx, |c, cx| c.set_problems(vec![self.problem.to_string()], cx));
        }))
    }
    fn watch(&self, _env: &str, _cx: &mut Context<Cluster>) -> Option<Task<()>> {
        self.watches.set(self.watches.get() + 1);
        Some(Task::ready(()))
    }
    fn namespace(&self, env: &str) -> String {
        env.into()
    }
    fn writes(&self) -> Result<(), String> {
        Err("no".into())
    }
}

#[gpui::test]
fn a_newer_load_wins_over_an_older_one_still_running(cx: &mut TestAppContext) {
    let config = cx.update(|cx| devdeck_app::test_dir("config", cx));
    cx.update(|cx| devdeck_app::init_with_config_dir(&config, cx));
    let watches = Rc::new(Cell::new(0));
    let slow = Rc::new(Counting { watches: watches.clone(), problem: "old folder", delay: std::time::Duration::from_millis(300) });
    let c = cx.update(|cx| new_cluster(vec![env("dev35")], slow, None, cx));
    let fast = Rc::new(Counting { watches, problem: "new folder", delay: std::time::Duration::from_millis(50) });
    cx.update(|cx| c.update(cx, |c, cx| c.set_driver(vec![env("dev35")], fast, cx)));
    cx.executor().advance_clock(std::time::Duration::from_secs(1));
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| c.read(cx).problems.clone()), ["new folder"], "the replaced load never lands");
    cx.update(|cx| c.update(cx, |c, cx| c.clear(cx)));
    cx.executor().advance_clock(std::time::Duration::from_secs(1));
    cx.run_until_parked();
    assert!(cx.read(|cx| c.read(cx).problems.is_empty()));
}

#[gpui::test]
fn envs_change_in_place_and_every_env_wanted_includes_new_ones(cx: &mut TestAppContext) {
    let watches = Rc::new(Cell::new(0));
    let driver = Rc::new(Counting { watches: watches.clone(), problem: "", delay: std::time::Duration::from_secs(3600) });
    let c = cx.update(|cx| new_cluster(vec![env("dev35"), env("dls")], driver, None, cx));
    cx.update(|cx| c.update(cx, |c, cx| c.want_all(cx)));
    assert_eq!(watches.get(), 2);
    // dls becomes protected, lab arrives: only lab starts a watch.
    let envs = vec![env("dev35"), Env { name: "dls".into(), protected: true }, env("lab")];
    cx.update(|cx| c.update(cx, |c, cx| c.set_envs(envs, cx)));
    assert_eq!(watches.get(), 3, "the others keep their watch");
    cx.read(|cx| {
        let c = c.read(cx);
        assert!(c.wanted().contains("lab") && c.env("dls").unwrap().protected);
    });
    cx.update(|cx| c.update(cx, |c, cx| c.set_envs(vec![env("lab")], cx)));
    assert_eq!(cx.read(|cx| c.read(cx).wanted().iter().cloned().collect::<Vec<_>>()), ["lab"]);
}

// ------------------------------------------------------------------ kube driver, config side

/// A kubeconfig with the dev envs' layout (fake token, a server nothing listens on).
fn kubeconfig(contexts: &[&str]) -> String {
    let mut s = String::from(
        "apiVersion: v1\nkind: Config\nclusters:\n- name: km011gcp\n  cluster:\n    server: https://127.0.0.1:1\nusers:\n- name: tailscale-auth\n  user:\n    token: fake\ncontexts:\n",
    );
    for c in contexts {
        s.push_str(&format!("- name: {c}\n  context:\n    cluster: km011gcp\n    user: tailscale-auth\n    namespace: {c}\n"));
    }
    s
}

fn kubeconfigs(cx: &mut TestAppContext, files: &[(&str, &str)]) -> PathBuf {
    let dir = cx.update(|cx| devdeck_app::test_dir("kubeconfigs", cx));
    for (name, text) in files {
        std::fs::write(dir.join(name), text).unwrap();
    }
    dir
}

fn start(cx: &mut TestAppContext, cluster: Option<ClusterConfig>, folder: &Path) -> Entity<Shell> {
    let config = cx.update(|cx| devdeck_app::test_dir("config", cx));
    cx.update(|cx| devdeck_app::init_with_config_dir(&config, cx));
    let envs = vec![env("dev35"), Env { name: "dls".into(), protected: true }, env("hades")];
    let ws = WorkspaceConfig { id: "work".into(), name: "work".into(), folders: vec![folder.to_path_buf()], envs, cluster, ..Default::default() };
    let shell = cx.update(|cx| Shell::start(vec![WorkspaceSpec::from_config(ws)], "work", None, cx));
    cx.update(|cx| shell.update(cx, |s, _| s.quit_for_real = false));
    cx.run_until_parked();
    shell
}

#[gpui::test]
fn a_cluster_section_brings_the_cluster_area_and_its_kubeconfig_problems(cx: &mut TestAppContext) {
    let folder = kubeconfigs(cx, &[("dev-km011gcp", &kubeconfig(&["dev35"])), ("broken", "contexts: [[["), ("README.md", "* not yaml")]);
    let contexts = BTreeMap::from([("hades".to_string(), "hades00-hecate".to_string())]);
    let shell = start(cx, Some(ClusterConfig { kubeconfigs: folder.clone(), contexts }), &folder);
    let s = cx.read(|cx| shell.read(cx).workspaces[0].services.clone());
    let c = cx.read(|cx| s.read(cx).cluster.clone()).expect("a cluster");
    let sidebar = cx.read(|cx| shell.read(cx).active_sidebar());
    assert!(cx.read(|cx| sidebar.read(cx).activities(cx).contains(&Activity::Cluster)), "R2.3");

    // Read off the UI thread at start: the broken file is a workspace problem, the README isn't.
    let problems = cx.read(|cx| s.read(cx).all_problems(cx));
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].contains("broken: not a kubeconfig"), "{problems:?}");

    // An env whose context isn't there says so; nothing is read and nothing renders after.
    cx.update(|cx| c.update(cx, |c, cx| c.want("hades", cx)));
    cx.run_until_parked();
    let shown = cx.read(|cx| match c.read(cx).peek("hades") {
        Remote::Error(e) => e,
        other => panic!("{other:?}"),
    });
    assert!(shown.starts_with("no context hades00-hecate in "), "{shown}");

    // The kube driver itself can write: only the short name is missing.
    let nameless = devdeck_app::cluster::NO_SHORT_NAME.to_string();
    cx.read(|cx| assert_eq!(c.read(cx).writes(cx), Err(nameless.clone())));
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    let r = ViewAddr::Svc { env: "hades".into(), name: "graphservice".into() }.to_ref();
    let open = r.clone();
    cx.update(|cx| deck.update(cx, |d, cx| d.apply(cx, |l| l.open(open, l.main().focused).map(|_| ()))));
    cx.run_until_parked();
    let view = cx.read(|cx| deck.read(cx).view(&r).unwrap().downcast::<ServiceView>().unwrap());
    assert_eq!(cx.read(|cx| view.read(cx).save_blocked(cx)), Some(nameless), "Save says why: the short name first");
}

#[gpui::test]
fn a_cluster_removed_then_back_is_the_same_one_filled_again(cx: &mut TestAppContext) {
    let folder = kubeconfigs(cx, &[("dev-km011gcp", &kubeconfig(&["dev35"]))]);
    let cluster = ClusterConfig { kubeconfigs: folder.clone(), contexts: BTreeMap::new() };
    let shell = start(cx, Some(cluster), &folder);
    let s = cx.read(|cx| shell.read(cx).workspaces[0].services.clone());
    let first = cx.read(|cx| s.read(cx).cluster.clone()).unwrap();
    let with = cx.read(|cx| s.read(cx).config.clone()).unwrap();
    let without = WorkspaceConfig { cluster: None, ..with.clone() };
    cx.update(|cx| s.update(cx, |s, cx| s.rescan(Some(without), cx)));
    cx.run_until_parked();
    assert!(cx.read(|cx| s.read(cx).cluster.is_none()));
    assert!(cx.read(|cx| first.read(cx).envs.is_empty()), "emptied for the views holding it");
    cx.update(|cx| s.update(cx, |s, cx| s.rescan(Some(with), cx)));
    cx.run_until_parked();
    let back = cx.read(|cx| s.read(cx).cluster.clone()).unwrap();
    assert_eq!(back.entity_id(), first.entity_id(), "the views' cluster");
    assert_eq!(cx.read(|cx| back.read(cx).envs.len()), 3);
}

#[gpui::test]
fn a_workspace_without_a_cluster_section_has_no_cluster_area(cx: &mut TestAppContext) {
    let folder = cx.update(|cx| devdeck_app::test_dir("projects", cx));
    let shell = start(cx, None, &folder);
    let s = cx.read(|cx| shell.read(cx).workspaces[0].services.clone());
    assert!(cx.read(|cx| s.read(cx).cluster.is_none()));
    let sidebar = cx.read(|cx| shell.read(cx).active_sidebar());
    assert!(cx.read(|cx| !sidebar.read(cx).activities(cx).contains(&Activity::Cluster)));
}
