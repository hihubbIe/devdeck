//! M3 steps 5 and 6: changing the deployed branch and redeploying it (R7.3, R7.4, R7.5,
//! R7.6, R5.6). Deploys over a scripted
//! driver whose watch and answers the test plays: guards in order, the write pinned to
//! what was confirmed, a rollout followed to done, failing with its reason and recovering,
//! stuck, replaced, deploys seen from elsewhere, the history kept and read back, the
//! access review; redeploys: their guards, the write pinned to the image shown, known as
//! devdeck's by their `restartedAt`, one that changes nothing, one left open. Nothing here
//! reaches a cluster.

use devdeck_app::cluster::{Access, Cluster, ClusterDriver, EnvUpdate, ImageChange, PodScope, PodUpdate, Restart, WriteState, new_cluster};
use devdeck_core::cluster::{Container, ContainerStatus, Deployment, Env, HISTORY_KEPT, Image, Pod, Progress, Resources, RolloutState, restart_stamp};
use devdeck_core::state::StateStore;
use gpui::{Context, Entity, Task, TestAppContext};
use std::cell::RefCell;
use std::rc::Rc;

const ENV: &str = "dev33";
const SVC: &str = "graphservice";
/// The field manager these writes carry.
const ME: &str = "i.alglave via Kugelblitz";

/// Records writes and access reviews; the test answers them and plays the watch.
#[derive(Default)]
struct Scripted {
    writes: RefCell<Vec<(String, ImageChange)>>,
    restarts: RefCell<Vec<(String, Restart)>>,
    reviews: RefCell<usize>,
    /// No short name of its own (and Settings names nobody here).
    nameless: bool,
}

impl ClusterDriver for Scripted {
    fn watch(&self, _env: &str, _cx: &mut Context<Cluster>) -> Option<Task<()>> {
        None
    }
    fn namespace(&self, env: &str) -> String {
        env.into()
    }
    fn writes(&self) -> Result<(), String> {
        Ok(())
    }
    fn short_name(&self) -> Option<String> {
        (!self.nameless).then(|| "i.alglave".into())
    }
    fn watch_pods(&self, _env: &str, _scope: PodScope, _selector: Option<&str>, _cx: &mut Context<Cluster>) -> Option<Task<()>> {
        Some(Task::ready(()))
    }
    fn set_image(&self, _env: &str, name: &str, change: ImageChange, _cx: &mut Context<Cluster>) -> Option<Task<()>> {
        self.writes.borrow_mut().push((name.into(), change));
        Some(Task::ready(()))
    }
    fn restart(&self, _env: &str, name: &str, restart: Restart, _cx: &mut Context<Cluster>) -> Option<Task<()>> {
        self.restarts.borrow_mut().push((name.into(), restart));
        Some(Task::ready(()))
    }
    fn can_deploy(&self, _env: &str, _cx: &mut Context<Cluster>) -> Option<Task<()>> {
        *self.reviews.borrow_mut() += 1;
        Some(Task::ready(()))
    }
}

fn dep(tag: &str, revision: u64, generation: u64, observed: u64, progress: Progress) -> Deployment {
    let c = Container { name: SVC.into(), image: Image::parse(&format!("registry/{SVC}:{tag}")), ports: vec![], resources: Resources::default(), config_maps: Vec::new(), pull_policy: None };
    Deployment { name: SVC.into(), containers: vec![c], replicas: 2, ready: 2, updated: 2, progress, selector: format!("app={SVC}"), revision, generation, observed, template: Some(format!("t-{tag}")), ..Default::default() }
}

fn pod(tag: &str, id: &str, ready: bool, state: &str, restarts: u32, message: Option<&str>) -> Pod {
    let c = ContainerStatus { name: SVC.into(), ready, restarts, state: state.into(), message: message.map(String::from), ..Default::default() };
    Pod { name: format!("{SVC}-t-{tag}-{id}"), owner: Some(SVC.into()), template: format!("t-{tag}"), ready, tag: tag.into(), containers: vec![c], started: Some(1), ..Default::default() }
}

struct T {
    cluster: Entity<Cluster>,
    driver: Rc<Scripted>,
}

fn setup(cx: &mut TestAppContext, protected: bool) -> T {
    let driver = Rc::new(Scripted::default());
    let envs = vec![Env { name: ENV.into(), protected }];
    let d = driver.clone();
    let cluster = cx.update(|cx| new_cluster(envs, d, Some(1_000), cx));
    cx.update(|cx| cluster.update(cx, |c, cx| c.want(ENV, cx)));
    let t = T { cluster, driver };
    watch(&t, cx, dep("main", 3, 5, 5, Progress::Settled));
    t
}

