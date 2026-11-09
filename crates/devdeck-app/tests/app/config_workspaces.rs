//! M2 step 3: workspaces from config files (R10.1, R6.1, R6.5): apps found in the
//! workspace's folders, tools that follow the machine's environment, rediscovery, the
//! problems view, the welcome screen.

use devdeck_app::environment::{EnvState, environment};
use devdeck_app::shell::{Shell, WorkspaceSpec};
use devdeck_core::config::WorkspaceConfig;
use devdeck_core::environment::Toolbox;
use devdeck_core::layout::ViewRef;
use devdeck_core::view_addr::ViewAddr;
use gpui::{Entity, TestAppContext};
use std::path::{Path, PathBuf};

/// A workspace folder holding one pnpm project, `web`, with a `dev` script.
fn projects(cx: &mut TestAppContext) -> PathBuf {
    let root = cx.update(|cx| devdeck_app::test_dir("projects", cx));
    std::fs::create_dir_all(root.join("web")).unwrap();
    std::fs::write(root.join("web/package.json"), r#"{"scripts": {"dev": "vite", "build": "vite build"}}"#).unwrap();
    std::fs::write(root.join("web/pnpm-lock.yaml"), "").unwrap();
    root
}

fn start(cx: &mut TestAppContext, folder: &Path) -> Entity<Shell> {
    let config = cx.update(|cx| devdeck_app::test_dir("config", cx));
    cx.update(|cx| devdeck_app::init_with_config_dir(&config, cx));
    let ws = WorkspaceConfig { id: "work".into(), name: "work".into(), folders: vec![folder.to_path_buf()], ..Default::default() };
    let shell = cx.update(|cx| Shell::start(vec![WorkspaceSpec::from_config(ws)], "work", None, cx));
    cx.update(|cx| shell.update(cx, |s, _| s.quit_for_real = false));
    cx.run_until_parked();
    shell
}

fn services(shell: &Entity<Shell>, cx: &TestAppContext) -> Entity<devdeck_app::services::Services> {
    cx.read(|cx| shell.read(cx).workspaces[0].services.clone())
}

#[gpui::test]
fn a_config_workspace_lists_the_apps_of_its_folders(cx: &mut TestAppContext) {
    let root = projects(cx);
    let shell = start(cx, &root);
    let s = services(&shell, cx);
    let apps: Vec<String> = cx.read(|cx| s.read(cx).catalog.apps.iter().map(|a| a.id.clone()).collect());
    assert_eq!(apps, ["web"]);
    let cmds: Vec<String> = cx.read(|cx| s.read(cx).catalog.app("web").unwrap().commands.iter().map(|c| c.id.clone()).collect());
    assert_eq!(cmds, ["pnpm:build", "pnpm:dev", "pnpm:install"]);
    assert!(cx.read(|cx| s.read(cx).cluster.is_none()), "no [cluster], no cluster");
}

#[gpui::test]
fn tools_are_checked_once_the_environment_is_known(cx: &mut TestAppContext) {
    let root = projects(cx);
    let shell = start(cx, &root);
    let s = services(&shell, cx);
    let refusal = |cx: &TestAppContext| {
        cx.read(|cx| {
            let s = s.read(cx);
            s.runner.read(cx).refusal(&devdeck_app::runner::RunKey::new("web", "pnpm:dev"))
        })
    };
    assert_eq!(refusal(cx).as_deref(), Some("checking tools…"));
    // The machine has no pnpm on its PATH.
    let env = cx.update(environment);
    cx.update(|cx| {
        env.update(cx, |e, cx| {
            e.state = EnvState::Ready(Toolbox { vars: vec![("PATH".into(), "/nowhere".into())], ..Toolbox::default() });
            cx.notify();
        })
    });
    cx.run_until_parked();
    assert_eq!(refusal(cx).as_deref(), Some("pnpm not found"));
    let unusable = cx.read(|cx| s.read(cx).unusable.clone());
    assert_eq!(unusable.get("web").map(String::as_str), Some("pnpm not found"), "checked once per catalog, for the list");
}

#[gpui::test]
fn a_rescan_finds_a_new_project(cx: &mut TestAppContext) {
    let root = projects(cx);
    let shell = start(cx, &root);
    let s = services(&shell, cx);
    std::fs::create_dir_all(root.join("api")).unwrap();
    std::fs::write(root.join("api/Makefile"), "run:\n\techo run\n").unwrap();
    cx.update(|cx| s.update(cx, |s, cx| s.rescan(None, cx)));
    cx.run_until_parked();
    let apps: Vec<String> = cx.read(|cx| s.read(cx).catalog.apps.iter().map(|a| a.id.clone()).collect());
    assert_eq!(apps, ["api", "web"]);
}

#[gpui::test]
fn problems_are_one_toast_and_a_view(cx: &mut TestAppContext) {
    let root = projects(cx);
    std::fs::create_dir_all(root.join("broken")).unwrap();
    std::fs::write(root.join("broken/package.json"), "{ not json").unwrap();
    let shell = start(cx, &root);
    let s = services(&shell, cx);
    assert_eq!(cx.read(|cx| s.read(cx).problems.len()), 1);
    let toasts: Vec<String> = cx.update(|cx| devdeck_ui::toasts(cx).read(cx).queue.items.iter().map(|t| t.text.to_string()).collect());
    assert_eq!(toasts, ["1 problem reading the config or the projects"]);
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    cx.update(|cx| devdeck_app::views::open_in(&deck.downgrade(), ViewAddr::Problems, cx));
    cx.run_until_parked();
    assert!(cx.read(|cx| deck.read(cx).view(&ViewRef::new("problems")).is_some()));
}
