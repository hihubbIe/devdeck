//! The overlay components on their own, in a host window: the context menu, the modal
//! stack and the toasts. The mouse goes through the `VisualTestContext`; key bindings are
//! the app's, so keys are the overlay actions (`MenuDown`, `ModalNext`…) dispatched to
//! the focused element, as a bound key would. Rows and buttons are found from the theme's
//! sizes, the way the components lay them out.

mod common;

use common::{host, the_theme};
use devdeck_ui::overlay::{
    MenuAction, MenuClose, MenuCloseSub, MenuConfirm, MenuDown, MenuOpenSub, MenuUp, ModalClose, ModalNext, ModalPrev, SUBMENU_DELAY,
    layer_state, toast_lifetime,
};
use devdeck_ui::{MenuItem, ModalFocus, Theme, ToastKind, logical, modal_frame, open_menu, pop_modal, push_modal, theme, toasts};
use gpui::{
    AnyWindowHandle, AppContext, Context, Entity, FocusHandle, InteractiveElement, IntoElement, Modifiers, MouseButton,
    ParentElement, Pixels, Point, Render, ScrollDelta, ScrollEvent, SharedString, StatefulInteractiveElement, Styled,
    TestAppContext, VisualTestContext, WeakFocusHandle, Window, div, point, prelude::FluentBuilder, size,
};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

/// The host window's size in these tests.
const W: f32 = 800.;
const H: f32 = 600.;

/// What sits under the overlays: a focused surface and a target counting the mouse downs
/// and clicks that reach it.
#[derive(Clone)]
struct Under {
    focus: FocusHandle,
    /// The target's own focus, for focus moving elsewhere while an overlay is open.
    other: FocusHandle,
    downs: Rc<Cell<usize>>,
    clicks: Rc<Cell<usize>>,
}

impl Under {
    fn reached(&self) -> (usize, usize) {
        (self.downs.get(), self.clicks.get())
    }
}

fn setup(cx: &mut TestAppContext) -> (VisualTestContext, AnyWindowHandle, Under) {
    let under = cx.update(|cx| Under { focus: cx.focus_handle(), other: cx.focus_handle(), downs: Rc::default(), clicks: Rc::default() });
    let u = under.clone();
    let window = host(cx, move |_, cx| {
        let t = theme(cx).clone();
        let (downs, clicks) = (u.downs.clone(), u.clicks.clone());
        div()
            .size_full()
            .track_focus(&u.focus)
            .child(
                div()
                    .id("under")
                    .debug_selector(|| "under".into())
                    .track_focus(&u.other)
                    .w(t.size(600.))
                    .h(t.size(500.))
                    .on_mouse_down(MouseButton::Left, move |_, _, _| downs.set(downs.get() + 1))
                    .on_click(move |_, _, _| clicks.set(clicks.get() + 1)),
            )
            .into_any_element()
    });
    cx.simulate_window_resize(window, size(logical(W), logical(H)));
    let mut vcx = VisualTestContext::from_window(window, cx);
    let f = under.focus.clone();
    // Active, as a window the user works in: focus events only fire in an active window.
    vcx.update(|window, cx| {
        window.activate_window();
        f.focus(window, cx);
    });
    vcx.run_until_parked();
    (vcx, window, under)
}

fn focused(vcx: &mut VisualTestContext, handle: &FocusHandle) -> bool {
    vcx.update(|window, _| handle.is_focused(window))
}

// ------------------------------------------------------------------ menu

type Log = Rc<RefCell<Vec<String>>>;

fn item(name: &str, log: &Log) -> MenuItem {
    let (name, log) = (name.to_string(), log.clone());
    MenuItem::new(name.clone(), move |_, _| log.borrow_mut().push(name.clone()))
}

fn ran(log: &Log) -> Vec<String> {
    log.borrow().clone()
}

fn open(vcx: &mut VisualTestContext, at: Point<Pixels>, items: Vec<MenuItem>) {
    vcx.update(|window, cx| open_menu(at, items, window, cx));
    vcx.run_until_parked();
}

fn menu_open(vcx: &mut VisualTestContext, window: AnyWindowHandle) -> bool {
    vcx.update(|_, cx| layer_state(window, cx).0)
}

/// Middle of a menu's row whose top is `rows` rows and `separators` separators below the
/// list's top edge (border and padding included).
fn row_at(top: Point<Pixels>, rows: usize, separators: usize, t: &Theme) -> Point<Pixels> {
    let separator = t.size(8.) + t.hairline;
    point(top.x + t.size(40.), top.y + t.size(5.) + separator * separators as f32 + t.row_h * (rows as f32 + 0.5))
}

fn hover(vcx: &mut VisualTestContext, at: Point<Pixels>) {
    vcx.simulate_mouse_move(at, None, Modifiers::none());
}