/// The watch hears `d`.
fn watch(t: &T, cx: &mut TestAppContext, d: Deployment) {
    cx.update(|cx| t.cluster.update(cx, |c, cx| c.apply(ENV, EnvUpdate::Deployments(vec![d]), cx)));
    cx.run_until_parked();
}

fn pods(t: &T, cx: &mut TestAppContext, pods: Vec<Pod>) {
    cx.update(|cx| t.cluster.update(cx, |c, cx| c.apply_pods(ENV, PodScope::Service(SVC.into()), PodUpdate::Pods(pods), cx)));
    cx.run_until_parked();
}

fn deploy(t: &T, cx: &mut TestAppContext, from: &str, to: &str) -> Result<(), String> {
    cx.update(|cx| t.cluster.update(cx, |c, cx| c.deploy(ENV, SVC, &format!("registry/{SVC}:{from}"), to, false, crate::confirmed(), cx)))
}

fn answer(t: &T, cx: &mut TestAppContext, result: Result<u64, String>) {
    cx.update(|cx| t.cluster.update(cx, |c, cx| c.write_done(ENV, SVC, result, cx)));
    cx.run_until_parked();
}

fn state(t: &T, cx: &TestAppContext) -> Option<RolloutState> {
    cx.read(|cx| t.cluster.read(cx).rollout(ENV, SVC).map(|r| r.state.clone()))
}

fn history(t: &T, cx: &TestAppContext) -> Vec<(String, String, String)> {
    cx.read(|cx| t.cluster.read(cx).history(ENV, SVC).iter().map(|r| (r.to.clone(), r.by.clone(), r.outcome.clone())).collect())
}

#[gpui::test]
fn guards_come_in_order_below_the_ui(cx: &mut TestAppContext) {
    let t = setup(cx, true);
    let blocked = |cx: &mut TestAppContext, to: &str| cx.read(|cx| t.cluster.read(cx).deploy_blocked(ENV, SVC, to, false, cx));
    cx.update(|cx| t.cluster.update(cx, |c, cx| c.check_access(ENV, cx)));
    cx.update(|cx| t.cluster.update(cx, |c, cx| c.access_known(ENV, Ok(false), cx)));
    assert_eq!(blocked(cx, "feature"), Some(format!("your account can't change deployments in {ENV}")), "the account, before anything is asked");
    cx.update(|cx| t.cluster.update(cx, |c, cx| c.access_known(ENV, Ok(true), cx)));
    assert_eq!(blocked(cx, "feature"), Some(format!("{ENV} is protected: unlock it first")), "the protected env (R7.6)");
    assert_eq!(cx.read(|cx| t.cluster.read(cx).deploy_blocked(ENV, SVC, "main", true, cx)), Some(format!("{SVC} already runs main")));
    watch(&t, cx, dep("main", 3, 5, 5, Progress::Paused));
    assert_eq!(cx.read(|cx| t.cluster.read(cx).deploy_blocked(ENV, SVC, "feature", true, cx)), Some(format!("rollouts of {SVC} are paused")));
    let mut two = dep("main", 3, 5, 5, Progress::Settled);
    two.containers[0].name = "app".into();
    two.containers.push(Container { name: "envoy".into(), image: Image::parse("registry/envoy:v1"), ports: vec![], resources: Resources::default(), config_maps: Vec::new(), pull_policy: None });
    watch(&t, cx, two);
    assert_eq!(cx.read(|cx| t.cluster.read(cx).deploy_blocked(ENV, SVC, "feature", true, cx)), Some(format!("{SVC} has several containers and none is named {SVC}")), "never a sidecar");
}

#[gpui::test]
fn nothing_is_written_without_a_short_name(cx: &mut TestAppContext) {
    let driver = Rc::new(Scripted { nameless: true, ..Default::default() });
    let d = driver.clone();
    let cluster = cx.update(|cx| new_cluster(vec![Env { name: ENV.into(), protected: false }], d, Some(1_000), cx));
    cx.update(|cx| cluster.update(cx, |c, cx| c.want(ENV, cx)));
    let t = T { cluster, driver };
    watch(&t, cx, dep("main", 3, 5, 5, Progress::Settled));
    let why = devdeck_app::cluster::NO_SHORT_NAME.to_string();
    assert_eq!(cx.read(|cx| t.cluster.read(cx).deploy_blocked(ENV, SVC, "feature", false, cx)), Some(why.clone()));
    assert_eq!(deploy(&t, cx, "main", "feature"), Err(why.clone()));
    assert_eq!(cx.update(|cx| t.cluster.update(cx, |c, cx| c.redeploy(ENV, SVC, MAIN, false, crate::confirmed(), cx))), Err(why), "a redeploy too");
    assert!(t.driver.writes.borrow().is_empty() && t.driver.restarts.borrow().is_empty(), "nothing sent");
}

