//! M3 step 5: changing the deployed branch (R7.3, R7.4, R7.6, R5.6). Deploys over a scripted
//! driver whose watch and answers the test plays: guards in order, the write pinned to
//! what was confirmed, a rollout followed to done, failing with its reason and recovering,
//! stuck, replaced, deploys seen from elsewhere, the history kept and read back, the
//! access review. Nothing here reaches a cluster.

use devdeck_app::cluster::{Access, Cluster, ClusterDriver, EnvUpdate, ImageChange, PodScope, PodUpdate, WriteState, new_cluster};
use devdeck_core::cluster::{Container, ContainerStatus, Deployment, Env, HISTORY_KEPT, Image, Pod, Progress, Resources, RolloutState};
use devdeck_core::state::StateStore;
use gpui::{Context, Entity, Task, TestAppContext};
use std::cell::RefCell;
use std::rc::Rc;

const ENV: &str = "dev33";
const SVC: &str = "graphservice";

/// Records writes and access reviews; the test answers them and plays the watch.
#[derive(Default)]
struct Scripted {
    writes: RefCell<Vec<(String, ImageChange)>>,
    reviews: RefCell<usize>,
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
    fn watch_pods(&self, _env: &str, _scope: PodScope, _selector: Option<&str>, _cx: &mut Context<Cluster>) -> Option<Task<()>> {
        Some(Task::ready(()))
    }
    fn set_image(&self, _env: &str, name: &str, change: ImageChange, _cx: &mut Context<Cluster>) -> Option<Task<()>> {
        self.writes.borrow_mut().push((name.into(), change));
        Some(Task::ready(()))
    }
    fn can_deploy(&self, _env: &str, _cx: &mut Context<Cluster>) -> Option<Task<()>> {
        *self.reviews.borrow_mut() += 1;
        Some(Task::ready(()))
    }
}

fn dep(tag: &str, revision: u64, generation: u64, observed: u64, progress: Progress) -> Deployment {
    let c = Container { name: SVC.into(), image: Image::parse(&format!("registry/{SVC}:{tag}")), ports: vec![], resources: Resources::default(), config_maps: Vec::new() };
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
    cx.update(|cx| devdeck_app::edit_mode::set(true, cx));
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
    cx.update(|cx| t.cluster.update(cx, |c, cx| c.deploy(ENV, SVC, &format!("registry/{SVC}:{from}"), to, false, cx)))
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
    cx.update(|cx| devdeck_app::edit_mode::set(false, cx));
    assert_eq!(blocked(cx, "feature"), Some(devdeck_app::edit_mode::OFF.into()), "edit mode first (R5.6)");
    cx.update(|cx| devdeck_app::edit_mode::set(true, cx));
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
    two.containers.push(Container { name: "envoy".into(), image: Image::parse("registry/envoy:v1"), ports: vec![], resources: Resources::default(), config_maps: Vec::new() });
    watch(&t, cx, two);
    assert_eq!(cx.read(|cx| t.cluster.read(cx).deploy_blocked(ENV, SVC, "feature", true, cx)), Some(format!("{SVC} has several containers and none is named {SVC}")), "never a sidecar");
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
    assert_eq!(history(&t, cx), [("feature".to_string(), "devdeck".to_string(), String::new())]);
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
    assert_eq!(history(&t, cx), [("feature".to_string(), "devdeck".to_string(), "replaced by hotfix".to_string()), ("hotfix".to_string(), "kubectl-set".to_string(), String::new())]);
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
    assert_eq!((r.from.as_str(), r.to.as_str(), r.by.as_str()), ("main", "feature", "devdeck"));
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
    cx.update(|cx| t.cluster.update(cx, |c, cx| c.deploy(ENV, SVC, &format!("registry/{SVC}"), "feature", false, cx))).unwrap();
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