#[gpui::test]
fn clicking_a_row_runs_it_and_closes_the_menu(cx: &mut TestAppContext) {
    let (mut vcx, window, under) = setup(cx);
    let t = the_theme(&mut vcx);
    let log = Log::default();
    let at = point(t.size(100.), t.size(100.));
    open(&mut vcx, at, vec![item("first", &log), item("second", &log)]);
    vcx.simulate_click(row_at(at, 1, 0, &t), Modifiers::none());
    assert_eq!(ran(&log), ["second"], "the row under the pointer ran (the backdrop under the list didn't take the click)");
    assert!(!menu_open(&mut vcx, window));
    assert_eq!(under.reached(), (0, 0));
    assert!(focused(&mut vcx, &under.focus), "focus went back to what had it");
}

#[gpui::test]
fn clicking_outside_closes_without_running_or_clicking_through(cx: &mut TestAppContext) {
    let (mut vcx, window, under) = setup(cx);
    let t = the_theme(&mut vcx);
    let log = Log::default();
    open(&mut vcx, point(t.size(100.), t.size(100.)), vec![item("a", &log), item("b", &log)]);
    let target = vcx.debug_bounds("under").expect("drawn");
    vcx.simulate_click(target.bottom_right() - point(t.size(10.), t.size(10.)), Modifiers::none());
    assert!(!menu_open(&mut vcx, window));
    assert!(ran(&log).is_empty());
    assert_eq!(under.reached(), (0, 0), "neither the mouse down nor the click reached what's under the menu");
    assert!(focused(&mut vcx, &under.focus));
    // With the menu gone, the same spot takes clicks again.
    vcx.simulate_click(target.bottom_right() - point(t.size(10.), t.size(10.)), Modifiers::none());
    assert_eq!(under.reached(), (1, 1));
}

#[gpui::test]
fn hovering_a_row_selects_it(cx: &mut TestAppContext) {
    let (mut vcx, _, _) = setup(cx);
    let t = the_theme(&mut vcx);
    let log = Log::default();
    let at = point(t.size(100.), t.size(100.));
    open(&mut vcx, at, vec![item("a", &log), item("b", &log), item("c", &log)]);
    hover(&mut vcx, row_at(at, 2, 0, &t));
    vcx.dispatch_action(MenuConfirm);
    assert_eq!(ran(&log), ["c"]);
}

#[gpui::test]
fn disabled_rows_are_selectable_but_dont_run(cx: &mut TestAppContext) {
    let (mut vcx, window, _) = setup(cx);
    let t = the_theme(&mut vcx);
    let log = Log::default();
    let at = point(t.size(100.), t.size(100.));
    open(&mut vcx, at, vec![item("a", &log), item("b", &log).disabled("not now")]);
    vcx.simulate_click(row_at(at, 1, 0, &t), Modifiers::none());
    assert!(menu_open(&mut vcx, window), "a click on a disabled row keeps the menu open (it shows the reason)");
    vcx.dispatch_action(MenuConfirm);
    assert!(ran(&log).is_empty(), "the click selected it; confirming it doesn't run it either");
    assert!(menu_open(&mut vcx, window));
    vcx.dispatch_action(MenuUp);
    vcx.dispatch_action(MenuConfirm);
    assert_eq!(ran(&log), ["a"]);
}

#[gpui::test]
fn keyboard_selection_skips_separators_and_wraps(cx: &mut TestAppContext) {
    let (mut vcx, _, _) = setup(cx);
    let t = the_theme(&mut vcx);
    let log = Log::default();
    let at = point(t.size(100.), t.size(100.));
    let items = || vec![item("a", &log), MenuItem::separator(), item("b", &log), item("c", &log)];
    open(&mut vcx, at, items());
    vcx.dispatch_action(MenuDown);
    vcx.dispatch_action(MenuConfirm);
    assert_eq!(ran(&log), ["b"], "down from the first row skips the separator");
    open(&mut vcx, at, items());
    vcx.dispatch_action(MenuUp);
    vcx.dispatch_action(MenuConfirm);
    assert_eq!(ran(&log), ["b", "c"], "up from the first row wraps to the last");
    open(&mut vcx, at, items());
    for _ in 0..3 {
        vcx.dispatch_action(MenuDown);
    }
    vcx.dispatch_action(MenuConfirm);
    assert_eq!(ran(&log), ["b", "c", "a"], "down from the last row wraps to the first");
    open(&mut vcx, at, items());
    vcx.dispatch_action(MenuUp);
    vcx.dispatch_action(MenuUp);
    vcx.dispatch_action(MenuConfirm);
    assert_eq!(ran(&log), ["b", "c", "a", "b"]);
}

#[gpui::test]
fn separators_cant_be_clicked_or_hovered(cx: &mut TestAppContext) {
    let (mut vcx, window, under) = setup(cx);
    let t = the_theme(&mut vcx);
    let log = Log::default();
    let at = point(t.size(100.), t.size(100.));
    open(&mut vcx, at, vec![item("a", &log), MenuItem::separator(), item("b", &log)]);
    vcx.dispatch_action(MenuDown);
    // The separator's line, between the two rows.
    let line = point(at.x + t.size(40.), at.y + t.size(5.) + t.row_h + t.size(4.) + t.hairline * 0.5);
    hover(&mut vcx, line);
    vcx.simulate_click(line, Modifiers::none());
    assert!(menu_open(&mut vcx, window), "a click on the separator is the menu's, not the backdrop's");
    assert!(ran(&log).is_empty());
    assert_eq!(under.reached(), (0, 0));
    vcx.dispatch_action(MenuConfirm);
    assert_eq!(ran(&log), ["b"], "the selection didn't move to the separator");
    // The row after the separator is where the layout puts it.
    open(&mut vcx, at, vec![item("a", &log), MenuItem::separator(), item("b", &log)]);
    vcx.simulate_click(row_at(at, 1, 1, &t), Modifiers::none());
    assert_eq!(ran(&log), ["b", "b"]);
}

