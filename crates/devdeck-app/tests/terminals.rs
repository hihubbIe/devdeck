//! M2 step 4: terminal tabs, on the demo (its terminals' shell is the fake driver's).
//! Opening one, closing ends it, a layout switch keeps it, closing a detached window
//! ends its terminals; no toasts; the quit list counts a terminal only while it runs
//! something.

use devdeck_app::deck::Deck;
use devdeck_app::fake_driver::STOP_DELAY;
use devdeck_app::runner::{RunKey, Runner};
use devdeck_app::shell::Shell;
use devdeck_app::views::run_view::RunView;
use devdeck_core::layout::ViewRef;
use devdeck_core::layout_book::DEFAULT_ID;
use devdeck_core::runs::RunState;
use devdeck_core::view_addr::ViewAddr;
use gpui::{AnyWindowHandle, AppContext, Entity, Modifiers, MouseButton, TestAppContext, VisualTestContext};
use std::time::Duration;

fn start(cx: &mut TestAppContext, tag: &str) -> Entity<Shell> {
    let config = cx.update(|cx| devdeck_app::test_dir(&format!("{tag}-config"), cx));
    cx.update(|cx| devdeck_app::init_with_config_dir(&config, cx));
    let shell = cx.update(|cx| Shell::start(devdeck_app::demo_workspaces(), "work", None, cx));
    cx.update(|cx| shell.update(cx, |s, _| s.quit_for_real = false));
    cx.run_until_parked();
    shell
}

fn deck(shell: &Entity<Shell>, cx: &TestAppContext) -> Entity<Deck> {
    cx.read(|cx| shell.read(cx).active_deck())
}

fn runner(shell: &Entity<Shell>, cx: &TestAppContext) -> Entity<Runner> {
    cx.read(|cx| shell.read(cx).workspaces.iter().find(|w| w.id == "work").unwrap().services.read(cx).runner.clone())
}

fn main_window(shell: &Entity<Shell>, cx: &TestAppContext) -> AnyWindowHandle {
    cx.read(|cx| shell.read(cx).main_window().unwrap().into())
}

/// The terminal tabs of the live layout.
fn terminal_tabs(deck: &Entity<Deck>, cx: &TestAppContext) -> Vec<ViewRef> {
    cx.read(|cx| deck.read(cx).layout().views().into_iter().filter(|r| matches!(ViewAddr::parse(r), ViewAddr::Terminal { .. })).collect())
}

fn settle(cx: &mut TestAppContext) {
    cx.executor().advance_clock(STOP_DELAY * 2 + Duration::from_secs(3));
    cx.run_until_parked();
}

#[gpui::test]
fn the_key_opens_a_terminal_in_the_focused_tabs_app_folder(cx: &mut TestAppContext) {
    let shell = start(cx, "term-open");
    let main = main_window(&shell, cx);
    let deck = deck(&shell, cx);
    // With an app's tab active, its folder.
    let app = ViewAddr::App { app: "eventservice".into() }.to_ref();
    cx.update(|cx| deck.update(cx, |d, cx| d.apply(cx, |l| l.open(app.clone(), l.main().focused).map(|_| ()))));
    cx.run_until_parked();
    let r = runner(&shell, cx);
    let dir = cx.read(|cx| shell.read(cx).workspaces.iter().find(|w| w.id == "work").unwrap().services.read(cx).catalog.app("eventservice").unwrap().dir.clone());
    assert_eq!(cx.read(|cx| deck.read(cx).terminal_folder(None, cx)), dir);
    cx.update_window(main, |_, window, cx| {
        let f = deck.read(cx).view_focus(deck.read(cx).layout().main().focused).unwrap();
        f.focus(window, cx);
    })
    .unwrap();
    cx.simulate_keystrokes(main, "ctrl-shift-t");
    cx.run_until_parked();
    let tabs = terminal_tabs(&deck, cx);
    assert_eq!(tabs.len(), 1, "one terminal tab");
    let ViewAddr::Terminal { n, dir: opened } = ViewAddr::parse(&tabs[0]) else { unreachable!() };
    assert_eq!(opened, dir.display().to_string(), "in the app's folder");
    let key = RunKey::terminal(n);
    assert_eq!(cx.read(|cx| r.read(cx).state(&key)), Some(RunState::Running), "its shell started");
    let view = cx.read(|cx| deck.read(cx).view(&tabs[0]).unwrap().downcast::<RunView>().unwrap());
    assert!(cx.update_window(main, |_, window, cx| view.read(cx).terminal_focused(window)).unwrap(), "the keys go to it");
    assert!(cx.read(|cx| r.read(cx).running()).iter().all(|t| !t.starts_with("terminal")), "idle: not in the quit list");
    assert!(cx.read(|cx| r.read(cx).alive()), "but quitting waits for it");
    // A second one gets the next number.
    cx.simulate_keystrokes(main, "ctrl-shift-t");
    cx.run_until_parked();
    let numbers: Vec<u32> = terminal_tabs(&deck, cx)
        .iter()
        .filter_map(|r| match ViewAddr::parse(r) {
            ViewAddr::Terminal { n, .. } => Some(n),
            _ => None,
        })
        .collect();
    assert_eq!(numbers.len(), 2);
    assert_ne!(numbers[0], numbers[1]);
}

