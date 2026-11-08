//! M1 step 3: workspaces, persistence and the quit guard (R2, R8.1, R10.1), on GPUI's
//! test platform. The demo has `personal` (notes, gallery) and `work` (with the log
//! stream, which counts as running).

use devdeck_app::deck::{Deck, SAVE_DELAY};
use devdeck_app::fake_driver::STOP_DELAY;
use devdeck_app::shell::{QuitState, Shell, WorkspaceSpec};
use devdeck_app::views::app_view::AppView;
use devdeck_app::views::run_view::RunView;
use devdeck_core::view_addr::ViewAddr;
use devdeck_core::layout::{Layout, ViewRef, WindowId};
use devdeck_core::state::{AppState, Loaded, StateStore};
use devdeck_ui::overlay::{layer_count, layer_state};
use devdeck_ui::{logical, open_menu, toasts};
use gpui::{AnyWindowHandle, AppContext, Entity, TestAppContext, VisualTestContext, point};
use std::path::PathBuf;

/// A folder kept as long as the test's app.
fn temp(cx: &mut TestAppContext, tag: &str) -> PathBuf {
    cx.update(|cx| devdeck_app::test_dir(tag, cx))
}

fn start(cx: &mut TestAppContext, specs: Vec<WorkspaceSpec>, store: Option<StateStore>, tag: &str) -> Entity<Shell> {
    let config = temp(cx, &format!("{tag}-config"));
    cx.update(|cx| devdeck_app::init_with_config_dir(&config, cx));
    let shell = cx.update(|cx| Shell::start(specs, "work", store, cx));
    cx.update(|cx| shell.update(cx, |s, _| s.quit_for_real = false));
    cx.run_until_parked();
    shell
}

fn demo(cx: &mut TestAppContext, tag: &str) -> Entity<Shell> {
    start(cx, devdeck_app::demo_workspaces(), None, tag)
}

fn index(shell: &Entity<Shell>, cx: &TestAppContext, id: &str) -> usize {
    cx.read(|cx| shell.read(cx).workspaces.iter().position(|w| w.id == id).expect("workspace"))
}

fn deck(shell: &Entity<Shell>, cx: &TestAppContext, id: &str) -> Entity<Deck> {
    let i = index(shell, cx, id);
    cx.read(|cx| shell.read(cx).workspaces[i].deck.clone())
}

fn switch(shell: &Entity<Shell>, cx: &mut TestAppContext, id: &str) {
    let i = index(shell, cx, id);
    cx.update(|cx| shell.update(cx, |s, cx| s.switch(i, cx)));
    cx.run_until_parked();
}

fn detach_notes(deck: &Entity<Deck>, cx: &mut TestAppContext) -> WindowId {
    let w = cx.update(|cx| {
        deck.update(cx, |d, cx| {
            d.apply(cx, |l| {
                let (p, i) = l.find_view(&ViewRef::new("demo/notes")).expect("notes open");
                l.detach(p, i)
            })
        })
    });
    cx.run_until_parked();
    w.expect("detached")
}

fn main_window(shell: &Entity<Shell>, cx: &TestAppContext) -> AnyWindowHandle {
    cx.read(|cx| shell.read(cx).main_window().expect("main window").into())
}

fn modals(shell: &Entity<Shell>, cx: &mut TestAppContext) -> usize {
    let main = main_window(shell, cx);
    cx.update(|cx| layer_state(main, cx).1)
}

fn quit_state(shell: &Entity<Shell>, cx: &TestAppContext) -> (QuitState, usize) {
    cx.read(|cx| (shell.read(cx).quit_state, shell.read(cx).quits))
}

#[gpui::test]
fn workspaces_are_alphabetical_and_start_on_the_default(cx: &mut TestAppContext) {
    let shell = demo(cx, "order");
    let names: Vec<String> = cx.read(|cx| shell.read(cx).workspaces.iter().map(|w| w.name.clone()).collect());
    assert_eq!(names, ["personal", "work"]);
    assert_eq!(cx.read(|cx| shell.read(cx).active_name().to_string()), "work");
    assert_eq!(cx.windows().len(), 1, "one main window for every workspace");
}