#[gpui::test]
fn writes_carry_who_made_them_and_the_history_says_it(cx: &mut TestAppContext) {
    let t = setup(cx, false);
    deploy(&t, cx, "main", "feature").unwrap();
    assert_eq!(t.driver.writes.borrow()[0].1.manager, ME, "the field manager sent: <short name> via Kugelblitz");
    answer(&t, cx, Ok(6));
    watch(&t, cx, dep("feature", 4, 6, 6, Progress::Settled));
    assert_eq!(history(&t, cx).last().map(|r| r.1.clone()), Some(ME.into()), "the history says it as written");
    cx.update(|cx| t.cluster.update(cx, |c, cx| c.redeploy(ENV, SVC, "registry/graphservice:feature", false, crate::confirmed(), cx))).unwrap();
    assert_eq!(t.driver.restarts.borrow()[0].1.manager, ME, "a redeploy too");
}

#[gpui::test]
fn a_deploy_is_pinned_to_what_was_confirmed_and_followed_to_done(cx: &mut TestAppContext) {
    let t = setup(cx, false);
    deploy(&t, cx, "main", "feature").unwrap();
    let (name, change) = t.driver.writes.borrow()[0].clone();
    assert_eq!((name.as_str(), change.index, change.container.as_str(), change.from.as_str(), change.to.as_str()), (SVC, 0, SVC, "registry/graphservice:main", "registry/graphservice:feature"));
    assert_eq!(cx.read(|cx| t.cluster.read(cx).write_state(ENV, SVC).cloned()), Some(WriteState::Sending));
    assert_eq!(deploy(&t, cx, "main", "other"), Err(format!("a deploy of {SVC} is being sent")), "one write in flight");
    assert_eq!(cx.read(|cx| t.cluster.read(cx).following()), 1, "its pods are held while followed");
    // The watch sees it before the answer: still devdeck's.
    watch(&t, cx, dep("feature", 4, 6, 5, Progress::Stuck));
    answer(&t, cx, Ok(6));
    assert_eq!(state(&t, cx), Some(RolloutState::Rolling { ready: 0, of: 2 }), "an old stuck state isn't this deploy's until observed");
    assert_eq!(history(&t, cx), [("feature".to_string(), ME.to_string(), String::new())]);
    watch(&t, cx, dep("feature", 4, 6, 6, Progress::Rolling));
    pods(&t, cx, vec![pod("feature", "a", true, "running", 0, None), pod("main", "b", true, "running", 0, None)]);
    assert_eq!(state(&t, cx), Some(RolloutState::Rolling { ready: 1, of: 2 }));
    watch(&t, cx, dep("feature", 4, 6, 6, Progress::Settled));
    pods(&t, cx, vec![pod("feature", "a", true, "running", 0, None), pod("feature", "c", true, "running", 0, None)]);
    assert_eq!(state(&t, cx), Some(RolloutState::Done));
    assert_eq!(history(&t, cx)[0].2, "done");
    assert_eq!(cx.read(|cx| t.cluster.read(cx).following()), 0, "done: its pods go");
}

#[gpui::test]
fn a_refused_write_says_why_and_follows_nothing(cx: &mut TestAppContext) {
    let t = setup(cx, false);
    cx.update(|cx| t.cluster.update(cx, |c, _| c.watch_write(ENV, SVC, true)));
    deploy(&t, cx, "main", "feature").unwrap();
    answer(&t, cx, Err(format!("{SVC} changed since you confirmed")));
    assert_eq!(cx.read(|cx| t.cluster.read(cx).write_state(ENV, SVC).cloned()), Some(WriteState::Failed(format!("{SVC} changed since you confirmed"))));
    assert_eq!(cx.read(|cx| t.cluster.read(cx).following()), 0);
    assert!(history(&t, cx).is_empty());
}

#[gpui::test]
fn failing_keeps_following_and_can_recover(cx: &mut TestAppContext) {
    let t = setup(cx, false);
    deploy(&t, cx, "main", "feature").unwrap();
    answer(&t, cx, Ok(6));
    watch(&t, cx, dep("feature", 4, 6, 6, Progress::Rolling));
    let message = "rpc error: manifest for registry/graphservice:feature not found: manifest unknown";
    pods(&t, cx, vec![pod("feature", "a", false, "waiting: ImagePullBackOff", 0, Some(message)), pod("main", "b", true, "running", 0, None)]);
    let failing = RolloutState::Failing { ready: 0, of: 2, reason: "image registry/graphservice:feature not found".into() };
    assert_eq!(state(&t, cx), Some(failing));
    assert_eq!(history(&t, cx)[0].2, "failing: image registry/graphservice:feature not found");
    // CI pushed the image meanwhile: the kubelet's next pull works.
    watch(&t, cx, dep("feature", 4, 6, 6, Progress::Settled));
    pods(&t, cx, vec![pod("feature", "a", true, "running", 0, None), pod("feature", "c", true, "running", 0, None)]);
    assert_eq!(state(&t, cx), Some(RolloutState::Done));
    assert_eq!(history(&t, cx)[0].2, "done", "the outcome follows");
}