#[gpui::test]
fn closing_a_terminal_ends_it_and_a_layout_switch_keeps_it(cx: &mut TestAppContext) {
    let shell = start(cx, "term-close");
    let main = main_window(&shell, cx);
    let deck = deck(&shell, cx);
    let r = runner(&shell, cx);
    let other = cx.update(|cx| deck.update(cx, |d, cx| d.save_layout_as("other", cx).unwrap()));
    cx.simulate_keystrokes(main, "ctrl-shift-t");
    cx.run_until_parked();
    let tab = terminal_tabs(&deck, cx)[0].clone();
    let ViewAddr::Terminal { n, .. } = ViewAddr::parse(&tab) else { unreachable!() };
    let key = RunKey::terminal(n);
    // A layout without it: hidden, still running; back: the same shell.
    // "Save as" made "other" current: "default" is the layout without it.
    cx.update(|cx| deck.update(cx, |d, cx| d.switch_layout(DEFAULT_ID, cx).unwrap()));
    settle(cx);
    assert!(terminal_tabs(&deck, cx).is_empty());
    assert_eq!(cx.read(|cx| r.read(cx).state(&key)), Some(RunState::Running), "a switch doesn't end it");
    cx.update(|cx| deck.update(cx, |d, cx| d.switch_layout(&other, cx).unwrap()));
    settle(cx);
    assert_eq!(terminal_tabs(&deck, cx), std::slice::from_ref(&tab));
    assert_eq!(cx.read(|cx| r.read(cx).run(&key).map(|run| run.runs)), Some(1), "the same shell, not a new one");
    // Closing its tab ends it, and it's forgotten.
    cx.update(|cx| deck.update(cx, |d, cx| d.apply(cx, |l| l.close_view(&tab))));
    settle(cx);
    assert_eq!(cx.read(|cx| r.read(cx).state(&key)), None, "ended and forgotten");
    assert!(cx.read(|cx| r.read(cx).terminals()).is_empty());
    let toasts: Vec<String> = cx.update(|cx| devdeck_ui::toasts(cx).read(cx).queue.items.iter().map(|t| t.text.to_string()).collect());
    assert!(toasts.iter().all(|t| !t.contains("terminal")), "no toast for a shell: {toasts:?}");
}

#[gpui::test]
fn closing_a_detached_window_ends_its_terminals(cx: &mut TestAppContext) {
    let shell = start(cx, "term-detached");
    let main = main_window(&shell, cx);
    let deck = deck(&shell, cx);
    let r = runner(&shell, cx);
    cx.simulate_keystrokes(main, "ctrl-shift-t");
    cx.run_until_parked();
    let tab = terminal_tabs(&deck, cx)[0].clone();
    let ViewAddr::Terminal { n, .. } = ViewAddr::parse(&tab) else { unreachable!() };
    let window = cx.update(|cx| {
        deck.update(cx, |d, cx| {
            d.apply(cx, |l| {
                let (p, i) = l.find_view(&tab).expect("open");
                l.detach(p, i)
            })
        })
    });
    cx.run_until_parked();
    let window = window.expect("detached");
    assert_eq!(cx.read(|cx| r.read(cx).state(&RunKey::terminal(n))), Some(RunState::Running), "detaching keeps it");
    cx.update(|cx| deck.update(cx, |d, cx| d.close_window(window, cx)));
    settle(cx);
    assert_eq!(cx.read(|cx| r.read(cx).state(&RunKey::terminal(n))), None, "closing the window ends it");
}

