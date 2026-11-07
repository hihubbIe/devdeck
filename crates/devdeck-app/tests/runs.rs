//! M1 step 5a: the fake runner, the App and Run views, crash toasts (R6.3–R6.7, R5.5),
//! on GPUI's test platform with the fake clock. `work` autostarts entityservice and
//! mastertadaapp; `personal` autostarts blog.

use devdeck_app::fake_driver::{CRASH_AFTER, LINE_GAP, STOP_DELAY};
use devdeck_app::runner::RunKey;
use devdeck_app::shell::Shell;
use devdeck_app::views::run_view::RunView;
use devdeck_core::layout::{Layout, ViewRef};
use devdeck_core::runs::RunState;
use devdeck_core::view_addr::ViewAddr;
use devdeck_ui::toasts;
use gpui::{AnyWindowHandle, AppContext, Entity, TestAppContext};
use std::time::Duration;

fn start_with(cx: &mut TestAppContext, tag: &str, layout: fn() -> Layout) -> Entity<Shell> {
    let config = cx.update(|cx| devdeck_app::test_dir(tag, cx));
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
    assert_eq!(state(&shell, cx, "work", "pagerendererservice", "go:test"), Some(RunState::Running));
    advance(cx, LINE_GAP * 5);
    assert_eq!(state(&shell, cx, "work", "pagerendererservice", "go:test"), Some(RunState::Exited(1)));
    assert!(toast_texts(cx).iter().any(|t| t == "work · pagerendererservice · test exited 1"), "a failure is announced");
    // o opens the output next to it.
    cx.simulate_keystrokes(main, "o");
    cx.run_until_parked();
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    let run = ViewAddr::Run { app: "pagerendererservice".into(), cmd: "go:test".into() }.to_ref();
    assert!(cx.read(|cx| deck.read(cx).layout().find_view(&run).is_some()));
}

#[gpui::test]
fn stop_takes_time_and_a_restart_waits_for_it(cx: &mut TestAppContext) {
    let shell = start(cx, "stop");
    let i = ws(&shell, cx, "work");
    let runner = cx.read(|cx| shell.read(cx).workspaces[i].services.read(cx).runner.clone());
    let key = RunKey::new("entityservice", "go:run");
    assert_eq!(state(&shell, cx, "work", "entityservice", "go:run"), Some(RunState::Running), "autostarted");
    cx.update(|cx| runner.update(cx, |r, cx| r.stop(&key, cx)));
    assert_eq!(state(&shell, cx, "work", "entityservice", "go:run"), Some(RunState::Stopping));
    assert!(cx.read(|cx| shell.read(cx).running(cx)).iter().any(|(_, r)| r.iter().any(|t| t == "entityservice · run")), "stopping still counts");
    cx.update(|cx| runner.update(cx, |r, cx| r.start(&key, Some("dev35".into()), cx))).unwrap();
    advance(cx, STOP_DELAY);
    assert_eq!(state(&shell, cx, "work", "entityservice", "go:run"), Some(RunState::Running), "started again once stopped");
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
    let err = cx.update(|cx| runner.update(cx, |r, cx| r.start(&RunKey::new("remotedesktopapp", "compose:up"), None, cx)));
    assert_eq!(err, Err("docker not found".into()));
    let item = cx.read(|cx| sidebar.read(cx).items(cx).into_iter().find(|i| i.key == "remotedesktopapp").unwrap());
    assert_eq!(item.warning.as_deref(), Some("docker not found"), "the sidebar row says why");
    let lint = cx.read(|cx| runner.read(cx).refusal(&RunKey::new("entityservice", "go:lint")));
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
    cx.update(|cx| runner.update(cx, |r, cx| r.stop(&RunKey::new("entityservice", "go:run"), cx)));
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
    cx.update(|cx| runner.update(cx, |r, cx| r.start(&RunKey::new("boardservice", "go:run"), None, cx))).unwrap();
    advance(cx, LINE_GAP * 4);
    advance(cx, CRASH_AFTER);
    assert_eq!(state(&shell, cx, "work", "boardservice", "go:run"), Some(RunState::Exited(2)));
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
    let run = ViewAddr::Run { app: "boardservice".into(), cmd: "go:run".into() }.to_ref();
    assert!(cx.read(|cx| deck.read(cx).layout().find_view(&run).is_some()), "and opens the run");
}