#[gpui::test]
fn switching_keeps_each_workspace_alive(cx: &mut TestAppContext) {
    let shell = demo(cx, "alive");
    let work = deck(&shell, cx, "work");
    let personal = deck(&shell, cx, "personal");
    let (work_layout, work_views) = cx.read(|cx| (work.read(cx).layout().clone(), work.read(cx).live_views()));
    switch(&shell, cx, "personal");
    assert!(cx.read(|cx| personal.read(cx).is_active() && !work.read(cx).is_active()));
    assert_eq!(cx.read(|cx| work.read(cx).live_views()), work_views, "hidden workspace keeps its views");
    assert!(cx.read(|cx| shell.read(cx).running(cx)).iter().any(|(ws, _)| ws == "work"), "and keeps running");
    switch(&shell, cx, "work");
    assert_eq!(cx.read(|cx| work.read(cx).layout().clone()), work_layout);
    assert_eq!(cx.windows().len(), 1);
}

#[gpui::test]
fn detached_windows_follow_their_workspace(cx: &mut TestAppContext) {
    let shell = demo(cx, "detached");
    let work = deck(&shell, cx, "work");
    detach_notes(&work, cx);
    assert_eq!(cx.windows().len(), 2);
    switch(&shell, cx, "personal");
    assert_eq!(cx.windows().len(), 1, "work's detached window closes while hidden");
    assert_eq!(cx.read(|cx| work.read(cx).layout().windows.len()), 2, "but stays in the model");
    switch(&shell, cx, "work");
    assert_eq!(cx.windows().len(), 2, "and comes back");
    // Quick A → B → A without letting deferred work run in between.
    let (p, w) = (index(&shell, cx, "personal"), index(&shell, cx, "work"));
    cx.update(|cx| {
        shell.update(cx, |s, cx| {
            s.switch(p, cx);
            s.switch(w, cx);
        })
    });
    cx.run_until_parked();
    assert_eq!(cx.windows().len(), 2, "exactly one detached window, not zero or two");
}

#[gpui::test]
fn ctrl_alt_n_switches_in_alphabetical_order(cx: &mut TestAppContext) {
    let shell = demo(cx, "keys");
    let main = main_window(&shell, cx);
    cx.simulate_keystrokes(main, "ctrl-alt-1");
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| shell.read(cx).active_name().to_string()), "personal");
    cx.simulate_keystrokes(main, "ctrl-alt-2");
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| shell.read(cx).active_name().to_string()), "work");
    cx.simulate_keystrokes(main, "ctrl-alt-9");
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| shell.read(cx).active_name().to_string()), "work", "no ninth workspace: nothing happens");
}

#[gpui::test]
fn the_workspace_menu_switches_by_mouse(cx: &mut TestAppContext) {
    let shell = demo(cx, "menu-click");
    let main = main_window(&shell, cx);
    let t = cx.update(|cx| devdeck_ui::theme(cx).clone());
    let mut vcx = VisualTestContext::from_window(main, cx);
    let button = vcx.debug_bounds("workspace-button").expect("title bar button");
    vcx.simulate_click(button.center(), gpui::Modifiers::none());
    // The menu opens at the click, alphabetical: `personal` is the first row.
    vcx.simulate_click(point(button.center().x + t.size(40.), button.center().y + t.row_h * 0.5 + t.size(4.)), gpui::Modifiers::none());
    assert_eq!(vcx.read(|cx| shell.read(cx).active_name().to_string()), "personal");
}

#[gpui::test]
fn quit_with_nothing_running_quits_at_once(cx: &mut TestAppContext) {
    let shell = start(cx, vec![WorkspaceSpec::new("empty", "empty", devdeck_app::personal_layout)], None, "idle");
    cx.update(|cx| shell.update(cx, |s, cx| s.request_quit(cx)));
    cx.run_until_parked();
    assert_eq!(quit_state(&shell, cx), (QuitState::Idle, 1));
    assert_eq!(modals(&shell, cx), 0, "no guard");
}

