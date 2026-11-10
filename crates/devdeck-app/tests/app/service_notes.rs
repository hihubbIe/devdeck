//! Notifications step C: a service's notify dropdown and what it tells (plan
//! "notifications"). Over a scripted driver: each option notifies once when ticked and
//! never when not, one notification per (rollout, kind), crashes outside a rollout once per
//! template until it is ready again, devdeck's own deploys always saying how they end. On
//! the demo: the dropdown under `b` (toggles, colors, keys), the rocket's states,
//! subscriptions outliving the view, Open from such a toast, the palette's "turn all off".

use devdeck_app::cluster::{Cluster, ClusterDriver, EnvUpdate, ImageChange, LOAD_DELAY, PodScope, PodUpdate, ROLLOUT_STEP, Restart, new_cluster};
use devdeck_app::cluster_notes::{Notify, STEADY};
use devdeck_core::state::StateStore;
use devdeck_app::notifications::{self, ServiceNotifications};
use devdeck_app::shell::Shell;
use devdeck_app::views::service_view::ServiceView;
use devdeck_core::cluster::{Container, ContainerStatus, Deployment, Env, Image, Pod, Progress, Resources};
use devdeck_core::view_addr::ViewAddr;
use devdeck_ui::overlay::{kind_look, layer, layer_state};
use devdeck_ui::{MenuItem, Note, ToastKind, theme, toasts};
use gpui::{AnyWindowHandle, AppContext, Context, Entity, Task, TestAppContext};
use std::collections::BTreeSet;
use std::rc::Rc;
use std::time::Duration;

const ENV: &str = "dev33";
const SVC: &str = "graphservice";

/// Takes writes and pod watches; the test plays the watch and the answers.
struct Scripted;

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
    fn set_image(&self, _env: &str, _name: &str, _change: ImageChange, _cx: &mut Context<Cluster>) -> Option<Task<()>> {
        Some(Task::ready(()))
    }
    fn restart(&self, _env: &str, _name: &str, _restart: Restart, _cx: &mut Context<Cluster>) -> Option<Task<()>> {
        Some(Task::ready(()))
    }
}

fn dep(tag: &str, revision: u64, generation: u64, observed: u64, progress: Progress) -> Deployment {
    let c = Container { name: SVC.into(), image: Image::parse(&format!("registry/{SVC}:{tag}")), ports: vec![], resources: Resources::default(), config_maps: Vec::new(), pull_policy: None };
    Deployment { name: SVC.into(), containers: vec![c], replicas: 2, ready: 2, updated: 2, progress, selector: format!("app={SVC}"), revision, generation, observed, template: Some(format!("t-{tag}")), ..Default::default() }
}

/// `d` changed by `by` (someone else's deploy).
fn by(mut d: Deployment, by: &str) -> Deployment {
    d.manager = Some(by.into());
    d
}

fn failing(mut d: Deployment, why: &str) -> Deployment {
    d.failure = Some(why.into());
    d.ready = 0;
    d
}

fn pod(tag: &str, id: &str, ready: bool, state: &str, restarts: u32) -> Pod {
    let c = ContainerStatus { name: SVC.into(), ready, restarts, state: state.into(), ..Default::default() };
    Pod { name: format!("{SVC}-t-{tag}-{id}"), owner: Some(SVC.into()), template: format!("t-{tag}"), ready, tag: tag.into(), containers: vec![c], started: Some(1), ..Default::default() }
}

const CRASH: &str = "waiting: CrashLoopBackOff";

fn setup(cx: &mut TestAppContext) -> Entity<Cluster> {
    let envs = vec![Env { name: ENV.into(), protected: false }];
    let cluster = cx.update(|cx| new_cluster(envs, Rc::new(Scripted), Some(1_000), cx));
    cx.update(|cx| devdeck_app::edit_mode::set(true, cx));
    cx.update(|cx| {
        cluster.update(cx, |c, cx| {
            c.workspace = Some("work".into());
            c.want(ENV, cx);
        })
    });
    watch(&cluster, cx, dep("main", 3, 5, 5, Progress::Settled));
    cluster
}

fn watch(c: &Entity<Cluster>, cx: &mut TestAppContext, d: Deployment) {
    cx.update(|cx| c.update(cx, |c, cx| c.apply(ENV, EnvUpdate::Deployments(vec![d]), cx)));
    cx.run_until_parked();
}

fn pods(c: &Entity<Cluster>, cx: &mut TestAppContext, pods: Vec<Pod>) {
    cx.update(|cx| c.update(cx, |c, cx| c.apply_pods(ENV, PodScope::Service(SVC.into()), PodUpdate::Pods(pods), cx)));
    cx.run_until_parked();
}

