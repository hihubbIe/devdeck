//! M1 step 1: settings view and UI scale (R9.4), on GPUI's test platform. The last
//! test uses the real text system to check that text never wraps at a large scale.

use devdeck_app::shell::Shell;
use devdeck_app::{app_settings, deck::Deck};
use devdeck_core::layout::MAIN_WINDOW;
use devdeck_core::settings::SETTINGS_FILE;
use devdeck_ui::logical;
use gpui::{Entity, Modifiers, TestAppContext, TestDispatcher, VisualTestContext, size};
use std::path::PathBuf;

/// A config folder kept as long as the test's app.
fn config_dir(cx: &mut TestAppContext, tag: &str) -> PathBuf {
    cx.update(|cx| devdeck_app::test_dir(tag, cx))
}

fn setup(cx: &mut TestAppContext, tag: &str) -> (Entity<Deck>, PathBuf) {
    let dir = config_dir(cx, tag);
    cx.update(|cx| devdeck_app::init_with_config_dir(&dir, cx));
    let shell = cx.update(|cx| Shell::start_single(devdeck_app::initial_layout, cx));
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    cx.run_until_parked();
    (deck, dir)
}

fn scale(cx: &TestAppContext) -> f32 {
    cx.read(app_settings::scale)
}

fn saved(dir: &std::path::Path) -> String {
    std::fs::read_to_string(dir.join(SETTINGS_FILE)).unwrap_or_default()
}

#[gpui::test]
fn scale_keys_step_save_and_reset(cx: &mut TestAppContext) {
    let (_deck, dir) = setup(cx, "keys");
    let main = cx.windows()[0];
    assert_eq!(scale(cx), 1.0);
    cx.simulate_keystrokes(main, "ctrl-=");
    assert_eq!(scale(cx), 1.1);
    cx.simulate_keystrokes(main, "ctrl-= ctrl-=");
    assert_eq!(scale(cx), 1.5);
    assert!(saved(&dir).contains("ui_scale = 1.5"), "saved: {}", saved(&dir));
    cx.simulate_keystrokes(main, "ctrl-= ");
    assert_eq!(scale(cx), 1.5, "stops at the largest step");
    cx.simulate_keystrokes(main, "ctrl-0");
    assert_eq!(scale(cx), 1.0);
    cx.simulate_keystrokes(main, "ctrl--");
    assert_eq!(scale(cx), 0.9);
    assert_eq!(cx.read(|cx| devdeck_ui::theme(cx).row_h), devdeck_ui::Theme::devdeck_dark(0.9).row_h, "theme follows the scale");
}

#[gpui::test]
fn settings_view_blocks_pane_keys_and_restores_focus(cx: &mut TestAppContext) {
    let (deck, _dir) = setup(cx, "focus");
    let main = cx.windows()[0];
    let panes = |cx: &TestAppContext| cx.read(|cx| deck.read(cx).layout().panes_in(MAIN_WINDOW).unwrap().len());
    cx.simulate_keystrokes(main, "ctrl-,");
    assert!(cx.read(app_settings::is_open));
    cx.simulate_keystrokes(main, "ctrl-\\");
    assert_eq!(panes(cx), 3, "pane keys do nothing behind settings");
    cx.simulate_keystrokes(main, "escape");
    assert!(!cx.read(app_settings::is_open));
    cx.simulate_keystrokes(main, "ctrl-\\");
    assert_eq!(panes(cx), 4, "focus is back in the panes after closing settings");
}

#[gpui::test]
fn settings_buttons_work_with_the_mouse(cx: &mut TestAppContext) {
    let (_deck, _dir) = setup(cx, "mouse");
    let main = cx.windows()[0];
    cx.simulate_keystrokes(main, "ctrl-,");
    let mut vcx = VisualTestContext::from_window(main, cx);
    vcx.run_until_parked();
    let up = vcx.debug_bounds("scale-up").expect("+ button rendered");
    vcx.simulate_click(up.center(), Modifiers::none());
    assert_eq!(vcx.read(app_settings::scale), 1.1);
    // The layout re-scales after each click, so look the button up again every time.
    for expected in [1.0, 0.9] {
        let down = vcx.debug_bounds("scale-down").expect("− button rendered");
        vcx.simulate_click(down.center(), Modifiers::none());
        assert_eq!(vcx.read(app_settings::scale), expected);
    }
}