#[gpui::test]
fn guard_opens_once_and_cancel_keeps_the_app(cx: &mut TestAppContext) {
    let shell = demo(cx, "cancel");
    switch(&shell, cx, "personal");
    cx.update(|cx| shell.update(cx, |s, cx| s.request_quit(cx)));
    cx.update(|cx| shell.update(cx, |s, cx| s.request_quit(cx)));
    cx.run_until_parked();
    assert_eq!(quit_state(&shell, cx), (QuitState::Asking, 0), "work runs in the background, so ask");
    assert_eq!(modals(&shell, cx), 1, "asking twice shows one guard");
    let main = main_window(&shell, cx);
    cx.simulate_keystrokes(main, "escape");
    cx.run_until_parked();
    assert_eq!(quit_state(&shell, cx), (QuitState::Idle, 0));
    assert_eq!(modals(&shell, cx), 0);
    assert!(!cx.read(|cx| shell.read(cx).running(cx)).is_empty(), "nothing was stopped");
}

#[gpui::test]
fn stop_all_and_quit_stops_then_quits(cx: &mut TestAppContext) {
    let shell = demo(cx, "stop");
    cx.update(|cx| shell.update(cx, |s, cx| s.request_quit(cx)));
    cx.run_until_parked();
    let main = main_window(&shell, cx);
    cx.simulate_keystrokes(main, "enter");
    cx.run_until_parked();
    assert_eq!(quit_state(&shell, cx), (QuitState::Stopping, 0), "enter on the default button stops first");
    cx.executor().advance_clock(STOP_DELAY + std::time::Duration::from_millis(100));
    cx.run_until_parked();
    assert_eq!(quit_state(&shell, cx), (QuitState::Idle, 1), "then quits");
    assert!(cx.read(|cx| shell.read(cx).running(cx)).is_empty(), "everything stopped first");
}

#[gpui::test]
fn every_quit_path_reaches_the_guard(cx: &mut TestAppContext) {
    let shell = demo(cx, "paths");
    // ctrl+q from a detached window.
    let work = deck(&shell, cx, "work");
    detach_notes(&work, cx);
    let main = main_window(&shell, cx);
    let detached = cx.windows().into_iter().find(|w| *w != main).expect("detached window");
    cx.simulate_keystrokes(detached, "ctrl-q");
    cx.run_until_parked();
    assert_eq!(quit_state(&shell, cx), (QuitState::Asking, 0));
    assert_eq!(modals(&shell, cx), 1, "the guard shows in the main window");
    cx.simulate_keystrokes(main, "escape");
    cx.run_until_parked();
    // The window manager closing the main window.
    let closed = VisualTestContext::from_window(main, cx).simulate_close();
    assert!(!closed, "the main window never closes on its own");
    cx.run_until_parked();
    assert_eq!(quit_state(&shell, cx), (QuitState::Asking, 0));
}

#[gpui::test]
fn layout_is_saved_once_after_a_burst(cx: &mut TestAppContext) {
    let dir = temp(cx, "save");
    let shell = start(cx, devdeck_app::demo_workspaces(), Some(StateStore::new(dir.clone())), "save");
    let work = deck(&shell, cx, "work");
    for v in ["demo/a", "demo/b", "demo/c"] {
        cx.update(|cx| work.update(cx, |d, cx| d.apply(cx, |l| l.open(ViewRef::new(v), l.main().focused).map(|_| ()))));
    }
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| work.read(cx).saves), 0, "nothing written before the delay");
    cx.executor().advance_clock(SAVE_DELAY);
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| work.read(cx).saves), 1, "one write for the burst");
    let saved = StateStore::new(dir.clone()).load_layout("work");
    assert_eq!(saved, Loaded::Ok(cx.read(|cx| work.read(cx).layout().clone())));
    // A pending change is written on quit.
    cx.update(|cx| work.update(cx, |d, cx| d.apply(cx, |l| l.close_view(&ViewRef::new("demo/a")))));
    cx.update(|cx| shell.update(cx, |s, cx| s.flush(cx)));
    assert_eq!(cx.read(|cx| work.read(cx).saves), 2);
    let app = StateStore::new(dir).load_app();
    assert!(matches!(app, Loaded::Ok(AppState { active_workspace: Some(ref w), .. }) if w == "work"));
}