fn tick(c: &Entity<Cluster>, cx: &mut TestAppContext, what: &[Notify]) {
    for w in what {
        cx.update(|cx| c.update(cx, |c, cx| c.set_notify(ENV, SVC, *w, true, cx)));
    }
    cx.run_until_parked();
}

/// What was told, oldest first: kind, title, text.
fn told(cx: &mut TestAppContext) -> Vec<(ToastKind, String, String)> {
    notes(cx).into_iter().map(|n| (n.kind, n.title.map(|t| t.to_string()).unwrap_or_default(), n.text.to_string())).collect()
}

fn notes(cx: &mut TestAppContext) -> Vec<Note> {
    cx.update(|cx| toasts(cx).read(cx).notes.notes().iter().rev().cloned().collect())
}

fn texts(cx: &mut TestAppContext) -> Vec<String> {
    told(cx).into_iter().map(|(_, _, t)| t).collect()
}

/// Someone else deploys `feature`, it finishes; then redeploys it; then deploys a broken
/// tag that fails, then is stuck.
fn others_deploy(c: &Entity<Cluster>, cx: &mut TestAppContext) {
    watch(c, cx, by(dep("feature", 4, 6, 5, Progress::Rolling), "helm"));
    watch(c, cx, by(dep("feature", 4, 6, 6, Progress::Settled), "helm"));
    watch(c, cx, by(dep("feature", 5, 7, 7, Progress::Rolling), "kubectl-rollout"));
    watch(c, cx, by(dep("feature", 5, 7, 7, Progress::Settled), "kubectl-rollout"));
    watch(c, cx, failing(by(dep("broken", 6, 8, 8, Progress::Rolling), "helm"), "image registry/graphservice:broken not found"));
    // The reason changes with every pod update: still one notification.
    watch(c, cx, failing(by(dep("broken", 6, 8, 8, Progress::Rolling), "helm"), "back-off pulling image"));
    watch(c, cx, failing(by(dep("broken", 6, 8, 8, Progress::Stuck), "helm"), "it went past its progress deadline"));
}

#[gpui::test]
fn nothing_is_told_unless_ticked(cx: &mut TestAppContext) {
    let c = setup(cx);
    others_deploy(&c, cx);
    assert_eq!(told(cx), [], "off at start");
    assert_eq!(cx.read(|cx| c.read(cx).crash_watches()), 0, "no pods held for it");
}

#[gpui::test]
fn each_option_notifies_once_when_ticked(cx: &mut TestAppContext) {
    let c = setup(cx);
    tick(&c, cx, &[Notify::BranchDeployed, Notify::Redeployed, Notify::Finished, Notify::Failed]);
    others_deploy(&c, cx);
    let title = format!("{SVC} on {ENV}");
    assert_eq!(
        told(cx),
        [
            (ToastKind::Info, title.clone(), "feature deployed by helm (was main)".into()),
            (ToastKind::Ok, title.clone(), "now running feature (was main)".into()),
            (ToastKind::Info, title.clone(), "feature redeployed by kubectl-rollout".into()),
            (ToastKind::Ok, title.clone(), "redeployed feature: every pod ready".into()),
            (ToastKind::Info, title.clone(), "broken deployed by helm (was feature)".into()),
            (ToastKind::Err, title.clone(), "rollout to broken failing (0/2 ready): image registry/graphservice:broken not found".into()),
        ],
        "one per (rollout, kind): failing then stuck is one kind"
    );
    let actions: Vec<Vec<String>> = notes(cx).iter().map(|n| n.actions.iter().map(|a| a.label.to_string()).collect()).collect();
    assert_eq!(actions[0], ["Open here"], "Open (here: no view of it open)");
    assert_eq!(actions[5], ["Open here", "Logs"], "a failure has Logs too");
    assert_eq!(notes(cx)[0].source.as_deref(), Some("work"), "the cluster's workspace, whichever shows");
    // Replaced isn't an event; the deploy replacing it is.
    watch(&c, cx, by(dep("fixed", 7, 9, 8, Progress::Rolling), "helm"));
    assert_eq!(texts(cx).last().map(String::as_str), Some("fixed deployed by helm (was broken)"));
    assert_eq!(told(cx).len(), 7);
}