/// Real text shaping: at 150 % in a narrow window, single-line texts stay one line.
/// Linux only: on macOS GPUI's real platform must start on the main thread, and a test
/// runs on a thread of its own.
#[cfg(target_os = "linux")]
#[test]
fn text_never_wraps_at_large_scale() {
    let text = gpui_platform::current_platform(true).text_system();
    let mut cx = TestAppContext::build_with_text_system(TestDispatcher::new(0), None, text);
    let (_deck, _dir) = setup(&mut cx, "nowrap");
    cx.update(|cx| app_settings::set_scale(1.5, cx));
    let main = cx.windows()[0];
    cx.simulate_window_resize(main, size(logical(700.), logical(500.)));
    let mut vcx = VisualTestContext::from_window(main, &cx);
    vcx.run_until_parked();
    let line = vcx.read(|cx| devdeck_ui::theme(cx).line_height);
    for selector in ["matrix-heading", "tab-title"] {
        let b = vcx.debug_bounds(selector).unwrap_or_else(|| panic!("{selector} rendered"));
        assert!(b.size.height <= line + logical(1.), "{selector} wrapped: height {:?} > line {:?}", b.size.height, line);
    }
}

#[gpui::test]
fn theme_picker_switches_globally_and_saves(cx: &mut TestAppContext) {
    let dir = config_dir(cx, "theme");
    std::fs::create_dir_all(dir.join("themes")).unwrap();
    std::fs::write(dir.join("themes/paper.toml"), "name = \"paper\"\n[colors]\nbg = \"#fafafa\"\nfg = \"#111111\"\n").unwrap();
    cx.update(|cx| devdeck_app::init_with_config_dir(&dir, cx));
    let names: Vec<String> = cx.read(|cx| app_settings::settings(cx).read(cx).themes.iter().map(|t| t.name.clone()).collect());
    assert_eq!(names, ["devdeck-dark", "paper"]);
    cx.update(|cx| app_settings::set_theme("paper", cx));
    assert_eq!(cx.read(|cx| devdeck_ui::theme(cx).name.to_string()), "paper");
    assert!(saved(&dir).contains("theme = \"paper\""));
    cx.update(|cx| app_settings::set_theme("does-not-exist", cx));
    assert_eq!(cx.read(|cx| devdeck_ui::theme(cx).name.to_string()), "paper", "unknown names are ignored");
}

#[gpui::test]
fn unknown_saved_theme_falls_back_with_a_message(cx: &mut TestAppContext) {
    let dir = config_dir(cx, "theme-missing");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(SETTINGS_FILE), "theme = \"gone\"\n").unwrap();
    cx.update(|cx| devdeck_app::init_with_config_dir(&dir, cx));
    assert_eq!(cx.read(|cx| devdeck_ui::theme(cx).name.to_string()), "devdeck-dark");
    let problems = cx.read(|cx| app_settings::settings(cx).read(cx).theme_problems.clone());
    assert!(problems.iter().any(|p| p.contains("gone")), "{problems:?}");
}

#[gpui::test]
fn settings_show_the_control_api(cx: &mut TestAppContext) {
    let (_deck, dir) = setup(cx, "api");
    let config = devdeck_app::api::Config { runtime: devdeck_app::api::no_runtime(), state_dir: dir.clone(), default_state_dir: dir, audit_file: false };
    cx.update(|cx| devdeck_app::api::start(config, cx));
    let main = cx.windows()[0];
    cx.simulate_keystrokes(main, "ctrl-,");
    cx.run_until_parked();
    // Found by the search, shown by Enter.
    cx.simulate_input(main, "socket");
    cx.simulate_keystrokes(main, "enter");
    let mut vcx = VisualTestContext::from_window(main, cx);
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("settings-api").is_some(), "the API page is drawn");
}

