//! M1 step 6: the action registry, cheat sheet, palette, Send to and context
//! menus (R5.1–R5.5, R3.11), on GPUI's test platform.

use devdeck_app::registry;
use devdeck_app::runner::RunKey;
use devdeck_app::shell::Shell;
use devdeck_core::layout::{MAIN_WINDOW, ViewRef};
use devdeck_core::runs::RunState;

use devdeck_ui::overlay::layer_state;
use gpui::{AnyWindowHandle, Entity, Modifiers, MouseButton, TestAppContext, VisualTestContext};
use std::collections::HashSet;

fn start(cx: &mut TestAppContext, tag: &str) -> Entity<Shell> {
    let config = cx.update(|cx| devdeck_app::test_dir(tag, cx));
    cx.update(|cx| devdeck_app::init_with_config_dir(&config, cx));
    let shell = cx.update(|cx| Shell::start(devdeck_app::demo_workspaces(), "work", None, cx));
    cx.update(|cx| shell.update(cx, |s, _| s.quit_for_real = false));
    cx.run_until_parked();
    shell
}

fn main_window(shell: &Entity<Shell>, cx: &TestAppContext) -> AnyWindowHandle {
    cx.read(|cx| shell.read(cx).main_window().expect("main").into())
}

/// (menu open, modal count) of the main window.
fn overlay(shell: &Entity<Shell>, cx: &mut TestAppContext) -> (bool, usize) {
    let main = main_window(shell, cx);
    cx.update(|cx| layer_state(main, cx))
}

#[gpui::test]
fn registry_ids_are_unique_and_no_key_is_bound_twice_in_one_context() {
    let mut ids = HashSet::new();
    let mut keys = HashSet::new();
    for s in registry::all() {
        assert!(ids.insert(s.id), "duplicate id {}", s.id);
        for k in s.keys {
            assert!(keys.insert((*k, s.context)), "{k} bound twice in {:?} ({})", s.context, s.id);
        }
    }
    assert_eq!(registry::key_text("ctrl-shift-d"), "ctrl shift d");
    assert_eq!(registry::key_text("ctrl--"), "ctrl -");
    assert_eq!(registry::by_id("pane.split_right").and_then(|s| s.key_text()).as_deref(), Some("ctrl \\"));
}

#[gpui::test]
fn app_keys_wait_while_a_modal_is_open(cx: &mut TestAppContext) {
    let shell = start(cx, "modal-keys");
    let main = main_window(&shell, cx);
    cx.simulate_keystrokes(main, "ctrl-/");
    cx.run_until_parked();
    assert_eq!(overlay(&shell, cx).1, 1, "cheat sheet open");
    let sidebar = cx.read(|cx| shell.read(cx).active_sidebar());
    cx.simulate_keystrokes(main, "ctrl-b ctrl-k ctrl-alt-1");
    cx.run_until_parked();
    assert!(cx.read(|cx| sidebar.read(cx).state.visible), "ctrl+b does nothing under a modal");
    assert_eq!(overlay(&shell, cx).1, 1, "no palette on top of it");
    assert_eq!(cx.read(|cx| shell.read(cx).active_name().to_string()), "work", "no workspace switch");
    cx.simulate_keystrokes(main, "escape");
    cx.run_until_parked();
    assert_eq!(overlay(&shell, cx).1, 0);
    // `?` in a text input is a character.
    cx.simulate_keystrokes(main, "ctrl-1 /");
    cx.simulate_input(main, "?");
    cx.run_until_parked();
    assert_eq!(overlay(&shell, cx).1, 0);
    assert_eq!(cx.read(|cx| sidebar.read(cx).filter.read(cx).text().to_string()), "?");
}

#[gpui::test]
fn shortcuts_are_listed_in_the_cheat_sheet_only(cx: &mut TestAppContext) {
    let shell = start(cx, "keys-page");
    let main = main_window(&shell, cx);
    // Menus name actions, not keys.
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    let sidebar = cx.read(|cx| shell.read(cx).active_sidebar());
    let menus = cx.update(|cx| {
        let mut all = devdeck_app::window_view::workspace_menu_items(&shell, cx);
        all.extend(devdeck_app::layouts::menu(&deck, cx));
        all.extend(devdeck_app::sidebar::Sidebar::item_menu(&sidebar, "proxyapi", false, cx));
        all
    });
    assert!(menus.iter().all(|m| m.hint.is_none()), "no key hints in menus");
    // The cheat sheet lists every bound action with its keys and filters them.
    cx.simulate_keystrokes(main, "ctrl-/");
    cx.simulate_input(main, "split right");
    cx.run_until_parked();
    let sheet = cx.update(|cx| {
        let modal = devdeck_ui::overlay::layer(main, cx).read(cx).modals.last().cloned().unwrap();
        modal.downcast::<devdeck_app::cheat_sheet::CheatSheet>().unwrap()
    });
    let labels: Vec<&str> = cx.read(|cx| sheet.read(cx).entries(cx).iter().map(|s| s.label).collect());
    assert_eq!(labels, ["split right"]);
}