#[gpui::test]
fn checked_rows_run_like_the_others(cx: &mut TestAppContext) {
    let (mut vcx, window, _) = setup(cx);
    let t = the_theme(&mut vcx);
    let log = Log::default();
    let at = point(t.size(100.), t.size(100.));
    let follow = item("follow", &log).checked(true);
    assert!(follow.checked);
    open(&mut vcx, at, vec![item("a", &log), follow]);
    vcx.simulate_click(row_at(at, 1, 0, &t), Modifiers::none());
    assert_eq!(ran(&log), ["follow"]);
    assert!(!menu_open(&mut vcx, window));
}

#[gpui::test]
fn menu_close_runs_nothing_and_gives_focus_back(cx: &mut TestAppContext) {
    let (mut vcx, window, under) = setup(cx);
    let t = the_theme(&mut vcx);
    let log = Log::default();
    open(&mut vcx, point(t.size(100.), t.size(100.)), vec![item("a", &log)]);
    assert!(!focused(&mut vcx, &under.focus), "the menu took focus");
    vcx.dispatch_action(MenuClose);
    assert!(!menu_open(&mut vcx, window));
    assert!(ran(&log).is_empty());
    assert!(focused(&mut vcx, &under.focus));
}

#[gpui::test]
fn confirm_runs_the_item_and_gives_focus_back(cx: &mut TestAppContext) {
    let (mut vcx, window, under) = setup(cx);
    let t = the_theme(&mut vcx);
    let log = Log::default();
    open(&mut vcx, point(t.size(100.), t.size(100.)), vec![item("a", &log), item("b", &log)]);
    vcx.dispatch_action(MenuConfirm);
    assert_eq!(ran(&log), ["a"], "the first row is selected on open");
    assert!(!menu_open(&mut vcx, window));
    assert!(focused(&mut vcx, &under.focus));
}

/// A menu taller than the window, opened halfway down it.
fn open_long_menu(vcx: &mut VisualTestContext, log: &Log, t: &Theme) {
    let items = (0..100).map(|i| item(&i.to_string(), log)).collect();
    open(vcx, point(t.size(100.), t.size(300.)), items);
}

/// The long menu's list is as tall as the window allows (`max_h`) and pushed up to fit.
fn long_menu_top(t: &Theme) -> Point<Pixels> {
    let height = logical(H) - t.size(16.);
    point(t.size(100.), logical(H) - t.size(4.) - height)
}

#[gpui::test]
fn a_long_menu_stays_within_the_window_and_scrolls(cx: &mut TestAppContext) {
    let (mut vcx, _, _) = setup(cx);
    let t = the_theme(&mut vcx);
    let log = Log::default();
    let top = long_menu_top(&t);
    let bottom = point(top.x + t.size(40.), logical(H) - t.size(12.));
    let last_visible = ((bottom.y - top.y - t.size(5.)) / t.row_h).floor() as usize;
    open_long_menu(&mut vcx, &log, &t);
    vcx.dispatch_action(MenuDown);
    hover(&mut vcx, row_at(top, 0, 0, &t));
    vcx.dispatch_action(MenuConfirm);
    assert_eq!(ran(&log), ["0"], "the list's top is inside the window");
    open_long_menu(&mut vcx, &log, &t);
    hover(&mut vcx, bottom);
    vcx.dispatch_action(MenuConfirm);
    assert_eq!(ran(&log), ["0".to_string(), last_visible.to_string()], "the list reaches down to the window's bottom, not past it");
    open_long_menu(&mut vcx, &log, &t);
    vcx.simulate_event(ScrollEvent {
        position: bottom,
        delta: ScrollDelta::Pixels(point(Pixels::ZERO, -t.row_h * 200.)),
        ..Default::default()
    });
    hover(&mut vcx, bottom);
    vcx.dispatch_action(MenuConfirm);
    assert_eq!(ran(&log).last().map(String::as_str), Some("99"), "scrolled to the end");
}

#[gpui::test]
fn keyboard_selection_scrolls_the_row_into_view(cx: &mut TestAppContext) {
    let (mut vcx, _, _) = setup(cx);
    let t = the_theme(&mut vcx);
    let log = Log::default();
    let bottom = point(t.size(140.), logical(H) - t.size(12.));
    open_long_menu(&mut vcx, &log, &t);
    vcx.dispatch_action(MenuUp);
    // Hovering selects the row under the pointer, telling which row is there now.
    hover(&mut vcx, bottom);
    vcx.dispatch_action(MenuConfirm);
    assert_eq!(ran(&log), ["99"], "up wrapped to the last row and scrolled it into view");
}