/// The short name: typed in Settings, refused with its reason (whitespace, control
/// characters, too long, empty), saved when fine.
#[gpui::test]
fn the_short_name_is_checked_before_it_is_saved(cx: &mut TestAppContext) {
    let (_deck, dir) = setup(cx, "short-name");
    let main = cx.windows()[0];
    assert_eq!(cx.read(app_settings::short_name), None, "no default");
    cx.simulate_keystrokes(main, "ctrl-,");
    cx.run_until_parked();
    cx.simulate_input(main, "short name");
    cx.simulate_keystrokes(main, "enter");
    cx.run_until_parked();
    let mut vcx = VisualTestContext::from_window(main, cx);
    let field = vcx.debug_bounds("short-name").expect("shown in Settings");
    let try_name = |vcx: &mut VisualTestContext, typed: &str| {
        vcx.simulate_click(field.center(), Modifiers::none());
        vcx.simulate_keystrokes("ctrl-a backspace");
        vcx.simulate_input(typed);
        vcx.simulate_keystrokes("enter");
        vcx.run_until_parked();
        vcx.debug_bounds("short-name-error").is_some()
    };
    for bad in ["j doe", "j\u{7}doe", &"a".repeat(33)] {
        assert!(try_name(&mut vcx, bad), "{bad:?} refused, said");
    }
    vcx.simulate_click(field.center(), Modifiers::none());
    vcx.simulate_keystrokes("ctrl-a backspace enter");
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("short-name-error").is_some(), "empty refused");
    assert_eq!(vcx.read(app_settings::short_name), None, "nothing saved");
    assert!(!try_name(&mut vcx, "j.doe"));
    assert_eq!(vcx.read(app_settings::short_name).as_deref(), Some("j.doe"));
    assert!(saved(&dir).contains("short_name = \"j.doe\""), "saved: {}", saved(&dir));
}

// ------------------------------------------------------------------ categories and search

use devdeck_app::settings_view::{Category, SettingsView};

/// Settings, opened with ctrl+,.
fn open(main: gpui::AnyWindowHandle, cx: &mut TestAppContext) -> Entity<SettingsView> {
    cx.simulate_keystrokes(main, "ctrl-,");
    cx.run_until_parked();
    let w = main.downcast::<devdeck_app::window_view::WindowView>().unwrap();
    cx.update(|cx| w.read(cx).unwrap().settings_view()).expect("opened")
}

fn listed(view: &Entity<SettingsView>, cx: &TestAppContext) -> Vec<(&'static str, Option<usize>)> {
    cx.read(|cx| view.read(cx).listed(cx).into_iter().map(|(c, n)| (c.label(), n)).collect())
}

fn panel(main: gpui::AnyWindowHandle, cx: &mut TestAppContext) -> gpui::Bounds<gpui::Pixels> {
    let mut vcx = VisualTestContext::from_window(main, cx);
    vcx.run_until_parked();
    vcx.debug_bounds("modal-panel").expect("the modal")
}

#[gpui::test]
fn the_categories_are_alphabetical_and_it_opens_on_the_last(cx: &mut TestAppContext) {
    let (_deck, _dir) = setup(cx, "categories");
    let main = cx.windows()[0];
    let view = open(main, cx);
    assert_eq!(listed(&view, cx).into_iter().map(|(c, _)| c).collect::<Vec<_>>(), ["Agents", "API", "Appearance", "Deploys", "Files", "Machine", "Notifications"]);
    assert_eq!(cx.read(|cx| view.read(cx).category), Category::Appearance, "Appearance first time");
    let size = panel(main, cx);
    // ↓ from the search: the next category; every page the same size (R9.10).
    for _ in 0..4 {
        cx.simulate_keystrokes(main, "down");
        assert_eq!(panel(main, cx), size);
    }
    assert_eq!(cx.read(|cx| view.read(cx).category), Category::Notifications, "↓ stops at the last");
    cx.simulate_keystrokes(main, "up");
    assert_eq!(cx.read(|cx| view.read(cx).category), Category::Machine);
    cx.simulate_keystrokes(main, "escape");
    assert!(!cx.read(app_settings::is_open));
    open(main, cx);
    assert_eq!(cx.read(|cx| view.read(cx).category), Category::Machine, "opens where it was");
}