fn restored() -> Layout {
    let mut l = Layout::new();
    let p = l.main().focused;
    l.open(ViewAddr::Run { app: "eventservice".into(), cmd: "go:build".into() }.to_ref(), p).unwrap();
    l.open(ViewRef::new("deploy/dev35/old-ref"), p).unwrap();
    l.activate(p, 0).unwrap();
    l
}

#[gpui::test]
fn restored_run_tabs_offer_to_run_and_unknown_refs_say_so(cx: &mut TestAppContext) {
    let shell = start_with(cx, "restored", restored);
    let main = main_window(&shell, cx);
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    let run = ViewAddr::Run { app: "eventservice".into(), cmd: "go:build".into() }.to_ref();
    let view = cx.read(|cx| deck.read(cx).view(&run).unwrap());
    assert!(view.downcast::<RunView>().is_ok(), "a run view, idle");
    assert_eq!(state(&shell, cx, "work", "eventservice", "go:build"), None);
    cx.simulate_keystrokes(main, "enter");
    cx.run_until_parked();
    assert_eq!(state(&shell, cx, "work", "eventservice", "go:build"), Some(RunState::Running), "⏎ runs it");
    advance(cx, LINE_GAP * 4);
    assert_eq!(state(&shell, cx, "work", "eventservice", "go:build"), Some(RunState::Exited(0)));
    let old = cx.read(|cx| deck.read(cx).view(&ViewRef::new("deploy/dev35/old-ref")));
    assert!(old.is_some(), "an unknown ref still gets a (message) view instead of breaking the layout");
}

#[gpui::test]
fn closed_run_and_app_views_are_released(cx: &mut TestAppContext) {
    let shell = start(cx, "leak");
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    let refs = [ViewAddr::App { app: "entityservice".into() }.to_ref(), ViewAddr::Run { app: "entityservice".into(), cmd: "go:run".into() }.to_ref()];
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
    open(&shell, cx, ViewAddr::Run { app: "entityservice".into(), cmd: "go:run".into() });
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    let r = ViewAddr::Run { app: "entityservice".into(), cmd: "go:run".into() }.to_ref();
    let view = cx.read(|cx| deck.read(cx).view(&r).unwrap().downcast::<RunView>().unwrap());
    assert!(cx.read(|cx| view.read(cx).follow));
    // Into the sidebar and back with Esc: the run view has the keys again.
    cx.simulate_keystrokes(main, "ctrl-1 escape alt-f");
    cx.run_until_parked();
    assert!(!cx.read(|cx| view.read(cx).follow), "alt+f toggled follow");
}

fn runner_of(shell: &Entity<Shell>, cx: &TestAppContext) -> Entity<devdeck_app::runner::Runner> {
    let i = ws(shell, cx, "work");
    cx.read(|cx| shell.read(cx).workspaces[i].services.read(cx).runner.clone())
}

fn lines(runner: &Entity<devdeck_app::runner::Runner>, key: &RunKey, cx: &TestAppContext) -> Vec<String> {
    cx.read(|cx| runner.read(cx).run(key).map(|r| r.terminal.lines()).unwrap_or_default())
}

