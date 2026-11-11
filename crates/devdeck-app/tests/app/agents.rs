//! Agent sessions (R6.14), on the demo: a session is a record whose process is one of the
//! runner's terminals running its agent's line. It starts when shown, resumes when shown
//! again, keeps running off screen while it works or rang, and stops once off screen and
//! quiet for its agent's `idle_stop`. The Agents list is its own; Terminals doesn't list
//! it.

use devdeck_app::agents::Agents;
use devdeck_app::deck::Deck;
use devdeck_app::runner::{RunId, RunKey, Runner};
use devdeck_app::shell::Shell;
use devdeck_app::sidebar::{Activity, Sidebar};
use devdeck_core::agents::{AgentKind, Status};
use devdeck_core::view_addr::ViewAddr;
use gpui::{Entity, TestAppContext};
use std::path::Path;
use std::time::Duration;

const IDLE: Duration = Duration::from_secs(20);

fn kind() -> AgentKind {
    AgentKind {
        id: "claude".into(),
        name: "Claude Code".into(),
        command: "claude".into(),
        args: vec![],
        start: vec!["--session-id".into(), "{id}".into()],
        resume: vec!["--resume".into(), "{id}".into()],
        idle_stop: IDLE,
    }
}

struct World {
    deck: Entity<Deck>,
    sidebar: Entity<Sidebar>,
    agents: Entity<Agents>,
    runner: Entity<Runner>,
}

fn start(cx: &mut TestAppContext, tag: &str) -> World {
    let config = cx.update(|cx| devdeck_app::test_dir(&format!("{tag}-config"), cx));
    cx.update(|cx| devdeck_app::init_with_config_dir(&config, cx));
    cx.update(|cx| {
        let env = devdeck_app::environment::environment(cx);
        env.update(cx, |e, _| e.agents = vec![kind()]);
    });
    let shell = cx.update(|cx| Shell::start(devdeck_app::demo_workspaces(), "work", None, cx));
    cx.update(|cx| shell.update(cx, |s, _| s.quit_for_real = false));
    cx.run_until_parked();
    cx.read(|cx| {
        let w = shell.read(cx).workspaces.iter().find(|w| w.id == "work").unwrap();
        let s = w.services.read(cx);
        World { deck: w.deck.clone(), sidebar: w.sidebar.clone(), agents: s.agents.clone(), runner: s.runner.clone() }
    })
}

/// A new session in `/tmp/api`, from the sidebar's way in; its id.
fn new_session(w: &World, cx: &mut TestAppContext) -> String {
    cx.update(|cx| devdeck_app::sidebar::new_agent_session(&w.deck, "claude", Path::new("/tmp/api"), cx));
    cx.run_until_parked();
    cx.read(|cx| w.agents.read(cx).sessions().last().unwrap().id.clone())
}

fn key(w: &World, id: &str, cx: &TestAppContext) -> RunKey {
    cx.read(|cx| w.agents.read(cx).key(id).unwrap())
}

fn running(w: &World, id: &str, cx: &TestAppContext) -> bool {
    cx.read(|cx| w.agents.read(cx).running(id, cx))
}

fn wait(cx: &mut TestAppContext, d: Duration) {
    cx.executor().advance_clock(d);
    cx.run_until_parked();
}

/// Long enough for a stop asked now to end the (fake) process.
fn settle(cx: &mut TestAppContext) {
    wait(cx, devdeck_app::fake_driver::STOP_DELAY * 2);
}

#[gpui::test]
fn a_new_session_shows_in_the_agents_view_and_starts_with_its_id(cx: &mut TestAppContext) {
    let w = start(cx, "agents-new");
    let id = new_session(&w, cx);
    cx.read(|cx| {
        let s = w.sidebar.read(cx);
        assert_eq!((s.activity(), s.selected.as_deref()), (Activity::Agents, Some(id.as_str())));
        assert_eq!(w.deck.read(cx).outside().and_then(|o| o.last().cloned()), Some(ViewAddr::Agent { id: id.clone() }.to_ref()), "its full view shows it");
        assert!(w.agents.read(cx).shown().contains(&id));
        let rec = w.agents.read(cx).session(&id).unwrap();
        assert!(rec.started);
        assert_eq!(rec.label("Claude Code"), "Claude Code · api");
    });
    assert!(running(&w, &id, cx));
    let k = key(&w, &id, cx);
    cx.read(|cx| {
        let r = w.runner.read(cx);
        assert!(r.is_program(&k));
        assert_eq!(r.terminal_line(&k), Some(format!("exec claude --resume {id}").as_str()), "a later start resumes it");
        // Its list is Agents', not Terminals'.
        let terminals = w.sidebar.read(cx).items_in(Activity::Terminals, cx);
        assert!(!terminals.iter().any(|i| i.key == k.terminal_number().unwrap().to_string()));
        let agents = w.sidebar.read(cx).items_in(Activity::Agents, cx);
        assert_eq!(agents.iter().map(|i| i.key.as_str()).collect::<Vec<_>>(), [id.as_str()]);
        assert!(agents[0].running);
    });
    // A terminal made later takes another number.
    let n = cx.read(|cx| w.deck.read(cx).next_terminal(cx));
    assert!(n > k.terminal_number().unwrap());
}