#[gpui::test]
fn a_menu_opened_at_the_corner_stays_inside_the_window(cx: &mut TestAppContext) {
    let (mut vcx, _, _) = setup(cx);
    let t = the_theme(&mut vcx);
    let log = Log::default();
    let corner = point(logical(W), logical(H));
    open(&mut vcx, corner - point(t.size(2.), t.size(2.)), vec![item("a", &log), item("b", &log), item("c", &log)]);
    hover(&mut vcx, corner - point(t.size(12.), t.size(12.)));
    vcx.dispatch_action(MenuConfirm);
    assert_eq!(ran(&log), ["c"], "the last row was moved under the window's corner");
}

#[gpui::test]
fn opening_a_menu_over_another_replaces_it(cx: &mut TestAppContext) {
    let (mut vcx, window, under) = setup(cx);
    let t = the_theme(&mut vcx);
    let log = Log::default();
    open(&mut vcx, point(t.size(100.), t.size(100.)), vec![item("old", &log)]);
    open(&mut vcx, point(t.size(300.), t.size(100.)), vec![item("new", &log)]);
    assert!(menu_open(&mut vcx, window), "the new menu is open");
    vcx.dispatch_action(MenuConfirm);
    assert_eq!(ran(&log), ["new"]);
    assert!(!menu_open(&mut vcx, window));
    assert!(focused(&mut vcx, &under.focus), "focus went back to what had it before both menus");
}

#[gpui::test]
fn focus_moved_elsewhere_closes_the_menu_and_stays_there(cx: &mut TestAppContext) {
    let (mut vcx, window, under) = setup(cx);
    let t = the_theme(&mut vcx);
    let log = Log::default();
    open(&mut vcx, point(t.size(100.), t.size(100.)), vec![item("a", &log)]);
    // Something else takes focus (a pane, a modal): the menu is done, and doesn't pull
    // focus back to what had it before.
    let other = under.other.clone();
    vcx.update(|window, cx| other.focus(window, cx));
    vcx.run_until_parked();
    assert!(!menu_open(&mut vcx, window));
    assert!(ran(&log).is_empty());
    assert!(focused(&mut vcx, &under.other), "focus stayed where it was moved");
}

/// A menu whose first row ("here") has a submenu of other places, then a plain row.
fn places_menu(log: &Log) -> Vec<MenuItem> {
    vec![item("here", log).submenu(vec![item("there", log), item("elsewhere", log)]), item("plain", log)]
}

fn submenu_shown(vcx: &mut VisualTestContext) -> bool {
    vcx.run_until_parked();
    vcx.debug_bounds("submenu-item-0").is_some()
}

#[gpui::test]
fn resting_on_an_item_opens_its_submenu_after_the_delay(cx: &mut TestAppContext) {
    let (mut vcx, window, _) = setup(cx);
    let t = the_theme(&mut vcx);
    let log = Log::default();
    let at = point(t.size(100.), t.size(100.));
    open(&mut vcx, at, places_menu(&log));
    hover(&mut vcx, row_at(at, 0, 0, &t));
    advance(&mut vcx, SUBMENU_DELAY - Duration::from_millis(50));
    assert!(!submenu_shown(&mut vcx), "not before the delay");
    advance(&mut vcx, Duration::from_millis(50));
    assert!(submenu_shown(&mut vcx));
    let row = vcx.debug_bounds("menu-item-0").expect("drawn");
    let first = vcx.debug_bounds("submenu-item-0").expect("drawn");
    assert!(first.left() >= row.right() && first.top() == row.top(), "beside its row, its first item level with it: {first:?} {row:?}");
    let second = vcx.debug_bounds("submenu-item-1").expect("drawn").center();
    vcx.simulate_click(second, Modifiers::none());
    assert_eq!(ran(&log), ["elsewhere"], "the submenu item ran, not the row's default");
    assert!(!menu_open(&mut vcx, window));
}

#[gpui::test]
fn clicking_an_item_with_a_submenu_does_its_default(cx: &mut TestAppContext) {
    let (mut vcx, window, _) = setup(cx);
    let t = the_theme(&mut vcx);
    let log = Log::default();
    let at = point(t.size(100.), t.size(100.));
    open(&mut vcx, at, places_menu(&log));
    vcx.simulate_click(row_at(at, 0, 0, &t), Modifiers::none());
    assert_eq!(ran(&log), ["here"]);
    assert!(!menu_open(&mut vcx, window));
    advance(&mut vcx, SUBMENU_DELAY);
    assert!(!menu_open(&mut vcx, window), "the rest timer doesn't bring it back");
}

#[gpui::test]
fn the_pointer_can_cross_rows_on_its_way_to_the_submenu(cx: &mut TestAppContext) {
    let (mut vcx, _, _) = setup(cx);
    let t = the_theme(&mut vcx);
    let log = Log::default();
    let at = point(t.size(100.), t.size(100.));
    open(&mut vcx, at, places_menu(&log));
    hover(&mut vcx, row_at(at, 0, 0, &t));
    advance(&mut vcx, SUBMENU_DELAY);
    // On the way down to "elsewhere", the pointer passes over "plain".
    hover(&mut vcx, row_at(at, 1, 0, &t));
    assert!(submenu_shown(&mut vcx), "passing over another row keeps the submenu");
    let second = vcx.debug_bounds("submenu-item-1").expect("drawn").center();
    hover(&mut vcx, second);
    advance(&mut vcx, SUBMENU_DELAY);
    assert!(submenu_shown(&mut vcx), "reaching it in time keeps it");
    vcx.dispatch_action(MenuConfirm);
    assert_eq!(ran(&log), ["elsewhere"]);
}