#[gpui::test]
fn a_crash_loop_counts_from_two_restarts_and_stuck_is_final(cx: &mut TestAppContext) {
    let t = setup(cx, false);
    deploy(&t, cx, "main", "feature").unwrap();
    answer(&t, cx, Ok(6));
    watch(&t, cx, dep("feature", 4, 6, 6, Progress::Rolling));
    pods(&t, cx, vec![pod("feature", "a", false, "waiting: CrashLoopBackOff", 1, None)]);
    assert_eq!(state(&t, cx), Some(RolloutState::Rolling { ready: 0, of: 2 }));
    pods(&t, cx, vec![pod("feature", "a", false, "waiting: CrashLoopBackOff", 2, None)]);
    assert!(matches!(state(&t, cx), Some(RolloutState::Failing { reason, .. }) if reason.starts_with("crash loop")));
    watch(&t, cx, dep("feature", 4, 6, 6, Progress::Stuck));
    assert_eq!(state(&t, cx), Some(RolloutState::Stuck { reason: "it went past its progress deadline".into() }));
    assert_eq!(cx.read(|cx| t.cluster.read(cx).following()), 0);
}

#[gpui::test]
fn deploys_seen_from_elsewhere_are_followed_and_replace_ours(cx: &mut TestAppContext) {
    let t = setup(cx, false);
    deploy(&t, cx, "main", "feature").unwrap();
    answer(&t, cx, Ok(6));
    watch(&t, cx, dep("feature", 4, 6, 6, Progress::Rolling));
    // Someone deploys again before it's done (Slack, a colleague).
    let mut other = dep("hotfix", 5, 7, 6, Progress::Rolling);
    other.manager = Some("kubectl-set".into());
    watch(&t, cx, other);
    assert_eq!(history(&t, cx), [("feature".to_string(), ME.to_string(), "replaced by hotfix".to_string()), ("hotfix".to_string(), "kubectl-set".to_string(), String::new())]);
    assert_eq!(cx.read(|cx| t.cluster.read(cx).following()), 0, "a deploy seen elsewhere doesn't hold pods");
    // The same branch again: a revision that grows is a deploy too.
    watch(&t, cx, dep("hotfix", 6, 8, 8, Progress::Rolling));
    assert_eq!(history(&t, cx).last().map(|r| r.1.clone()), Some("seen".into()), "redeployed, nobody named");
    // A scale isn't one.
    let mut scaled = dep("hotfix", 6, 9, 9, Progress::Rolling);
    scaled.replicas = 3;
    watch(&t, cx, scaled);
    assert_eq!(history(&t, cx).len(), 3);
}

#[gpui::test]
fn the_history_keeps_ten_and_comes_back_after_a_restart(cx: &mut TestAppContext) {
    let dir = cx.update(|cx| devdeck_app::test_dir("deploy-history", cx));
    let t = setup(cx, false);
    cx.update(|cx| t.cluster.update(cx, |c, cx| c.keep_history(StateStore::new(dir.clone()), "work", cx)));
    for i in 0..(HISTORY_KEPT as u64 + 2) {
        watch(&t, cx, dep(&format!("b{i}"), 4 + i, 6 + i, 6 + i, Progress::Rolling));
    }
    assert_eq!(history(&t, cx).len(), HISTORY_KEPT);
    cx.run_until_parked();
    // devdeck closed mid-rollout: the next one follows it again, or closes it.
    let driver = Rc::new(Scripted::default());
    let again = cx.update(|cx| new_cluster(vec![Env { name: ENV.into(), protected: false }], driver, Some(1_000), cx));
    cx.update(|cx| again.update(cx, |c, cx| c.keep_history(StateStore::new(dir.clone()), "work", cx)));
    assert_eq!(cx.read(|cx| again.read(cx).history(ENV, SVC).len()), HISTORY_KEPT);
    cx.update(|cx| again.update(cx, |c, cx| c.want(ENV, cx)));
    let last = format!("b{}", HISTORY_KEPT + 1);
    cx.update(|cx| again.update(cx, |c, cx| c.apply(ENV, EnvUpdate::Deployments(vec![dep(&last, 99, 99, 99, Progress::Rolling)]), cx)));
    cx.run_until_parked();
    assert!(matches!(cx.read(|cx| again.read(cx).rollout(ENV, SVC).map(|r| r.state.clone())), Some(RolloutState::Rolling { .. })), "still runs its image: followed again");
    let third = cx.update(|cx| new_cluster(vec![Env { name: ENV.into(), protected: false }], Rc::new(Scripted::default()), Some(1_000), cx));
    cx.update(|cx| third.update(cx, |c, cx| c.keep_history(StateStore::new(dir.clone()), "work", cx)));
    cx.update(|cx| third.update(cx, |c, cx| c.want(ENV, cx)));
    cx.update(|cx| third.update(cx, |c, cx| c.apply(ENV, EnvUpdate::Deployments(vec![dep("elsewhere", 99, 99, 99, Progress::Settled)]), cx)));
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| third.read(cx).history(ENV, SVC).last().map(|r| r.outcome.clone())), Some("unknown: devdeck was closed".into()));
}

