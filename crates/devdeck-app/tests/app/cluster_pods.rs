//! M3 step 2: pods and metrics (R7.1, R7.2, R7.8). Who wants which pods and the watches
//! that follow (a service's own, the env-wide one of the list's **watch** toggle), pods by
//! owner, running since from pods, metrics read at the driver's pace while someone looks.
//! Over a scripted driver: nothing here reaches a cluster.

use devdeck_app::cluster::{Cluster, ClusterDriver, EnvUpdate, MetricsRead, PodScope, PodUpdate, new_cluster};
use devdeck_core::cluster::{Container, ContainerStatus, Deployment, Env, Image, Pod, PodPhase, Progress, Remote, Resources, Usage};
use gpui::{Context, Entity, Task, TestAppContext};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::Duration;

/// Starts pod watches that do nothing (the test applies what they'd hear), and records
/// them; reads metrics by handing back `usage`, counting the reads.
#[derive(Default)]
struct Pods {
    started: RefCell<Vec<(String, PodScope, Option<String>)>>,
    reads: Cell<usize>,
    refuse: Cell<bool>,
    /// Reads never answer (a slow cluster).
    hold: Cell<bool>,
}

impl ClusterDriver for Pods {
    fn watch(&self, _env: &str, _cx: &mut Context<Cluster>) -> Option<Task<()>> {
        None
    }
    fn namespace(&self, env: &str) -> String {
        env.into()
    }
    fn writes(&self) -> Result<(), String> {
        Err("no".into())
    }
    fn watch_pods(&self, env: &str, scope: PodScope, selector: Option<&str>, _cx: &mut Context<Cluster>) -> Option<Task<()>> {
        self.started.borrow_mut().push((env.into(), scope, selector.map(String::from)));
        Some(Task::ready(()))
    }
    fn sample(&self, env: &str, cx: &mut Context<Cluster>) -> Option<Task<()>> {
        self.reads.set(self.reads.get() + 1);
        if self.hold.get() {
            return Some(cx.spawn(async move |_, _| std::future::pending::<()>().await));
        }
        let read = if self.refuse.get() { MetricsRead::Refused("your account can't read metrics in dev35".into()) } else { MetricsRead::Usage(BTreeMap::from([("api-a1-x".to_string(), Usage { cpu_m: 30, mem_mi: 64 })])) };
        let env = env.to_string();
        Some(cx.spawn(async move |this, cx| {
            let _ = this.update(cx, |c, cx| c.apply_metrics(&env, read, cx));
        }))
    }
    fn metrics_every(&self) -> Duration {
        Duration::from_secs(15)
    }
}

fn dep(name: &str) -> Deployment {
    let image = Image::parse(&format!("registry/{name}:main"));
    let c = Container { name: name.into(), image, ports: vec![], resources: Resources::default(), config_maps: Vec::new() };
    Deployment { name: name.into(), containers: vec![c], replicas: 2, ready: 2, updated: 2, progress: Progress::Settled, selector: format!("app={name}"), since: Some(1) }
}

fn pod(owner: &str, template: &str, id: &str, started: u64) -> Pod {
    let status = ContainerStatus { name: owner.into(), init: false, ready: true, restarts: 0, state: "running".into(), last_termination: None };
    Pod {
        name: format!("{owner}-{template}-{id}"),
        owner: Some(owner.into()),
        template: template.into(),
        phase: PodPhase::Running,
        ready: true,
        restarts: 0,
        last_termination: None,
        started: Some(started),
        node: "node-1".into(),
        tag: "main".into(),
        containers: vec![status],
    }
}

fn setup(cx: &mut TestAppContext) -> (Entity<Cluster>, Rc<Pods>) {
    let driver = Rc::new(Pods::default());
    let envs = vec![Env { name: "dev35".into(), protected: false }];
    let c = cx.update(|cx| new_cluster(envs, driver.clone(), Some(10_000), cx));
    cx.update(|cx| c.update(cx, |c, cx| c.want("dev35", cx)));
    let deployments = EnvUpdate::Deployments(vec![dep("api"), dep("web")]);
    cx.update(|cx| c.update(cx, |c, cx| c.apply("dev35", deployments, cx)));
    (c, driver)
}

fn watches(c: &Entity<Cluster>, cx: &TestAppContext) -> Vec<(String, PodScope)> {
    cx.read(|cx| c.read(cx).pod_watches())
}

fn service(name: &str) -> (String, PodScope) {
    ("dev35".into(), PodScope::Service(name.into()))
}

fn env_wide() -> (String, PodScope) {
    ("dev35".into(), PodScope::Env)
}

