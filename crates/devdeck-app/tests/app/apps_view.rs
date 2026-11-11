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
    assert!(vcx.debug_bounds("tools-found").is_some(), "what's there, on one line");
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

// ------------------------------------------------------------------ 3b: what it links to

#[gpui::test]
fn a_deployment_row_goes_to_it_in_cluster(cx: &mut TestAppContext) {
    let shell = start(cx, "apps-deploy");
    select(&shell, cx, "graphservice");
    cx.executor().advance_clock(devdeck_app::cluster::LOAD_DELAY);
    cx.run_until_parked();
    let mut vcx = VisualTestContext::from_window(main_window(&shell, cx), cx);
    let row = vcx.debug_bounds("deployment-dev33").expect("graphservice runs on dev33");
    assert!(vcx.debug_bounds("pods-dev33").is_some(), "its pods");
    vcx.simulate_click(row.center(), Modifiers::none());
    vcx.run_until_parked();
    let s = sidebar(&shell, cx);
    assert_eq!(cx.read(|cx| s.read(cx).activity()), Activity::Cluster, "a pivot to Cluster");
    assert_eq!(cx.read(|cx| s.read(cx).list_env(cx)).as_deref(), Some("dev33"), "on its env");
    assert_eq!(cx.read(|cx| s.read(cx).selected.clone()).as_deref(), Some("graphservice"), "it selected");
    let svc = ViewAddr::Svc { env: "dev33".into(), name: "graphservice".into() }.to_ref();
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    assert!(cx.read(|cx| deck.read(cx).layout().find_view(&svc).is_some()), "its service view in the pulse, as a click on its row does");
    assert_eq!(details(&shell, cx), None, "Cluster shows the pulse's panes, for now");
}

