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
        fields: [
            ("model", "/model/display_name"),
            ("context", "/context_window/used_percentage"),
            ("cost", "/cost/total_cost_usd"),
            ("added", "/cost/total_lines_added"),
            ("removed", "/cost/total_lines_removed"),
            ("message", "/last_assistant_message"),
            ("prompt", "/prompt"),
            ("asks", "/tool_name"),
            ("title", "/session_name"),
            ("session", "/session_id"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect(),
        limits: vec![devdeck_core::agents::LimitSpec { name: "5 hours".into(), used: "/rate_limits/five_hour/used_percentage".into(), resets: Some("/rate_limits/five_hour/resets_at".into()) }],
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
        assert_eq!(rec.label("Claude Code"), format!("Claude Code #{}", rec.n));
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
fn quitting_asks_while_a_session_works_or_waits_on_you(cx: &mut TestAppContext) {
    let w = start(cx, "agents-quit");
    let id = new_session(&w, cx);
    let k = key(&w, &id, cx);
    wait(cx, Duration::from_secs(3));
    let shell = cx.read(devdeck_app::shell::shell);
    let listed = |cx: &mut TestAppContext| cx.read(|cx| shell.read(cx).running(cx).into_iter().flat_map(|(_, r)| r).filter(|t| t.starts_with("Claude Code")).collect::<Vec<_>>());
    assert!(listed(cx).is_empty(), "ready: nothing to lose");
    let runs = cx.read(|cx| w.runner.read(cx).run(&k).unwrap().runs);
    cx.update(|cx| w.runner.update(cx, |r, cx| r.output(&RunId { key: k.clone(), n: runs }, b"thinking", cx)));
    cx.run_until_parked();
    let n = cx.read(|cx| w.agents.read(cx).session(&id).unwrap().n);
    assert_eq!(listed(cx), [format!("Claude Code #{n} · working")]);
    wait(cx, Duration::from_secs(3));
    cx.update(|cx| w.runner.update(cx, |r, cx| r.output(&RunId { key: k.clone(), n: runs }, b"\x07", cx)));
    cx.run_until_parked();
    wait(cx, Duration::from_secs(3));
    assert_eq!(listed(cx), [format!("Claude Code #{n} · needs you")]);
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
    let n = cx.read(|cx| w.agents.read(cx).session(&id).unwrap().n);
    assert_eq!(cx.read(|cx| w.deck.read(cx).title(&view, cx)), format!("Claude Code #{n}"));
    cx.update(|cx| w.agents.update(cx, |a, cx| a.rename(&id, " Fix login ", cx)));
    assert_eq!(cx.read(|cx| w.deck.read(cx).title(&view, cx)), "Fix login");
    let k = key(&w, &id, cx);
    cx.update(|cx| w.agents.update(cx, |a, cx| a.remove(&id, cx)));
    cx.run_until_parked();
    assert!(cx.read(|cx| w.agents.read(cx).sessions().is_empty()));
    wait(cx, Duration::from_secs(5));
    assert!(!cx.read(|cx| w.runner.read(cx).state(&k).is_some_and(|s| s.is_running())), "its process stopped");
}

/// A report from inside session `id`, as its hooks and status line make one through the
/// API (`devdeck-ctl report <event>` from its terminal).
fn report(w: &World, id: &str, event: &str, data: serde_json::Value, cx: &mut TestAppContext) -> Result<serde_json::Value, devdeck_api::ApiError> {
    use devdeck_app::api::Who;
    use devdeck_app::api::dispatch::{self, CallCx};
    let n = key(w, id, cx).terminal_number().unwrap();
    let who = Who::Terminal { workspace: "work".into(), n, uid: "uid".into(), shell: 1, shell_start: 1, starts: 1 };
    let r = cx.update(|cx| dispatch::call(&CallCx { who: &who, instance: "1-2", state: "demo", conn: None }, "agent.report", serde_json::json!({ "event": event, "data": data }), cx));
    cx.run_until_parked();
    r
}

fn status(w: &World, id: &str, cx: &TestAppContext) -> Status {
    cx.read(|cx| w.agents.read(cx).status(id, cx))
}

fn last_toast(cx: &mut TestAppContext) -> String {
    cx.update(|cx| devdeck_ui::overlay::toasts(cx).read(cx).queue.items.last().map(|t| t.text.to_string()).unwrap_or_default())
}

#[gpui::test]
fn what_the_agent_reports_says_its_state_and_its_facts(cx: &mut TestAppContext) {
    let w = start(cx, "agents-report");
    let id = new_session(&w, cx);
    // Its status line: facts, limits, and a line back for it to show.
    let line = report(&w, &id, "status", serde_json::json!({
        "model": { "display_name": "Opus" },
        "context_window": { "used_percentage": 37 },
        "cost": { "total_cost_usd": 0.42, "total_lines_added": 120, "total_lines_removed": 8 },
        "session_name": "Fix login",
        "rate_limits": { "five_hour": { "used_percentage": 64, "resets_at": 4102444800u64 } }
    }), cx)
    .unwrap();
    assert_eq!(line["line"], "Opus · context 37% · $0.42");
    cx.read(|cx| {
        let a = w.agents.read(cx);
        let rec = a.session(&id).unwrap();
        assert_eq!((rec.facts.context, rec.facts.added, rec.facts.removed), (Some(37.0), Some(120), Some(8)));
        assert_eq!(a.label(&id, cx), "Fix login");
        assert_eq!(a.limits().get("claude").map(|l| l[0].used), Some(64.0));
    });
    // Off screen, its turn reported as working: never stopped, however quiet.
    cx.update(|cx| w.sidebar.update(cx, |s, cx| s.show(Activity::Apps, cx)));
    cx.run_until_parked();
    report(&w, &id, "working", serde_json::json!({ "prompt": "fix the login" }), cx).unwrap();
    assert_eq!(status(&w, &id, cx), Status::Working);
    wait(cx, IDLE * 4);
    settle(cx);
    assert!(running(&w, &id, cx), "working: kept");
    assert_eq!(status(&w, &id, cx), Status::Working);
    assert!(cx.read(|cx| w.agents.read(cx).history(&id).total()) > 60.0, "its minutes of work are counted");
    // It asks: the user hears it, the icon counts it.
    report(&w, &id, "needs_you", serde_json::json!({ "tool_name": "Bash" }), cx).unwrap();
    assert_eq!(status(&w, &id, cx), Status::NeedsYou);
    assert_eq!(last_toast(cx), "Fix login needs you: Bash");
    assert_eq!(cx.read(|cx| w.sidebar.read(cx).agents_needing(cx)), 1);
    // Its turn ends off screen: done, and said so.
    report(&w, &id, "your_turn", serde_json::json!({ "last_assistant_message": "The login works again." }), cx).unwrap();
    assert_eq!(status(&w, &id, cx), Status::Done);
    assert_eq!(last_toast(cx), "Fix login is done");
    assert_eq!(cx.read(|cx| w.agents.read(cx).session(&id).unwrap().facts.message.clone()).as_deref(), Some("The login works again."));
    assert_eq!(cx.read(|cx| w.agents.read(cx).session(&id).unwrap().facts.prompts), 1);
    // Then quiet past idle_stop: it stops.
    wait(cx, IDLE + Duration::from_secs(1));
    settle(cx);
    assert!(!running(&w, &id, cx));
}

#[gpui::test]
fn a_report_from_elsewhere_is_refused(cx: &mut TestAppContext) {
    use devdeck_app::api::Who;
    use devdeck_app::api::dispatch::{self, CallCx};
    let w = start(cx, "agents-report-refused");
    let id = new_session(&w, cx);
    assert!(report(&w, &id, "bogus", serde_json::json!({}), cx).is_err(), "an unknown event");
    let shell = Who::Terminal { workspace: "work".into(), n: 999, uid: "u".into(), shell: 1, shell_start: 1, starts: 1 };
    let outside = Who::Outside { pid: Some(1), program: "x".into() };
    for who in [shell, outside] {
        let r = cx.update(|cx| dispatch::call(&CallCx { who: &who, instance: "1-2", state: "demo", conn: None }, "agent.report", serde_json::json!({ "event": "working" }), cx));
        assert!(r.is_err(), "{who:?}");
    }
}

#[gpui::test]
fn after_clear_or_resume_inside_it_resumes_the_conversation_it_moved_to(cx: &mut TestAppContext) {
    let w = start(cx, "agents-moved");
    let id = new_session(&w, cx);
    report(&w, &id, "started", serde_json::json!({ "session_id": "other-conversation" }), cx).unwrap();
    let k = key(&w, &id, cx);
    assert_eq!(cx.read(|cx| w.runner.read(cx).terminal_line(&k).map(str::to_string)), Some("exec claude --resume other-conversation".into()));
    assert_eq!(cx.read(|cx| w.agents.read(cx).session(&id).unwrap().conversation().to_string()), "other-conversation");
    // What isn't a plain id is ignored: it would be an option on the next start.
    report(&w, &id, "started", serde_json::json!({ "session_id": "--dangerously-skip-permissions" }), cx).unwrap();
    assert_eq!(cx.read(|cx| w.runner.read(cx).terminal_line(&k).map(str::to_string)), Some("exec claude --resume other-conversation".into()));
}

#[gpui::test]
fn with_nothing_selected_agents_shows_the_overview(cx: &mut TestAppContext) {
    let w = start(cx, "agents-overview");
    let id = new_session(&w, cx);
    report(&w, &id, "status", serde_json::json!({ "context_window": { "used_percentage": 50 }, "rate_limits": { "five_hour": { "used_percentage": 90 } } }), cx).unwrap();
    cx.update(|cx| w.sidebar.update(cx, |s, cx| s.show_overview(cx)));
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| w.deck.read(cx).outside().and_then(|o| o.last().cloned())), Some(ViewAddr::Agents.to_ref()));
    assert!(cx.read(|cx| !w.agents.read(cx).shown().contains(&id)), "the session is off screen now");
    let shell = cx.read(devdeck_app::shell::shell);
    let main = cx.read(|cx| shell.read(cx).main_window().unwrap());
    let mut vcx = gpui::VisualTestContext::from_window(main.into(), cx);
    vcx.run_until_parked();
    for part in ["agents-view", "agents-now", "agents-usage", "agents-sessions", "agents-all"] {
        assert!(vcx.debug_bounds(part).is_some(), "{part} drawn");
    }
    // A row goes to its session.
    cx.update(|cx| devdeck_app::agents::show_session("work", &id, cx));
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| w.sidebar.read(cx).selected.clone()), Some(id.clone()));
}