#[gpui::test]
fn only_what_is_ticked_is_told(cx: &mut TestAppContext) {
    let c = setup(cx);
    tick(&c, cx, &[Notify::Finished]);
    others_deploy(&c, cx);
    assert_eq!(texts(cx), ["now running feature (was main)", "redeployed feature: every pod ready"]);
    cx.update(|cx| c.update(cx, |c, cx| c.set_notify(ENV, SVC, Notify::Finished, false, cx)));
    watch(&c, cx, by(dep("other", 7, 9, 9, Progress::Settled), "helm"));
    assert_eq!(told(cx).len(), 2, "unticked: nothing more");
    assert!(cx.read(|cx| c.read(cx).subscriptions().is_empty()), "nothing ticked: no subscription left");
}

#[gpui::test]
fn own_deploys_always_say_how_they_end_never_that_they_began(cx: &mut TestAppContext) {
    let c = setup(cx);
    tick(&c, cx, &[Notify::BranchDeployed]);
    let deploy = |cx: &mut TestAppContext, from: &str, to: &str| cx.update(|cx| c.update(cx, |c, cx| c.deploy(ENV, SVC, &format!("registry/{SVC}:{from}"), to, false, cx))).unwrap();
    let answer = |cx: &mut TestAppContext, g: u64| {
        cx.update(|cx| c.update(cx, |c, cx| c.write_done(ENV, SVC, Ok(g), cx)));
        cx.run_until_parked();
    };
    deploy(cx, "main", "feature");
    answer(cx, 6);
    watch(&c, cx, dep("feature", 4, 6, 5, Progress::Rolling));
    assert_eq!(told(cx), [], "no 'branch deployed' for devdeck's own, ticked or not");
    pods(&c, cx, vec![pod("feature", "a", true, "running", 0), pod("feature", "b", true, "running", 0)]);
    watch(&c, cx, dep("feature", 4, 6, 6, Progress::Settled));
    assert_eq!(told(cx), [(ToastKind::Ok, format!("{SVC} on {ENV}"), "now running feature (was main)".into())], "finished, though not ticked");
    deploy(cx, "feature", "broken");
    answer(cx, 7);
    watch(&c, cx, dep("broken", 5, 7, 7, Progress::Rolling));
    pods(&c, cx, vec![pod("broken", "a", false, CRASH, 2)]);
    pods(&c, cx, vec![pod("broken", "a", false, CRASH, 3)]);
    watch(&c, cx, dep("broken", 5, 7, 7, Progress::Stuck));
    let errs: Vec<String> = told(cx).into_iter().filter(|(k, _, _)| *k == ToastKind::Err).map(|(_, _, t)| t).collect();
    assert_eq!(errs.len(), 1, "failing then stuck: one, though not ticked: {errs:?}");
    assert!(errs[0].starts_with("rollout to broken failing (0/2 ready): crash loop"), "{}", errs[0]);
}

#[gpui::test]
fn crashes_outside_a_rollout_once_per_template_until_it_is_ready_again(cx: &mut TestAppContext) {
    let c = setup(cx);
    tick(&c, cx, &[Notify::Crashing]);
    assert_eq!(cx.read(|cx| c.read(cx).crash_watches()), 1, "its pods are held while ticked");
    pods(&c, cx, vec![pod("main", "a", true, "running", 0), pod("main", "b", false, CRASH, 1)]);
    assert_eq!(told(cx), [], "under pod_failure's threshold");
    pods(&c, cx, vec![pod("main", "a", true, "running", 0), pod("main", "b", false, CRASH, 2)]);
    pods(&c, cx, vec![pod("main", "a", true, "running", 0), pod("main", "b", false, CRASH, 5)]);
    pods(&c, cx, vec![pod("main", "a", true, "running", 0), pod("main", "b", false, "running", 5)]);
    pods(&c, cx, vec![pod("main", "a", true, "running", 0), pod("main", "b", false, CRASH, 6)]);
    assert_eq!(told(cx).len(), 1, "once, however it swings, while one of its pods isn't ready");
    let (kind, _, text) = told(cx)[0].clone();
    assert_eq!(kind, ToastKind::Err);
    assert!(text.starts_with(&format!("pod {SVC}-t-main-b: crash loop")), "{text}");
    assert_eq!(notes(cx)[0].actions.iter().map(|a| a.label.to_string()).collect::<Vec<_>>(), ["Open here", "Logs"]);
    pods(&c, cx, vec![pod("main", "a", true, "running", 0), pod("main", "b", true, "running", 6)]);
    advance(cx, STEADY + Duration::from_secs(60));
    pods(&c, cx, vec![pod("main", "a", true, "running", 0), pod("main", "b", false, CRASH, 8)]);
    assert_eq!(told(cx).len(), 2, "every pod ready, restarts unchanged, for a while: a new crash is told");
    // During a rollout its own events say it.
    watch(&c, cx, by(dep("feature", 4, 6, 6, Progress::Rolling), "helm"));
    pods(&c, cx, vec![pod("feature", "a", false, CRASH, 4)]);
    assert_eq!(told(cx).len(), 2, "not during a rollout");
    cx.update(|cx| c.update(cx, |c, cx| c.set_notify(ENV, SVC, Notify::Crashing, false, cx)));
    assert_eq!(cx.read(|cx| c.read(cx).crash_watches()), 0, "unticked: its pods are let go");
}