#[gpui::test]
fn resting_on_another_row_closes_the_submenu(cx: &mut TestAppContext) {
    let (mut vcx, _, _) = setup(cx);
    let t = the_theme(&mut vcx);
    let log = Log::default();
    let at = point(t.size(100.), t.size(100.));
    open(&mut vcx, at, places_menu(&log));
    hover(&mut vcx, row_at(at, 0, 0, &t));
    advance(&mut vcx, SUBMENU_DELAY);
    hover(&mut vcx, row_at(at, 1, 0, &t));
    advance(&mut vcx, SUBMENU_DELAY);
    assert!(!submenu_shown(&mut vcx));
    vcx.dispatch_action(MenuConfirm);
    assert_eq!(ran(&log), ["plain"], "the row the pointer rests on is the selected one");
}

#[gpui::test]
fn keys_open_walk_and_close_the_submenu(cx: &mut TestAppContext) {
    let (mut vcx, window, _) = setup(cx);
    let t = the_theme(&mut vcx);
    let log = Log::default();
    let at = point(t.size(100.), t.size(100.));
    open(&mut vcx, at, places_menu(&log));
    vcx.dispatch_action(MenuOpenSub);
    assert!(submenu_shown(&mut vcx), "→ opens the selected (first) row's at once");
    vcx.dispatch_action(MenuDown);
    vcx.dispatch_action(MenuConfirm);
    assert_eq!(ran(&log), ["elsewhere"], "its first item was selected; ↓ moved within it");
    assert!(!menu_open(&mut vcx, window));

    open(&mut vcx, at, places_menu(&log));
    vcx.dispatch_action(MenuOpenSub);
    vcx.dispatch_action(MenuCloseSub);
    assert!(!submenu_shown(&mut vcx), "← closes it");
    vcx.dispatch_action(MenuConfirm);
    assert_eq!(ran(&log), ["elsewhere", "here"], "back in the menu, on its row");

    open(&mut vcx, at, places_menu(&log));
    vcx.dispatch_action(MenuOpenSub);
    vcx.dispatch_action(MenuClose);
    assert!(!submenu_shown(&mut vcx) && menu_open(&mut vcx, window), "Esc closes the submenu first");
    vcx.dispatch_action(MenuClose);
    assert!(!menu_open(&mut vcx, window));
    assert_eq!(ran(&log), ["elsewhere", "here"]);
}

#[gpui::test]
fn a_closed_submenu_stays_closed_while_the_pointer_stays_on_its_row(cx: &mut TestAppContext) {
    let (mut vcx, _, _) = setup(cx);
    let t = the_theme(&mut vcx);
    let log = Log::default();
    let at = point(t.size(100.), t.size(100.));
    let row = row_at(at, 0, 0, &t);
    open(&mut vcx, at, places_menu(&log));
    for esc in [false, true] {
        hover(&mut vcx, row);
        advance(&mut vcx, SUBMENU_DELAY);
        assert!(submenu_shown(&mut vcx));
        if esc {
            vcx.dispatch_action(MenuClose);
        } else {
            vcx.dispatch_action(MenuCloseSub);
        }
        hover(&mut vcx, row + point(t.size(3.), Pixels::ZERO));
        advance(&mut vcx, SUBMENU_DELAY);
        assert!(!submenu_shown(&mut vcx), "a jitter on the row doesn't reopen it (esc: {esc})");
        // Leaving the row and coming back does.
        hover(&mut vcx, row_at(at, 1, 0, &t));
    }
}

#[gpui::test]
fn a_key_wins_over_a_pending_rest(cx: &mut TestAppContext) {
    let (mut vcx, _, _) = setup(cx);
    let t = the_theme(&mut vcx);
    let log = Log::default();
    let at = point(t.size(100.), t.size(100.));
    open(&mut vcx, at, places_menu(&log));
    // → before the rest is over, then ↓ in the submenu: the rest doesn't reset it.
    hover(&mut vcx, row_at(at, 0, 0, &t));
    vcx.dispatch_action(MenuOpenSub);
    vcx.dispatch_action(MenuDown);
    advance(&mut vcx, SUBMENU_DELAY);
    vcx.dispatch_action(MenuConfirm);
    assert_eq!(ran(&log), ["elsewhere"]);
    // On the way to another row, a key: the keys act in the open submenu, which stays.
    open(&mut vcx, at, places_menu(&log));
    hover(&mut vcx, row_at(at, 0, 0, &t));
    advance(&mut vcx, SUBMENU_DELAY);
    hover(&mut vcx, row_at(at, 1, 0, &t));
    vcx.dispatch_action(MenuDown);
    advance(&mut vcx, SUBMENU_DELAY);
    assert!(submenu_shown(&mut vcx), "the key cancelled the rest on the other row");
    vcx.dispatch_action(MenuConfirm);
    assert_eq!(ran(&log), ["elsewhere", "elsewhere"]);
}

