//! Cluster as a full view (docs/plans/cluster-view.md, 4b): its list, then the selected
//! service's page; the branch matrix the whole width; the page's panels, its pod rows
//! opening in place, its env rows and its app. 4c: a deployment's parts (the front-end
//! apps inside `frontendapp`), each deployed on its own.

use devdeck_app::cluster::{Cluster, LOAD_DELAY, ROLLOUT_STEP};
use devdeck_app::shell::Shell;
use devdeck_app::sidebar::{Activity, Sidebar};
use devdeck_app::views::service_view::ServiceView;
use devdeck_core::layout::ViewRef;
use devdeck_core::view_addr::ViewAddr;
use gpui::{AnyWindowHandle, Entity, Modifiers, TestAppContext, VisualTestContext};

fn start(cx: &mut TestAppContext, tag: &str) -> Entity<Shell> {
    let config = cx.update(|cx| devdeck_app::test_dir(&format!("{tag}-config"), cx));
    cx.update(|cx| devdeck_app::init_with_config_dir(&config, cx));
    let shell = cx.update(|cx| Shell::start(devdeck_app::demo_workspaces(), "work", None, cx));
    cx.update(|cx| shell.update(cx, |s, _| s.quit_for_real = false));
    cx.run_until_parked();
    shell
}

fn main_window(shell: &Entity<Shell>, cx: &TestAppContext) -> AnyWindowHandle {
    cx.read(|cx| shell.read(cx).main_window().expect("main").into())
}

fn sidebar(shell: &Entity<Shell>, cx: &TestAppContext) -> Entity<Sidebar> {
    cx.read(|cx| shell.read(cx).active_sidebar())
}

fn details(shell: &Entity<Shell>, cx: &TestAppContext) -> Option<Vec<ViewRef>> {
    cx.read(|cx| shell.read(cx).active_deck().read(cx).outside().map(<[ViewRef]>::to_vec))
}

fn svc(env: &str, name: &str) -> ViewRef {
    ViewAddr::Svc { env: env.into(), name: name.into() }.to_ref()
}

/// Shows Cluster with `name` selected, its env read.
fn select(shell: &Entity<Shell>, cx: &mut TestAppContext, name: &str) {
    let s = sidebar(shell, cx);
    cx.update(|cx| s.update(cx, |s, cx| s.show_item(Activity::Cluster, name, cx)));
    cx.executor().advance_clock(LOAD_DELAY);
    cx.run_until_parked();
}

fn page(shell: &Entity<Shell>, cx: &TestAppContext, r: &ViewRef) -> Entity<ServiceView> {
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    cx.read(|cx| deck.read(cx).view(r).expect("built").downcast::<ServiceView>().expect("a service page"))
}

#[gpui::test]
fn the_page_follows_the_selection_on_the_lists_env(cx: &mut TestAppContext) {
    let shell = start(cx, "cl-follow");
    let s = sidebar(&shell, cx);
    cx.update(|cx| s.update(cx, |s, cx| s.show(Activity::Cluster, cx)));
    cx.executor().advance_clock(LOAD_DELAY);
    cx.run_until_parked();
    assert_eq!(details(&shell, cx), Some(vec![]), "nothing selected yet");
    let env = cx.read(|cx| s.read(cx).list_env(cx)).expect("the list's env");
    select(&shell, cx, "graphservice");
    assert_eq!(details(&shell, cx), Some(vec![svc(&env, "graphservice")]));
    select(&shell, cx, "proxyapi");
    assert_eq!(details(&shell, cx), Some(vec![svc(&env, "proxyapi")]), "a new selection, a new page");
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    assert!(cx.read(|cx| deck.read(cx).layout().find_view(&svc(&env, "proxyapi")).is_none()), "no tab in the pulse");
    let mut vcx = VisualTestContext::from_window(main_window(&shell, cx), cx);
    assert!(vcx.debug_bounds("row-pods-proxyapi").is_some(), "a row ends with its pods");
}