#[gpui::test]
fn the_access_review_is_asked_once_per_env_and_again_after_a_reload(cx: &mut TestAppContext) {
    let t = setup(cx, false);
    let check = |cx: &mut TestAppContext| cx.update(|cx| t.cluster.update(cx, |c, cx| c.check_access(ENV, cx)));
    check(cx);
    check(cx);
    assert_eq!(*t.driver.reviews.borrow(), 1);
    assert_eq!(cx.read(|cx| t.cluster.read(cx).access(ENV).cloned()), Some(Access::Asking));
    cx.update(|cx| t.cluster.update(cx, |c, cx| c.access_known(ENV, Err("timeout".into()), cx)));
    assert_eq!(cx.read(|cx| t.cluster.read(cx).deploy_blocked(ENV, SVC, "feature", false, cx)), None, "unknown: the write decides");
    cx.update(|cx| t.cluster.update(cx, |c, cx| c.reload(cx)));
    check(cx);
    assert_eq!(*t.driver.reviews.borrow(), 2, "permissions may have changed");
}

#[gpui::test]
fn the_image_changes_before_the_revision_moves(cx: &mut TestAppContext) {
    let t = setup(cx, false);
    deploy(&t, cx, "main", "feature").unwrap();
    answer(&t, cx, Ok(6));
    // What a real cluster sends: the patched spec first, the controller's revision after.
    watch(&t, cx, dep("feature", 3, 6, 5, Progress::Rolling));
    assert!(history(&t, cx).is_empty(), "not a deploy until the revision moves");
    watch(&t, cx, dep("feature", 4, 6, 6, Progress::Rolling));
    let r = cx.read(|cx| t.cluster.read(cx).rollout(ENV, SVC).cloned()).unwrap();
    assert_eq!((r.from.as_str(), r.to.as_str(), r.by.as_str()), ("main", "feature", ME));
}

#[gpui::test]
fn a_deploy_nobody_watches_the_pods_of_still_ends(cx: &mut TestAppContext) {
    let t = setup(cx, false);
    watch(&t, cx, dep("ci-build", 4, 6, 6, Progress::Rolling));
    assert_eq!(cx.read(|cx| t.cluster.read(cx).following()), 0);
    watch(&t, cx, dep("ci-build", 4, 6, 6, Progress::Settled));
    assert_eq!(state(&t, cx), Some(RolloutState::Done), "from the deployment's own counts");
    assert_eq!(history(&t, cx)[0].2, "done");
}

#[gpui::test]
fn an_untagged_image_is_pinned_as_the_spec_writes_it(cx: &mut TestAppContext) {
    let t = setup(cx, false);
    let mut d = dep("main", 3, 5, 5, Progress::Settled);
    d.containers[0].image = Image::parse(&format!("registry/{SVC}"));
    watch(&t, cx, d);
    cx.update(|cx| t.cluster.update(cx, |c, cx| c.deploy(ENV, SVC, &format!("registry/{SVC}"), "feature", false, crate::confirmed(), cx))).unwrap();
    assert_eq!(t.driver.writes.borrow()[0].1.from, format!("registry/{SVC}"), "not registry/graphservice:latest, which the spec doesn't say");
}

#[gpui::test]
fn a_deploy_left_failing_when_devdeck_closed_is_followed_again(cx: &mut TestAppContext) {
    let dir = cx.update(|cx| devdeck_app::test_dir("deploy-failing", cx));
    let t = setup(cx, false);
    cx.update(|cx| t.cluster.update(cx, |c, cx| c.keep_history(StateStore::new(dir.clone()), "work", cx)));
    // A service view holds its pods.
    let _view = cx.update(|cx| t.cluster.update(cx, |c, cx| c.pods_interest(ENV, PodScope::Service(SVC.into()), cx)));
    watch(&t, cx, dep("feature", 4, 6, 6, Progress::Rolling));
    pods(&t, cx, vec![pod("feature", "a", false, "waiting: ImagePullBackOff", 0, Some("not found"))]);
    assert!(history(&t, cx)[0].2.starts_with("failing"));
    cx.run_until_parked();
    let again = cx.update(|cx| new_cluster(vec![Env { name: ENV.into(), protected: false }], Rc::new(Scripted::default()), Some(1_000), cx));
    cx.update(|cx| again.update(cx, |c, cx| c.keep_history(StateStore::new(dir.clone()), "work", cx)));
    cx.update(|cx| again.update(cx, |c, cx| c.want(ENV, cx)));
    cx.update(|cx| again.update(cx, |c, cx| c.apply(ENV, EnvUpdate::Deployments(vec![dep("feature", 4, 6, 6, Progress::Settled)]), cx)));
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| again.read(cx).history(ENV, SVC)[0].outcome.clone()), "done", "followed again, and it recovered");
}