#[gpui::test]
fn a_stop_ends_stopped_and_is_not_a_failure(cx: &mut TestAppContext) {
    let shell = start(cx, "stopped");
    let runner = runner_of(&shell, cx);
    let key = RunKey::new("entityservice", "go:run");
    cx.update(|cx| runner.update(cx, |r, cx| r.stop(&key, cx)));
    advance(cx, STOP_DELAY);
    assert_eq!(state(&shell, cx, "work", "entityservice", "go:run"), Some(RunState::Stopped));
    assert!(toast_texts(cx).is_empty(), "the process's exit status after our stop isn't a crash");
    let out = lines(&runner, &key, cx);
    assert_eq!(out[out.len() - 3..], ["^C", "stopped", ""], "the empty row is the cursor's");
}

#[gpui::test]
fn output_is_kept_without_a_view_and_wraps_at_the_terminal_width(cx: &mut TestAppContext) {
    let shell = start(cx, "wrap");
    let runner = runner_of(&shell, cx);
    let key = RunKey::new("entityservice", "go:run");
    advance(cx, LINE_GAP * 5);
    let out = lines(&runner, &key, cx);
    assert!(out.iter().any(|l| l.starts_with("INF listening on :")), "kept while no view shows it: {out:?}");
    cx.update(|cx| runner.update(cx, |r, cx| r.resize(&key, devdeck_core::terminal::TermSize { cols: 12, rows: 10 }, cx)));
    let out = lines(&runner, &key, cx);
    assert!(out.iter().all(|l| l.chars().count() <= 12), "{out:?}");
    assert!(out.windows(2).any(|w| w[0] == "INF listenin" && w[1] == "g on :5720"), "rewrapped at the column: {out:?}");
}

/// A driver the test drives by hand: it records launches and does nothing else.
struct Manual(std::rc::Rc<std::cell::RefCell<Vec<devdeck_app::runner::RunId>>>, Option<String>);

struct Idle;

impl devdeck_app::runner::RunControl for Idle {
    fn write(&self, _: &[u8]) {}
    fn resize(&self, _: devdeck_core::terminal::TermSize) {}
    fn stop(&self) {}
}

impl devdeck_app::runner::RunDriver for Manual {
    fn spawn(&self, launch: devdeck_app::runner::Launch, _: &mut gpui::Context<devdeck_app::runner::Runner>) -> Result<devdeck_app::runner::Started, String> {
        if let Some(why) = &self.1 {
            return Err(why.clone());
        }
        self.0.borrow_mut().push(launch.id);
        Ok(devdeck_app::runner::Started { control: Box::new(Idle), task: gpui::Task::ready(()) })
    }
}

fn manual_runner(cx: &mut TestAppContext, fail: Option<&str>) -> (Entity<devdeck_app::runner::Runner>, std::rc::Rc<std::cell::RefCell<Vec<devdeck_app::runner::RunId>>>) {
    let launches = std::rc::Rc::default();
    let driver = std::rc::Rc::new(Manual(std::rc::Rc::clone(&launches), fail.map(String::from)));
    let catalog = std::rc::Rc::new(devdeck_core::demo::catalog("work"));
    (cx.new(|_| devdeck_app::runner::Runner::new("work".into(), catalog, driver)), launches)
}

#[gpui::test]
fn a_run_that_cannot_start_says_why(cx: &mut TestAppContext) {
    let (runner, _) = manual_runner(cx, Some("folder gone"));
    let failures = std::rc::Rc::new(std::cell::Cell::new(0));
    let seen = failures.clone();
    cx.update(|cx| {
        cx.subscribe(&runner, move |_, e: &devdeck_app::runner::RunnerEvent, _| {
            if let devdeck_app::runner::RunnerEvent::Failed { code: 127, .. } = e {
                seen.set(seen.get() + 1);
            }
        })
        .detach()
    });
    let key = RunKey::new("eventservice", "go:build");
    cx.update(|cx| runner.update(cx, |r, cx| r.start(&key, None, cx))).unwrap();
    let (s, why) = cx.read(|cx| runner.read(cx).run(&key).map(|r| (r.state, r.error.clone())).unwrap());
    assert_eq!((s, why.as_deref()), (RunState::NotStarted, Some("folder gone")));
    assert_eq!(failures.get(), 1);
    assert!(lines(&runner, &key, cx).iter().any(|l| l == "couldn't start: folder gone"));
}

