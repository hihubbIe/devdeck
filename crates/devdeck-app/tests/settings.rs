//! M1 step 1: settings view and UI scale (R9.4), on GPUI's test platform. The last
//! test uses the real text system to check that text never wraps at a large scale.

use devdeck_app::shell::Shell;
use devdeck_app::{app_settings, deck::Deck};
use devdeck_core::layout::MAIN_WINDOW;
use devdeck_core::settings::SETTINGS_FILE;
use devdeck_ui::logical;
use gpui::{Entity, Modifiers, TestAppContext, TestDispatcher, VisualTestContext, size};
use std::path::PathBuf;

fn config_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("devdeck-settingstest-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn setup(cx: &mut TestAppContext, tag: &str) -> (Entity<Deck>, PathBuf) {
    let dir = config_dir(tag);
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
    for selector in ["kube-message", "tab-title"] {
        let b = vcx.debug_bounds(selector).unwrap_or_else(|| panic!("{selector} rendered"));
        assert!(b.size.height <= line + logical(1.), "{selector} wrapped: height {:?} > line {:?}", b.size.height, line);
    }
}

#[gpui::test]
fn theme_picker_switches_globally_and_saves(cx: &mut TestAppContext) {
    let dir = config_dir("theme");
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
    let dir = config_dir("theme-missing");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(SETTINGS_FILE), "theme = \"gone\"\n").unwrap();
    cx.update(|cx| devdeck_app::init_with_config_dir(&dir, cx));
    assert_eq!(cx.read(|cx| devdeck_ui::theme(cx).name.to_string()), "devdeck-dark");
    let problems = cx.read(|cx| app_settings::settings(cx).read(cx).theme_problems.clone());
    assert!(problems.iter().any(|p| p.contains("gone")), "{problems:?}");
}