fn custom_work() -> Layout {
    let mut l = devdeck_app::initial_layout();
    l.close_view(&ViewRef::new("demo/long")).unwrap();
    l
}

#[gpui::test]
fn saved_state_is_restored_and_broken_files_fall_back(cx: &mut TestAppContext) {
    let dir = temp(cx, "restore");
    let store = StateStore::new(dir.clone());
    store.save_layout("work", &custom_work()).unwrap();
    store.save_app(&AppState { active_workspace: Some("personal".into()), main_size: None, sidebar_w: None }).unwrap();
    std::fs::create_dir_all(dir.join("workspaces/personal")).unwrap();
    std::fs::write(dir.join("workspaces/personal/layout.json"), "{oops").unwrap();

    let shell = start(cx, devdeck_app::demo_workspaces(), Some(store), "restore");
    assert_eq!(cx.read(|cx| shell.read(cx).active_name().to_string()), "personal", "active workspace restored");
    let work = deck(&shell, cx, "work");
    assert_eq!(cx.read(|cx| work.read(cx).layout().clone()), custom_work(), "saved layout restored");
    let personal = deck(&shell, cx, "personal");
    assert_eq!(cx.read(|cx| personal.read(cx).layout().clone()), devdeck_app::personal_layout(), "broken file → default");
    assert!(dir.join("workspaces/personal/layout.json.bad").exists(), "broken file kept aside");
    let toasts = cx.update(|cx| toasts(cx).read(cx).queue.items.len());
    assert_eq!(toasts, 1, "the problem is reported");
}

/// The work layout with eventservice's server output and its app view open.
fn with_run_and_app() -> Layout {
    let mut l = devdeck_app::initial_layout();
    let p = l.main().focused;
    for addr in [ViewAddr::App { app: "eventservice".into() }, ViewAddr::Run { app: "eventservice".into(), cmd: "go:run".into() }] {
        l.open(addr.to_ref(), p).unwrap();
    }
    l
}

fn run_ref() -> ViewRef {
    ViewAddr::Run { app: "eventservice".into(), cmd: "go:run".into() }.to_ref()
}

#[gpui::test]
fn quitting_saves_what_the_views_keep_and_what_ran(cx: &mut TestAppContext) {
    let dir = temp(cx, "views-save");
    let shell = start(cx, devdeck_app::demo_workspaces(), Some(StateStore::new(dir.clone())), "views-save");
    let work = deck(&shell, cx, "work");
    cx.update(|cx| work.update(cx, |d, cx| d.apply(cx, |l| { *l = with_run_and_app(); Ok(()) })));
    cx.run_until_parked();
    let run = cx.read(|cx| work.read(cx).view(&run_ref()).unwrap().downcast::<RunView>().unwrap());
    let app = cx.read(|cx| work.read(cx).view(&ViewAddr::App { app: "eventservice".into() }.to_ref()).unwrap().downcast::<AppView>().unwrap());
    let runner = cx.read(|cx| shell.read(cx).workspaces.iter().find(|w| w.id == "work").unwrap().services.read(cx).runner.clone());
    cx.update(|cx| runner.update(cx, |r, cx| r.start(&devdeck_app::runner::RunKey::new("eventservice", "go:run"), None, cx))).unwrap();
    cx.update(|cx| run.update(cx, |v, cx| v.toggle_follow(cx)));
    cx.update(|cx| app.update(cx, |v, _| v.show_hidden = true));
    // Quitting stops the runs first, then writes everything.
    cx.update(|cx| runner.update(cx, |r, cx| r.stop_all(cx)));
    cx.update(|cx| shell.update(cx, |s, cx| s.flush(cx)));
    let Loaded::Ok(views) = StateStore::new(dir).load_views("work") else { panic!("views saved") };
    assert_eq!(views[run_ref().as_str()], serde_json::json!({"follow": false, "running": true, "args": ""}), "running when devdeck quit");
    assert_eq!(views[ViewAddr::App { app: "eventservice".into() }.to_ref().as_str()]["show_hidden"], true);
}