#[gpui::test]
fn the_agents_icon_counts_sessions_waiting_on_you(cx: &mut TestAppContext) {
    let w = start(cx, "agents-badge");
    let id = new_session(&w, cx);
    let shell = cx.read(devdeck_app::shell::shell);
    let main = cx.read(|cx| shell.read(cx).main_window().unwrap());
    let mut vcx = gpui::VisualTestContext::from_window(main.into(), cx);
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("activity-agents-badge").is_none());
    let icon = vcx.debug_bounds("activity-icon-agents").unwrap();
    report(&w, &id, "needs_you", serde_json::json!({}), cx).unwrap();
    let mut vcx = gpui::VisualTestContext::from_window(main.into(), cx);
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("activity-agents-badge").is_some(), "counted");
    assert_eq!(vcx.debug_bounds("activity-icon-agents"), Some(icon), "the icon doesn't move (R9.10)");
}

#[gpui::test]
fn the_new_session_button_picks_a_project_then_makes_one_and_shows_it(cx: &mut TestAppContext) {
    // Regression: the button made the session from inside the sidebar's own update, then
    // updated the sidebar again to select it (a panic). It picks the project first now.
    let w = start(cx, "agents-button");
    cx.update(|cx| w.sidebar.update(cx, |s, cx| s.show(Activity::Agents, cx)));
    cx.run_until_parked();
    let shell = cx.read(devdeck_app::shell::shell);
    let main = cx.read(|cx| shell.read(cx).main_window().unwrap());
    let mut vcx = gpui::VisualTestContext::from_window(main.into(), cx);
    vcx.run_until_parked();
    let button = vcx.debug_bounds("sidebar-new-session").expect("the button");
    vcx.simulate_click(button.center(), gpui::Modifiers::none());
    vcx.run_until_parked();
    assert!(cx.read(|cx| w.agents.read(cx).sessions().is_empty()), "a picker first");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    let s = cx.read(|cx| w.agents.read(cx).sessions().last().cloned()).expect("a session");
    let project = s.project.clone().expect("in the picked project");
    let app_dir = cx.read(|cx| w.deck.read(cx).services().unwrap().read(cx).catalog.app(&project).map(|a| a.dir.clone()));
    assert_eq!(app_dir.as_deref(), Some(s.dir.as_path()), "in its folder");
    let id = s.id;
    assert_eq!(cx.read(|cx| w.sidebar.read(cx).selected.clone()), Some(id.clone()), "selected");
    assert!(running(&w, &id, cx), "shown: started");
}