/// One replica without a readiness probe turns ready after every restart: that isn't
/// recovering, so its crash loop is told once.
#[gpui::test]
fn a_pod_ready_between_its_crashes_is_told_once(cx: &mut TestAppContext) {
    let c = setup(cx);
    tick(&c, cx, &[Notify::Crashing]);
    for restarts in [3, 4, 5] {
        pods(&c, cx, vec![pod("main", "a", false, CRASH, restarts)]);
        advance(cx, Duration::from_secs(30));
        pods(&c, cx, vec![pod("main", "a", true, "running", restarts)]);
        advance(cx, Duration::from_secs(60));
    }
    pods(&c, cx, vec![pod("main", "a", false, CRASH, 6)]);
    assert_eq!(told(cx).len(), 1, "once for the whole loop");
    // Steady at last: ready, restarts unchanged, longer than STEADY.
    pods(&c, cx, vec![pod("main", "a", true, "running", 6)]);
    advance(cx, STEADY / 2);
    pods(&c, cx, vec![pod("main", "a", true, "running", 6)]);
    advance(cx, STEADY / 2 + Duration::from_secs(1));
    pods(&c, cx, vec![pod("main", "a", false, CRASH, 7)]);
    assert_eq!(told(cx).len(), 2, "after a steady stretch, a crash is told again");
}

#[gpui::test]
fn a_template_gone_is_forgotten(cx: &mut TestAppContext) {
    let c = setup(cx);
    tick(&c, cx, &[Notify::Crashing]);
    pods(&c, cx, vec![pod("main", "a", false, CRASH, 3)]);
    assert_eq!(cx.read(|cx| c.read(cx).crash_marks()), 1);
    watch(&c, cx, by(dep("feature", 4, 6, 6, Progress::Rolling), "helm"));
    pods(&c, cx, vec![pod("feature", "a", true, "running", 0), pod("feature", "b", true, "running", 0)]);
    watch(&c, cx, by(dep("feature", 4, 6, 6, Progress::Settled), "helm"));
    assert_eq!(cx.read(|cx| c.read(cx).crash_marks()), 0, "no pod of it, and the deployment runs another");
}

#[gpui::test]
fn a_failing_rollouts_pods_are_not_told_again_as_crashes(cx: &mut TestAppContext) {
    let c = setup(cx);
    tick(&c, cx, &[Notify::Failed, Notify::Crashing]);
    watch(&c, cx, by(dep("broken", 4, 6, 6, Progress::Rolling), "helm"));
    pods(&c, cx, vec![pod("broken", "a", false, CRASH, 2)]);
    watch(&c, cx, by(dep("broken", 4, 6, 6, Progress::Stuck), "helm"));
    pods(&c, cx, vec![pod("broken", "a", false, CRASH, 4)]);
    let errs: Vec<String> = told(cx).into_iter().map(|(_, _, t)| t).collect();
    assert_eq!(errs.len(), 1, "the rollout's failure, not its pods again once it's stuck: {errs:?}");
    assert!(errs[0].starts_with("rollout to broken failing"), "{}", errs[0]);
}

/// Without the newest template (not reported yet, or stale), the pods in trouble when the
/// failure is told are what marks it.
#[gpui::test]
fn a_failure_marks_its_pods_templates_when_the_template_is_unknown(cx: &mut TestAppContext) {
    let c = setup(cx);
    tick(&c, cx, &[Notify::Failed, Notify::Crashing]);
    let unknown = |p: Progress| {
        let mut d = by(dep("broken", 4, 6, 6, p), "helm");
        d.template = None;
        d
    };
    watch(&c, cx, unknown(Progress::Rolling));
    pods(&c, cx, vec![pod("broken", "a", false, CRASH, 2)]);
    watch(&c, cx, unknown(Progress::Stuck));
    pods(&c, cx, vec![pod("broken", "a", false, CRASH, 5)]);
    assert_eq!(told(cx).len(), 1, "one toast: {:?}", texts(cx));
}

