//! Apps as a full view (docs/plans/apps-view.md, 3a): its list, then its selection's
//! details instead of the pulse's panes; drilling into what the details open and back;
//! what opens in the pulse shows Pulses first.

use devdeck_app::shell::Shell;
use devdeck_app::sidebar::{Activity, Sidebar};
use devdeck_app::views::app_view::AppView;
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

fn app(id: &str) -> ViewRef {
    ViewAddr::App { app: id.into() }.to_ref()
}

/// Shows Apps with `id` selected.
fn select(shell: &Entity<Shell>, cx: &mut TestAppContext, id: &str) {
    let s = sidebar(shell, cx);
    cx.update(|cx| s.update(cx, |s, cx| s.show_item(Activity::Apps, id, cx)));
    cx.run_until_parked();
}

#[gpui::test]
fn the_details_follow_the_selection_and_hide_the_pulse(cx: &mut TestAppContext) {
    let shell = start(cx, "apps-follow");
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    let tab = cx.read(|cx| deck.read(cx).layout().views().into_iter().find(|r| deck.read(cx).is_visible(r))).expect("a tab on screen");
    assert_eq!(details(&shell, cx), None, "Pulses: the panes show");
    let s = sidebar(&shell, cx);
    cx.update(|cx| s.update(cx, |s, cx| s.show(Activity::Apps, cx)));
    cx.run_until_parked();
    assert_eq!(details(&shell, cx), Some(vec![]), "nothing selected yet");
    assert!(!cx.read(|cx| deck.read(cx).is_visible(&tab)), "the pulse's tabs are hidden");
    let mut vcx = VisualTestContext::from_window(main_window(&shell, cx), cx);
    assert!(vcx.debug_bounds("details").is_some(), "the details take the panes' place");
    select(&shell, cx, "entityservice");
    assert_eq!(details(&shell, cx), Some(vec![app("entityservice")]));
    assert!(cx.read(|cx| deck.read(cx).is_visible(&app("entityservice"))), "the app view is on screen");
    select(&shell, cx, "graphservice");
    assert_eq!(details(&shell, cx), Some(vec![app("graphservice")]));
    // Back to Pulses: the panes again.
    cx.update(|cx| s.update(cx, |s, cx| s.show(Activity::Layouts, cx)));
    cx.run_until_parked();
    assert_eq!(details(&shell, cx), None);
    assert!(cx.read(|cx| deck.read(cx).is_visible(&tab)));
}

