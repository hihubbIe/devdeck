//! "Open in" (R6.13, R6.14): what a row, a note or a tab offers (the thing itself from a
//! full view, a shell and each agent where there's a folder; pulses outside a pulse, panes
//! inside it, Terminals or Agents first for a shell or a session), and what each choice
//! does.

use devdeck_app::deck::Deck;
use devdeck_app::open_in::{self, OpenedFrom, Subject};
use devdeck_app::runner::RunKey;
use devdeck_app::shell::Shell;
use devdeck_app::sidebar::{Activity, Sidebar};
use devdeck_core::layout::{MAIN_WINDOW, ViewRef};
use devdeck_core::view_addr::ViewAddr;
use gpui::{AnyWindowHandle, AppContext, Entity, TestAppContext};
use std::path::PathBuf;

fn start(cx: &mut TestAppContext, tag: &str, agents: bool) -> Entity<Shell> {
    let config = cx.update(|cx| devdeck_app::test_dir(&format!("{tag}-config"), cx));
    cx.update(|cx| devdeck_app::init_with_config_dir(&config, cx));
    let kinds = if agents { vec![super::agents::kind()] } else { Vec::new() };
    cx.update(|cx| devdeck_app::environment::environment(cx).update(cx, |e, _| e.agents = kinds));
    let shell = cx.update(|cx| Shell::start(devdeck_app::demo_workspaces(), "work", None, cx));
    cx.update(|cx| shell.update(cx, |s, _| s.quit_for_real = false));
    cx.run_until_parked();
    shell
}

fn deck(shell: &Entity<Shell>, cx: &TestAppContext) -> Entity<Deck> {
    cx.read(|cx| shell.read(cx).active_deck())
}

fn sidebar(shell: &Entity<Shell>, cx: &TestAppContext) -> Entity<Sidebar> {
    cx.read(|cx| shell.read(cx).active_sidebar())
}

fn main_window(shell: &Entity<Shell>, cx: &TestAppContext) -> AnyWindowHandle {
    cx.read(|cx| shell.read(cx).main_window().unwrap().into())
}

fn app_dir(shell: &Entity<Shell>, app: &str, cx: &TestAppContext) -> PathBuf {
    let deck = deck(shell, cx);
    cx.read(|cx| deck.read(cx).services().unwrap().read(cx).catalog.app(app).unwrap().dir.clone())
}

/// Each WHAT with its WHERE, as values.
fn offered(deck: &Entity<Deck>, subject: &Subject, from: OpenedFrom, cx: &TestAppContext) -> Vec<(String, Vec<String>)> {
    cx.read(|cx| open_in::choices(deck, subject, from, cx).into_iter().map(|c| (c.what.value, c.wheres.into_iter().map(|w| w.value).collect())).collect())
}

fn choose(shell: &Entity<Shell>, subject: &Subject, from: OpenedFrom, what: &str, place: &str, cx: &mut TestAppContext) {
    let (main, deck) = (main_window(shell, cx), deck(shell, cx));
    cx.update_window(main, |_, window, cx| open_in::choose(&deck, subject, from, what, place, window, cx)).unwrap();
    cx.run_until_parked();
}

fn pulses(deck: &Entity<Deck>, cx: &TestAppContext) -> Vec<String> {
    cx.read(|cx| deck.read(cx).book.entries().into_iter().map(|(id, _)| format!("pulse:{id}")).collect())
}

fn terminal_tabs(deck: &Entity<Deck>, cx: &TestAppContext) -> Vec<ViewRef> {
    cx.read(|cx| deck.read(cx).layout().views().into_iter().filter(|r| matches!(ViewAddr::parse(r), ViewAddr::Terminal { .. })).collect())
}

#[gpui::test]
fn outside_a_pulse_only_pulses_are_offered(cx: &mut TestAppContext) {
    let shell = start(cx, "open-in-outside", true);
    let deck = deck(&shell, cx);
    cx.update(|cx| deck.update(cx, |d, cx| d.save_layout_as("triage", cx))).unwrap();
    let all = pulses(&deck, cx);
    assert_eq!(all.len(), 2);
    let with_home = |home: &str| std::iter::once(home.to_string()).chain(all.iter().cloned()).collect::<Vec<_>>();
    // An app: itself, a shell and a session in its folder.
    let app = ViewAddr::App { app: "eventservice".into() }.to_ref();
    let dir = cx.read(|cx| deck.read(cx).own_folder(&app, cx));
    assert_eq!(dir, Some(app_dir(&shell, "eventservice", cx)), "an app's own folder");
    assert_eq!(
        offered(&deck, &Subject::view(app, dir), OpenedFrom::Outside, cx),
        [("view".to_string(), all.clone()), ("terminal".into(), with_home("home")), ("agent:claude".into(), with_home("home"))]
    );
    // A note has no folder: itself only.
    let note = ViewAddr::Note { id: "n1".into() }.to_ref();
    assert_eq!(cx.read(|cx| deck.read(cx).own_folder(&note, cx)), None);
    assert_eq!(offered(&deck, &Subject::view(note, None), OpenedFrom::Outside, cx), [("view".to_string(), all.clone())]);
}

