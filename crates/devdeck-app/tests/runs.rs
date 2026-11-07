//! M1 step 5a: the fake runner, the App and Run views, crash toasts (R6.3–R6.7, R5.5),
//! on GPUI's test platform with the fake clock. `work` autostarts entityservice and
//! mastertadaapp; `personal` autostarts blog.

use devdeck_app::runner::{CRASH_AFTER, LINE_GAP, RunKey, STOP_DELAY};
use devdeck_app::shell::Shell;
use devdeck_app::views::run_view::RunView;
use devdeck_core::layout::{Layout, ViewRef};
use devdeck_core::runs::RunState;
use devdeck_core::view_addr::ViewAddr;
use devdeck_ui::toasts;
use gpui::{AnyWindowHandle, AppContext, Entity, TestAppContext};
use std::time::Duration;

fn start_with(cx: &mut TestAppContext, tag: &str, layout: fn() -> Layout) -> Entity<Shell> {
    let config = std::env::temp_dir().join(format!("devdeck-runtest-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&config);
    cx.update(|cx| devdeck_app::init_with_config_dir(&config, cx));
    let mut specs = devdeck_app::demo_workspaces();
    specs.iter_mut().filter(|s| s.id == "work").for_each(|s| s.layout = layout);
    let shell = cx.update(|cx| Shell::start(specs, "work", None, cx));
    cx.update(|cx| shell.update(cx, |s, _| s.quit_for_real = false));
    cx.run_until_parked();
    shell
}

fn start(cx: &mut TestAppContext, tag: &str) -> Entity<Shell> {
    start_with(cx, tag, devdeck_app::initial_layout)
}

fn main_window(shell: &Entity<Shell>, cx: &TestAppContext) -> AnyWindowHandle {
    cx.read(|cx| shell.read(cx).main_window().expect("main").into())
}

fn ws(shell: &Entity<Shell>, cx: &TestAppContext, id: &str) -> usize {
    cx.read(|cx| shell.read(cx).workspaces.iter().position(|w| w.id == id).unwrap())
}

fn state(shell: &Entity<Shell>, cx: &TestAppContext, workspace: &str, app: &str, cmd: &str) -> Option<RunState> {
    let i = ws(shell, cx, workspace);
    cx.read(|cx| shell.read(cx).workspaces[i].services.read(cx).runner.read(cx).state(&RunKey::new(app, cmd)))
}

fn advance(cx: &mut TestAppContext, d: Duration) {
    cx.executor().advance_clock(d);
    cx.run_until_parked();
}

fn open(shell: &Entity<Shell>, cx: &mut TestAppContext, addr: ViewAddr) {
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    cx.update(|cx| {
        deck.update(cx, |d, cx| {
            d.apply(cx, |l| {
                let p = l.main().focused;
                l.open(addr.to_ref(), p).map(|_| ())
            });
        })
    });
    cx.run_until_parked();
}

fn toast_texts(cx: &mut TestAppContext) -> Vec<String> {
    cx.update(|cx| toasts(cx).read(cx).queue.items.iter().map(|t| t.text.to_string()).collect())
}

#[gpui::test]
fn app_view_keys_run_a_command_to_its_exit_code(cx: &mut TestAppContext) {
    let shell = start(cx, "keys");
    let main = main_window(&shell, cx);
    open(&shell, cx, ViewAddr::App { app: "pagerendererservice".into() });
    // The new tab's view has the keys; ↓ ↓ ↓ = run, build, test (tier order, then A–Z).
    cx.simulate_keystrokes(main, "down down down enter");
    cx.run_until_parked();
    assert_eq!(state(&shell, cx, "work", "pagerendererservice", "test"), Some(RunState::Running));
    advance(cx, LINE_GAP * 5);
    assert_eq!(state(&shell, cx, "work", "pagerendererservice", "test"), Some(RunState::Exited(1)));
    assert!(toast_texts(cx).iter().any(|t| t == "work · pagerendererservice · test exited 1"), "a failure is announced");
    // o opens the output next to it.
    cx.simulate_keystrokes(main, "o");
    cx.run_until_parked();
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    let run = ViewAddr::Run { app: "pagerendererservice".into(), cmd: "test".into() }.to_ref();
    assert!(cx.read(|cx| deck.read(cx).layout().find_view(&run).is_some()));
}

#[gpui::test]
fn stop_takes_time_and_a_restart_waits_for_it(cx: &mut TestAppContext) {
    let shell = start(cx, "stop");
    let i = ws(&shell, cx, "work");
    let runner = cx.read(|cx| shell.read(cx).workspaces[i].services.read(cx).runner.clone());
    let key = RunKey::new("entityservice", "run");
    assert_eq!(state(&shell, cx, "work", "entityservice", "run"), Some(RunState::Running), "autostarted");
    cx.update(|cx| runner.update(cx, |r, cx| r.stop(&key, cx)));
    assert_eq!(state(&shell, cx, "work", "entityservice", "run"), Some(RunState::Stopping));
    assert!(cx.read(|cx| shell.read(cx).running(cx)).iter().any(|(_, r)| r.iter().any(|t| t == "entityservice · run")), "stopping still counts");
    cx.update(|cx| runner.update(cx, |r, cx| r.start(&key, Some("dev35".into()), cx))).unwrap();
    advance(cx, STOP_DELAY);
    assert_eq!(state(&shell, cx, "work", "entityservice", "run"), Some(RunState::Running), "started again once stopped");
    let runs = cx.read(|cx| runner.read(cx).run(&key).unwrap().runs);
    assert_eq!(runs, 2);
}

#[gpui::test]
fn missing_tools_refuse_with_the_reason(cx: &mut TestAppContext) {
    let shell = start(cx, "tools");
    let i = ws(&shell, cx, "work");
    let (runner, sidebar) = cx.read(|cx| {
        let w = &shell.read(cx).workspaces[i];
        (w.services.read(cx).runner.clone(), w.sidebar.clone())
    });
    let err = cx.update(|cx| runner.update(cx, |r, cx| r.start(&RunKey::new("remotedesktopapp", "up"), None, cx)));
    assert_eq!(err, Err("docker not found".into()));
    let item = cx.read(|cx| sidebar.read(cx).items(cx).into_iter().find(|i| i.key == "remotedesktopapp").unwrap());
    assert_eq!(item.warning.as_deref(), Some("docker not found"), "the sidebar row says why");
    let lint = cx.read(|cx| runner.read(cx).refusal(&RunKey::new("entityservice", "lint")));
    assert_eq!(lint.as_deref(), Some("golangci-lint not found"));
    let go = cx.read(|cx| sidebar.read(cx).items(cx).into_iter().find(|i| i.key == "entityservice").unwrap());
    assert!(go.warning.is_none(), "a missing tool for a hidden command doesn't disable the app");
}

#[gpui::test]
fn sidebar_and_quit_guard_follow_the_runner(cx: &mut TestAppContext) {
    let shell = start(cx, "follow");
    let running = cx.read(|cx| shell.read(cx).running(cx));
    assert_eq!(
        running,
        [
            ("personal".to_string(), vec!["blog · dev".to_string()]),
            ("work".to_string(), vec!["entityservice · run".to_string(), "front-end/mastertadaapp · start:dev".to_string()])
        ]
    );
    let i = ws(&shell, cx, "work");
    let (runner, sidebar) = cx.read(|cx| {
        let w = &shell.read(cx).workspaces[i];
        (w.services.read(cx).runner.clone(), w.sidebar.clone())
    });
    let running_row = |cx: &TestAppContext| cx.read(|cx| sidebar.read(cx).items(cx).into_iter().find(|i| i.key == "entityservice").unwrap().running);
    assert!(running_row(cx));
    cx.update(|cx| runner.update(cx, |r, cx| r.stop(&RunKey::new("entityservice", "run"), cx)));
    advance(cx, STOP_DELAY);
    assert!(!running_row(cx), "the indicator follows the run");
}

#[gpui::test]
fn a_crash_in_a_hidden_workspace_names_it_and_leads_there(cx: &mut TestAppContext) {
    let shell = start(cx, "crash");
    let main = main_window(&shell, cx);
    let (work, personal) = (ws(&shell, cx, "work"), ws(&shell, cx, "personal"));
    cx.update(|cx| shell.update(cx, |s, cx| s.switch(personal, cx)));
    cx.run_until_parked();
    let runner = cx.read(|cx| shell.read(cx).workspaces[work].services.read(cx).runner.clone());
    cx.update(|cx| runner.update(cx, |r, cx| r.start(&RunKey::new("boardservice", "run"), None, cx))).unwrap();
    advance(cx, LINE_GAP * 4);
    advance(cx, CRASH_AFTER);
    assert_eq!(state(&shell, cx, "work", "boardservice", "run"), Some(RunState::Exited(2)));
    let action = cx.update(|cx| {
        toasts(cx)
            .read(cx)
            .queue
            .items
            .iter()
            .find(|t| t.text.as_ref() == "work · boardservice · run exited 2")
            .and_then(|t| t.actions.first().map(|(_, a)| a.clone()))
    });
    let action = action.expect("toast with View output");
    cx.update_window(main, |_, window, cx| action(window, cx)).unwrap();
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| shell.read(cx).active), work, "View output shows the workspace");
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    let run = ViewAddr::Run { app: "boardservice".into(), cmd: "run".into() }.to_ref();
    assert!(cx.read(|cx| deck.read(cx).layout().find_view(&run).is_some()), "and opens the run");
}