fn work_runner(shell: &Entity<Shell>, cx: &TestAppContext) -> Entity<devdeck_app::runner::Runner> {
    cx.read(|cx| shell.read(cx).workspaces.iter().find(|w| w.id == "work").unwrap().services.read(cx).runner.clone())
}

fn saved_run(dir: &std::path::Path) -> Option<serde_json::Value> {
    let Loaded::Ok(views) = StateStore::new(dir.to_path_buf()).load_views("work") else { return None };
    views.get(run_ref().as_str()).cloned()
}

#[gpui::test]
fn a_run_starting_is_saved_without_waiting_for_quit(cx: &mut TestAppContext) {
    let dir = temp(cx, "views-soon");
    let shell = start(cx, devdeck_app::demo_workspaces(), Some(StateStore::new(dir.clone())), "views-soon");
    let work = deck(&shell, cx, "work");
    cx.update(|cx| work.update(cx, |d, cx| d.apply(cx, |l| { *l = with_run_and_app(); Ok(()) })));
    cx.executor().advance_clock(SAVE_DELAY);
    cx.run_until_parked();
    assert_eq!(saved_run(&dir), Some(serde_json::json!({"follow": true, "running": false, "args": ""})));
    let runner = work_runner(&shell, cx);
    cx.update(|cx| runner.update(cx, |r, cx| r.start(&devdeck_app::runner::RunKey::new("eventservice", "go:run"), None, cx))).unwrap();
    cx.run_until_parked();
    cx.executor().advance_clock(SAVE_DELAY);
    cx.run_until_parked();
    assert_eq!(saved_run(&dir), Some(serde_json::json!({"follow": true, "running": true, "args": ""})), "saved soon, a crash wouldn't lose it");
}

#[gpui::test]
fn a_signal_quit_keeps_what_ran_even_once_killed(cx: &mut TestAppContext) {
    let dir = temp(cx, "views-signal");
    let shell = start(cx, devdeck_app::demo_workspaces(), Some(StateStore::new(dir.clone())), "views-signal");
    let work = deck(&shell, cx, "work");
    cx.update(|cx| work.update(cx, |d, cx| d.apply(cx, |l| { *l = with_run_and_app(); Ok(()) })));
    let runner = work_runner(&shell, cx);
    let key = devdeck_app::runner::RunKey::new("eventservice", "go:run");
    cx.update(|cx| runner.update(cx, |r, cx| r.start(&key, None, cx))).unwrap();
    cx.update(|cx| shell.update(cx, |s, cx| s.kill_to_quit(cx)));
    // The killed run's exit arrives after: it still counts as running when devdeck quit.
    let id = devdeck_app::runner::RunId { key: key.clone(), n: cx.read(|cx| runner.read(cx).run(&key).unwrap().runs) };
    cx.update(|cx| runner.update(cx, |r, cx| r.exited(&id, devdeck_core::runs::Exit::Signal(9), cx)));
    cx.update(|cx| shell.update(cx, |s, cx| s.flush(cx)));
    assert_eq!(saved_run(&dir).unwrap()["running"], true);
}