#[gpui::test]
fn late_events_of_an_earlier_start_are_ignored(cx: &mut TestAppContext) {
    use devdeck_core::runs::Exit;
    let (runner, launches) = manual_runner(cx, None);
    let key = RunKey::new("eventservice", "go:run");
    cx.update(|cx| runner.update(cx, |r, cx| r.start(&key, None, cx))).unwrap();
    let first = launches.borrow()[0].clone();
    cx.update(|cx| runner.update(cx, |r, cx| r.restart(&key, None, cx))).unwrap();
    assert_eq!(cx.read(|cx| runner.read(cx).state(&key)), Some(RunState::Stopping), "waits for the exit");
    cx.update(|cx| runner.update(cx, |r, cx| r.exited(&first, Exit::Code(130), cx)));
    assert_eq!(cx.read(|cx| runner.read(cx).state(&key)), Some(RunState::Running), "restarted once the first exited");
    assert_eq!(launches.borrow().len(), 2);
    cx.update(|cx| runner.update(cx, |r, cx| {
        r.output(&first, b"stale\r\n", cx);
        r.exited(&first, Exit::Code(1), cx);
    }));
    assert_eq!(cx.read(|cx| runner.read(cx).state(&key)), Some(RunState::Running));
    assert!(!lines(&runner, &key, cx).iter().any(|l| l == "stale"));
}

#[gpui::test]
fn stop_after_restart_cancels_the_restart(cx: &mut TestAppContext) {
    use devdeck_core::runs::Exit;
    let (runner, launches) = manual_runner(cx, None);
    let key = RunKey::new("eventservice", "go:run");
    cx.update(|cx| runner.update(cx, |r, cx| r.start(&key, None, cx))).unwrap();
    let first = launches.borrow()[0].clone();
    cx.update(|cx| runner.update(cx, |r, cx| r.restart(&key, None, cx))).unwrap();
    cx.update(|cx| runner.update(cx, |r, cx| r.stop(&key, cx)));
    cx.update(|cx| runner.update(cx, |r, cx| r.exited(&first, Exit::Code(130), cx)));
    assert_eq!(cx.read(|cx| runner.read(cx).state(&key)), Some(RunState::Stopped));
    assert_eq!(launches.borrow().len(), 1, "not started again");
}

#[gpui::test]
fn arguments_reach_the_line_and_a_restart_keeps_them(cx: &mut TestAppContext) {
    let shell = start(cx, "args");
    let runner = runner_of(&shell, cx);
    let key = RunKey::new("eventservice", "go:run");
    cx.update(|cx| runner.update(cx, |r, cx| r.set_args(&key, " serve --port 3001 ", cx)));
    assert_eq!(cx.read(|cx| runner.read(cx).args(&key).to_string()), "serve --port 3001", "trimmed");
    cx.update(|cx| runner.update(cx, |r, cx| r.start(&key, None, cx))).unwrap();
    let header = lines(&runner, &key, cx).into_iter().find(|l| l.starts_with("── run 1")).unwrap();
    assert!(header.contains("serve --port 3001"), "{header}");
    cx.update(|cx| runner.update(cx, |r, cx| r.restart(&key, None, cx))).unwrap();
    advance(cx, STOP_DELAY * 2);
    let again = lines(&runner, &key, cx).into_iter().find(|l| l.starts_with("── run 2")).unwrap();
    assert!(again.contains("serve --port 3001"), "{again}");
    cx.update(|cx| runner.update(cx, |r, cx| r.set_args(&key, "", cx)));
    let bare = cx.read(|cx| runner.read(cx).line(&key, None)).unwrap();
    let command = cx.read(|_| devdeck_core::demo::catalog("work").app("eventservice").unwrap().command("go:run").unwrap().line.clone());
    assert_eq!(bare, command, "none: the bare line");
    assert!(cx.read(|cx| runner.read(cx).recent_args(&key)).is_empty(), "the demo keeps nothing");
}