#[gpui::test]
fn the_terminals_list_has_every_shell_shown_or_not(cx: &mut TestAppContext) {
    use devdeck_app::sidebar::Activity;
    let shell = start(cx, "term-list");
    let main = main_window(&shell, cx);
    let deck = deck(&shell, cx);
    let r = runner(&shell, cx);
    cx.update(|cx| deck.update(cx, |d, cx| d.save_layout_as("other", cx).unwrap()));
    cx.simulate_keystrokes(main, "ctrl-shift-t");
    cx.run_until_parked();
    let ViewAddr::Terminal { n, .. } = ViewAddr::parse(&terminal_tabs(&deck, cx)[0]) else { unreachable!() };
    cx.update(|cx| deck.update(cx, |d, cx| d.switch_layout(DEFAULT_ID, cx).unwrap()));
    settle(cx);
    cx.simulate_keystrokes(main, "ctrl-5");
    cx.run_until_parked();
    let sidebar = cx.read(|cx| shell.read(cx).active_sidebar());
    assert_eq!(cx.read(|cx| sidebar.read(cx).activity()), Activity::Terminals);
    let listed = |cx: &TestAppContext| cx.read(|cx| sidebar.read(cx).items(cx).into_iter().map(|i| (i.key, i.detail)).collect::<Vec<_>>());
    assert_eq!(listed(cx), [(n.to_string(), "idle · in other".to_string())], "listed though not shown here");
    // Open: into this layout, the same shell.
    cx.update(|cx| sidebar.update(cx, |s, cx| s.open(&n.to_string(), cx)));
    cx.run_until_parked();
    assert_eq!(terminal_tabs(&deck, cx).len(), 1);
    assert_eq!(listed(cx)[0].1, "idle · here");
    assert_eq!(cx.read(|cx| r.read(cx).run(&RunKey::terminal(n)).map(|run| run.runs)), Some(1));
    // End: its tab closes, its shell ends, it leaves the list.
    cx.update(|cx| sidebar.update(cx, |s, cx| s.end_terminal(n, cx)));
    settle(cx);
    assert!(terminal_tabs(&deck, cx).is_empty());
    assert!(listed(cx).is_empty());
}

#[gpui::test]
fn an_ended_terminal_stays_ended_through_layout_switches(cx: &mut TestAppContext) {
    let shell = start(cx, "term-ended");
    let main = main_window(&shell, cx);
    let deck = deck(&shell, cx);
    let r = runner(&shell, cx);
    cx.simulate_keystrokes(main, "ctrl-shift-t");
    cx.run_until_parked();
    let tab = terminal_tabs(&deck, cx)[0].clone();
    let ViewAddr::Terminal { n, .. } = ViewAddr::parse(&tab) else { unreachable!() };
    let key = RunKey::terminal(n);
    // In two layouts; closed in one: gone from both, not started again by the other.
    let other = cx.update(|cx| deck.update(cx, |d, cx| d.save_layout_as("other", cx).unwrap()));
    cx.update(|cx| deck.update(cx, |d, cx| d.apply(cx, |l| l.close_view(&tab))));
    settle(cx);
    cx.update(|cx| deck.update(cx, |d, cx| d.switch_layout(DEFAULT_ID, cx).unwrap()));
    settle(cx);
    assert!(terminal_tabs(&deck, cx).is_empty(), "closing is closing, in every layout");
    assert_eq!(cx.read(|cx| r.read(cx).state(&key)), None);
    // Stopped (■, or `exit`): its tab keeps it ended through a round trip; ⏎ starts it.
    cx.simulate_keystrokes(main, "ctrl-shift-t");
    cx.run_until_parked();
    let tab = terminal_tabs(&deck, cx)[0].clone();
    let ViewAddr::Terminal { n, .. } = ViewAddr::parse(&tab) else { unreachable!() };
    let key = RunKey::terminal(n);
    cx.update(|cx| r.update(cx, |r, cx| r.stop(&key, cx)));
    settle(cx);
    cx.update(|cx| deck.update(cx, |d, cx| d.switch_layout(&other, cx).unwrap()));
    cx.update(|cx| deck.update(cx, |d, cx| d.switch_layout(DEFAULT_ID, cx).unwrap()));
    settle(cx);
    assert_eq!(terminal_tabs(&deck, cx), std::slice::from_ref(&tab));
    assert_eq!(cx.read(|cx| r.read(cx).state(&key)), Some(RunState::Stopped), "not started again by the switch");
    assert_eq!(cx.read(|cx| r.read(cx).run(&key).map(|run| run.runs)), Some(1));
}