#[gpui::test]
fn a_tab_whose_app_was_missing_comes_back_with_it(cx: &mut TestAppContext) {
    let shell = demo(cx, "missing");
    let work = deck(&shell, cx, "work");
    let services = cx.read(|cx| shell.read(cx).workspaces.iter().find(|w| w.id == "work").unwrap().services.clone());
    let full = cx.read(|cx| (*services.read(cx).catalog).clone());
    let mut without = full.clone();
    without.apps.retain(|a| a.id != "eventservice");
    cx.update(|cx| services.update(cx, |s, cx| s.set_catalog(without, cx)));
    cx.update(|cx| work.update(cx, |d, cx| d.apply(cx, |l| { *l = with_run_and_app(); Ok(()) })));
    cx.run_until_parked();
    assert!(cx.read(|cx| work.read(cx).view(&run_ref()).unwrap().downcast::<RunView>().is_err()), "a message meanwhile");
    cx.update(|cx| services.update(cx, |s, cx| s.set_catalog(full, cx)));
    cx.run_until_parked();
    assert!(cx.read(|cx| work.read(cx).view(&run_ref()).unwrap().downcast::<RunView>().is_ok()), "built again once the app is back");
}

#[gpui::test]
fn reopening_brings_views_back_and_restarts_what_ran(cx: &mut TestAppContext) {
    let dir = temp(cx, "views-restore");
    let store = StateStore::new(dir.clone());
    store.save_layout("work", &with_run_and_app()).unwrap();
    let views: devdeck_core::state::ViewStates = [
        (run_ref().as_str().to_string(), serde_json::json!({"follow": false, "running": true, "args": "serve -p 9"})),
        (ViewAddr::App { app: "eventservice".into() }.to_ref().as_str().to_string(), serde_json::json!({"selected": "go:build", "show_hidden": true})),
    ]
    .into();
    store.save_views("work", &views).unwrap();
    store.save_app(&AppState { active_workspace: Some("work".into()), main_size: None, sidebar_w: None }).unwrap();
    let shell = start(cx, devdeck_app::demo_workspaces(), Some(store), "views-restore");
    let work = deck(&shell, cx, "work");
    let run = cx.read(|cx| work.read(cx).view(&run_ref()).unwrap().downcast::<RunView>().unwrap());
    assert!(!cx.read(|cx| run.read(cx).follow), "follow off, as saved");
    let app = cx.read(|cx| work.read(cx).view(&ViewAddr::App { app: "eventservice".into() }.to_ref()).unwrap().downcast::<AppView>().unwrap());
    assert_eq!(cx.read(|cx| (app.read(cx).selected.clone(), app.read(cx).show_hidden)), (Some("go:build".into()), true));
    let state = cx.read(|cx| {
        let w = shell.read(cx).workspaces.iter().find(|w| w.id == "work").unwrap();
        w.services.read(cx).runner.read(cx).state(&devdeck_app::runner::RunKey::new("eventservice", "go:run"))
    });
    assert_eq!(state, Some(devdeck_core::runs::RunState::Running), "it ran when devdeck quit: it starts again");
    let runner = work_runner(&shell, cx);
    let args = cx.read(|cx| runner.read(cx).args(&devdeck_app::runner::RunKey::new("eventservice", "go:run")).to_string());
    assert_eq!(args, "serve -p 9", "with its arguments");
}

#[gpui::test]
fn overlay_layers_are_freed_with_their_window(cx: &mut TestAppContext) {
    let shell = demo(cx, "layers");
    let work = deck(&shell, cx, "work");
    let id = detach_notes(&work, cx);
    let main = main_window(&shell, cx);
    let detached = cx.windows().into_iter().find(|w| *w != main).expect("detached window");
    let before = cx.read(layer_count);
    let _ = cx.update_window(detached, |_, window, cx| open_menu(point(logical(10.), logical(10.)), vec![], window, cx));
    assert_eq!(cx.read(layer_count), before + 1);
    cx.update(|cx| work.update(cx, |d, cx| d.reattach(id, cx)));
    cx.run_until_parked();
    assert_eq!(cx.windows().len(), 1);
    assert_eq!(cx.read(layer_count), before, "the closed window's layer is gone");
}

// ------------------------------------------------------------------ named layouts (step 7)