#[gpui::test]
fn a_disabled_item_doesnt_open_its_submenu(cx: &mut TestAppContext) {
    let (mut vcx, _, _) = setup(cx);
    let t = the_theme(&mut vcx);
    let log = Log::default();
    let at = point(t.size(100.), t.size(100.));
    let mut items = places_menu(&log);
    items[0] = items[0].clone().disabled("no folder");
    open(&mut vcx, at, items);
    hover(&mut vcx, row_at(at, 0, 0, &t));
    advance(&mut vcx, SUBMENU_DELAY);
    vcx.dispatch_action(MenuOpenSub);
    assert!(!submenu_shown(&mut vcx));
}

// ------------------------------------------------------------------ modal

/// A modal with focusable fields and an OK button; it closes itself and gives focus back
/// to what had it, as the app's modals do.
struct TestModal {
    focus: ModalFocus,
    /// Focused on open when set, instead of the first field.
    container: Option<FocusHandle>,
    restore: Option<WeakFocusHandle>,
    ok: &'static str,
    clicks: Rc<Cell<usize>>,
}

impl TestModal {
    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        pop_modal(window, cx);
        if let Some(h) = self.restore.as_ref().and_then(WeakFocusHandle::upgrade) {
            h.focus(window, cx);
        }
    }
}

impl Render for TestModal {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let clicks = self.clicks.clone();
        let content = div()
            .key_context("Modal")
            .when_some(self.container.clone(), |d, h| d.track_focus(&h))
            .flex()
            .flex_col()
            .gap_2()
            .on_action(cx.listener(|this, _: &ModalNext, window, cx| this.focus.cycle(1, window, cx)))
            .on_action(cx.listener(|this, _: &ModalPrev, window, cx| this.focus.cycle(-1, window, cx)))
            .on_action(cx.listener(|this, _: &ModalClose, window, cx| this.close(window, cx)))
            .children(self.focus.0.iter().enumerate().map(|(i, h)| div().id(("field", i)).h(t.row_h).track_focus(h)))
            .child({
                let ok = self.ok;
                devdeck_ui::button("ok", "OK", &t).debug_selector(move || ok.into()).on_click(move |_, _, _| clicks.set(clicks.get() + 1))
            });
        modal_frame(content.into_any_element(), window, &t)
    }
}

/// Pushes a modal with `fields` fields, focusing the first (or its container).
fn push(vcx: &mut VisualTestContext, fields: usize, container: bool, ok: &'static str) -> Entity<TestModal> {
    let modal = vcx.update(|window, cx| {
        let restore = window.focused(cx).map(|f| f.downgrade());
        let modal = cx.new(|cx| TestModal {
            focus: ModalFocus((0..fields).map(|_| cx.focus_handle()).collect()),
            container: container.then(|| cx.focus_handle()),
            restore,
            ok,
            clicks: Rc::default(),
        });
        push_modal(modal.clone().into(), window, cx);
        let m = modal.read(cx);
        let first = m.container.clone().unwrap_or_else(|| m.focus.0[0].clone());
        first.focus(window, cx);
        modal
    });
    vcx.run_until_parked();
    modal
}

fn modals(vcx: &mut VisualTestContext, window: AnyWindowHandle) -> usize {
    vcx.update(|_, cx| layer_state(window, cx).1)
}

/// Which of the modal's fields has focus.
fn field(vcx: &mut VisualTestContext, modal: &Entity<TestModal>) -> Option<usize> {
    vcx.update(|window, cx| modal.read(cx).focus.0.iter().position(|h| h.is_focused(window)))
}

#[gpui::test]
fn a_modal_blocks_clicks_to_what_is_under_it(cx: &mut TestAppContext) {
    let (mut vcx, window, under) = setup(cx);
    let t = the_theme(&mut vcx);
    let modal = push(&mut vcx, 1, false, "ok");
    vcx.simulate_click(point(t.size(20.), t.size(20.)), Modifiers::none());
    assert_eq!(under.reached(), (0, 0), "the backdrop took the click");
    assert_eq!(modals(&mut vcx, window), 1, "and a backdrop click doesn't close the modal");
    let ok = vcx.debug_bounds("ok").expect("drawn");
    vcx.simulate_click(ok.center(), Modifiers::none());
    assert_eq!(vcx.read(|cx| modal.read(cx).clicks.get()), 1, "the modal's own controls take clicks");
    assert_eq!(under.reached(), (0, 0));
}

#[gpui::test]
fn modal_next_and_prev_cycle_through_its_fields_only(cx: &mut TestAppContext) {
    let (mut vcx, _, under) = setup(cx);
    let modal = push(&mut vcx, 3, false, "ok");
    assert_eq!(field(&mut vcx, &modal), Some(0));
    let mut seen = vec![];
    for _ in 0..4 {
        vcx.dispatch_action(ModalNext);
        seen.push(field(&mut vcx, &modal));
        assert!(!focused(&mut vcx, &under.focus));
    }
    assert_eq!(seen, [Some(1), Some(2), Some(0), Some(1)], "next wraps");
    vcx.dispatch_action(ModalPrev);
    vcx.dispatch_action(ModalPrev);
    assert_eq!(field(&mut vcx, &modal), Some(2), "prev wraps");
}