/// devdeck's own deploy left failing, then devdeck closed: the history it reads back.
fn left_failing(cx: &mut TestAppContext, tag: &str) -> std::path::PathBuf {
    let dir = cx.update(|cx| devdeck_app::test_dir(tag, cx));
    let c = setup(cx);
    cx.update(|cx| c.update(cx, |c, cx| c.keep_history(StateStore::new(dir.clone()), "work", cx)));
    cx.update(|cx| c.update(cx, |c, cx| c.deploy(ENV, SVC, &format!("registry/{SVC}:main"), "feature", false, cx))).unwrap();
    cx.update(|cx| c.update(cx, |c, cx| c.write_done(ENV, SVC, Ok(6), cx)));
    watch(&c, cx, dep("feature", 4, 6, 6, Progress::Rolling));
    pods(&c, cx, vec![pod("feature", "a", false, CRASH, 3)]);
    assert_eq!(told(cx).len(), 1, "its failure, told once");
    dir
}

/// devdeck started again on that history: the cluster, its first list `d`.
fn relaunch(cx: &mut TestAppContext, dir: &std::path::Path, d: Deployment) -> Entity<Cluster> {
    let c = cx.update(|cx| new_cluster(vec![Env { name: ENV.into(), protected: false }], Rc::new(Scripted), Some(1_000), cx));
    cx.update(|cx| {
        c.update(cx, |c, cx| {
            c.workspace = Some("work".into());
            c.keep_history(StateStore::new(dir.to_path_buf()), "work", cx);
            c.want(ENV, cx);
        })
    });
    watch(&c, cx, d);
    c
}

#[gpui::test]
fn a_deploy_left_failing_isnt_told_again_at_launch(cx: &mut TestAppContext) {
    let dir = left_failing(cx, "notes-left-failing");
    let c = relaunch(cx, &dir, failing(dep("feature", 4, 6, 6, Progress::Rolling), "crash loop: exit 1"));
    watch(&c, cx, failing(dep("feature", 4, 6, 6, Progress::Rolling), "back-off restarting failed container"));
    assert_eq!(told(cx).len(), 1, "still failing, whatever the reason: what its record ended on, said before");
    watch(&c, cx, dep("feature", 4, 6, 6, Progress::Settled));
    assert_eq!(texts(cx).last().map(String::as_str), Some("now running feature (was main)"), "recovered: told");
    assert_eq!(told(cx).len(), 2);
}

#[gpui::test]
fn a_deploy_that_finished_while_devdeck_was_closed_says_so_once(cx: &mut TestAppContext) {
    let dir = left_failing(cx, "notes-finished-closed");
    let c = relaunch(cx, &dir, dep("feature", 4, 6, 6, Progress::Settled));
    watch(&c, cx, dep("feature", 4, 6, 6, Progress::Settled));
    assert_eq!(texts(cx), ["rollout to feature failing (0/2 ready): crash loop: it keeps exiting", "now running feature (was main)"]);
}

// ------------------------------------------------------------------ on the demo

fn start(cx: &mut TestAppContext, tag: &str) -> Entity<Shell> {
    let config = cx.update(|cx| devdeck_app::test_dir(tag, cx));
    cx.update(|cx| devdeck_app::init_with_config_dir(&config, cx));
    cx.update(|cx| devdeck_app::edit_mode::set(true, cx));
    let shell = cx.update(|cx| Shell::start(devdeck_app::demo_workspaces(), "work", None, cx));
    cx.update(|cx| shell.update(cx, |s, _| s.quit_for_real = false));
    cx.run_until_parked();
    shell
}

fn main_window(shell: &Entity<Shell>, cx: &TestAppContext) -> AnyWindowHandle {
    cx.read(|cx| shell.read(cx).main_window().expect("main").into())
}

fn advance(cx: &mut TestAppContext, d: Duration) {
    cx.executor().advance_clock(d);
    cx.run_until_parked();
}

fn cluster_of(shell: &Entity<Shell>, cx: &TestAppContext) -> Entity<Cluster> {
    cx.read(|cx| shell.read(cx).workspaces.iter().find(|w| w.id == "work").unwrap().services.read(cx).cluster.clone().unwrap())
}

const DEMO_ENV: &str = "dev35";
const DEMO_SVC: &str = "entityservice";

fn svc_ref() -> devdeck_core::layout::ViewRef {
    ViewAddr::Svc { env: DEMO_ENV.into(), name: DEMO_SVC.into() }.to_ref()
}

fn open_service(shell: &Entity<Shell>, cx: &mut TestAppContext) -> Entity<ServiceView> {
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    let r = svc_ref();
    let open = r.clone();
    cx.update(|cx| deck.update(cx, |d, cx| d.apply(cx, |l| l.open(open, l.main().focused).map(|_| ()))));
    advance(cx, LOAD_DELAY);
    cx.read(|cx| deck.read(cx).view(&r).unwrap().downcast::<ServiceView>().unwrap())
}

