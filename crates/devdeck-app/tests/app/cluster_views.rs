//! M1 step 5c: logs, pod, branch matrix and port views, and the sidebar following
//! rollouts (R7.5, R7.7, R6.9, R4.3). Fake clock only.

use devdeck_app::cluster::{Cluster, LOAD_DELAY, ROLLOUT_STEP};
use devdeck_app::fake_driver::STOP_DELAY;
use devdeck_app::shell::Shell;
use devdeck_app::sidebar::Activity;
use devdeck_app::views::logs_view::{LOGS_TICK, MinLevel, ServiceLogsView};
use devdeck_app::views::matrix_view::MatrixView;
use devdeck_app::views::port_view::PortView;
use devdeck_core::runs::Level;
use devdeck_core::view_addr::ViewAddr;
use gpui::{AnyView, AnyWindowHandle, AppContext, Entity, Focusable, Modifiers, TestAppContext, VisualTestContext};
use std::time::Duration;

fn start(cx: &mut TestAppContext, tag: &str) -> Entity<Shell> {
    let config = cx.update(|cx| devdeck_app::test_dir(tag, cx));
    cx.update(|cx| devdeck_app::init_with_config_dir(&config, cx));
    // These deploy (the demo's simulated rollouts): edit mode on (R5.6, its own tests in edit_mode.rs).
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

fn cluster(shell: &Entity<Shell>, cx: &TestAppContext) -> Entity<Cluster> {
    cx.read(|cx| shell.read(cx).workspaces.iter().find(|w| w.id == "work").unwrap().services.read(cx).cluster.clone().unwrap())
}

fn open(shell: &Entity<Shell>, cx: &mut TestAppContext, addr: ViewAddr) -> AnyView {
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    let r = addr.to_ref();
    let o = r.clone();
    cx.update(|cx| deck.update(cx, |d, cx| d.apply(cx, |l| l.open(o, l.main().focused).map(|_| ()))));
    advance(cx, LOAD_DELAY);
    cx.read(|cx| deck.read(cx).view(&r).unwrap())
}

fn focus_view<V: Focusable>(view: &Entity<V>, main: AnyWindowHandle, cx: &mut TestAppContext) {
    let h = cx.read(|cx| view.read(cx).focus_handle(cx));
    cx.update_window(main, |_, window, cx| h.focus(window, cx)).unwrap();
}

#[gpui::test]
fn logs_filter_find_pause_and_stop_when_hidden(cx: &mut TestAppContext) {
    let shell = start(cx, "logs");
    let main = main_window(&shell, cx);
    let v = open(&shell, cx, ViewAddr::Logs { env: "dev35".into(), name: "graphservice".into() }).downcast::<ServiceLogsView>().unwrap();
    advance(cx, LOGS_TICK * 20);
    let lines = cx.read(|cx| v.read(cx).feed.buffer.len());
    assert!(lines >= 20, "every running pod writes ({lines})");
    cx.simulate_keystrokes(main, "1");
    let only_errors = cx.read(|cx| {
        let b = &v.read(cx).feed.buffer;
        (0..b.row_count()).all(|r| b.row(r).is_some_and(|e| e.level == Level::Error))
    });
    assert!(only_errors && cx.read(|cx| v.read(cx).min_level) == MinLevel::Error);
    cx.simulate_keystrokes(main, "4 / ");
    cx.simulate_input(main, "slow n");
    assert_eq!(cx.read(|cx| v.read(cx).find.read(cx).text().to_string()), "slow n", "n typed in find is text");
    cx.simulate_keystrokes(main, "backspace backspace");
    let found = cx.read(|cx| v.read(cx).matches());
    assert!(found > 1);
    let first = cx.read(|cx| v.read(cx).current);
    assert!(first.is_some(), "typing jumps to the first match");
    focus_view(&v, main, cx);
    cx.simulate_keystrokes(main, "n");
    assert_ne!(cx.read(|cx| v.read(cx).current), first, "n goes to the next match");
    cx.simulate_keystrokes(main, "shift-n");
    assert_eq!(cx.read(|cx| v.read(cx).current), first);
    cx.simulate_keystrokes(main, "p");
    let paused_at = cx.read(|cx| v.read(cx).feed.buffer.len());
    advance(cx, LOGS_TICK * 4);
    assert_eq!(cx.read(|cx| v.read(cx).feed.buffer.len()), paused_at, "paused");
    assert!(cx.read(|cx| v.read(cx).feed.held()) > 0, "what came meanwhile waits");
    cx.simulate_keystrokes(main, "p");
    assert!(cx.read(|cx| v.read(cx).feed.buffer.len()) > paused_at, "shown on resume");
    // Another tab in front: the feed stops.
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    let r = ViewAddr::Logs { env: "dev35".into(), name: "graphservice".into() }.to_ref();
    cx.update(|cx| deck.update(cx, |d, cx| d.apply(cx, |l| l.activate(l.find_view(&r).unwrap().0, 0))));
    cx.run_until_parked();
    assert!(!cx.read(|cx| v.read(cx).is_streaming()));
}

#[gpui::test]
fn a_pod_that_goes_away_says_so(cx: &mut TestAppContext) {
    let shell = start(cx, "pod");
    let c = cluster(&shell, cx);
    let _ = open(&shell, cx, ViewAddr::Svc { env: "dev35".into(), name: "vesselservice".into() });
    cx.update(|cx| c.update(cx, |c, cx| c.set_image("dev35", "vesselservice", "lan-broken", false, cx))).unwrap();
    advance(cx, ROLLOUT_STEP);
    let pull = cx.read(|cx| c.read(cx).pods("dev35", "vesselservice").iter().find(|p| p.tag == "lan-broken").unwrap().name.clone());
    let _ = open(&shell, cx, ViewAddr::Pod { env: "dev35".into(), pod: pull.clone() });
    assert!(cx.read(|cx| c.read(cx).find_pod("dev35", &pull).is_some()));
    cx.update(|cx| c.update(cx, |c, cx| c.undo("dev35", "vesselservice", false, cx))).unwrap();
    cx.run_until_parked();
    assert!(cx.read(|cx| c.read(cx).find_pod("dev35", &pull).is_none()), "the pull pod is gone; its view says so");
}

#[gpui::test]
fn matrix_highlights_a_tag_and_follows_rollouts(cx: &mut TestAppContext) {
    let shell = start(cx, "matrix");
    let main = main_window(&shell, cx);
    let c = cluster(&shell, cx);
    let m = open(&shell, cx, ViewAddr::Matrix).downcast::<MatrixView>().unwrap();
    // dev31 is the first column, visible even in a narrow pane.
    let tag = cx.read(|cx| m.read(cx).cell("dev31", "entityservice", cx)).expect("deployed");
    let missing = devdeck_core::demo::catalog("work").cluster.unwrap().services.into_iter().find(|s| devdeck_core::demo::deployment("dls", s).is_none()).unwrap();
    assert_eq!(cx.read(|cx| m.read(cx).cell("dls", &missing, cx)), None, "empty cell");
    let mut vcx = VisualTestContext::from_window(main, cx);
    let cell = vcx.debug_bounds("cell-dev31-entityservice").expect("cell drawn");
    vcx.simulate_click(cell.center(), Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(vcx.read(|cx| m.read(cx).highlight.clone()), Some(tag.clone()));
    vcx.simulate_keystrokes("escape");
    assert_eq!(vcx.read(|cx| m.read(cx).highlight.clone()), None, "esc clears");
    vcx.update(|_, cx| c.update(cx, |c, cx| c.set_image("dev35", "entityservice", "lan-177", false, cx))).unwrap();
    let replicas = vcx.read(|cx| c.read(cx).deployment("dev35", "entityservice").unwrap().replicas);
    vcx.executor().advance_clock(ROLLOUT_STEP * replicas);
    vcx.run_until_parked();
    assert_eq!(vcx.read(|cx| m.read(cx).cell("dev35", "entityservice", cx)).as_deref(), Some("lan-177"));
}

#[gpui::test]
fn ports_show_their_holder_and_stop_devdeck_runs(cx: &mut TestAppContext) {
    let shell = start(cx, "ports");
    let main = main_window(&shell, cx);
    let p = open(&shell, cx, ViewAddr::Port { port: "5720".into() }).downcast::<PortView>().unwrap();
    let holder = cx.read(|cx| p.read(cx).holder(cx)).expect("entityservice run listens on 5720");
    assert_eq!(holder.holder, "entityservice · run");
    let mut vcx = VisualTestContext::from_window(main, cx);
    let b = vcx.debug_bounds("stop-holder").expect("button");
    vcx.simulate_click(b.center(), Modifiers::none());
    vcx.executor().advance_clock(STOP_DELAY);
    vcx.run_until_parked();
    assert!(vcx.read(|cx| p.read(cx).holder(cx)).is_none(), "nothing listens once stopped");
    let fwd = open(&shell, cx, ViewAddr::Port { port: "5900".into() }).downcast::<PortView>().unwrap();
    assert!(cx.read(|cx| fwd.read(cx).holder(cx)).is_some_and(|h| h.run.is_none()), "an outside holder can't be stopped here");
}

#[gpui::test]
fn the_cluster_list_follows_a_rollout(cx: &mut TestAppContext) {
    let shell = start(cx, "list");
    let c = cluster(&shell, cx);
    let sidebar = cx.read(|cx| shell.read(cx).active_sidebar());
    cx.update(|cx| sidebar.update(cx, |s, cx| s.show(Activity::Cluster, cx)));
    advance(cx, LOAD_DELAY);
    cx.update(|cx| c.update(cx, |c, cx| c.set_image("dev35", "proxyapi", "lan-190", false, cx))).unwrap();
    cx.run_until_parked();
    let item = |cx: &TestAppContext| cx.read(|cx| sidebar.read(cx).items(cx).into_iter().find(|i| i.key == "proxyapi").unwrap());
    assert!(item(cx).busy, "rolling");
    let replicas = cx.read(|cx| c.read(cx).deployment("dev35", "proxyapi").unwrap().replicas);
    advance(cx, ROLLOUT_STEP * replicas);
    assert!(!item(cx).busy, "done");
    assert_eq!(item(cx).detail, "lan-190");
}