#[gpui::test]
fn the_palette_finds_drills_runs_on_an_env_and_dispatches_actions(cx: &mut TestAppContext) {
    let shell = start(cx, "palette");
    let main = main_window(&shell, cx);
    cx.simulate_keystrokes(main, "ctrl-k");
    cx.run_until_parked();
    assert_eq!(overlay(&shell, cx).1, 1);
    cx.simulate_input(main, "eventserv");
    cx.simulate_keystrokes(main, "tab");
    cx.run_until_parked();
    cx.simulate_input(main, "run @33");
    cx.simulate_keystrokes(main, "enter");
    cx.run_until_parked();
    assert_eq!(overlay(&shell, cx).1, 0, "running closes the palette");
    let services = cx.read(|cx| shell.read(cx).workspaces.iter().find(|w| w.id == "work").unwrap().services.clone());
    let run = cx.read(|cx| services.read(cx).runner.read(cx).run(&RunKey::new("eventservice", "go:run")).map(|r| (r.state, r.env.clone())));
    assert_eq!(run, Some((RunState::Running, Some("dev33".into()))), "@33 picks dev33 for this run");
    assert_eq!(cx.read(|cx| services.read(cx).env.clone()).as_deref(), Some("dev35"), "only for this run");
    // Esc leaves a drill-down first.
    cx.simulate_keystrokes(main, "ctrl-k");
    cx.simulate_input(main, "proxy");
    cx.simulate_keystrokes(main, "tab escape");
    cx.run_until_parked();
    assert_eq!(overlay(&shell, cx).1, 1, "back to the list, still open");
    cx.simulate_keystrokes(main, "escape");
    cx.run_until_parked();
    assert_eq!(overlay(&shell, cx).1, 0);
    // An action runs where focus was (the panes).
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    let panes = cx.read(|cx| deck.read(cx).layout().panes_in(MAIN_WINDOW).unwrap().len());
    cx.simulate_keystrokes(main, "ctrl-k");
    cx.simulate_input(main, "split right");
    cx.simulate_keystrokes(main, "enter");
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| deck.read(cx).layout().panes_in(MAIN_WINDOW).unwrap().len()), panes + 1);
    // A disabled item says why and keeps the palette open.
    cx.simulate_keystrokes(main, "ctrl-k");
    cx.simulate_input(main, "remotedesktopapp up");
    cx.simulate_keystrokes(main, "enter");
    cx.run_until_parked();
    assert_eq!(overlay(&shell, cx).1, 1);
}

#[gpui::test]
fn send_to_by_number_new_window_pane_and_cancel(cx: &mut TestAppContext) {
    let shell = start(cx, "send");
    let main = main_window(&shell, cx);
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    let find = |cx: &TestAppContext, v: &str| cx.read(|cx| deck.read(cx).layout().find_view(&ViewRef::new(v)).map(|(p, _)| p));
    let deployments = find(cx, "matrix").unwrap();
    // Focused pane: logs (number 1). Send its active tab to pane 2 (deployments).
    cx.simulate_keystrokes(main, "ctrl-shift-s");
    cx.run_until_parked();
    assert_eq!(overlay(&shell, cx).1, 1);
    cx.simulate_keystrokes(main, "1");
    cx.run_until_parked();
    assert!(cx.read(|cx| deck.read(cx).send.is_some()), "its own pane is refused");
    cx.simulate_keystrokes(main, "2");
    cx.run_until_parked();
    assert_eq!(find(cx, "demo/logs"), Some(deployments));
    assert_eq!(overlay(&shell, cx).1, 0, "done: the list closes");
    // Esc cancels.
    cx.simulate_keystrokes(main, "ctrl-shift-s escape");
    cx.run_until_parked();
    assert!(cx.read(|cx| deck.read(cx).send.is_none()));
    // New window.
    cx.simulate_keystrokes(main, "ctrl-shift-s n");
    cx.run_until_parked();
    assert_eq!(cx.windows().len(), 2);
    // A whole pane, by clicking a badge.
    let notes = find(cx, "demo/notes").unwrap();
    cx.update(|cx| deck.update(cx, |d, cx| d.apply(cx, |l| l.focus(notes))));
    let main_focus = cx.read(|cx| deck.read(cx).layout().main().focused);
    assert_eq!(main_focus, notes);
    cx.simulate_keystrokes(main, "ctrl-alt-shift-s");
    cx.run_until_parked();
    let mut vcx = VisualTestContext::from_window(main, cx);
    let badge = vcx.debug_bounds("send-target-1").expect("pick targets drawn");
    vcx.simulate_mouse_down(badge.center(), MouseButton::Left, Modifiers::none());
    vcx.run_until_parked();
    assert!(vcx.read(|cx| deck.read(cx).layout().pane(notes).is_err()), "the pane moved out whole");
    assert!(vcx.read(|cx| deck.read(cx).layout().find_view(&ViewRef::new("demo/notes")).is_some()), "and its tab is still open");
}