fn notes_only() -> Layout {
    let mut l = Layout::new();
    let p = l.main().focused;
    l.open(ViewRef::new("demo/notes"), p).unwrap();
    l
}

#[gpui::test]
fn layouts_save_switch_keep_and_persist(cx: &mut TestAppContext) {
    let dir = temp(cx, "layouts");
    let shell = start(cx, devdeck_app::demo_workspaces(), Some(StateStore::new(dir.clone())), "layouts");
    let work = deck(&shell, cx, "work");
    let main = main_window(&shell, cx);
    // Save the demo arrangement as "triage", then make a sparse one as "focus".
    let triage = cx.update(|cx| work.update(cx, |d, cx| d.save_layout_as("triage", cx))).unwrap();
    let logs = ViewRef::new("demo/logs");
    let logs_view = cx.read(|cx| work.read(cx).view(&logs).unwrap());
    cx.update(|cx| work.update(cx, |d, cx| d.toggle_keep(&logs, cx)));
    let focus = cx.update(|cx| work.update(cx, |d, cx| d.save_layout_as("focus", cx))).unwrap();
    cx.update(|cx| {
        work.update(cx, |d, cx| {
            d.apply(cx, |l| {
                *l = notes_only();
                Ok(())
            });
        })
    });
    cx.run_until_parked();
    assert!(cx.read(|cx| work.read(cx).book.keep.is_empty()), "closing a kept tab clears its mark");
    cx.update(|cx| work.update(cx, |d, cx| d.toggle_keep(&ViewRef::new("demo/notes"), cx)));
    // Switch to triage: its tabs come back, the kept notes tab follows.
    cx.update(|cx| work.update(cx, |d, cx| d.switch_layout(&triage, cx))).unwrap();
    cx.run_until_parked();
    let l = cx.read(|cx| work.read(cx).layout().clone());
    assert!(l.find_view(&logs).is_some() && l.find_view(&ViewRef::new("demo/notes")).is_some());
    assert!(!cx.read(|cx| work.read(cx).view(&logs).unwrap().entity_id() == logs_view.entity_id()), "logs closed in between: a new instance");
    // Written at once: both files agree.
    let store = StateStore::new(dir.clone());
    assert_eq!(store.load_layout("work"), Loaded::Ok(l.clone()), "layout.json holds the new tree right away");
    let Loaded::Ok(book) = store.load_book("work") else { panic!("layouts.json") };
    assert_eq!(book.current, triage);
    assert_eq!(book.current_name(), "triage");
    assert_eq!(book.layouts[&focus].layout, Some(notes_only()), "the layout left behind is stored under its id");
    // Switching to the current one does nothing.
    let saves = cx.read(|cx| work.read(cx).saves);
    cx.update(|cx| work.update(cx, |d, cx| d.switch_layout(&triage, cx))).unwrap();
    assert_eq!(cx.read(|cx| work.read(cx).saves), saves);
    // Unknown ones are refused with the reason.
    let err = cx.update(|cx| work.update(cx, |d, cx| d.switch_layout("nope", cx))).unwrap_err();
    assert_eq!(err.to_string(), "no such layout: nope");
    let _ = main;
}