#[gpui::test]
fn a_refusal_nobody_waits_on_is_said_and_forgotten(cx: &mut TestAppContext) {
    let t = setup(cx, false);
    deploy(&t, cx, "main", "feature").unwrap();
    answer(&t, cx, Err("your account can't change deployments in dev33".into()));
    assert_eq!(cx.read(|cx| t.cluster.read(cx).write_state(ENV, SVC).cloned()), None, "said in a toast, nothing left behind");
    cx.update(|cx| t.cluster.update(cx, |c, _| c.watch_write(ENV, SVC, true)));
    deploy(&t, cx, "main", "feature").unwrap();
    answer(&t, cx, Err("nope".into()));
    assert_eq!(cx.read(|cx| t.cluster.read(cx).write_state(ENV, SVC).cloned()), Some(WriteState::Failed("nope".into())), "a confirmation waiting shows it");
}

#[gpui::test]
fn a_new_target_is_a_new_baseline(cx: &mut TestAppContext) {
    let t = setup(cx, false);
    cx.update(|cx| t.cluster.update(cx, |c, cx| c.restart([ENV.to_string()], cx)));
    cx.run_until_parked();
    watch(&t, cx, dep("other-namespace", 40, 50, 50, Progress::Settled));
    assert!(history(&t, cx).is_empty(), "another context's revision isn't a deploy");
}

// ------------------------------------------------------------------ redeploys (step 6)

/// The image the setup's deployment runs.
const MAIN: &str = "registry/graphservice:main";

fn redeploy(t: &T, cx: &mut TestAppContext) -> Result<(), String> {
    cx.update(|cx| t.cluster.update(cx, |c, cx| c.redeploy(ENV, SVC, MAIN, false, crate::confirmed(), cx)))
}

/// `main` redeployed: revision `revision`, generation `generation` observed, its template
/// `t-main-<revision>` stamped `at`.
fn redeployed(revision: u64, generation: u64, progress: Progress, at: &str) -> Deployment {
    let mut d = dep("main", revision, generation, generation, progress);
    d.template = Some(format!("t-main-{revision}"));
    d.restarted_at = Some(at.into());
    d.template_annotated = true;
    d
}

/// A pod of `main`'s template `template`.
fn pod_of(template: &str, id: &str, ready: bool) -> Pod {
    let mut p = pod("main", id, ready, if ready { "running" } else { "waiting: ContainerCreating" }, 0, None);
    p.template = template.into();
    p
}

/// The first redeploy's stamp on the tests' clock (1 000 s).
fn first_stamp() -> String {
    restart_stamp(1_000 * 1_000_000_000 + 1)
}

#[gpui::test]
fn a_redeploy_has_every_guard_of_a_write_but_already_runs(cx: &mut TestAppContext) {
    let t = setup(cx, true);
    let blocked = |cx: &mut TestAppContext, unlocked: bool| cx.read(|cx| t.cluster.read(cx).write_blocked(ENV, SVC, unlocked, cx));
    cx.update(|cx| t.cluster.update(cx, |c, cx| c.check_access(ENV, cx)));
    cx.update(|cx| t.cluster.update(cx, |c, cx| c.access_known(ENV, Ok(false), cx)));
    assert_eq!(blocked(cx, true), Some(format!("your account can't change deployments in {ENV}")));
    cx.update(|cx| t.cluster.update(cx, |c, cx| c.access_known(ENV, Ok(true), cx)));
    assert_eq!(blocked(cx, false), Some(format!("{ENV} is protected: unlock it first")), "the protected env (R7.6)");
    assert_eq!(blocked(cx, true), None, "the same image is what a redeploy asks for");
    assert_eq!(cx.read(|cx| t.cluster.read(cx).deploy_blocked(ENV, SVC, "main", true, cx)), Some(format!("{SVC} already runs main")), "a branch change still refuses it");
    assert_eq!(redeploy(&t, cx), Err(format!("{ENV} is protected: unlock it first")), "below the UI");
    cx.update(|cx| t.cluster.update(cx, |c, cx| c.redeploy(ENV, SVC, MAIN, true, crate::confirmed(), cx))).unwrap();
    assert_eq!(blocked(cx, true), Some(format!("a deploy of {SVC} is being sent")), "one write in flight");
    answer(&t, cx, Err("nope".into()));
    watch(&t, cx, dep("main", 3, 5, 5, Progress::Paused));
    assert_eq!(blocked(cx, true), Some(format!("rollouts of {SVC} are paused")));
    let mut two = dep("main", 3, 5, 5, Progress::Settled);
    two.containers[0].name = "app".into();
    two.containers.push(Container { name: "envoy".into(), image: Image::parse("registry/envoy:v1"), ports: vec![], resources: Resources::default(), config_maps: Vec::new(), pull_policy: None });
    watch(&t, cx, two);
    assert_eq!(blocked(cx, true), Some(format!("{SVC} has several containers and none is named {SVC}")), "never a sidecar");
}