#[gpui::test]
fn a_terminal_has_no_arguments(cx: &mut TestAppContext) {
    let shell = start(cx, "term-args");
    let main = main_window(&shell, cx);
    let deck = deck(&shell, cx);
    let r = runner(&shell, cx);
    cx.simulate_keystrokes(main, "ctrl-shift-t");
    cx.run_until_parked();
    let ViewAddr::Terminal { n, .. } = ViewAddr::parse(&terminal_tabs(&deck, cx)[0]) else { unreachable!() };
    cx.simulate_keystrokes(main, "ctrl-shift-a");
    cx.run_until_parked();
    assert!(!cx.update(|cx| devdeck_ui::overlay::layer_state(main, cx).0), "no arguments menu");
    assert_eq!(cx.read(|cx| r.read(cx).run(&RunKey::terminal(n)).map(|run| run.runs)), Some(1), "the shell wasn't restarted");
}

#[gpui::test]
fn quitting_with_idle_shells_hangs_them_up_first(cx: &mut TestAppContext) {
    let shell = start(cx, "term-quit");
    let main = main_window(&shell, cx);
    let deck = deck(&shell, cx);
    let r = runner(&shell, cx);
    // Only the terminal: the demo's own runs stopped first.
    cx.update(|cx| r.update(cx, |r, cx| r.stop_all(cx)));
    settle(cx);
    cx.simulate_keystrokes(main, "ctrl-shift-t");
    cx.run_until_parked();
    let ViewAddr::Terminal { n, .. } = ViewAddr::parse(&terminal_tabs(&deck, cx)[0]) else { unreachable!() };
    for w in cx.read(|cx| shell.read(cx).workspaces.iter().map(|w| w.services.read(cx).runner.clone()).collect::<Vec<_>>()) {
        if w != r {
            cx.update(|cx| w.update(cx, |r, cx| r.stop_all(cx)));
        }
    }
    settle(cx);
    assert!(cx.read(|cx| shell.read(cx).running(cx)).iter().all(|(_, runs)| runs.is_empty()), "nothing to ask about");
    cx.update(|cx| shell.update(cx, |s, cx| s.request_quit(cx)));
    settle(cx);
    assert!(!cx.read(|cx| r.read(cx).state(&RunKey::terminal(n))).is_some_and(|s| s.is_running()), "hung up before quitting");
    assert_eq!(cx.read(|cx| shell.read(cx).quits), 1);
}

fn app_dir(shell: &Entity<Shell>, app: &str, cx: &TestAppContext) -> String {
    cx.read(|cx| shell.read(cx).workspaces.iter().find(|w| w.id == "work").unwrap().services.read(cx).catalog.app(app).unwrap().dir.display().to_string())
}

fn terminal_dir(r: &ViewRef) -> String {
    let ViewAddr::Terminal { dir, .. } = ViewAddr::parse(r) else { panic!("not a terminal: {r:?}") };
    dir
}