#[gpui::test]
fn a_pick_ends_when_the_workspace_hides(cx: &mut TestAppContext) {
    let shell = start(cx, "send-switch");
    let main = main_window(&shell, cx);
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    cx.simulate_keystrokes(main, "ctrl-shift-s");
    cx.run_until_parked();
    let personal = cx.read(|cx| shell.read(cx).workspaces.iter().position(|w| w.id == "personal").unwrap());
    cx.update(|cx| shell.update(cx, |s, cx| s.switch(personal, cx)));
    cx.run_until_parked();
    assert!(cx.read(|cx| deck.read(cx).send.is_none()));
    assert_eq!(overlay(&shell, cx).1, 0, "the list closed");
}

#[gpui::test]
fn shift_f10_and_right_click_open_menus(cx: &mut TestAppContext) {
    let shell = start(cx, "menus");
    let main = main_window(&shell, cx);
    cx.simulate_keystrokes(main, "shift-f10");
    cx.run_until_parked();
    assert!(overlay(&shell, cx).0, "the focused tab's menu");
    cx.simulate_keystrokes(main, "escape ctrl-1 down shift-f10");
    cx.run_until_parked();
    assert!(overlay(&shell, cx).0, "the selected row's menu");
    cx.simulate_keystrokes(main, "escape");
    let mut vcx = VisualTestContext::from_window(main, cx);
    let tab = vcx.debug_bounds("tab-title").expect("a tab");
    vcx.simulate_mouse_down(tab.center(), MouseButton::Right, Modifiers::none());
    vcx.run_until_parked();
    assert!(vcx.update(|_, cx| layer_state(main, cx).0), "right-click on a tab");
}

#[gpui::test]
fn layouts_from_the_modal_the_sidebar_and_the_palette(cx: &mut TestAppContext) {
    let shell = start(cx, "layouts-ui");
    let main = main_window(&shell, cx);
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    // Save as from the palette's action; an existing name is refused in the modal.
    cx.simulate_keystrokes(main, "ctrl-k");
    cx.simulate_input(main, "save pulse as");
    cx.simulate_keystrokes(main, "enter");
    cx.run_until_parked();
    assert_eq!(overlay(&shell, cx).1, 1, "the name modal");
    cx.simulate_input(main, "Default");
    cx.simulate_keystrokes(main, "enter");
    cx.run_until_parked();
    assert_eq!(overlay(&shell, cx).1, 1, "refused: stays open");
    cx.simulate_keystrokes(main, "ctrl-a");
    cx.simulate_input(main, "triage");
    cx.simulate_keystrokes(main, "enter");
    cx.run_until_parked();
    assert_eq!(overlay(&shell, cx).1, 0);
    assert_eq!(cx.read(|cx| deck.read(cx).book.current_name().to_string()), "triage");
    // Change the live layout, then switch back to default from the Pulses list.
    cx.simulate_keystrokes(main, "ctrl-w");
    cx.run_until_parked();
    let fewer = cx.read(|cx| deck.read(cx).layout().views().len());
    cx.simulate_keystrokes(main, "ctrl-4 down enter");
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| deck.read(cx).book.current_name().to_string()), "default", "first row (alphabetical) is default");
    assert_eq!(cx.read(|cx| deck.read(cx).layout().views().len()), fewer + 1, "default has the closed tab");
    // From the palette, back to triage.
    cx.simulate_keystrokes(main, "escape ctrl-k");
    cx.simulate_input(main, "pulse triage");
    cx.simulate_keystrokes(main, "enter");
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| deck.read(cx).book.current_name().to_string()), "triage");
    // Focus lands in the panes, so pane keys work right away.
    let panes = cx.read(|cx| deck.read(cx).layout().panes_in(MAIN_WINDOW).unwrap().len());
    cx.simulate_keystrokes(main, "ctrl-\\");
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| deck.read(cx).layout().panes_in(MAIN_WINDOW).unwrap().len()), panes + 1);
    // New pulse from the palette: an empty one, current; triage keeps its tabs.
    let triage = cx.read(|cx| deck.read(cx).layout().clone());
    cx.simulate_keystrokes(main, "ctrl-k");
    cx.simulate_input(main, "new pulse");
    cx.simulate_keystrokes(main, "enter");
    cx.run_until_parked();
    assert_eq!(overlay(&shell, cx).1, 1, "the name modal");
    cx.simulate_input(main, "blank");
    cx.simulate_keystrokes(main, "enter");
    cx.run_until_parked();
    assert_eq!(overlay(&shell, cx).1, 0);
    assert_eq!(cx.read(|cx| deck.read(cx).book.current_name().to_string()), "blank");
    assert_eq!(cx.read(|cx| deck.read(cx).layout().clone()), devdeck_core::layout::Layout::new(), "one empty pane");
    let id = cx.read(|cx| deck.read(cx).book.id_named("triage").unwrap());
    cx.update(|cx| deck.update(cx, |d, cx| d.switch_layout(&id, cx))).unwrap();
    assert_eq!(cx.read(|cx| deck.read(cx).layout().clone()), triage, "triage is as it was left");
}