#[gpui::test]
fn a_sessions_one_header_says_its_name_state_and_facts(cx: &mut TestAppContext) {
    let w = start(cx, "agents-header");
    let id = new_session(&w, cx);
    report(&w, &id, "status", serde_json::json!({ "session_name": "Fix login", "model": { "display_name": "Opus" }, "context_window": { "used_percentage": 42 } }), cx).unwrap();
    report(&w, &id, "working", serde_json::json!({}), cx).unwrap();
    let view = ViewAddr::Agent { id: id.clone() }.to_ref();
    cx.read(|cx| {
        let d = w.deck.read(cx);
        assert_eq!(d.title(&view, cx), "Fix login", "its name, not its terminal's");
        let status = d.view_status(&view, cx).expect("a state");
        assert_eq!(status.text, "working · Opus · context 42%");
        assert_eq!(status.tone, devdeck_app::views::Tone::Ok);
        let actions: Vec<&str> = d.view_actions(&view, cx).iter().map(|a| a.id).collect();
        assert_eq!(actions, ["run.stop", "pane.follow"], "few: no restart (it resumes by itself), no arguments");
    });
    let shell = cx.read(devdeck_app::shell::shell);
    let main = cx.read(|cx| shell.read(cx).main_window().unwrap());
    let mut vcx = gpui::VisualTestContext::from_window(main.into(), cx);
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("tool-run.stop").is_some(), "in the details header");
    assert!(vcx.debug_bounds("run-state").is_none(), "no row of its own");
}

