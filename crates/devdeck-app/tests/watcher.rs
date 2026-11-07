//! M2 step 3: rediscovery (R6.1, R10.1). Real file events take real time: the tests
//! poll, advancing the test clock past the watcher's pause.

use devdeck_app::shell::{Shell, WorkspaceSpec};
use devdeck_app::watcher::{self, PAUSE};
use devdeck_core::config::{WORKSPACES_DIR, load_workspaces};
use gpui::{Entity, TestAppContext};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

struct Setup {
    shell: Entity<Shell>,
    projects: PathBuf,
    config: PathBuf,
    _watcher: watcher::Watcher,
}

/// A config workspace `work` over a folder holding the project `web`, watched.
fn setup(cx: &mut TestAppContext) -> Setup {
    cx.executor().allow_parking();
    let projects = cx.update(|cx| devdeck_app::test_dir("projects", cx));
    write(&projects.join("web/Makefile"), "dev:\n\techo dev\n");
    let config = cx.update(|cx| devdeck_app::test_dir("config", cx));
    write(&config.join(WORKSPACES_DIR).join("work.toml"), &format!("folders = [{:?}]\n", projects.display().to_string()));
    cx.update(|cx| devdeck_app::init_with_config_dir(&config, cx));
    let (configs, errors) = load_workspaces(&config);
    assert!(errors.is_empty(), "{errors:?}");
    let shell = cx.update(|cx| Shell::start(configs.into_iter().map(WorkspaceSpec::from_config).collect(), "work", None, cx));
    cx.update(|cx| shell.update(cx, |s, _| s.quit_for_real = false));
    let w = cx.update(|cx| watcher::start(&shell, config.clone(), cx)).expect("watching");
    cx.run_until_parked();
    Setup { shell, projects, config, _watcher: w }
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn apps(s: &Setup, cx: &TestAppContext) -> Vec<String> {
    cx.read(|cx| s.shell.read(cx).workspaces[0].services.read(cx).catalog.apps.iter().map(|a| format!("{} {}", a.name, a.commands.len())).collect())
}

/// Polls until `f` holds (real time for the file events, the test clock for the pause).
fn until(cx: &mut TestAppContext, what: &str, f: impl Fn(&TestAppContext) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !f(cx) {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
        cx.executor().advance_clock(PAUSE);
        cx.run_until_parked();
    }
}

#[gpui::test]
fn a_new_project_appears(cx: &mut TestAppContext) {
    let s = setup(cx);
    assert_eq!(apps(&s, cx), ["web 1"]);
    write(&s.projects.join("api/Makefile"), "run:\n\techo run\n");
    until(cx, "api", |cx| apps(&s, cx) == ["api 1", "web 1"]);
}

#[gpui::test]
fn a_changed_manifest_and_a_removed_project_are_seen(cx: &mut TestAppContext) {
    let s = setup(cx);
    write(&s.projects.join("web/Makefile"), "dev:\n\techo dev\ntest:\n\techo test\n");
    until(cx, "a second target", |cx| apps(&s, cx) == ["web 2"]);
    std::fs::remove_dir_all(s.projects.join("web")).unwrap();
    until(cx, "web gone", |cx| apps(&s, cx).is_empty());
}

#[gpui::test]
fn a_config_edit_applies(cx: &mut TestAppContext) {
    let s = setup(cx);
    let file = s.config.join(WORKSPACES_DIR).join("work.toml");
    let text = std::fs::read_to_string(&file).unwrap();
    std::fs::write(&file, format!("{text}\n[apps.web]\nname = \"website\"\n")).unwrap();
    until(cx, "the new name", |cx| apps(&s, cx) == ["website 1"]);
}