fn restored() -> Layout {
    let mut l = Layout::new();
    let p = l.main().focused;
    l.open(ViewAddr::Run { app: "eventservice".into(), cmd: "build".into() }.to_ref(), p).unwrap();
    l.open(ViewRef::new("deploy/dev35/old-ref"), p).unwrap();
    l.activate(p, 0).unwrap();
    l
}

#[gpui::test]
fn restored_run_tabs_offer_to_run_and_unknown_refs_say_so(cx: &mut TestAppContext) {
    let shell = start_with(cx, "restored", restored);
    let main = main_window(&shell, cx);
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    let run = ViewAddr::Run { app: "eventservice".into(), cmd: "build".into() }.to_ref();
    let view = cx.read(|cx| deck.read(cx).view(&run).unwrap());
    assert!(view.downcast::<RunView>().is_ok(), "a run view, idle");
    assert_eq!(state(&shell, cx, "work", "eventservice", "build"), None);
    cx.simulate_keystrokes(main, "enter");
    cx.run_until_parked();
    assert_eq!(state(&shell, cx, "work", "eventservice", "build"), Some(RunState::Running), "⏎ runs it");
    advance(cx, LINE_GAP * 4);
    assert_eq!(state(&shell, cx, "work", "eventservice", "build"), Some(RunState::Exited(0)));
    let old = cx.read(|cx| deck.read(cx).view(&ViewRef::new("deploy/dev35/old-ref")));
    assert!(old.is_some(), "an unknown ref still gets a (message) view instead of breaking the layout");
}