#[gpui::test]
fn a_redeploy_is_pinned_to_the_image_shown_and_followed_to_done(cx: &mut TestAppContext) {
    let t = setup(cx, false);
    redeploy(&t, cx).unwrap();
    let (name, restart) = t.driver.restarts.borrow()[0].clone();
    assert_eq!(name, SVC);
    assert_eq!(restart, Restart { index: 0, container: SVC.into(), image: MAIN.into(), at: first_stamp(), generation: 5, annotated: false, manager: ME.into() });
    assert_eq!(first_stamp(), "1970-01-01T00:16:40.000000001Z", "RFC 3339 with fractional seconds");
    assert!(t.driver.writes.borrow().is_empty(), "no image change");
    assert_eq!(cx.read(|cx| t.cluster.read(cx).following()), 1, "its pods are held while followed");
    // The spec first (the stamp, the same image), the revision after.
    let mut spec = redeployed(3, 6, Progress::Rolling, &first_stamp());
    spec.observed = 5;
    watch(&t, cx, spec);
    assert!(history(&t, cx).is_empty(), "not a deploy until the revision moves");
    answer(&t, cx, Ok(6));
    watch(&t, cx, redeployed(4, 6, Progress::Rolling, &first_stamp()));
    let r = cx.read(|cx| t.cluster.read(cx).rollout(ENV, SVC).cloned()).unwrap();
    assert_eq!((r.from.as_str(), r.to.as_str(), r.by.as_str(), r.redeploy()), ("main", "main", ME, true));
    pods(&t, cx, vec![pod_of("t-main-4", "a", true), pod_of("t-main", "b", true)]);
    assert_eq!(state(&t, cx), Some(RolloutState::Rolling { ready: 1, of: 2 }), "the old pods run the same tag, but not the new template");
    assert_eq!(cx.read(|cx| t.cluster.read(cx).rollout(ENV, SVC).and_then(|r| r.label())), Some("redeploying main 1/2".into()));
    watch(&t, cx, redeployed(4, 6, Progress::Settled, &first_stamp()));
    pods(&t, cx, vec![pod_of("t-main-4", "a", true), pod_of("t-main-4", "c", true)]);
    assert_eq!(state(&t, cx), Some(RolloutState::Done));
    let record = cx.read(|cx| t.cluster.read(cx).history(ENV, SVC)[0].clone());
    assert_eq!((record.from.as_str(), record.to.as_str(), record.by.as_str(), record.outcome.as_str(), record.redeploy()), ("main", "main", ME, "done", true));
    assert_eq!((record.revision, record.restarted_at), (Some(4), Some(first_stamp())));
    assert_eq!(cx.read(|cx| t.cluster.read(cx).following()), 0, "done: its pods go");
}

#[gpui::test]
fn a_redeploy_seen_while_devdecks_is_sending_isnt_devdecks(cx: &mut TestAppContext) {
    let t = setup(cx, false);
    redeploy(&t, cx).unwrap();
    // A colleague's `kubectl rollout restart` lands first: the same image, another stamp.
    let mut theirs = redeployed(4, 6, Progress::Rolling, "1970-01-01T00:16:40Z");
    theirs.manager = Some("kubectl-rollout".into());
    watch(&t, cx, theirs);
    assert_eq!(history(&t, cx), [("main".to_string(), "kubectl-rollout".to_string(), String::new())]);
    assert_eq!(cx.read(|cx| t.cluster.read(cx).write_state(ENV, SVC).cloned()), Some(WriteState::Sending), "devdeck's still waits for its answer");
    // Then devdeck's, with its own stamp.
    answer(&t, cx, Ok(7));
    watch(&t, cx, redeployed(5, 7, Progress::Rolling, &first_stamp()));
    assert_eq!(
        history(&t, cx),
        [("main".to_string(), "kubectl-rollout".to_string(), "replaced by a redeploy".to_string()), ("main".to_string(), ME.to_string(), String::new())]
    );
}

