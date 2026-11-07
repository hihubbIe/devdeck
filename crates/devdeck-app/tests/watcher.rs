//! M2 step 3: rediscovery (R6.1, R10.1). Real file events take real time: the tests
//! poll, advancing the test clock past the watcher's pause.

use devdeck_app::shell::{Shell, WorkspaceSpec};
use devdeck_app::watcher::{self, PAUSE};
use devdeck_app::services::Rescanned;
use devdeck_core::config::{MACHINE_FILE, WORKSPACES_DIR, load_workspaces};
use std::cell::Cell;
use std::rc::Rc;
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

fn rescans(s: &Setup, cx: &mut TestAppContext) -> (Rc<Cell<usize>>, gpui::Subscription) {
    let n = Rc::new(Cell::new(0));
    let services = cx.read(|cx| s.shell.read(cx).workspaces[0].services.clone());
    let count = n.clone();
    let sub = cx.update(|cx| cx.subscribe(&services, move |_, _: &Rescanned, _| count.set(count.get() + 1)));
    (n, sub)
}

/// Lets real time and the test clock pass, for events that should change nothing.
fn settle(cx: &mut TestAppContext) {
    for _ in 0..15 {
        std::thread::sleep(Duration::from_millis(20));
        cx.executor().advance_clock(PAUSE);
        cx.run_until_parked();
    }
}

#[gpui::test]
fn files_written_next_to_manifests_dont_rescan(cx: &mut TestAppContext) {
    let s = setup(cx);
    let (n, _sub) = rescans(&s, cx);
    for name in ["debug.log", "tsconfig.tsbuildinfo", ".env"] {
        write(&s.projects.join("web").join(name), "x");
    }
    settle(cx);
    assert_eq!(n.get(), 0, "nothing discovery reads changed");
    write(&s.projects.join("web/Makefile"), "dev:\n\techo dev\nlint:\n\techo lint\n");
    until(cx, "a rescan", |_| n.get() > 0);
    assert_eq!(apps(&s, cx), ["web 2"]);
}

fn config_problems(cx: &TestAppContext) -> Vec<String> {
    cx.update(|cx| devdeck_app::environment::environment(cx).read(cx).config_problems.clone())
}

fn toast_texts(cx: &TestAppContext) -> Vec<String> {
    cx.update(|cx| devdeck_ui::toasts(cx).read(cx).queue.items.iter().map(|t| t.text.to_string()).collect())
}

#[gpui::test]
fn config_problems_follow_the_files(cx: &mut TestAppContext) {
    let s = setup(cx);
    let file = s.config.join(WORKSPACES_DIR).join("work.toml");
    let good = std::fs::read_to_string(&file).unwrap();
    std::fs::write(&file, "folders = [").unwrap();
    until(cx, "the problem", |cx| !config_problems(cx).is_empty());
    assert!(config_problems(cx)[0].contains("work.toml"), "{:?}", config_problems(cx));
    assert!(!toast_texts(cx).iter().any(|t| t.starts_with("Restart devdeck")), "the same files: no restart needed");
    std::fs::write(&file, good).unwrap();
    until(cx, "cleared", |cx| config_problems(cx).is_empty());
}

#[gpui::test]
fn a_broken_machine_file_keeps_the_settings(cx: &mut TestAppContext) {
    let s = setup(cx);
    std::fs::write(s.config.join(MACHINE_FILE), "node_dirs = 3").unwrap();
    until(cx, "the toast", |cx| toast_texts(cx).iter().any(|t| t.ends_with("keeping the previous machine settings")));
    assert!(config_problems(cx).iter().any(|p| p.contains(MACHINE_FILE)), "listed too");
    let loading = cx.update(|cx| matches!(devdeck_app::environment::environment(cx).read(cx).state, devdeck_app::environment::EnvState::Loading));
    assert!(loading, "no load with a broken file");
}