#[gpui::test]
fn the_matrix_takes_the_whole_width_and_gives_the_list_back(cx: &mut TestAppContext) {
    let shell = start(cx, "cl-matrix");
    select(&shell, cx, "graphservice");
    let s = sidebar(&shell, cx);
    let mut vcx = VisualTestContext::from_window(main_window(&shell, cx), cx);
    let button = vcx.debug_bounds("open-matrix").expect("the matrix icon in the list's header");
    vcx.simulate_click(button.center(), Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(details(&shell, cx), Some(vec![ViewAddr::Matrix.to_ref()]));
    let mut vcx = VisualTestContext::from_window(main_window(&shell, cx), cx);
    assert!(vcx.debug_bounds("open-matrix").is_none(), "no list beside it");
    let back = vcx.debug_bounds("details-list").expect("a way back to the list");
    vcx.simulate_click(back.center(), Modifiers::none());
    vcx.run_until_parked();
    let env = cx.read(|cx| s.read(cx).list_env(cx)).unwrap();
    assert_eq!(details(&shell, cx), Some(vec![svc(&env, "graphservice")]), "the service left");
    assert!(VisualTestContext::from_window(main_window(&shell, cx), cx).debug_bounds("open-matrix").is_some(), "the list again");
}

#[gpui::test]
fn the_page_has_its_panels_and_a_pod_row_opens_in_place(cx: &mut TestAppContext) {
    let shell = start(cx, "cl-page");
    select(&shell, cx, "graphservice");
    let s = sidebar(&shell, cx);
    let env = cx.read(|cx| s.read(cx).list_env(cx)).unwrap();
    let view = page(&shell, cx, &svc(&env, "graphservice"));
    let mut vcx = VisualTestContext::from_window(main_window(&shell, cx), cx);
    for panel in ["svc-deployment", "svc-pods-panel", "svc-logs", "svc-envs", "svc-history", "svc-containers"] {
        assert!(vcx.debug_bounds(panel).is_some(), "{panel}");
    }
    // Deploying's actions in the Deployment panel, side by side.
    let panel = vcx.debug_bounds("svc-deployment").unwrap();
    let (deploy, redeploy, bell) = (vcx.debug_bounds("svc-deploy").unwrap(), vcx.debug_bounds("svc-redeploy").unwrap(), vcx.debug_bounds("tag-notify").unwrap());
    assert!(panel.contains(&deploy.center()) && panel.contains(&bell.center()), "in the panel");
    assert!(deploy.right() <= redeploy.left() && redeploy.right() <= bell.left(), "together: {deploy:?} {redeploy:?} {bell:?}");
    assert!((deploy.center().y - bell.center().y).abs() < devdeck_ui::logical(1.), "on one line: {deploy:?} {bell:?}");
    // A pod's row opens under itself, and its Logs narrow the page's logs to it.
    assert!(vcx.debug_bounds("pod-logs-0").is_none(), "closed");
    let row = vcx.debug_bounds("pod-0").expect("a pod row");
    vcx.simulate_click(row.center(), Modifiers::none());
    vcx.run_until_parked();
    let pod = vcx.read(|cx| view.read(cx).open_pods.iter().next().cloned()).expect("its row open");
    let logs = vcx.debug_bounds("pod-logs-0").expect("its Logs");
    let after = vcx.debug_bounds("pod-1").expect("the next row");
    assert!(after.top() >= logs.bottom(), "the next row moves down, under it");
    vcx.simulate_click(logs.center(), Modifiers::none());
    vcx.run_until_parked();
    let shown = vcx.read(|cx| view.read(cx).logs.read(cx).feed.buffer.pod().map(ToString::to_string));
    assert_eq!(shown.as_deref(), Some(pod.as_str()), "the logs panel shows its lines");
    let row = vcx.debug_bounds("pod-0").unwrap();
    vcx.simulate_click(row.center(), Modifiers::none());
    vcx.run_until_parked();
    assert!(vcx.read(|cx| view.read(cx).open_pods.is_empty()), "closed again");
}

#[gpui::test]
fn an_env_row_shows_that_env_and_the_header_goes_to_its_app(cx: &mut TestAppContext) {
    let shell = start(cx, "cl-envs");
    select(&shell, cx, "graphservice");
    let s = sidebar(&shell, cx);
    let env = cx.read(|cx| s.read(cx).list_env(cx)).unwrap();
    let other = if env == "dev33" { "dev34" } else { "dev33" };
    cx.executor().advance_clock(LOAD_DELAY);
    cx.run_until_parked();
    let mut vcx = VisualTestContext::from_window(main_window(&shell, cx), cx);
    assert!(vcx.debug_bounds(Box::leak(format!("env-pods-{other}").into_boxed_str())).is_some(), "every env read, its pods");
    let row = vcx.debug_bounds(Box::leak(format!("env-{other}").into_boxed_str())).expect("its row");
    vcx.simulate_click(row.center(), Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(cx.read(|cx| s.read(cx).list_env(cx)).as_deref(), Some(other), "the list on that env");
    assert_eq!(details(&shell, cx), Some(vec![svc(other, "graphservice")]), "its page there");
    // Its app, from the header.
    cx.executor().advance_clock(LOAD_DELAY);
    cx.run_until_parked();
    let mut vcx = VisualTestContext::from_window(main_window(&shell, cx), cx);
    let app = vcx.debug_bounds("details-action-svc.app").expect("its app's icon");
    vcx.simulate_click(app.center(), Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(cx.read(|cx| s.read(cx).activity()), Activity::Apps);
    assert_eq!(cx.read(|cx| s.read(cx).selected.clone()).as_deref(), Some("graphservice"));
}

fn cluster(shell: &Entity<Shell>, cx: &TestAppContext) -> Entity<Cluster> {
    cx.read(|cx| shell.read(cx).workspaces.iter().find(|w| w.id == "work").unwrap().services.read(cx).cluster.clone().unwrap())
}

#[gpui::test]
fn a_part_deploys_on_its_own_through_the_picker_and_its_confirmation(cx: &mut TestAppContext) {
    let shell = start(cx, "cl-parts");
    select(&shell, cx, "frontendapp");
    let main = main_window(&shell, cx);
    let s = sidebar(&shell, cx);
    let env = cx.read(|cx| s.read(cx).list_env(cx)).unwrap();
    let c = cluster(&shell, cx);
    let before = cx.read(|cx| c.read(cx).deployment(&env, "frontendapp").cloned()).expect("frontendapp runs there");
    let mut vcx = VisualTestContext::from_window(main, cx);
    for i in 0..before.init_containers.len() {
        assert!(vcx.debug_bounds(Box::leak(format!("part-{i}").into_boxed_str())).is_some(), "a row per app in it");
    }
    let deploy = vcx.debug_bounds("part-deploy-0").expect("its Deploy…");
    vcx.simulate_click(deploy.center(), Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(vcx.update(|_, cx| devdeck_ui::overlay::layer_state(main, cx).1), 1, "the picker");
    vcx.simulate_input("v9.9.9");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("deploy-part").is_some(), "the confirmation says only that app changes");
    vcx.simulate_keystrokes("tab enter");
    vcx.run_until_parked();
    cx.executor().advance_clock(ROLLOUT_STEP * (before.replicas + 1));
    cx.run_until_parked();
    let after = cx.read(|cx| c.read(cx).deployment(&env, "frontendapp").cloned()).unwrap();
    let part = before.init_containers[0].image.short_name().to_string();
    assert_eq!(after.part(&part).unwrap().1.image.label(), "v9.9.9", "its init container's image");
    assert_eq!(after.main().image, before.main().image, "the deployment's own image stays");
    assert_eq!(after.init_containers[1..], before.init_containers[1..], "the other apps keep theirs");
    let last = cx.read(|cx| c.read(cx).history(&env, "frontendapp").last().cloned()).expect("recorded");
    assert_eq!((last.from, last.to), (format!("{part}:{}", before.init_containers[0].image.label()), format!("{part}:v9.9.9")));
    assert_eq!(last.outcome, "done", "followed to its end");
}

#[gpui::test]
fn a_front_end_apps_deployments_are_its_part_in_frontendapp(cx: &mut TestAppContext) {
    let shell = start(cx, "cl-part-app");
    let s = sidebar(&shell, cx);
    cx.update(|cx| s.update(cx, |s, cx| s.show_item(Activity::Apps, "front-end/controlboardapp", cx)));
    cx.executor().advance_clock(LOAD_DELAY);
    cx.run_until_parked();
    let c = cluster(&shell, cx);
    let tag = cx.read(|cx| c.read(cx).deployment("dev35", "frontendapp").and_then(|d| d.part("controlboardapp").map(|(_, k)| k.image.label()))).expect("its part");
    let mut vcx = VisualTestContext::from_window(main_window(&shell, cx), cx);
    let row = vcx.debug_bounds("deployment-dev35").expect("its deployment on dev35");
    vcx.simulate_click(row.center(), Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(cx.read(|cx| s.read(cx).activity()), Activity::Cluster);
    assert_eq!(details(&shell, cx), Some(vec![svc("dev35", "frontendapp")]), "the deployment it's in");
    assert!(!tag.is_empty());
}