#[gpui::test]
fn arguments_used_are_offered_again(cx: &mut TestAppContext) {
    let dir = cx.update(|cx| devdeck_app::test_dir("recent-args", cx));
    let catalog = std::rc::Rc::new(devdeck_core::demo::catalog("work"));
    let driver = std::rc::Rc::new(devdeck_app::fake_driver::FakeDriver);
    let recent = devdeck_core::recent_args::RecentArgs::load(&dir);
    let runner = cx.update(|cx| cx.new(|_| devdeck_app::runner::Runner::new("work".into(), catalog, driver).with_recent_args(recent)));
    let key = RunKey::new("eventservice", "go:run");
    for args in ["-p 1", "-p 2", "-p 1"] {
        cx.update(|cx| {
            runner.update(cx, |r, cx| {
                r.set_args(&key, args, cx);
                r.stop(&key, cx);
            })
        });
        advance(cx, STOP_DELAY * 2);
        cx.update(|cx| runner.update(cx, |r, cx| r.start(&key, None, cx))).unwrap();
    }
    assert_eq!(cx.read(|cx| runner.read(cx).recent_args(&key)), ["-p 1", "-p 2"], "most recent first, once");
    assert_eq!(devdeck_core::recent_args::RecentArgs::load(&dir).get("work", "eventservice", "go:run"), ["-p 1", "-p 2"], "saved");
}

#[gpui::test]
fn the_arguments_field_offers_recent_ones_and_takes_new_ones(cx: &mut TestAppContext) {
    use gpui::{Focusable, Modifiers, VisualTestContext};
    let shell = start(cx, "args-field");
    let addr = ViewAddr::Run { app: "eventservice".into(), cmd: "go:run".into() };
    open(&shell, cx, addr.clone());
    let main = main_window(&shell, cx);
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    let view = cx.read(|cx| deck.read(cx).view(&addr.to_ref()).unwrap().downcast::<RunView>().unwrap());
    let runner = runner_of(&shell, cx);
    let key = RunKey::new("eventservice", "go:run");
    let mut vcx = VisualTestContext::from_window(main, cx);
    let field = vcx.debug_bounds("run-args").expect("drawn");
    vcx.simulate_click(field.center(), Modifiers::none());
    assert!(vcx.update(|_, cx| devdeck_ui::overlay::layer_state(main, cx).0), "a click opens the menu");
    vcx.simulate_keystrokes("down enter");
    assert!(vcx.debug_bounds("run-args-input").is_some(), "new arguments: the field is an input");
    vcx.simulate_input("serve -p 9");
    vcx.simulate_keystrokes("enter");
    assert_eq!(vcx.read(|cx| runner.read(cx).args(&key).to_string()), "serve -p 9");
    assert_eq!(vcx.read(|cx| runner.read(cx).state(&key)), Some(RunState::Running), "they start it");
    let header = vcx.read(|cx| runner.read(cx).run(&key).unwrap().terminal.lines()).into_iter().find(|l| l.starts_with("── run 1")).unwrap();
    assert!(header.contains("serve -p 9"), "{header}");
    assert!(vcx.debug_bounds("run-args-input").is_none(), "back to the field");
    // The key opens the menu too; the tab keeps its arguments.
    vcx.update(|window, cx| view.read(cx).focus_handle(cx).focus(window, cx));
    vcx.simulate_keystrokes("a");
    assert!(vcx.update(|_, cx| devdeck_ui::overlay::layer_state(main, cx).0), "a opens the menu");
    vcx.simulate_keystrokes("escape");
    let saved = vcx.read(|cx| deck.read(cx).view_states(cx));
    assert_eq!(saved[addr.to_ref().as_str()]["args"], "serve -p 9");
}