#[gpui::test]
fn a_switch_replaces_detached_windows_even_with_colliding_ids(cx: &mut TestAppContext) {
    let shell = demo(cx, "layout-windows");
    let work = deck(&shell, cx, "work");
    // "a": notes detached. "b": long detached (same window id in both layouts).
    detach_notes(&work, cx);
    let wa = cx.read(|cx| work.read(cx).layout().windows[1].id);
    let a = cx.update(|cx| work.update(cx, |d, cx| d.save_layout_as("a", cx))).unwrap();
    // Save as "b" (a copy, now current), then change b.
    cx.update(|cx| work.update(cx, |d, cx| d.save_layout_as("b", cx))).unwrap();
    // "b": the same window (same id) now holds long; notes went back to the main window.
    cx.update(|cx| {
        work.update(cx, |d, cx| {
            d.apply(cx, |l| {
                let detached = l.windows[1].focused;
                let main = l.main().focused;
                l.send_tab(&ViewRef::new("demo/long"), Some(detached))?;
                l.send_tab(&ViewRef::new("demo/notes"), Some(main))
            });
        })
    });
    cx.run_until_parked();
    let wb = cx.read(|cx| work.read(cx).layout().windows[1].id);
    assert_eq!(wa, wb, "the two layouts use the same detached window id");
    cx.update(|cx| work.update(cx, |d, cx| d.switch_layout(&a, cx))).unwrap();
    cx.run_until_parked();
    assert_eq!(cx.windows().len(), 2);
    let detached_view = cx.read(|cx| {
        let l = &work.read(cx).layout();
        l.pane(l.windows[1].focused).unwrap().active_view().cloned()
    });
    assert_eq!(detached_view, Some(ViewRef::new("demo/notes")), "a's detached window shows a's tab");
    // A layout without detached windows closes them.
    cx.update(|cx| work.update(cx, |d, cx| d.save_layout_as("plain", cx))).unwrap();
    cx.update(|cx| {
        work.update(cx, |d, cx| {
            d.apply(cx, |l| {
                *l = notes_only();
                Ok(())
            });
        })
    });
    cx.run_until_parked();
    assert_eq!(cx.windows().len(), 1);
}

#[gpui::test]
fn a_renamed_layout_stays_pinned(cx: &mut TestAppContext) {
    use devdeck_app::sidebar::Activity;
    let shell = demo(cx, "layout-rename");
    let work = deck(&shell, cx, "work");
    let triage = cx.update(|cx| work.update(cx, |d, cx| d.save_layout_as("triage", cx))).unwrap();
    let sidebar = cx.read(|cx| shell.read(cx).active_sidebar());
    cx.update(|cx| sidebar.update(cx, |s, cx| s.show(Activity::Layouts, cx)));
    cx.update(|cx| sidebar.update(cx, |s, cx| s.toggle_pin(&triage, cx)));
    cx.update(|cx| work.update(cx, |d, cx| d.rename_layout(&triage, "incident", cx))).unwrap();
    cx.run_until_parked();
    assert!(cx.read(|cx| sidebar.read(cx).state.pins["layouts"].contains(&triage)), "the pin is on the id, which a rename keeps");
    let row = cx.read(|cx| sidebar.read(cx).items(cx).into_iter().find(|i| i.key == triage).map(|i| i.label));
    assert_eq!(row.as_deref(), Some("incident"));
}

#[gpui::test]
fn layout_pins_saved_by_name_move_to_the_ids(cx: &mut TestAppContext) {
    use devdeck_core::state::SidebarState;
    use std::collections::{BTreeMap, BTreeSet};
    let dir = temp(cx, "layout-old-pins");
    let store = StateStore::new(dir.clone());
    // Saved before layouts had ids: the book by name, a layout pinned by its name.
    let old = serde_json::json!({ "current": "default", "layouts": { "default": notes_only(), "Triage": notes_only() } });
    std::fs::create_dir_all(dir.join("workspaces/work")).unwrap();
    std::fs::write(dir.join("workspaces/work/layouts.json"), old.to_string()).unwrap();
    let pins = BTreeMap::from([("layouts".to_string(), BTreeSet::from(["triage".to_string()]))]);
    store.save_sidebar("work", &SidebarState { pins, ..Default::default() }).unwrap();
    let shell = start(cx, devdeck_app::demo_workspaces(), Some(store), "layout-old-pins");
    let work = deck(&shell, cx, "work");
    let triage = cx.read(|cx| work.read(cx).book.id_named("triage")).expect("converted");
    assert_ne!(triage, "Triage", "an id, not the name");
    let sidebar = cx.read(|cx| shell.read(cx).active_sidebar());
    assert_eq!(cx.read(|cx| sidebar.read(cx).state.pins["layouts"].clone()), BTreeSet::from([triage]));
}