#[gpui::test]
fn a_service_watches_its_pods_while_held_and_the_toggle_takes_over_once_listed(cx: &mut TestAppContext) {
    let (c, driver) = setup(cx);
    assert!(watches(&c, cx).is_empty(), "nothing watched when nothing holds");
    let api = cx.update(|cx| c.update(cx, |c, cx| c.pods_interest("dev35", PodScope::Service("api".into()), cx)));
    assert_eq!(watches(&c, cx), [service("api")]);
    assert_eq!(driver.started.borrow().last().unwrap().2.as_deref(), Some("app=api"), "filtered by its selector");
    assert!(cx.read(|cx| matches!(c.read(cx).pods_state("dev35", "api"), Remote::Loading)));
    // A selector catching another deployment's pods: the owner decides.
    let heard = PodUpdate::Pods(vec![pod("api", "a1", "x", 100), pod("api-canary", "c1", "y", 100)]);
    cx.update(|cx| c.update(cx, |c, cx| c.apply_pods("dev35", PodScope::Service("api".into()), heard, cx)));
    assert_eq!(cx.read(|cx| c.read(cx).pods("dev35", "api").len()), 1);

    // The toggle on: the env-wide watch starts, the service keeps its own until it listed.
    let all = cx.update(|cx| c.update(cx, |c, cx| c.pods_interest("dev35", PodScope::Env, cx)));
    assert_eq!(watches(&c, cx), [env_wide(), service("api")]);
    assert_eq!(cx.read(|cx| c.read(cx).pods("dev35", "api").len()), 1, "no flash of no pods meanwhile");
    let listed = PodUpdate::Pods(vec![pod("api", "a1", "x", 100), pod("web", "w1", "z", 50)]);
    cx.update(|cx| c.update(cx, |c, cx| c.apply_pods("dev35", PodScope::Env, listed, cx)));
    assert_eq!(watches(&c, cx), [env_wide()], "the service's watch gave way");
    assert_eq!(cx.read(|cx| c.read(cx).pods("dev35", "web").len()), 1);

    // The toggle off: the service gets its own watch back, its pods shown meanwhile.
    drop(all);
    cx.run_until_parked();
    assert_eq!(watches(&c, cx), [service("api")]);
    assert!(cx.read(|cx| matches!(c.read(cx).pods_state("dev35", "api"), Remote::Ready(p) if p.len() == 1)), "no flash of loading");
    let own = PodUpdate::Pods(vec![pod("api", "a2", "n", 200)]);
    cx.update(|cx| c.update(cx, |c, cx| c.apply_pods("dev35", PodScope::Service("api".into()), own, cx)));
    assert_eq!(cx.read(|cx| c.read(cx).pods("dev35", "api")[0].name.clone()), "api-a2-n", "its own list replaced the env-wide one");
    assert!(cx.read(|cx| c.read(cx).pods("dev35", "web").is_empty()), "nobody holds web's pods any more");
    drop(api);
    cx.run_until_parked();
    assert!(watches(&c, cx).is_empty());
    assert_eq!(cx.read(|cx| c.read(cx).pod_interests()), 0);
}

#[gpui::test]
fn a_changed_target_or_selector_restarts_pod_watches_and_a_cluster_back_reinstates_them(cx: &mut TestAppContext) {
    let (c, driver) = setup(cx);
    let _api = cx.update(|cx| c.update(cx, |c, cx| c.pods_interest("dev35", PodScope::Service("api".into()), cx)));
    let started = || driver.started.borrow().len();
    assert_eq!(started(), 1);
    cx.update(|cx| c.update(cx, |c, cx| c.restart(vec!["dev35".to_string()], cx)));
    assert_eq!(started(), 2, "the target changed: the pod watch too");
    let mut api = dep("api");
    api.selector = "app=api,tier=backend".into();
    cx.update(|cx| c.update(cx, |c, cx| c.apply("dev35", EnvUpdate::Deployments(vec![api, dep("web")]), cx)));
    assert_eq!(started(), 3, "a new selector");
    assert_eq!(driver.started.borrow()[2].2.as_deref(), Some("app=api,tier=backend"));
    // The cluster goes and comes back: the view's interest gets its watch again.
    cx.update(|cx| c.update(cx, |c, cx| c.clear(cx)));
    assert!(watches(&c, cx).is_empty());
    let envs = vec![Env { name: "dev35".into(), protected: false }];
    cx.update(|cx| c.update(cx, |c, cx| c.set_envs(envs, cx)));
    assert!(cx.read(|cx| c.read(cx).wanted().contains("dev35")), "the holder wants its env again");
    cx.update(|cx| c.update(cx, |c, cx| c.apply("dev35", EnvUpdate::Deployments(vec![dep("api")]), cx)));
    assert_eq!(watches(&c, cx), [service("api")]);
}