#[gpui::test]
fn inside_a_pulse_a_tab_offers_a_shell_and_sessions_in_its_panes(cx: &mut TestAppContext) {
    let shell = start(cx, "open-in-inside", false);
    let deck = deck(&shell, cx);
    let (pane, panes) = cx.read(|cx| {
        let l = deck.read(cx).layout();
        (l.main().focused, l.numbered_panes().into_iter().map(|(p, _)| format!("pane:{}", p.0)).collect::<Vec<_>>())
    });
    let wheres: Vec<String> = std::iter::once("home".to_string()).chain(panes).chain(std::iter::once("window".to_string())).collect();
    // No agents: a shell only; the view itself isn't offered (Send to moves it). A pane
    // with no folder of its own still gets the folder a new terminal would.
    assert_eq!(offered(&deck, &Subject::default(), OpenedFrom::Pane(pane, MAIN_WINDOW), cx), [("terminal".to_string(), wheres)]);
}

#[gpui::test]
fn a_shell_in_terminals_is_selected_there_and_kept(cx: &mut TestAppContext) {
    let shell = start(cx, "open-in-terminals", false);
    let (deck, sidebar) = (deck(&shell, cx), sidebar(&shell, cx));
    let dir = app_dir(&shell, "eventservice", cx);
    let before = cx.read(|cx| deck.read(cx).terminals(cx).len());
    choose(&shell, &Subject::folder(dir.clone()), OpenedFrom::Outside, "terminal", "home", cx);
    assert_eq!(cx.read(|cx| sidebar.read(cx).activity()), Activity::Terminals);
    let n: u32 = cx.read(|cx| sidebar.read(cx).selected.clone()).expect("selected").parse().unwrap();
    let shown = cx.read(|cx| deck.read(cx).outside().map(<[ViewRef]>::to_vec)).unwrap_or_default();
    assert!(matches!(shown.as_slice(), [r] if matches!(ViewAddr::parse(r), ViewAddr::Terminal { n: m, dir: d } if m == n && std::path::Path::new(&d) == dir)), "{shown:?}");
    assert!(terminal_tabs(&deck, cx).is_empty(), "no tab in the pulse");
    assert_eq!(cx.read(|cx| deck.read(cx).terminals(cx).len()), before + 1, "known, though no tab shows it");
    let runner = cx.read(|cx| deck.read(cx).services().unwrap().read(cx).runner.clone());
    assert!(cx.read(|cx| runner.read(cx).state(&RunKey::terminal(n))).is_some_and(|s| s.is_running()), "its view started it");
}

#[gpui::test]
fn a_shell_into_a_saved_pulse_waits_there(cx: &mut TestAppContext) {
    let shell = start(cx, "open-in-saved", false);
    let deck = deck(&shell, cx);
    let triage = cx.update(|cx| deck.update(cx, |d, cx| d.save_layout_as("triage", cx))).unwrap();
    let default = cx.read(|cx| deck.read(cx).book.id_named("default")).expect("the default pulse");
    cx.update(|cx| deck.update(cx, |d, cx| d.switch_layout(&default, cx))).unwrap();
    cx.run_until_parked();
    choose(&shell, &Subject::folder(PathBuf::from("/tmp")), OpenedFrom::Outside, "terminal", &format!("pulse:{triage}"), cx);
    assert!(terminal_tabs(&deck, cx).is_empty(), "the live pulse untouched");
    let saved = cx.read(|cx| deck.read(cx).book.layouts[&triage].layout.clone()).unwrap();
    let first = saved.panes_in(MAIN_WINDOW).unwrap()[0];
    let tabs = saved.pane(first).unwrap().tabs.clone();
    assert!(tabs.iter().any(|r| matches!(ViewAddr::parse(r), ViewAddr::Terminal { dir, .. } if dir == "/tmp")), "in its first pane: {tabs:?}");
    let listed = cx.read(|cx| deck.read(cx).terminals(cx).into_values().filter(|t| t.layouts.iter().any(|(id, _)| *id == triage)).count());
    assert_eq!(listed, 1, "the Terminals list says where it waits");
}

#[gpui::test]
fn a_session_in_agents_is_selected_there(cx: &mut TestAppContext) {
    let shell = start(cx, "open-in-agents", true);
    let sidebar = sidebar(&shell, cx);
    let dir = app_dir(&shell, "eventservice", cx);
    choose(&shell, &Subject::folder(dir.clone()), OpenedFrom::Outside, "agent:claude", "home", cx);
    assert_eq!(cx.read(|cx| sidebar.read(cx).activity()), Activity::Agents);
    let deck = deck(&shell, cx);
    let agents = cx.read(|cx| deck.read(cx).services().unwrap().read(cx).agents.clone());
    let made = cx.read(|cx| agents.read(cx).sessions().last().map(|s| (s.id.clone(), s.dir.clone()))).expect("a session");
    assert_eq!(made.1, dir, "in the folder");
    assert_eq!(cx.read(|cx| sidebar.read(cx).selected.clone()), Some(made.0));
}