fn menu_items(main: AnyWindowHandle, cx: &mut TestAppContext) -> Option<Vec<MenuItem>> {
    cx.update(|cx| {
        let menu = layer(main, cx).read(cx).menu.clone()?;
        Some(menu.read(cx).items().to_vec())
    })
}

fn ticked(c: &Entity<Cluster>, cx: &TestAppContext) -> BTreeSet<Notify> {
    cx.read(|cx| c.read(cx).notify_options(DEMO_ENV, DEMO_SVC))
}

#[gpui::test]
fn b_opens_the_dropdown_whose_toggles_keep_it_open(cx: &mut TestAppContext) {
    let shell = start(cx, "notify-menu");
    let main = main_window(&shell, cx);
    let c = cluster_of(&shell, cx);
    let _view = open_service(&shell, cx);
    cx.simulate_keystrokes(main, "b");
    cx.run_until_parked();
    let items = menu_items(main, cx).expect("b opened it");
    let t = cx.update(|cx| theme(cx).clone());
    assert_eq!(items.iter().map(|i| i.label.to_string()).collect::<Vec<_>>(), ["branch deployed", "redeployed", "rollout finished", "deploy failed or stuck", "pods crashing"]);
    let colors: Vec<_> = Notify::ALL.iter().map(|n| kind_look(n.kind(), &t).1).collect();
    assert_eq!(items.iter().map(|i| i.label_color(&t)).collect::<Vec<_>>(), colors, "each label in its toast's color");
    assert_eq!(colors, [t.accent, t.accent, t.ok, t.err, t.err]);
    assert!(items.iter().all(|i| i.toggle && !i.checked && i.icon.is_none()), "checkboxes, none ticked, no icons");
    cx.simulate_keystrokes(main, "space down down space");
    cx.run_until_parked();
    assert!(menu_items(main, cx).is_some(), "ticking keeps it open");
    assert_eq!(ticked(&c, cx), BTreeSet::from([Notify::BranchDeployed, Notify::Finished]));
    cx.simulate_keystrokes(main, "enter");
    cx.run_until_parked();
    assert_eq!(ticked(&c, cx), BTreeSet::from([Notify::BranchDeployed]), "Enter toggles too");
    cx.simulate_keystrokes(main, "escape");
    cx.run_until_parked();
    assert!(!cx.update(|cx| layer_state(main, cx).0), "Esc closes it");
}

