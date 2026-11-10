//! The control API's reads (R13.9), on the demo (fake runs write scripted lines, then
//! one every `SERVER_TICK`): output as lines with a place to go on from, search, the
//! screen, notes, and files in the workspace's folders only.

use devdeck_api::ErrorKind;
use devdeck_api::types::{FileLines, Matches, NoteText, Output, Screen};
use devdeck_app::api::Who;
use devdeck_app::api::dispatch::{self, CallCx};
use devdeck_app::fake_driver::SERVER_TICK;
use devdeck_app::runner::{RunKey, Runner};
use devdeck_app::shell::Shell;
use devdeck_core::terminal::TermSize;
use gpui::{Entity, TestAppContext};
use serde_json::{Value, json};
use std::time::Duration;

const APP: &str = "eventservice";
const CMD: &str = "go:run";

fn start(cx: &mut TestAppContext, tag: &str) -> Entity<Shell> {
    let config = cx.update(|cx| devdeck_app::test_dir(&format!("{tag}-config"), cx));
    cx.update(|cx| devdeck_app::init_with_config_dir(&config, cx));
    let shell = cx.update(|cx| Shell::start(devdeck_app::demo_workspaces(), "work", None, cx));
    cx.update(|cx| shell.update(cx, |s, _| s.quit_for_real = false));
    cx.run_until_parked();
    shell
}

fn agent() -> Who {
    Who::Terminal { workspace: "work".into(), n: 3, uid: "uid-3".into(), shell: 1, shell_start: 1, starts: 1 }
}

fn call<T: serde::de::DeserializeOwned>(method: &str, params: Value, cx: &mut TestAppContext) -> Result<T, devdeck_api::ApiError> {
    let who = agent();
    let answer = cx.update(|cx| dispatch::answer(&CallCx { who: &who, instance: "i", state: "demo", conn: None }, method, params, cx));
    let v = match answer {
        dispatch::Answer::Now(r) => r?,
        // A file is read in the background.
        dispatch::Answer::Later(task) => cx.foreground_executor().block_test(task)?,
    };
    Ok(serde_json::from_value(v).unwrap())
}

fn runner(shell: &Entity<Shell>, cx: &TestAppContext) -> Entity<Runner> {
    cx.read(|cx| shell.read(cx).workspaces.iter().find(|w| w.id == "work").unwrap().services.read(cx).runner.clone())
}

fn wait(cx: &mut TestAppContext, d: Duration) {
    cx.executor().advance_clock(d);
    cx.run_until_parked();
}

#[gpui::test]
fn output_goes_on_from_where_the_last_read_stopped(cx: &mut TestAppContext) {
    let shell = start(cx, "reads-out");
    let r = runner(&shell, cx);
    let key = RunKey::new(APP, CMD);
    cx.update(|cx| r.update(cx, |r, cx| r.try_start(&key, None, cx))).unwrap();
    wait(cx, Duration::from_secs(5));
    let first: Output = call("run.output", json!({ "app": APP, "cmd": CMD }), cx).unwrap();
    assert!(!first.lines.is_empty());
    assert!(first.lines.iter().all(|l| !l.0.contains('\u{1b}')), "no colour codes");
    let two: Output = call("run.output", json!({ "app": APP, "cmd": CMD, "tail": 2 }), cx).unwrap();
    assert_eq!(two.lines.len(), 2);
    let nothing: Output = call("run.output", json!({ "app": APP, "cmd": CMD, "since": first.next }), cx).unwrap();
    assert!(nothing.lines.is_empty(), "{:?}", nothing.lines);
    wait(cx, SERVER_TICK * 2);
    let new: Output = call("run.output", json!({ "app": APP, "cmd": CMD, "since": first.next }), cx).unwrap();
    assert_eq!(new.lines.len(), 2, "two server lines since: {:?}", new.lines);
    // A resize rewraps, a restart starts over: the place no longer holds.
    cx.update(|cx| r.update(cx, |r, cx| r.resize(&key, TermSize { cols: 50, rows: 10 }, cx)));
    let e = call::<Output>("run.output", json!({ "app": APP, "cmd": CMD, "since": new.next }), cx).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Expired);
    let e = call::<Output>("run.output", json!({ "app": APP, "cmd": CMD, "since": "not-a-cursor" }), cx).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Invalid);
    assert_eq!(call::<Output>("run.output", json!({ "app": APP, "cmd": "go:test" }), cx).unwrap_err().kind, ErrorKind::NotFound, "never ran");
}