#[gpui::test]
fn open_terminal_here_from_a_tab_and_in_a_new_window(cx: &mut TestAppContext) {
    let shell = start(cx, "term-here");
    let main = main_window(&shell, cx);
    let deck = deck(&shell, cx);
    let app = ViewAddr::App { app: "eventservice".into() }.to_ref();
    let pane = cx.update(|cx| deck.update(cx, |d, cx| d.apply(cx, |l| l.open(app.clone(), l.main().focused)))).unwrap();
    cx.run_until_parked();
    let tab_menu = |cx: &mut TestAppContext, r: &ViewRef| {
        let i = cx.read(|cx| deck.read(cx).layout().find_view(r).unwrap().1);
        let mut vcx = VisualTestContext::from_window(main, cx);
        let at = vcx.debug_bounds(Box::leak(format!("tab-{}-{i}", pane.0).into_boxed_str())).expect("the tab").center();
        vcx.simulate_mouse_down(at, MouseButton::Right, Modifiers::none());
        vcx.run_until_parked();
    };
    // The tab's menu; "Open terminal here" is its last item: a click (⏎) opens it in this pane.
    tab_menu(cx, &app);
    cx.simulate_keystrokes(main, "up enter");
    cx.run_until_parked();
    let tabs = terminal_tabs(&deck, cx);
    assert_eq!(tabs.len(), 1);
    assert_eq!(terminal_dir(&tabs[0]), app_dir(&shell, "eventservice", cx), "in the tab's app folder");
    assert_eq!(cx.read(|cx| deck.read(cx).layout().find_view(&tabs[0]).map(|(p, _)| p)), Some(pane), "in this pane");
    let view = cx.read(|cx| deck.read(cx).view(&tabs[0]).unwrap().downcast::<RunView>().unwrap());
    assert!(cx.update_window(main, |_, window, cx| view.read(cx).terminal_focused(window)).unwrap(), "the keys go to it");
    // From the terminal's tab, its submenu's last place (→, then ↑ from the first): a new window.
    tab_menu(cx, &tabs[0]);
    cx.simulate_keystrokes(main, "up right up enter");
    cx.run_until_parked();
    let tabs = terminal_tabs(&deck, cx);
    assert_eq!(tabs.len(), 2, "a second shell");
    let new = tabs.iter().find(|r| cx.read(|cx| deck.read(cx).layout().find_view(r).map(|(p, _)| p)) != Some(pane)).expect("elsewhere").clone();
    let detached = cx.read(|cx| {
        let l = deck.read(cx).layout();
        l.find_view(&new).and_then(|(p, _)| l.window_of(p).ok()).is_some_and(|w| l.is_detached(w))
    });
    assert!(detached, "in a window of its own");
    assert_eq!(terminal_dir(&new), app_dir(&shell, "eventservice", cx), "a terminal's folder is its own");
}

#[gpui::test]
fn the_terminals_list_opens_a_shell_in_another_pane(cx: &mut TestAppContext) {
    use devdeck_app::places::Place;
    let shell = start(cx, "term-places");
    let main = main_window(&shell, cx);
    let deck = deck(&shell, cx);
    let r = runner(&shell, cx);
    let panes = cx.read(|cx| deck.read(cx).layout().numbered_panes());
    assert!(panes.len() >= 2, "the demo has several panes: {panes:?}");
    let (first, second) = (panes[0].0, panes[1].0);
    cx.update(|cx| deck.update(cx, |d, cx| d.open_terminal(std::path::Path::new("/tmp"), Place::Pane(first), cx)));
    cx.run_until_parked();
    let tab = terminal_tabs(&deck, cx)[0].clone();
    let ViewAddr::Terminal { n, .. } = ViewAddr::parse(&tab) else { unreachable!() };
    // Its row's menu: "Open" → its places; the first is where it is (refused), the
    // second takes it.
    cx.simulate_keystrokes(main, "ctrl-5 down shift-f10 right enter");
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| deck.read(cx).layout().find_view(&tab).map(|(p, _)| p)), Some(first), "already here: nothing moved");
    cx.simulate_keystrokes(main, "down enter");
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| deck.read(cx).layout().find_view(&tab).map(|(p, _)| p)), Some(second), "moved to the second pane");
    assert_eq!(cx.read(|cx| r.read(cx).run(&RunKey::terminal(n)).map(|run| run.runs)), Some(1), "the same shell");
}

#[gpui::test]
fn an_app_rows_menu_opens_a_terminal_in_its_folder(cx: &mut TestAppContext) {
    let shell = start(cx, "term-app-row");
    let main = main_window(&shell, cx);
    let deck = deck(&shell, cx);
    cx.simulate_keystrokes(main, "ctrl-1");
    cx.run_until_parked();
    let sidebar = cx.read(|cx| shell.read(cx).active_sidebar());
    cx.update(|cx| sidebar.update(cx, |s, _| s.selected = Some("eventservice".into())));
    cx.simulate_keystrokes(main, "shift-f10 up enter");
    cx.run_until_parked();
    let tabs = terminal_tabs(&deck, cx);
    assert_eq!(tabs.len(), 1);
    assert_eq!(terminal_dir(&tabs[0]), app_dir(&shell, "eventservice", cx));
    let focused = cx.read(|cx| deck.read(cx).layout().main().focused);
    assert_eq!(cx.read(|cx| deck.read(cx).layout().find_view(&tabs[0]).map(|(p, _)| p)), Some(focused), "in the main window's focused pane");
}