#[gpui::test]
fn the_details_drill_into_what_they_open_and_go_back(cx: &mut TestAppContext) {
    let shell = start(cx, "apps-drill");
    let main = main_window(&shell, cx);
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    select(&shell, cx, "entityservice");
    let tabs = cx.read(|cx| deck.read(cx).layout().views());
    let view = cx.read(|cx| deck.read(cx).view(&app("entityservice")).unwrap().downcast::<AppView>().unwrap());
    cx.update(|cx| view.update(cx, |v, cx| v.open_output("go:run", cx)));
    // Held here, it would count as a leak below.
    drop(view);
    let mut vcx = VisualTestContext::from_window(main, cx);
    vcx.run_until_parked();
    let run = ViewAddr::Run { app: "entityservice".into(), cmd: "go:run".into() }.to_ref();
    // Its output opens over the app view, not in the pulse.
    let open = |cx: &mut VisualTestContext| cx.update(|_, cx| shell.read(cx).active_deck().read(cx).outside().map(<[ViewRef]>::to_vec));
    assert_eq!(open(&mut vcx), Some(vec![app("entityservice"), run.clone()]));
    assert_eq!(vcx.update(|_, cx| deck.read(cx).layout().views()), tabs, "no tab opened");
    assert!(vcx.debug_bounds("details-back").is_some(), "a way back");
    // The breadcrumb's first part goes back to the app.
    let crumb = vcx.debug_bounds("crumb-0").expect("the app's part");
    vcx.simulate_click(crumb.center(), Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(open(&mut vcx), Some(vec![app("entityservice")]));
    assert!(vcx.debug_bounds("details-back").is_none(), "nothing to go back to");
    // Drilled again, then another app: a new stack, the run view let go.
    vcx.update(|_, cx| deck.update(cx, |d, cx| d.drill(&app("entityservice"), run.clone(), cx)));
    let back = vcx.debug_bounds("details-back").expect("drawn");
    vcx.simulate_click(back.center(), Modifiers::none());
    assert_eq!(open(&mut vcx), Some(vec![app("entityservice")]), "back: the one before");
    vcx.update(|_, cx| deck.update(cx, |d, cx| d.drill(&app("entityservice"), run.clone(), cx)));
    select(&shell, cx, "graphservice");
    assert_eq!(details(&shell, cx), Some(vec![app("graphservice")]));
    assert!(cx.read(|cx| deck.read(cx).view(&run).is_none()), "the run view went with its stack");
    // A few more syncs so it ages past the check window (as the deck's own leak test).
    for _ in 0..3 {
        cx.update(|cx| deck.update(cx, |d, cx| d.apply(cx, |_| Ok(()))));
        cx.run_until_parked();
    }
    assert_eq!(cx.update(|cx| deck.update(cx, |d, _| d.leaked_views(3))), 0, "and was released");
}

#[gpui::test]
fn a_drill_from_a_tab_opens_in_the_pulse(cx: &mut TestAppContext) {
    let shell = start(cx, "apps-tab");
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    // An app view as a tab of the pulse: what it opens goes into the pulse.
    cx.update(|cx| deck.update(cx, |d, cx| d.apply(cx, |l| l.open(app("entityservice"), l.main().focused).map(|_| ()))));
    let run = ViewAddr::Run { app: "entityservice".into(), cmd: "go:run".into() }.to_ref();
    assert!(!cx.update(|cx| deck.update(cx, |d, cx| d.drill(&app("entityservice"), run.clone(), cx))), "not in the details");
    assert_eq!(details(&shell, cx), None);
}

#[gpui::test]
fn opening_in_the_pulse_from_a_full_view_shows_pulses_first(cx: &mut TestAppContext) {
    let shell = start(cx, "apps-reveal");
    let main = main_window(&shell, cx);
    let s = sidebar(&shell, cx);
    select(&shell, cx, "entityservice");
    let notes = ViewAddr::Other("demo/notes".into()).to_ref();
    let revealed = cx.update(|cx| shell.update(cx, |sh, cx| sh.reveal("work", &notes, cx)));
    assert!(revealed.is_ok());
    assert_eq!(cx.read(|cx| s.read(cx).activity()), Activity::Layouts, "a reveal shows Pulses");
    assert_eq!(details(&shell, cx), None);
    // The palette: an app goes to the Apps view, selected.
    cx.simulate_keystrokes(main, "ctrl-k");
    cx.simulate_input(main, "proxyapi");
    cx.simulate_keystrokes(main, "enter");
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| s.read(cx).activity()), Activity::Apps);
    assert_eq!(cx.read(|cx| s.read(cx).selected.clone()).as_deref(), Some("proxyapi"));
    assert_eq!(details(&shell, cx), Some(vec![app("proxyapi")]));
}

#[gpui::test]
fn the_app_view_names_the_tools_its_commands_need(cx: &mut TestAppContext) {
    let shell = start(cx, "apps-tools");
    select(&shell, cx, "remotedesktopapp");
    let mut vcx = VisualTestContext::from_window(main_window(&shell, cx), cx);
    assert!(vcx.debug_bounds("tool-docker").is_some(), "docker, missing: what it blocks");
    select(&shell, cx, "entityservice");
    let mut vcx = VisualTestContext::from_window(main_window(&shell, cx), cx);
    assert!(vcx.debug_bounds("tool-go").is_some());
    assert!(vcx.debug_bounds("tool-docker").is_none(), "only what its commands need");
}

#[gpui::test]
fn a_run_says_how_long_it_has_run(cx: &mut TestAppContext) {
    let shell = start(cx, "apps-ran");
    let runner = cx.read(|cx| shell.read(cx).workspaces[shell.read(cx).active].services.read(cx).runner.clone());
    let key = devdeck_app::runner::RunKey::new("vesselservice", "go:run");
    cx.update(|cx| runner.update(cx, |r, cx| r.start(&key, None, cx))).expect("starts");
    // Its output comes every few hundred ms: a minute is long enough.
    cx.executor().advance_clock(std::time::Duration::from_secs(61));
    cx.run_until_parked();
    let now = cx.executor().now();
    let ran = cx.read(|cx| runner.read(cx).running_for(&key, now)).expect("running");
    assert!(ran >= std::time::Duration::from_secs(61), "{ran:?}");
    assert_eq!(cx.read(|cx| runner.read(cx).running_for(&devdeck_app::runner::RunKey::new("vesselservice", "go:test"), now)), None, "not running");
}