#[gpui::test]
fn modal_prev_from_outside_its_fields_goes_to_the_last(cx: &mut TestAppContext) {
    let (mut vcx, _, _) = setup(cx);
    let modal = push(&mut vcx, 3, true, "ok");
    assert_eq!(field(&mut vcx, &modal), None, "the modal's container has focus");
    vcx.dispatch_action(ModalPrev);
    assert_eq!(field(&mut vcx, &modal), Some(2));
    let modal = push(&mut vcx, 3, true, "ok2");
    vcx.dispatch_action(ModalNext);
    assert_eq!(field(&mut vcx, &modal), Some(0), "and next to the first");
}

#[gpui::test]
fn modal_close_and_pop_close_it_and_the_owner_restores_focus(cx: &mut TestAppContext) {
    let (mut vcx, window, under) = setup(cx);
    push(&mut vcx, 2, false, "ok");
    assert_eq!(modals(&mut vcx, window), 1);
    vcx.dispatch_action(ModalClose);
    assert_eq!(modals(&mut vcx, window), 0);
    assert!(focused(&mut vcx, &under.focus));
    push(&mut vcx, 2, false, "ok");
    vcx.update(pop_modal);
    vcx.run_until_parked();
    assert_eq!(modals(&mut vcx, window), 0);
    vcx.simulate_click(point(logical(10.), logical(10.)), Modifiers::none());
    assert_eq!(under.reached(), (1, 1), "nothing left blocking clicks");
}

#[gpui::test]
fn stacked_modals_close_from_the_top(cx: &mut TestAppContext) {
    let (mut vcx, window, under) = setup(cx);
    let first = push(&mut vcx, 2, false, "ok1");
    vcx.dispatch_action(ModalNext);
    let second = push(&mut vcx, 3, false, "ok2");
    assert_eq!(modals(&mut vcx, window), 2);
    vcx.dispatch_action(ModalPrev);
    assert_eq!(field(&mut vcx, &second), Some(2), "keys go to the top modal");
    assert_eq!(field(&mut vcx, &first), None);
    let ok = vcx.debug_bounds("ok2").expect("drawn");
    vcx.simulate_click(ok.center(), Modifiers::none());
    assert_eq!(vcx.read(|cx| (first.read(cx).clicks.get(), second.read(cx).clicks.get())), (0, 1), "the top modal takes the click");
    vcx.dispatch_action(ModalClose);
    assert_eq!(modals(&mut vcx, window), 1);
    assert_eq!(field(&mut vcx, &first), Some(1), "focus went back into the first modal");
    vcx.dispatch_action(ModalNext);
    assert_eq!(field(&mut vcx, &first), Some(0));
    vcx.dispatch_action(ModalClose);
    assert_eq!(modals(&mut vcx, window), 0);
    assert!(focused(&mut vcx, &under.focus));
}

// ------------------------------------------------------------------ toasts

fn push_toast(vcx: &mut VisualTestContext, kind: ToastKind, text: &str, actions: Vec<(SharedString, MenuAction)>) {
    let text = text.to_string();
    vcx.update(|_, cx| toasts(cx).update(cx, |s, cx| s.push(kind, text, actions, cx)));
    vcx.run_until_parked();
}

fn shown(vcx: &mut VisualTestContext) -> Vec<String> {
    vcx.update(|_, cx| toasts(cx).read(cx).queue.items.iter().map(|t| t.text.to_string()).collect())
}

fn timer_running(vcx: &mut VisualTestContext) -> bool {
    vcx.update(|_, cx| toasts(cx).read(cx).timer_running())
}

fn advance(vcx: &mut VisualTestContext, by: Duration) {
    vcx.executor().advance_clock(by);
    vcx.run_until_parked();
}

/// A toast row's height: its padding and border around a button.
fn toast_h(t: &Theme) -> Pixels {
    t.button_h + t.size(10.)
}

/// Right edge of the close button of the toast `from_bottom` rows above the newest, and
/// the vertical middle of that row.
fn close_button(from_bottom: usize, t: &Theme) -> (Pixels, Pixels) {
    let bottom = logical(H) - t.size(16.) - (toast_h(t) + t.size(8.)) * from_bottom as f32;
    (logical(W) - t.size(16.) - t.size(13.), bottom - toast_h(t) / 2.)
}

fn click_close(vcx: &mut VisualTestContext, from_bottom: usize, t: &Theme) {
    let (right, y) = close_button(from_bottom, t);
    vcx.simulate_click(point(right - t.size(3.), y), Modifiers::none());
}

/// Clicks the action button just left of the close button of the newest toast.
fn click_action(vcx: &mut VisualTestContext, t: &Theme) {
    let close_w = vcx.update(|window, _| {
        let run = window.text_style().to_run("×".len());
        window.text_system().layout_line("×", t.text, &[run], None).width
    }) + t.size(16.);
    let (right, y) = close_button(0, t);
    let action_right = right - close_w - t.size(12.);
    vcx.simulate_click(point(action_right - t.size(3.), y), Modifiers::none());
}