#[gpui::test]
fn running_since_comes_from_the_pods_of_the_newest_template(cx: &mut TestAppContext) {
    let (c, _) = setup(cx);
    let _all = cx.update(|cx| c.update(cx, |c, cx| c.pods_interest("dev35", PodScope::Env, cx)));
    let since = |cx: &TestAppContext| cx.read(|cx| c.read(cx).since("dev35", &dep("api")));
    assert_eq!(since(cx), Some(1), "no pods yet: the deployment's own");
    let listed = |pods: Vec<Pod>| PodUpdate::Pods(pods);
    cx.update(|cx| c.update(cx, |c, cx| c.apply_pods("dev35", PodScope::Env, listed(vec![pod("api", "a1", "x", 100), pod("api", "a1", "y", 400)]), cx)));
    assert_eq!(since(cx), Some(100), "a scale-up keeps it");
    let mut pending = pod("api", "a2", "p", 0);
    pending.phase = PodPhase::Pending;
    pending.started = None;
    let pods = vec![pod("api", "a1", "x", 100), pod("api", "a2", "z", 900), pod("api", "a2", "w", 950), pending];
    cx.update(|cx| c.update(cx, |c, cx| c.apply_pods("dev35", PodScope::Env, listed(pods), cx)));
    assert_eq!(since(cx), Some(900), "a newer version: its first pod; pending ones don't count");
    let mut rolling = dep("api");
    rolling.progress = Progress::Rolling;
    assert_eq!(cx.read(|cx| c.read(cx).since("dev35", &rolling)), None, "not while it rolls");
}

#[gpui::test]
fn metrics_are_read_at_once_then_at_the_drivers_pace_and_not_after_a_refusal(cx: &mut TestAppContext) {
    let (c, driver) = setup(cx);
    let sampling = cx.update(|cx| c.update(cx, |c, cx| c.sample("dev35", cx)));
    cx.run_until_parked();
    assert_eq!(driver.reads.get(), 1, "at once");
    assert_eq!(cx.read(|cx| c.read(cx).usage("dev35", "api-a1-x")), Some(Usage { cpu_m: 30, mem_mi: 64 }));
    cx.executor().advance_clock(Duration::from_secs(14));
    cx.run_until_parked();
    assert_eq!(driver.reads.get(), 1);
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    assert_eq!(driver.reads.get(), 2, "every 15 s");
    driver.refuse.set(true);
    cx.executor().advance_clock(Duration::from_secs(15));
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| c.read(cx).metrics_allowed("dev35")), Err("your account can't read metrics in dev35".to_string()));
    cx.executor().advance_clock(Duration::from_secs(60));
    cx.run_until_parked();
    assert_eq!(driver.reads.get(), 3, "refused: not read again");
    drop(sampling);
    cx.executor().advance_clock(Duration::from_secs(30));
    cx.run_until_parked();
    assert!(!cx.read(|cx| c.read(cx).is_sampling()));
    assert_eq!(cx.read(|cx| c.read(cx).samplers()), 0);
}

#[gpui::test]
fn sampling_survives_the_cluster_going_and_coming_back(cx: &mut TestAppContext) {
    let (c, driver) = setup(cx);
    let _sampling = cx.update(|cx| c.update(cx, |c, cx| c.sample("dev35", cx)));
    cx.run_until_parked();
    assert_eq!(driver.reads.get(), 1);
    cx.update(|cx| c.update(cx, |c, cx| c.clear(cx)));
    assert!(!cx.read(|cx| c.read(cx).is_sampling()));
    let envs = vec![Env { name: "dev35".into(), protected: false }];
    cx.update(|cx| c.update(cx, |c, cx| c.set_envs(envs, cx)));
    cx.run_until_parked();
    assert!(cx.read(|cx| c.read(cx).is_sampling()), "the view still holds it: reading again");
    assert_eq!(driver.reads.get(), 2);
}

#[gpui::test]
fn a_read_in_flight_isnt_doubled_and_a_new_holder_reads_at_once(cx: &mut TestAppContext) {
    let (c, driver) = setup(cx);
    driver.hold.set(true);
    let first = cx.update(|cx| c.update(cx, |c, cx| c.sample("dev35", cx)));
    cx.executor().advance_clock(Duration::from_secs(60));
    cx.run_until_parked();
    assert_eq!(driver.reads.get(), 1, "never two reads at once");
    assert_eq!(cx.read(|cx| c.read(cx).metric_ticks), 1);
    // The read ends; the last holder goes and a new one comes before the loop's turn.
    driver.hold.set(false);
    cx.update(|cx| c.update(cx, |c, cx| c.apply_metrics("dev35", MetricsRead::Usage(BTreeMap::new()), cx)));
    drop(first);
    let _again = cx.update(|cx| c.update(cx, |c, cx| c.sample("dev35", cx)));
    cx.run_until_parked();
    assert_eq!(driver.reads.get(), 2, "read at once, not at the loop's next turn");
}