#[gpui::test]
fn search_finds_lines_with_context(cx: &mut TestAppContext) {
    let shell = start(cx, "reads-search");
    let r = runner(&shell, cx);
    cx.update(|cx| r.update(cx, |r, cx| r.try_start(&RunKey::new(APP, CMD), None, cx))).unwrap();
    wait(cx, Duration::from_secs(10));
    let all: Output = call("run.output", json!({ "app": APP, "cmd": CMD, "tail": 2000 }), cx).unwrap();
    let word = all.lines.iter().flat_map(|l| l.0.split_whitespace()).find(|w| w.len() > 4 && w.chars().all(char::is_alphabetic)).unwrap().to_string();
    let m: Matches = call("run.search", json!({ "app": APP, "cmd": CMD, "pattern": word, "context": 1, "max": 1 }), cx).unwrap();
    assert_eq!(m.matches.len(), 1);
    assert!(m.matches[0].text.0.contains(&word));
    assert!(m.matches[0].before.len() <= 1 && m.matches[0].after.len() <= 1);
    let e = call::<Matches>("run.search", json!({ "app": APP, "cmd": CMD, "pattern": "(" }), cx).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Invalid);
}

#[gpui::test]
fn its_own_terminal_only(cx: &mut TestAppContext) {
    let shell = start(cx, "reads-term");
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    for n in [3, 5] {
        let tab = devdeck_core::view_addr::ViewAddr::Terminal { n, dir: std::env::temp_dir().display().to_string() }.to_ref();
        cx.update(|cx| deck.update(cx, |d, cx| d.apply(cx, |l| l.open(tab.clone(), l.main().focused).map(|_| ()))));
    }
    wait(cx, Duration::from_secs(2));
    // The agent runs in terminal 3.
    let s: Screen = call("terminal.screen", json!({ "n": 3 }), cx).unwrap();
    assert!(!s.rows.is_empty());
    let _: Output = call("terminal.output", json!({ "n": 3 }), cx).unwrap();
    for m in ["terminal.screen", "terminal.output"] {
        assert_eq!(call::<Value>(m, json!({ "n": 5 }), cx).unwrap_err().kind, ErrorKind::ApprovalRequired, "{m}: the user's shell");
    }
    assert_eq!(call::<Value>("terminal.search", json!({ "n": 5, "pattern": "x" }), cx).unwrap_err().kind, ErrorKind::ApprovalRequired);
    assert_eq!(call::<Screen>("terminal.screen", json!({ "n": 99 }), cx).unwrap_err().kind, ErrorKind::NotFound);
    assert_eq!(call::<Output>("run.output", json!({ "app": "", "cmd": "term/5" }), cx).unwrap_err().kind, ErrorKind::NotFound, "a shell isn't a run");
}

#[gpui::test]
fn notes_are_read_whole(cx: &mut TestAppContext) {
    let shell = start(cx, "reads-note");
    let notes = cx.read(|cx| shell.read(cx).workspaces.iter().find(|w| w.id == "work").unwrap().services.read(cx).notes.clone());
    let id = cx.update(|cx| notes.update(cx, |n, cx| n.create(APP, "todo", cx))).unwrap();
    cx.update(|cx| notes.update(cx, |n, cx| n.set_text(&id, "- [ ] fix the zoom\n".into(), cx)));
    let n: NoteText = call("note.read", json!({ "id": id }), cx).unwrap();
    assert_eq!((n.text.0.as_str(), n.truncated), ("- [ ] fix the zoom\n", false));
    assert_eq!(call::<NoteText>("note.read", json!({ "id": "nope" }), cx).unwrap_err().kind, ErrorKind::NotFound);
}