#[gpui::test]
fn a_redeploy_that_changes_nothing_says_so_and_leaves_nothing_pending(cx: &mut TestAppContext) {
    let t = setup(cx, false);
    cx.update(|cx| t.cluster.update(cx, |c, _| c.watch_write(ENV, SVC, true)));
    redeploy(&t, cx).unwrap();
    answer(&t, cx, Ok(5));
    assert_eq!(cx.read(|cx| t.cluster.read(cx).write_state(ENV, SVC).cloned()), Some(WriteState::Failed(format!("nothing changed: the cluster left {SVC} as it was"))));
    assert_eq!(cx.read(|cx| t.cluster.read(cx).following()), 0);
    // A later redeploy carrying devdeck's old stamp isn't devdeck's: nothing is pending.
    let mut later = redeployed(4, 6, Progress::Rolling, &first_stamp());
    later.manager = Some("kubectl-rollout".into());
    watch(&t, cx, later);
    assert_eq!(history(&t, cx), [("main".to_string(), "kubectl-rollout".to_string(), String::new())]);
}

#[gpui::test]
fn a_redeploy_left_open_is_followed_again_only_on_its_revision(cx: &mut TestAppContext) {
    let dir = cx.update(|cx| devdeck_app::test_dir("redeploy-reopen", cx));
    let t = setup(cx, false);
    cx.update(|cx| t.cluster.update(cx, |c, cx| c.keep_history(StateStore::new(dir.clone()), "work", cx)));
    watch(&t, cx, redeployed(4, 6, Progress::Rolling, "2026-11-10T09:00:00.1Z"));
    cx.run_until_parked();
    let reopen = |cx: &mut TestAppContext, d: Deployment| {
        let c = cx.update(|cx| new_cluster(vec![Env { name: ENV.into(), protected: false }], Rc::new(Scripted::default()), Some(1_000), cx));
        cx.update(|cx| c.update(cx, |c, cx| c.keep_history(StateStore::new(dir.clone()), "work", cx)));
        cx.update(|cx| c.update(cx, |c, cx| c.want(ENV, cx)));
        cx.update(|cx| c.update(cx, |c, cx| c.apply(ENV, EnvUpdate::Deployments(vec![d]), cx)));
        cx.run_until_parked();
        c
    };
    let same = reopen(cx, redeployed(4, 6, Progress::Rolling, "2026-11-10T09:00:00.1Z"));
    assert!(matches!(cx.read(|cx| same.read(cx).rollout(ENV, SVC).map(|r| r.state.clone())), Some(RolloutState::Rolling { .. })), "still its revision: followed again");
    drop(same);
    cx.run_until_parked();
    let again = reopen(cx, redeployed(5, 7, Progress::Settled, "2026-11-10T09:30:00.2Z"));
    assert_eq!(cx.read(|cx| again.read(cx).history(ENV, SVC).last().map(|r| r.outcome.clone())), Some("unknown: devdeck was closed".into()), "the same tag, redeployed again since");
}

#[gpui::test]
fn a_second_write_waits_until_the_first_is_picked_up(cx: &mut TestAppContext) {
    let t = setup(cx, false);
    redeploy(&t, cx).unwrap();
    answer(&t, cx, Ok(6));
    assert_eq!(redeploy(&t, cx), Err(format!("a deploy of {SVC} is still being picked up")), "answered, not seen yet");
    watch(&t, cx, redeployed(4, 6, Progress::Rolling, &first_stamp()));
    let next = cx.read(|cx| t.cluster.read(cx).write_blocked(ENV, SVC, false, cx));
    assert_eq!(next, None, "seen: the next write may go");
}

#[gpui::test]
fn a_redeploy_without_a_revision_is_known_by_its_stamp(cx: &mut TestAppContext) {
    let t = setup(cx, false);
    watch(&t, cx, dep("main", 0, 5, 5, Progress::Settled));
    redeploy(&t, cx).unwrap();
    answer(&t, cx, Ok(6));
    let mut d = redeployed(0, 6, Progress::Rolling, &first_stamp());
    d.revision = 0;
    watch(&t, cx, d);
    assert_eq!(history(&t, cx).last().map(|r| r.1.clone()), Some(ME.into()), "recorded, nothing left pending");
    assert_eq!(cx.read(|cx| t.cluster.read(cx).write_blocked(ENV, SVC, false, cx)), None);
}

#[gpui::test]
fn two_redeploys_never_share_a_stamp(cx: &mut TestAppContext) {
    let t = setup(cx, false);
    redeploy(&t, cx).unwrap();
    answer(&t, cx, Ok(6));
    watch(&t, cx, redeployed(4, 6, Progress::Settled, &first_stamp()));
    redeploy(&t, cx).unwrap();
    let stamps: Vec<String> = t.driver.restarts.borrow().iter().map(|(_, r)| r.at.clone()).collect();
    assert_eq!(stamps.len(), 2);
    assert_ne!(stamps[0], stamps[1], "the same clock second, two stamps");
}