#[gpui::test]
fn its_terminals_and_ports_go_to_their_activities(cx: &mut TestAppContext) {
    let shell = start(cx, "apps-pivots");
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    let dir = cx.read(|cx| shell.read(cx).workspaces[shell.read(cx).active].services.read(cx).catalog.app("entityservice").unwrap().dir.clone());
    let pane = cx.read(|cx| deck.read(cx).layout().main().focused);
    cx.update(|cx| deck.update(cx, |d, cx| d.open_terminal(&dir, devdeck_app::places::Place::Pane(pane), cx))).expect("opened");
    let n = cx.read(|cx| deck.read(cx).terminals(cx).into_keys().max()).unwrap();
    select(&shell, cx, "entityservice");
    let s = sidebar(&shell, cx);
    let mut vcx = VisualTestContext::from_window(main_window(&shell, cx), cx);
    // `debug_bounds` takes a name for good: this test's one is leaked.
    let name: &'static str = Box::leak(format!("terminal-{n}").into_boxed_str());
    let row = vcx.debug_bounds(name).expect("the shell in its folder");
    vcx.simulate_click(row.center(), Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(cx.read(|cx| s.read(cx).activity()), Activity::Terminals);
    assert_eq!(cx.read(|cx| s.read(cx).selected.clone()), Some(n.to_string()));
    select(&shell, cx, "entityservice");
    let mut vcx = VisualTestContext::from_window(main_window(&shell, cx), cx);
    let port = vcx.debug_bounds("port-5720").expect("its command's port");
    vcx.simulate_click(port.center(), Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(cx.read(|cx| s.read(cx).activity()), Activity::Ports);
}

#[gpui::test]
fn open_in_pulse_joins_a_pulses_first_pane(cx: &mut TestAppContext) {
    let shell = start(cx, "apps-pulse");
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    let triage = cx.update(|cx| deck.update(cx, |d, cx| d.save_layout_as("triage", cx))).unwrap();
    let default = cx.read(|cx| deck.read(cx).book.id_named("default")).expect("the default pulse");
    cx.update(|cx| deck.update(cx, |d, cx| d.switch_layout(&default, cx))).unwrap();
    select(&shell, cx, "entityservice");
    let mut vcx = VisualTestContext::from_window(main_window(&shell, cx), cx);
    assert!(vcx.debug_bounds("details-pulse").is_some(), "the details say where it can go");
    let view = app("entityservice");
    // A saved pulse: into its book, the live one untouched.
    let name = cx.update(|cx| deck.update(cx, |d, cx| d.open_in_pulse(&triage, view.clone(), cx))).unwrap();
    assert_eq!(name, "triage");
    let saved = cx.read(|cx| deck.read(cx).book.layouts[&triage].layout.clone()).unwrap();
    let first = saved.panes_in(devdeck_core::layout::MAIN_WINDOW).unwrap()[0];
    assert_eq!(saved.find_view(&view).map(|(p, _)| p), Some(first), "its main window's first pane");
    assert!(cx.read(|cx| deck.read(cx).layout().find_view(&view)).is_none());
    // The live one.
    cx.update(|cx| deck.update(cx, |d, cx| d.open_in_pulse(&default, view.clone(), cx))).unwrap();
    let live = cx.read(|cx| deck.read(cx).layout().clone());
    assert_eq!(live.find_view(&view).map(|(p, _)| p), Some(live.panes_in(devdeck_core::layout::MAIN_WINDOW).unwrap()[0]));
    assert_eq!(details(&shell, cx), Some(vec![view]), "the Apps view stays");
}

#[gpui::test]
fn the_title_bar_has_no_pulse_menu(cx: &mut TestAppContext) {
    let shell = start(cx, "apps-title");
    let mut vcx = VisualTestContext::from_window(main_window(&shell, cx), cx);
    assert!(vcx.debug_bounds("search-button").is_some());
    assert!(vcx.debug_bounds("layout-button").is_none(), "Pulses is ctrl+1, its list says which is current");
}

// ------------------------------------------------------------------ after the first look

#[gpui::test]
fn coming_back_to_apps_shows_the_app_left(cx: &mut TestAppContext) {
    let shell = start(cx, "apps-back");
    select(&shell, cx, "graphservice");
    let s = sidebar(&shell, cx);
    for away in [Activity::Cluster, Activity::Layouts] {
        cx.update(|cx| s.update(cx, |s, cx| s.show(away, cx)));
        cx.run_until_parked();
        assert_eq!(details(&shell, cx), None);
        cx.update(|cx| s.update(cx, |s, cx| s.show(Activity::Apps, cx)));
        cx.run_until_parked();
        assert_eq!(details(&shell, cx), Some(vec![app("graphservice")]), "back from {away:?}: where it was");
    }
}

#[gpui::test]
fn deployments_say_reading_until_every_env_is_read(cx: &mut TestAppContext) {
    let shell = start(cx, "apps-reading");
    select(&shell, cx, "front-end/mastertadaapp");
    let mut vcx = VisualTestContext::from_window(main_window(&shell, cx), cx);
    assert!(vcx.debug_bounds("deployments-reading").is_some(), "not read yet: it says so");
    assert!(vcx.debug_bounds("deployments-none").is_none(), "never \"not deployed\" before");
    cx.executor().advance_clock(devdeck_app::cluster::LOAD_DELAY);
    cx.run_until_parked();
    let mut vcx = VisualTestContext::from_window(main_window(&shell, cx), cx);
    assert!(vcx.debug_bounds("deployments-reading").is_none());
    assert!(vcx.debug_bounds("deployments-none").is_some(), "every env read, no deployment of it or of front-end");
}

#[gpui::test]
fn the_details_header_holds_the_views_icons(cx: &mut TestAppContext) {
    let shell = start(cx, "apps-header");
    select(&shell, cx, "entityservice");
    let mut vcx = VisualTestContext::from_window(main_window(&shell, cx), cx);
    let terminal = vcx.debug_bounds("details-action-app_view.terminal").expect("the app view's own icon, in the one header");
    assert!(vcx.debug_bounds("app-facts").is_some(), "the view keeps its facts, not a second header");
    // A dashboard: its numbers on top, its parts in panels.
    let stats = vcx.debug_bounds("app-stats").expect("the numbers");
    for name in ["stat-running", "stat-deployed", "stat-terminals", "stat-tools"] {
        assert!(vcx.debug_bounds(name).is_some(), "{name}");
    }
    for name in ["app-commands", "app-notes", "app-deployments", "app-terminals", "app-tools"] {
        let panel = vcx.debug_bounds(name).unwrap_or_else(|| panic!("{name}"));
        assert!(panel.origin.y > stats.origin.y, "{name} under the numbers");
    }
    vcx.simulate_click(terminal.center(), Modifiers::none());
    vcx.run_until_parked();
    let top = details(&shell, cx).and_then(|s| s.last().cloned()).expect("drilled");
    assert!(matches!(ViewAddr::parse(&top), ViewAddr::Terminal { .. }), "a terminal in its folder, over it");
}