#[gpui::test]
fn a_toast_shows_until_its_lifetime_ends_and_the_timer_stops(cx: &mut TestAppContext) {
    let (mut vcx, _, _) = setup(cx);
    assert!(!timer_running(&mut vcx), "no timer before any toast");
    push_toast(&mut vcx, ToastKind::Info, "hello", vec![]);
    assert_eq!(shown(&mut vcx), ["hello"]);
    assert!(timer_running(&mut vcx));
    let life = toast_lifetime(ToastKind::Info);
    advance(&mut vcx, life - Duration::from_millis(100));
    assert_eq!(shown(&mut vcx), ["hello"], "still there just before its lifetime");
    advance(&mut vcx, Duration::from_millis(200));
    assert!(shown(&mut vcx).is_empty());
    assert!(!timer_running(&mut vcx), "the timer stopped with the stack empty");
}

#[gpui::test]
fn each_kind_lives_its_own_lifetime(cx: &mut TestAppContext) {
    let (mut vcx, _, _) = setup(cx);
    for (kind, text) in [(ToastKind::Info, "info"), (ToastKind::Ok, "ok"), (ToastKind::Warn, "warn"), (ToastKind::Err, "err")] {
        push_toast(&mut vcx, kind, text, vec![]);
    }
    assert_eq!(shown(&mut vcx), ["info", "ok", "warn", "err"]);
    let short = toast_lifetime(ToastKind::Info);
    assert_eq!(short, toast_lifetime(ToastKind::Ok));
    assert_eq!(short, toast_lifetime(ToastKind::Warn));
    let long = toast_lifetime(ToastKind::Err);
    assert!(long > short, "errors stay longer");
    advance(&mut vcx, short + Duration::from_millis(100));
    assert_eq!(shown(&mut vcx), ["err"]);
    assert!(timer_running(&mut vcx), "the timer rearmed for the error");
    advance(&mut vcx, long - short);
    assert!(shown(&mut vcx).is_empty());
    assert!(!timer_running(&mut vcx));
}

#[gpui::test]
fn the_close_button_dismisses_its_toast(cx: &mut TestAppContext) {
    let (mut vcx, _, under) = setup(cx);
    let t = the_theme(&mut vcx);
    push_toast(&mut vcx, ToastKind::Info, "older", vec![]);
    push_toast(&mut vcx, ToastKind::Warn, "newer", vec![]);
    click_close(&mut vcx, 1, &t);
    assert_eq!(shown(&mut vcx), ["newer"], "the upper toast's close button dismissed the older one");
    assert!(timer_running(&mut vcx));
    click_close(&mut vcx, 0, &t);
    assert!(shown(&mut vcx).is_empty());
    assert!(!timer_running(&mut vcx), "dismissing the last toast stops the timer");
    assert_eq!(under.reached(), (0, 0));
}

#[gpui::test]
fn an_action_button_runs_and_dismisses_its_toast(cx: &mut TestAppContext) {
    let (mut vcx, _, under) = setup(cx);
    let t = the_theme(&mut vcx);
    let runs = Rc::new(Cell::new(0));
    let r = runs.clone();
    let retry: MenuAction = Rc::new(move |_, _| r.set(r.get() + 1));
    push_toast(&mut vcx, ToastKind::Err, "start failed", vec![("Retry".into(), retry)]);
    click_action(&mut vcx, &t);
    assert_eq!(runs.get(), 1);
    assert!(shown(&mut vcx).is_empty());
    assert!(!timer_running(&mut vcx));
    assert_eq!(under.reached(), (0, 0));
}

#[gpui::test]
fn several_toasts_stack_up_to_the_cap(cx: &mut TestAppContext) {
    let (mut vcx, _, _) = setup(cx);
    let t = the_theme(&mut vcx);
    for i in 0..6 {
        push_toast(&mut vcx, ToastKind::Info, &format!("t{i}"), vec![]);
    }
    assert_eq!(shown(&mut vcx), ["t2", "t3", "t4", "t5"], "the oldest are dropped past the cap");
    click_close(&mut vcx, 3, &t);
    assert_eq!(shown(&mut vcx), ["t3", "t4", "t5"], "the top of the stack is where the layout puts it");
    let id = vcx.update(|_, cx| toasts(cx).read(cx).queue.items[1].id);
    vcx.update(|_, cx| toasts(cx).update(cx, |s, cx| s.dismiss(id, cx)));
    assert_eq!(shown(&mut vcx), ["t3", "t5"]);
}

#[gpui::test]
fn toasts_stay_clickable_above_a_modal(cx: &mut TestAppContext) {
    let (mut vcx, window, _) = setup(cx);
    let t = the_theme(&mut vcx);
    push(&mut vcx, 1, false, "ok");
    push_toast(&mut vcx, ToastKind::Ok, "saved", vec![]);
    click_close(&mut vcx, 0, &t);
    assert!(shown(&mut vcx).is_empty());
    assert_eq!(modals(&mut vcx, window), 1, "the click didn't reach the modal's backdrop");
}