#[gpui::test]
fn the_rocket_is_dim_with_its_reason_until_another_tag_is_typed(cx: &mut TestAppContext) {
    let shell = start(cx, "rocket");
    let main = main_window(&shell, cx);
    let view = open_service(&shell, cx);
    assert_eq!(cx.read(|cx| view.read(cx).save_blocked(cx)), Some("the tag is unchanged".into()), "dim, its reason in its tooltip");
    let mut vcx = gpui::VisualTestContext::from_window(main, cx);
    let rocket = vcx.debug_bounds("tag-deploy").expect("the rocket");
    let bell = vcx.debug_bounds("tag-notify").expect("the notify dropdown");
    let field = vcx.debug_bounds("tag-editor").expect("the tag field");
    assert!(field.right() <= rocket.left() && rocket.right() <= bell.left(), "the tag field, the rocket, the dropdown");
    assert!(vcx.debug_bounds("tag-save").is_none(), "no save button any more");
    vcx.simulate_click(rocket.center(), gpui::Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(vcx.update(|_, cx| layer_state(main, cx).1), 0, "refused: no confirmation");
    let last = vcx.update(|_, cx| toasts(cx).read(cx).notes.notes().front().map(|n| n.text.to_string()));
    assert_eq!(last.as_deref(), Some("Not deployed: the tag is unchanged."), "a click says why");
    vcx.simulate_keystrokes("t ctrl-a");
    vcx.simulate_input("lan-160");
    vcx.run_until_parked();
    assert_eq!(vcx.read(|cx| view.read(cx).save_blocked(cx)), None, "ready");
    vcx.simulate_click(rocket.center(), gpui::Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(vcx.update(|_, cx| layer_state(main, cx).1), 1, "the confirmation");
    vcx.simulate_keystrokes("escape ctrl-s");
    vcx.run_until_parked();
    assert_eq!(vcx.update(|_, cx| layer_state(main, cx).1), 1, "ctrl+s still opens it");
}

#[gpui::test]
fn subscriptions_outlive_the_view_and_open_reveals_it(cx: &mut TestAppContext) {
    let shell = start(cx, "notify-keep");
    let main = main_window(&shell, cx);
    let c = cluster_of(&shell, cx);
    let view = open_service(&shell, cx);
    cx.simulate_keystrokes(main, "b down down space escape");
    cx.run_until_parked();
    assert_eq!(ticked(&c, cx), BTreeSet::from([Notify::Finished]));
    drop(view);
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    cx.update(|cx| deck.update(cx, |d, cx| d.apply(cx, |l| l.close_view(&svc_ref())))).unwrap();
    cx.run_until_parked();
    assert!(cx.read(|cx| deck.read(cx).view(&svc_ref()).is_none()), "closed");
    assert_eq!(ticked(&c, cx), BTreeSet::from([Notify::Finished]), "kept with the view closed");
    // devdeck's own deploy with the view closed: its outcome is always told.
    let (image, was, replicas) = cx.read(|cx| {
        let d = c.read(cx).deployment(DEMO_ENV, DEMO_SVC).unwrap();
        (d.main().image.raw.clone(), d.tag(), d.replicas)
    });
    cx.update(|cx| c.update(cx, |c, cx| c.deploy(DEMO_ENV, DEMO_SVC, &image, "lan-160", false, cx))).unwrap();
    advance(cx, ROLLOUT_STEP * (replicas + 1));
    let note = notes(cx).into_iter().rev().find(|n| n.title.as_deref() == Some("entityservice on dev35")).expect("told");
    assert_eq!((note.kind, note.text.to_string()), (ToastKind::Ok, format!("now running lan-160 (was {was})")));
    let open = note.actions[0].clone();
    assert_eq!(open.label.as_ref(), "Open here", "the view isn't open");
    let done = cx.update_window(main, |_, window, cx| notifications::resolve(&open, window, cx)).unwrap();
    cx.run_until_parked();
    assert!(done);
    let view = cx.read(|cx| deck.read(cx).view(&svc_ref()).expect("opened by Open").downcast::<ServiceView>().unwrap());
    assert!(cx.read(|cx| deck.read(cx).is_visible(&svc_ref())));
    let items = cx.read(|cx| view.read(cx).notify_items(cx));
    assert_eq!(items.iter().map(|i| i.checked).collect::<Vec<_>>(), [false, false, true, false, false], "shown ticked again");
}

#[gpui::test]
fn the_palettes_list_turns_every_subscription_off(cx: &mut TestAppContext) {
    let shell = start(cx, "notify-all-off");
    let main = main_window(&shell, cx);
    let c = cluster_of(&shell, cx);
    cx.update(|cx| {
        c.update(cx, |c, cx| {
            c.set_notify("dev35", "graphservice", Notify::Crashing, true, cx);
            c.set_notify("dev33", "entityservice", Notify::Failed, true, cx);
        })
    });
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| c.read(cx).crash_watches()), 1);
    cx.update_window(main, |_, window, cx| window.dispatch_action(Box::new(ServiceNotifications), cx)).unwrap();
    cx.run_until_parked();
    let items = menu_items(main, cx).expect("the palette's action opened the list");
    let labels: Vec<String> = items.iter().map(|i| i.label.to_string()).collect();
    assert_eq!(labels, ["entityservice on dev33", "graphservice on dev35", "", "turn all off"], "alphabetical, then turn all off");
    assert_eq!(items[1].submenu.iter().map(|i| i.checked).collect::<Vec<_>>(), [false, false, false, false, true], "each its dropdown");
    let off = items[3].run.clone().unwrap();
    cx.update_window(main, |_, window, cx| off(window, cx)).unwrap();
    cx.run_until_parked();
    assert!(cx.read(|cx| c.read(cx).subscriptions().is_empty()));
    assert_eq!(cx.read(|cx| c.read(cx).crash_watches()), 0);
    assert_eq!(cx.update(|cx| notifications::subscriptions_menu(cx)).iter().map(|i| i.label.to_string()).collect::<Vec<_>>(), ["no service notifications"]);
}

/// The palette's list by keyboard: → opens a row's dropdown, space ticks in it, the menu
/// open meanwhile; a click on a row opens that service.
#[gpui::test]
fn the_palettes_list_ticks_by_keyboard_and_opens_a_service(cx: &mut TestAppContext) {
    let shell = start(cx, "notify-list-keys");
    let main = main_window(&shell, cx);
    let c = cluster_of(&shell, cx);
    cx.update(|cx| c.update(cx, |c, cx| c.set_notify("dev33", "entityservice", Notify::Failed, true, cx)));
    cx.update_window(main, |_, window, cx| window.dispatch_action(Box::new(ServiceNotifications), cx)).unwrap();
    cx.run_until_parked();
    cx.simulate_keystrokes(main, "right space");
    cx.run_until_parked();
    assert!(menu_items(main, cx).is_some(), "still open");
    assert_eq!(cx.read(|cx| c.read(cx).notify_options("dev33", "entityservice")), BTreeSet::from([Notify::BranchDeployed, Notify::Failed]), "ticked in the submenu");
    cx.simulate_keystrokes(main, "escape escape");
    cx.run_until_parked();
    let row = notifications_row(cx);
    cx.update_window(main, |_, window, cx| row(window, cx)).unwrap();
    cx.run_until_parked();
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    let svc = ViewAddr::Svc { env: "dev33".into(), name: "entityservice".into() }.to_ref();
    assert!(cx.read(|cx| deck.read(cx).is_visible(&svc)), "the row opened its service");
}