#[gpui::test]
fn closed_run_and_app_views_are_released(cx: &mut TestAppContext) {
    let shell = start(cx, "leak");
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    let refs = [ViewAddr::App { app: "entityservice".into() }.to_ref(), ViewAddr::Run { app: "entityservice".into(), cmd: "run".into() }.to_ref()];
    for r in &refs {
        cx.update(|cx| deck.update(cx, |d, cx| d.apply(cx, |l| l.open(r.clone(), l.main().focused).map(|_| ()))));
        cx.run_until_parked();
    }
    for r in &refs {
        cx.update(|cx| deck.update(cx, |d, cx| d.apply(cx, |l| l.close_view(r))));
        cx.run_until_parked();
    }
    for _ in 0..3 {
        cx.update(|cx| deck.update(cx, |d, cx| d.apply(cx, |_| Ok(()))));
        cx.run_until_parked();
    }
    assert_eq!(cx.update(|cx| deck.update(cx, |d, _| d.leaked_views(3))), 0);
    let _ = cx.read(|cx| cx.windows().len());
}

#[gpui::test]
fn follow_keys_reach_the_view_after_any_way_back_to_the_panes(cx: &mut TestAppContext) {
    let shell = start(cx, "follow");
    let main = main_window(&shell, cx);
    open(&shell, cx, ViewAddr::Run { app: "entityservice".into(), cmd: "run".into() });
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    let r = ViewAddr::Run { app: "entityservice".into(), cmd: "run".into() }.to_ref();
    let view = cx.read(|cx| deck.read(cx).view(&r).unwrap().downcast::<RunView>().unwrap());
    assert!(cx.read(|cx| view.read(cx).follow));
    // Into the sidebar and back with Esc: the run view has the keys again.
    cx.simulate_keystrokes(main, "ctrl-1 escape alt-f");
    cx.run_until_parked();
    assert!(!cx.read(|cx| view.read(cx).follow), "alt+f toggled follow");
}
