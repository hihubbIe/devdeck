//! Tab labels (user, 2026-11-09): an icon saying what kind of view, a short name saying
//! which one, like a browser's tabs; a run's icon follows its state.

use devdeck_app::runner::RunKey;
use devdeck_app::shell::Shell;
use devdeck_app::views::{self, TabLabel, Tint};
use devdeck_core::view_addr::ViewAddr;
use devdeck_ui::Icon;
use gpui::{Entity, TestAppContext};

fn start(cx: &mut TestAppContext) -> Entity<Shell> {
    let config = cx.update(|cx| devdeck_app::test_dir("labels-config", cx));
    cx.update(|cx| devdeck_app::init_with_config_dir(&config, cx));
    let shell = cx.update(|cx| Shell::start(devdeck_app::demo_workspaces(), "work", None, cx));
    cx.update(|cx| shell.update(cx, |s, _| s.quit_for_real = false));
    cx.run_until_parked();
    shell
}

fn label(shell: &Entity<Shell>, addr: ViewAddr, cx: &TestAppContext) -> TabLabel {
    cx.read(|cx| {
        let services = shell.read(cx).workspaces.iter().find(|w| w.id == "work").unwrap().services.clone();
        views::tab_label(&addr.to_ref(), Some(services.read(cx)), cx)
    })
}

fn short(l: &TabLabel) -> (Icon, &str, Option<&str>) {
    (l.icon, l.name.as_str(), l.detail.as_deref())
}

#[gpui::test]
fn each_view_has_an_icon_and_a_short_name(cx: &mut TestAppContext) {
    let shell = start(cx);
    let l = |a, cx: &TestAppContext| label(&shell, a, cx);
    assert_eq!(short(&l(ViewAddr::Terminal { n: 2, dir: "/home/u/repos/mf-entities-graph".into() }, cx)), (Icon::Output, "mf-entities-graph", None), "no \"terminal ·\"");
    assert_eq!(short(&l(ViewAddr::Run { app: "eventservice".into(), cmd: "go:run".into() }, cx)), (Icon::Run, "eventservice", Some("run")));
    assert_eq!(short(&l(ViewAddr::App { app: "front-end/controlboardapp".into() }, cx)).0, Icon::Apps);
    assert_eq!(short(&l(ViewAddr::Logs { env: "dev35".into(), name: "entityservice".into() }, cx)), (Icon::Logs, "entityservice", Some("dev35")));
    let svc = l(ViewAddr::Svc { env: "hades".into(), name: "entityservice".into() }, cx);
    assert_eq!((svc.icon, svc.protected), (Icon::Cluster, true), "a protected env says so");
    assert!(!l(ViewAddr::Svc { env: "dev35".into(), name: "x".into() }, cx).protected);
    assert_eq!(short(&l(ViewAddr::Git { root: "/home/u/repos/entityservice".into() }, cx)), (Icon::Git, "entityservice", None));
    assert_eq!(short(&l(ViewAddr::File { path: "/r/src/main.rs".into() }, cx)), (Icon::FileCode, "main.rs", None));
    assert_eq!(short(&l(ViewAddr::File { path: "/r/README.md".into() }, cx)), (Icon::File, "README.md", None));
    assert_eq!(short(&l(ViewAddr::Port { port: "3000".into() }, cx)), (Icon::Ports, ":3000", None));
    assert_eq!(short(&l(ViewAddr::Matrix, cx)), (Icon::Matrix, "matrix", None));
    // A note by its own name, not "app · name".
    let notes = cx.read(|cx| shell.read(cx).workspaces.iter().find(|w| w.id == "work").unwrap().services.read(cx).notes.clone());
    let id = cx.update(|cx| notes.update(cx, |n, cx| n.create("eventservice", "todo", cx))).unwrap();
    assert_eq!(short(&l(ViewAddr::Note { id }, cx)), (Icon::Note, "todo", None));
}

#[gpui::test]
fn a_runs_icon_follows_its_state(cx: &mut TestAppContext) {
    let shell = start(cx);
    let run = ViewAddr::Run { app: "eventservice".into(), cmd: "go:run".into() };
    assert_eq!(label(&shell, run.clone(), cx).tint, Tint::Plain, "never started");
    let runner = cx.read(|cx| shell.read(cx).workspaces.iter().find(|w| w.id == "work").unwrap().services.read(cx).runner.clone());
    let key = RunKey::new("eventservice", "go:run");
    cx.update(|cx| runner.update(cx, |r, cx| r.try_start(&key, None, cx))).unwrap();
    cx.run_until_parked();
    assert_eq!(label(&shell, run.clone(), cx).tint, Tint::Running);
    cx.update(|cx| runner.update(cx, |r, cx| r.stop(&key, cx)));
    assert_eq!(label(&shell, run.clone(), cx).tint, Tint::Stopping);
    // The demo's boardservice server crashes a few seconds after it starts.
    let crash = RunKey::new("boardservice", "go:run");
    cx.update(|cx| runner.update(cx, |r, cx| r.try_start(&crash, None, cx))).unwrap();
    cx.executor().advance_clock(std::time::Duration::from_secs(10));
    cx.run_until_parked();
    assert_eq!(label(&shell, ViewAddr::Run { app: "boardservice".into(), cmd: "go:run".into() }, cx).tint, Tint::Failed);
}