/// The first row's click of the palette's list.
fn notifications_row(cx: &mut TestAppContext) -> devdeck_ui::overlay::MenuAction {
    cx.update(|cx| notifications::subscriptions_menu(cx))[0].run.clone().unwrap()
}

fn open_svc(shell: &Entity<Shell>, cx: &mut TestAppContext, env: &str, name: &str) -> Entity<ServiceView> {
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    let r = ViewAddr::Svc { env: env.into(), name: name.into() }.to_ref();
    let open = r.clone();
    cx.update(|cx| deck.update(cx, |d, cx| d.apply(cx, |l| l.open(open, l.main().focused).map(|_| ()))));
    advance(cx, LOAD_DELAY);
    cx.read(|cx| deck.read(cx).view(&r).unwrap().downcast::<ServiceView>().unwrap())
}

fn click(main: AnyWindowHandle, cx: &mut TestAppContext, selector: &'static str) {
    let mut vcx = gpui::VisualTestContext::from_window(main, cx);
    let at = vcx.debug_bounds(selector).expect("drawn");
    vcx.simulate_click(at.center(), gpui::Modifiers::none());
    vcx.run_until_parked();
}

fn last_toast(cx: &mut TestAppContext) -> Option<String> {
    texts(cx).last().cloned()
}

#[gpui::test]
fn the_rocket_on_a_protected_env_or_with_edit_mode_off(cx: &mut TestAppContext) {
    let shell = start(cx, "rocket-guards");
    let main = main_window(&shell, cx);
    let view = open_svc(&shell, cx, "hades", "proxyapi");
    cx.update(|cx| view.update(cx, |v, cx| v.tag.update(cx, |i, cx| i.set_text("lan-161", cx))));
    cx.run_until_parked();
    cx.update(|cx| devdeck_app::edit_mode::set(false, cx));
    assert_eq!(cx.read(|cx| view.read(cx).save_blocked(cx)), Some(devdeck_app::edit_mode::OFF.into()), "edit mode first: dim, said in its tooltip");
    click(main, cx, "tag-deploy");
    assert_eq!(cx.update(|cx| layer_state(main, cx).1), 0, "never the env's name to then refuse");
    assert_eq!(last_toast(cx), Some(format!("Not deployed: {}.", devdeck_app::edit_mode::OFF)));
    cx.update(|cx| devdeck_app::edit_mode::set(true, cx));
    assert_eq!(cx.read(|cx| view.read(cx).save_blocked(cx)), Some("hades is protected: unlock it first".into()), "locked: dim with its reason");
    click(main, cx, "tag-deploy");
    assert_eq!(cx.update(|cx| layer_state(main, cx).1), 1, "a click asks to unlock");
}

#[gpui::test]
fn an_own_redeploy_says_how_it_ended(cx: &mut TestAppContext) {
    let shell = start(cx, "notify-own-redeploy");
    let c = cluster_of(&shell, cx);
    let _view = open_service(&shell, cx);
    let (image, replicas) = cx.read(|cx| {
        let d = c.read(cx).deployment(DEMO_ENV, DEMO_SVC).unwrap();
        (d.main().image.raw.clone(), d.replicas)
    });
    let tag = cx.read(|cx| c.read(cx).deployment(DEMO_ENV, DEMO_SVC).unwrap().tag());
    cx.update(|cx| c.update(cx, |c, cx| c.redeploy(DEMO_ENV, DEMO_SVC, &image, false, cx))).unwrap();
    advance(cx, ROLLOUT_STEP * (replicas + 1));
    let note = notes(cx).into_iter().rev().find(|n| n.title.as_deref() == Some("entityservice on dev35")).expect("told, though nothing is ticked");
    assert_eq!((note.kind, note.text.to_string()), (ToastKind::Ok, format!("redeployed {tag}: every pod ready")));
    assert_eq!(note.actions.iter().map(|a| a.label.to_string()).collect::<Vec<_>>(), ["Open"], "its view is open");
}
