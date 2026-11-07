//! M1 step 2: shared components inside the real app shell (GPUI test platform).

use devdeck_app::shell::Shell;
use devdeck_app::{deck::Deck, gallery::Gallery};
use devdeck_core::layout::{MAIN_WINDOW, ViewRef};
use devdeck_ui::overlay::layer_state;
use devdeck_ui::{logical, open_menu, toasts};
use gpui::{AppContext, Entity, Focusable, Modifiers, TestAppContext, VisualTestContext, point, size};
use std::time::Duration;

fn setup(cx: &mut TestAppContext) -> Entity<Deck> {
    let dir = std::env::temp_dir().join(format!("devdeck-comptest-{}-{:?}", std::process::id(), std::thread::current().id()));
    let _ = std::fs::remove_dir_all(&dir);
    cx.update(|cx| devdeck_app::init_with_config_dir(&dir, cx));
    let shell = cx.update(|cx| Shell::start_single(devdeck_app::initial_layout, cx));
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    cx.run_until_parked();
    // Show the gallery tab.
    cx.update(|cx| {
        deck.update(cx, |d, cx| {
            d.apply(cx, |l| {
                let p = l.main().focused;
                l.open(ViewRef::new("demo/gallery"), p).map(|_| ())
            });
        })
    });
    cx.run_until_parked();
    deck
}

fn gallery(deck: &Entity<Deck>, cx: &gpui::App) -> Entity<Gallery> {
    deck.read(cx).view(&ViewRef::new("demo/gallery")).unwrap().downcast::<Gallery>().unwrap()
}

fn panes(deck: &Entity<Deck>, cx: &TestAppContext) -> usize {
    cx.read(|cx| deck.read(cx).layout().panes_in(MAIN_WINDOW).unwrap().len())
}

#[gpui::test]
fn typing_in_an_input_never_triggers_pane_keys(cx: &mut TestAppContext) {
    let deck = setup(cx);
    let main = cx.windows()[0];
    let input = cx.read(|cx| gallery(&deck, cx).read(cx).empty.clone());
    cx.update_window(main, |_, window, cx| input.read(cx).focus_handle(cx).focus(window, cx)).unwrap();
    let tabs_before = cx.read(|cx| deck.read(cx).layout().views().len());
    cx.simulate_input(main, "pw");
    cx.simulate_keystrokes(main, "ctrl-w alt-f");
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| input.read(cx).text().to_string()), "pw");
    assert_eq!(cx.read(|cx| deck.read(cx).layout().views().len()), tabs_before, "ctrl-w inside an input closed nothing");
}

#[gpui::test]
fn menu_keyboard_flow_runs_item_and_restores_focus(cx: &mut TestAppContext) {
    let deck = setup(cx);
    let main = cx.windows()[0];
    cx.update_window(main, |_, window, cx| open_menu(point(logical(400.), logical(300.)), devdeck_app::gallery::demo_menu_items(), window, cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| layer_state(main, cx)), (true, 0));
    // First item is selected; up wraps to the last one, "Show a toast".
    cx.simulate_keystrokes(main, "up enter");
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| layer_state(main, cx)), (false, 0), "running an item closes the menu");
    assert_eq!(cx.update(|cx| toasts(cx).read(cx).queue.items.len()), 1);
    cx.simulate_keystrokes(main, "ctrl-\\");
    assert_eq!(panes(&deck, cx), 4, "focus went back to the panes");
}

#[gpui::test]
fn outside_click_closes_menu_without_clicking_through(cx: &mut TestAppContext) {
    let deck = setup(cx);
    let main = cx.windows()[0];
    cx.update_window(main, |_, window, cx| open_menu(point(logical(900.), logical(500.)), devdeck_app::gallery::demo_menu_items(), window, cx))
        .unwrap();
    let mut vcx = VisualTestContext::from_window(main, cx);
    vcx.run_until_parked();
    let before = vcx.read(|cx| deck.read(cx).layout().clone());
    let modal_button = vcx.debug_bounds("open-modal").expect("gallery visible");
    vcx.simulate_click(modal_button.center(), Modifiers::none());
    assert_eq!(vcx.update(|_, cx| layer_state(main, cx)), (false, 0), "menu closed, and the button under it was not pressed");
    assert_eq!(vcx.read(|cx| deck.read(cx).layout().clone()), before);
}

#[gpui::test]
fn modal_blocks_pane_keys_and_esc_closes_it(cx: &mut TestAppContext) {
    let deck = setup(cx);
    let main = cx.windows()[0];
    let mut vcx = VisualTestContext::from_window(main, cx);
    vcx.run_until_parked();
    let b = vcx.debug_bounds("open-modal").expect("gallery visible");
    vcx.simulate_click(b.center(), Modifiers::none());
    assert_eq!(vcx.update(|_, cx| layer_state(main, cx)), (false, 1));
    vcx.simulate_keystrokes("tab tab tab ctrl-\\");
    assert_eq!(panes(&deck, &vcx), 3, "pane keys don't act behind a modal");
    vcx.simulate_keystrokes("escape");
    assert_eq!(vcx.update(|_, cx| layer_state(main, cx)), (false, 0));
}

#[gpui::test]
fn toast_timer_stops_when_the_stack_empties(cx: &mut TestAppContext) {
    let _deck = setup(cx);
    let t = cx.update(toasts);
    cx.update(|cx| t.update(cx, |t, cx| t.push(devdeck_ui::ToastKind::Info, "hello", vec![], cx)));
    assert!(cx.read(|cx| t.read(cx).timer_running()));
    cx.executor().advance_clock(Duration::from_secs(7));
    cx.run_until_parked();
    assert!(cx.read(|cx| t.read(cx).queue.items.is_empty()));
    assert!(!cx.read(|cx| t.read(cx).timer_running()), "no timer left running when idle");
}

#[gpui::test]
fn long_text_scrolls_to_keep_the_caret_visible(cx: &mut TestAppContext) {
    let deck = setup(cx);
    let main = cx.windows()[0];
    cx.simulate_window_resize(main, size(logical(900.), logical(600.)));
    let input = cx.read(|cx| gallery(&deck, cx).read(cx).empty.clone());
    cx.update_window(main, |_, window, cx| input.read(cx).focus_handle(cx).focus(window, cx)).unwrap();
    cx.simulate_input(main, &"long-branch-name-".repeat(20));
    let mut vcx = VisualTestContext::from_window(main, cx);
    vcx.run_until_parked();
    assert!(vcx.read(|cx| input.read(cx).scroll_x()) > gpui::Pixels::ZERO, "the line scrolled");
    vcx.simulate_keystrokes("home");
    vcx.run_until_parked();
    vcx.simulate_keystrokes("shift-end");
    vcx.run_until_parked();
    assert!(vcx.read(|cx| input.read(cx).scroll_x()) > gpui::Pixels::ZERO);
}