#[gpui::test]
fn the_search_narrows_the_list_and_enter_shows_a_match(cx: &mut TestAppContext) {
    let (_deck, _dir) = setup(cx, "search");
    let main = cx.windows()[0];
    let view = open(main, cx);
    let size = panel(main, cx);
    cx.simulate_input(main, "theme");
    assert_eq!(listed(&view, cx), [("Appearance", Some(1))], "the categories with matches, counted");
    assert_eq!(panel(main, cx), size, "a search doesn't resize it");
    // Word starts, not scattered letters: "on" finds nothing in "Connected".
    cx.simulate_keystrokes(main, "ctrl-a backspace");
    cx.simulate_input(main, "zzz");
    assert!(listed(&view, cx).is_empty());
    cx.simulate_keystrokes(main, "down enter");
    assert!(cx.read(app_settings::is_open), "nothing to show: nothing happens");
    cx.simulate_keystrokes(main, "ctrl-a backspace");
    cx.simulate_input(main, "audit");
    cx.simulate_keystrokes(main, "enter");
    assert_eq!(cx.read(|cx| view.read(cx).category), Category::Api);
    assert_eq!(cx.read(|cx| view.read(cx).highlight), Some("api.audit"), "the match tinted");
    assert!(cx.read(|cx| view.read(cx).search.read(cx).text().is_empty()), "the search ended");
    // Esc clears a search, then closes.
    cx.simulate_input(main, "path");
    cx.simulate_keystrokes(main, "escape");
    assert!(cx.read(app_settings::is_open), "the first Esc clears");
    assert!(cx.read(|cx| view.read(cx).search.read(cx).text().is_empty()));
    cx.simulate_keystrokes(main, "escape");
    assert!(!cx.read(app_settings::is_open));
    // ↓ among the matches, then Enter: the second one's category.
    open(main, cx);
    cx.simulate_input(main, "config");
    assert_eq!(listed(&view, cx), [("Files", Some(1)), ("Machine", Some(1))]);
    // The API page has no match: the first is selected; ↓ the second.
    cx.simulate_keystrokes(main, "down enter");
    assert_eq!(cx.read(|cx| view.read(cx).category), Category::Machine);
    assert_eq!(cx.read(|cx| view.read(cx).highlight), Some("machine.shell"));
    // Another category: the tint ends.
    cx.simulate_keystrokes(main, "up");
    assert_eq!(cx.read(|cx| view.read(cx).highlight), None);
}

#[gpui::test]
fn a_press_outside_closes_it_even_while_searching(cx: &mut TestAppContext) {
    let (_deck, _dir) = setup(cx, "outside");
    let main = cx.windows()[0];
    open(main, cx);
    cx.simulate_input(main, "scale");
    let mut vcx = VisualTestContext::from_window(main, cx);
    vcx.run_until_parked();
    let backdrop = vcx.debug_bounds("modal-panel").unwrap();
    vcx.simulate_click(gpui::point(backdrop.origin.x / 2., backdrop.origin.y / 2.), Modifiers::none());
    vcx.run_until_parked();
    assert!(!cx.read(app_settings::is_open));
}