#[gpui::test]
fn files_only_inside_the_workspaces_folders(cx: &mut TestAppContext) {
    let shell = start(cx, "reads-file");
    let inside = cx.update(|cx| devdeck_app::test_dir("reads-inside", cx));
    let outside = cx.update(|cx| devdeck_app::test_dir("reads-outside", cx));
    std::fs::write(inside.join("a.txt"), (1..=500).map(|i| format!("line {i}\n")).collect::<String>()).unwrap();
    std::fs::write(outside.join("secret.txt"), "no").unwrap();
    std::os::unix::fs::symlink(outside.join("secret.txt"), inside.join("link.txt")).unwrap();
    std::fs::write(inside.join("bin"), [0u8, 159, 146, 150]).unwrap();
    std::fs::write(inside.join("big"), vec![b'x'; 3 * 1024 * 1024]).unwrap();
    std::fs::write(inside.join(".env"), "TOKEN=x").unwrap();
    std::fs::write(inside.join(".env.example"), "TOKEN=").unwrap();
    devdeck_sys::fs::make_fifo(&inside.join("pipe")).unwrap();
    // An app whose folder is `inside`.
    let services = cx.read(|cx| shell.read(cx).workspaces.iter().find(|w| w.id == "work").unwrap().services.clone());
    cx.update(|cx| {
        services.update(cx, |s, cx| {
            let mut catalog = (*s.catalog).clone();
            let mut a = catalog.apps[0].clone();
            a.id = "files".into();
            a.dir = inside.clone();
            catalog.apps.push(a);
            s.set_catalog(catalog, cx);
        })
    });
    let path = |p: &std::path::Path| p.display().to_string();
    let f: FileLines = call("file.read", json!({ "path": path(&inside.join("a.txt")), "from_line": 10, "lines": 3 }), cx).unwrap();
    assert_eq!(f.lines.iter().map(|l| l.0.as_str()).collect::<Vec<_>>(), ["line 10", "line 11", "line 12"]);
    assert_eq!((f.first_line, f.total_lines, f.truncated), (10, 500, true));
    let kind = |p: String, cx: &mut TestAppContext| call::<FileLines>("file.read", json!({ "path": p }), cx).unwrap_err().kind;
    assert_eq!(kind(path(&outside.join("secret.txt")), cx), ErrorKind::PermissionDenied);
    assert_eq!(kind(path(&inside.join("link.txt")), cx), ErrorKind::PermissionDenied, "a link out of the folder");
    assert_eq!(kind(format!("{}/../{}/secret.txt", path(&inside), outside.file_name().unwrap().to_string_lossy()), cx), ErrorKind::PermissionDenied, "..");
    assert_eq!(kind("a.txt".into(), cx), ErrorKind::Invalid, "relative");
    assert_eq!(kind(path(&inside.join("bin")), cx), ErrorKind::Refused, "not text");
    assert_eq!(kind(path(&inside.join("big")), cx), ErrorKind::Refused, "too big");
    assert_eq!(kind(path(&inside.join("none")), cx), ErrorKind::NotFound);
    assert_eq!(kind(path(&inside.join("pipe")), cx), ErrorKind::Refused, "a FIFO doesn't block devdeck");
    assert_eq!(kind(path(&inside.join(".env")), cx), ErrorKind::PermissionDenied, "secrets");
    let ex: FileLines = call("file.read", json!({ "path": path(&inside.join(".env.example")) }), cx).unwrap();
    assert_eq!(ex.lines.len(), 1);
    // What git ignores in the app's repository.
    let git = |args: &[&str]| assert!(std::process::Command::new("git").args(args).current_dir(&inside).env("GIT_CONFIG_GLOBAL", "/dev/null").output().unwrap().status.success());
    git(&["init", "-q"]);
    std::fs::write(inside.join(".gitignore"), "local.json\n").unwrap();
    std::fs::write(inside.join("local.json"), "{\"token\": 1}").unwrap();
    assert_eq!(kind(path(&inside.join("local.json")), cx), ErrorKind::PermissionDenied, "ignored by git");
    let _: FileLines = call("file.read", json!({ "path": path(&inside.join("a.txt")) }), cx).unwrap();
    assert_eq!(kind(path(&inside.join(".git/config")), cx), ErrorKind::PermissionDenied, "git's own files");
}