#[gpui::test]
fn off_screen_and_quiet_it_stops_and_shown_again_it_resumes(cx: &mut TestAppContext) {
    let w = start(cx, "agents-idle");
    let id = new_session(&w, cx);
    wait(cx, Duration::from_secs(2));
    // Off screen: another activity.
    cx.update(|cx| w.sidebar.update(cx, |s, cx| s.show(Activity::Apps, cx)));
    cx.run_until_parked();
    assert!(!cx.read(|cx| w.agents.read(cx).shown().contains(&id)));
    wait(cx, IDLE - Duration::from_secs(2));
    assert!(running(&w, &id, cx), "not quiet long enough yet");
    wait(cx, Duration::from_secs(5));
    settle(cx);
    assert!(!running(&w, &id, cx), "stopped");
    assert_eq!(cx.read(|cx| w.agents.read(cx).status(&id, cx)), Status::Off);
    assert_eq!(cx.read(|cx| w.agents.read(cx).sessions().len()), 1, "the session stays");
    // Shown again: it starts, resuming.
    cx.update(|cx| w.sidebar.update(cx, |s, cx| s.show_item(Activity::Agents, &id, cx)));
    cx.run_until_parked();
    assert!(running(&w, &id, cx), "resumed");
    let k = key(&w, &id, cx);
    assert_eq!(cx.read(|cx| w.runner.read(cx).run(&k).map(|r| r.runs)), Some(1), "a new process for a forgotten terminal");
}

#[gpui::test]
fn off_screen_it_keeps_running_while_it_works_or_rang(cx: &mut TestAppContext) {
    let w = start(cx, "agents-busy");
    let id = new_session(&w, cx);
    let k = key(&w, &id, cx);
    cx.update(|cx| w.sidebar.update(cx, |s, cx| s.show(Activity::Apps, cx)));
    cx.run_until_parked();
    let say = |bytes: &'static [u8], cx: &mut TestAppContext| {
        let runs = cx.read(|cx| w.runner.read(cx).run(&k).unwrap().runs);
        cx.update(|cx| w.runner.update(cx, |r, cx| r.output(&RunId { key: k.clone(), n: runs }, bytes, cx)));
        cx.run_until_parked();
    };
    // Working: output keeps coming.
    for _ in 0..30 {
        say(b"thinking\r\n", cx);
        assert_eq!(cx.read(|cx| w.agents.read(cx).status(&id, cx)), Status::Working);
        wait(cx, Duration::from_secs(1));
    }
    assert!(running(&w, &id, cx), "working past idle_stop");
    // It went quiet off screen: done, not seen yet.
    wait(cx, Duration::from_secs(2));
    assert_eq!(cx.read(|cx| w.agents.read(cx).status(&id, cx)), Status::Done);
    // It rings: it waits on you, however long.
    say(b"\x07", cx);
    assert_eq!(cx.read(|cx| w.agents.read(cx).status(&id, cx)), Status::NeedsYou);
    let item = cx.read(|cx| w.sidebar.read(cx).items_in(Activity::Agents, cx).remove(0));
    assert_eq!(item.warning.as_deref(), Some("needs you"));
    wait(cx, IDLE * 3);
    assert!(running(&w, &id, cx), "it waits on you");
    // Answered (a key), quiet: it stops.
    cx.update(|cx| w.runner.update(cx, |r, _| r.write(&k, b"y")));
    wait(cx, IDLE + Duration::from_secs(1));
    settle(cx);
    assert!(!running(&w, &id, cx));
}

#[gpui::test]
fn a_session_opens_in_a_pulse_renames_and_goes(cx: &mut TestAppContext) {
    let w = start(cx, "agents-pulse");
    let pane = cx.read(|cx| w.deck.read(cx).layout().main().focused);
    cx.update(|cx| {
        w.deck.update(cx, |d, cx| {
            d.open_agent("claude", Path::new("/tmp/web"), devdeck_app::places::Place::Pane(pane), cx);
        })
    });
    cx.run_until_parked();
    let id = cx.read(|cx| w.agents.read(cx).sessions().last().unwrap().id.clone());
    let view = ViewAddr::Agent { id: id.clone() }.to_ref();
    assert_eq!(cx.read(|cx| w.deck.read(cx).layout().find_view(&view).map(|(p, _)| p)), Some(pane), "a tab of the pulse");
    assert!(running(&w, &id, cx), "shown in the pulse: started");
    assert_eq!(cx.read(|cx| w.deck.read(cx).title(&view, cx)), "Claude Code · web");
    cx.update(|cx| w.agents.update(cx, |a, cx| a.rename(&id, " Fix login ", cx)));
    assert_eq!(cx.read(|cx| w.deck.read(cx).title(&view, cx)), "Fix login");
    let k = key(&w, &id, cx);
    cx.update(|cx| w.agents.update(cx, |a, cx| a.remove(&id, cx)));
    cx.run_until_parked();
    assert!(cx.read(|cx| w.agents.read(cx).sessions().is_empty()));
    wait(cx, Duration::from_secs(5));
    assert!(!cx.read(|cx| w.runner.read(cx).state(&k).is_some_and(|s| s.is_running())), "its process stopped");
}