#[gpui::test]
fn tab_reaches_a_control_and_enter_uses_it(cx: &mut TestAppContext) {
    let (_deck, _dir) = setup(cx, "tab");
    let main = cx.windows()[0];
    open(main, cx);
    // From the search: the list, then Appearance's controls: −, +, the steps.
    cx.simulate_keystrokes(main, "tab tab tab");
    let mut vcx = VisualTestContext::from_window(main, cx);
    let enter = gpui::Keystroke::parse("enter").unwrap();
    vcx.simulate_event(gpui::KeyDownEvent { keystroke: enter.clone(), is_held: false, prefer_character_input: false });
    vcx.simulate_event(gpui::KeyUpEvent { keystroke: enter });
    vcx.run_until_parked();
    assert_eq!(scale(cx), 1.1, "+ used by the keys");
    // Typing a space in the search types it.
    cx.simulate_keystrokes(main, "shift-tab shift-tab shift-tab");
    cx.simulate_input(main, "ui scale");
    let w = main.downcast::<devdeck_app::window_view::WindowView>().unwrap();
    let view = cx.update(|cx| w.read(cx).unwrap().settings_view()).unwrap();
    assert_eq!(cx.read(|cx| view.read(cx).search.read(cx).text().to_string()), "ui scale");
}

#[gpui::test]
fn a_narrow_window_at_a_large_scale_keeps_it_inside(cx: &mut TestAppContext) {
    let (_deck, _dir) = setup(cx, "narrow");
    let main = cx.windows()[0];
    cx.update(|cx| app_settings::set_scale(1.5, cx));
    cx.simulate_window_resize(main, size(logical(480.), logical(320.)));
    open(main, cx);
    let mut vcx = VisualTestContext::from_window(main, cx);
    vcx.run_until_parked();
    let panel = vcx.debug_bounds("modal-panel").unwrap();
    let list = vcx.debug_bounds("settings-nav-0").unwrap();
    assert!(panel.right() <= logical(480.), "{panel:?}");
    assert!(list.size.width <= panel.size.width * 0.3, "the list a third at most: {list:?}");
}

#[gpui::test]
fn agents_machine_and_files_say_what_the_machine_has(cx: &mut TestAppContext) {
    let (_deck, dir) = setup(cx, "machine");
    let main = cx.windows()[0];
    let view = open(main, cx);
    // No agent: its name finds nothing.
    cx.update(|cx| view.update(cx, |v, cx| v.search.update(cx, |i, cx| i.set_text("claude", cx))));
    cx.run_until_parked();
    assert!(listed(&view, cx).is_empty(), "no agent named so on this machine");
    let machine = devdeck_core::config::MachineConfig { node_dirs: vec![PathBuf::from("/opt/node")], path: vec![PathBuf::from("/opt/bin")], agents: vec![super::agents::kind()] };
    cx.update(|cx| devdeck_app::environment::environment(cx).update(cx, |e, cx| {
        e.agents = machine.agents.clone();
        e.machine = machine;
        cx.notify();
    }));
    cx.run_until_parked();
    assert_eq!(listed(&view, cx), [("Agents", Some(1))], "the machine's agent, found by its name");
    let shown = |view: &Entity<SettingsView>, c: Category, cx: &mut TestAppContext| {
        cx.update(|cx| view.update(cx, |v, cx| v.show(c, cx)));
        let vcx = VisualTestContext::from_window(main, cx);
        vcx.run_until_parked();
        vcx
    };
    let mut vcx = shown(&view, Category::Agents, cx);
    assert!(vcx.debug_bounds("setting-agents").is_some(), "no agent: says how to add one");
    let mut vcx2 = shown(&view, Category::Machine, cx);
    assert!(vcx2.debug_bounds("setting-machine.node").is_some() && vcx2.debug_bounds("setting-machine.path").is_some());
    let mut vcx3 = shown(&view, Category::Files, cx);
    assert!(vcx3.debug_bounds("setting-files.folders").is_some());
    // The search finds what the machine file names: node finds Machine.
    cx.update(|cx| view.update(cx, |v, cx| v.search.update(cx, |i, cx| i.set_text("node", cx))));
    cx.run_until_parked();
    assert_eq!(listed(&view, cx), [("Machine", Some(1))]);
    let _ = dir;
}