/// A demo app's folder.
fn app_dir(w: &World, app: &str, cx: &TestAppContext) -> std::path::PathBuf {
    cx.read(|cx| w.deck.read(cx).services().unwrap().read(cx).catalog.app(app).unwrap().dir.clone())
}

#[gpui::test]
fn sessions_are_listed_by_project_and_a_project_starts_one(cx: &mut TestAppContext) {
    let w = start(cx, "agents-projects");
    let (event, board) = (app_dir(&w, "eventservice", cx), app_dir(&w, "boardservice", cx));
    for dir in [&event, &event.join("internal"), &board, &std::path::PathBuf::from("/tmp/elsewhere")] {
        let dir = dir.clone();
        cx.update(|cx| devdeck_app::sidebar::new_agent_session(&w.deck, "claude", &dir, cx));
        cx.run_until_parked();
    }
    let projects: Vec<Option<String>> = cx.read(|cx| w.agents.read(cx).sessions().iter().map(|s| s.project.clone()).collect());
    assert_eq!(projects, [Some("eventservice".into()), Some("eventservice".into()), Some("boardservice".into()), None], "its deepest app, else none");
    let tree = cx.read(|cx| w.sidebar.read(cx).agents_tree(cx));
    let node = devdeck_core::notes::project_node("eventservice");
    assert_eq!(tree.items.iter().filter(|i| i.path.as_deref().is_some_and(|p| p.starts_with(&format!("{node}/")))).count(), 2, "under its project");
    assert!(tree.items.iter().any(|i| i.path.is_none()), "a session in no app's folder at the top");
    assert_eq!(tree.names.get(&node).map(String::as_str), cx.read(|cx| w.deck.read(cx).services().unwrap().read(cx).catalog.app("eventservice").map(|a| a.name.clone())).as_deref());
    // A project row's menu: a session of each agent there.
    cx.update(|cx| w.sidebar.update(cx, |s, cx| s.show(Activity::Agents, cx)));
    let menu = cx.update(|cx| devdeck_app::sidebar::Sidebar::item_menu(&w.sidebar, &format!("{node}/"), false, cx).iter().map(|i| i.label.to_string()).collect::<Vec<_>>());
    assert_eq!(menu, ["Claude Code here"]);
    // A session's row: start here in its folder, after its own items.
    let id = cx.read(|cx| w.agents.read(cx).sessions()[0].id.clone());
    let menu = cx.update(|cx| devdeck_app::sidebar::Sidebar::item_menu(&w.sidebar, &id, false, cx).iter().map(|i| i.label.to_string()).collect::<Vec<_>>());
    assert!(menu.ends_with(&["".into(), "Open terminal here".into(), "".into(), "Claude Code here".into()]), "{menu:?}");
}

#[gpui::test]
fn an_apps_page_lists_its_sessions(cx: &mut TestAppContext) {
    let w = start(cx, "agents-app-page");
    let dir = app_dir(&w, "eventservice", cx);
    cx.update(|cx| devdeck_app::sidebar::new_agent_session(&w.deck, "claude", &dir, cx));
    cx.run_until_parked();
    cx.update(|cx| w.sidebar.update(cx, |s, cx| s.show_item(Activity::Apps, "eventservice", cx)));
    cx.run_until_parked();
    let shell = cx.read(devdeck_app::shell::shell);
    let main = cx.read(|cx| shell.read(cx).main_window().unwrap());
    let mut vcx = gpui::VisualTestContext::from_window(main.into(), cx);
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("app-agents").is_some(), "its Agents panel");
    let row = vcx.debug_bounds("app-agent-0").expect("its session");
    assert!(vcx.debug_bounds("app-new-session").is_some(), "and a new one from there");
    vcx.simulate_click(row.center(), gpui::Modifiers::none());
    vcx.run_until_parked();
    let id = cx.read(|cx| w.agents.read(cx).sessions()[0].id.clone());
    assert_eq!(cx.read(|cx| (w.sidebar.read(cx).activity(), w.sidebar.read(cx).selected.clone())), (Activity::Agents, Some(id)), "a row goes to it");
    // The details header has the session's menu.
    let header = vcx.debug_bounds("details").expect("the details");
    vcx.simulate_mouse_down(gpui::point(header.center().x, header.top() + devdeck_ui::logical(10.)), gpui::MouseButton::Right, gpui::Modifiers::none());
    vcx.run_until_parked();
    assert!(vcx.update(|_, cx| devdeck_ui::overlay::layer_state(main.into(), cx).0), "its row's menu, from its header");
}
