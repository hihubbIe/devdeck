//! The notification center (plan "notifications", step A): the bell's badge in every
//! window, the center as a cover of the main window (excluding Settings), its filters,
//! read state, clear and keys; toasts leading to it; the control API's toasts untouched.

use devdeck_api::types::Toasts;
use devdeck_app::api::Who;
use devdeck_app::api::dispatch::{self, Answer, CallCx};
use devdeck_app::app_settings::{self, Cover};
use devdeck_app::notifications::{self, Badge, NotificationsView, Source};
use devdeck_app::shell::Shell;
use devdeck_app::window_view::WindowView;
use devdeck_core::layout::ViewRef;
use devdeck_core::view_addr::ViewAddr;
use devdeck_ui::{NewToast, ToastKind, logical, toasts};
use gpui::{AnyWindowHandle, Entity, Modifiers, TestAppContext, VisualTestContext, WindowHandle, size};
use serde_json::json;

fn start(cx: &mut TestAppContext, tag: &str) -> Entity<Shell> {
    let config = cx.update(|cx| devdeck_app::test_dir(&format!("{tag}-config"), cx));
    cx.update(|cx| devdeck_app::init_with_config_dir(&config, cx));
    let shell = cx.update(|cx| Shell::start(devdeck_app::demo_workspaces(), "work", None, cx));
    cx.update(|cx| shell.update(cx, |s, _| s.quit_for_real = false));
    cx.update(devdeck_app::api::present::follow_toasts);
    cx.run_until_parked();
    // What starting said is not these tests' business.
    cx.update(|cx| {
        toasts(cx).update(cx, |t, cx| {
            t.queue.items.clear();
            t.notes.clear();
            cx.notify();
        })
    });
    cx.run_until_parked();
    shell
}

fn main_handle(shell: &Entity<Shell>, cx: &TestAppContext) -> WindowHandle<WindowView> {
    cx.read(|cx| shell.read(cx).main_window().expect("main"))
}

fn main_window(shell: &Entity<Shell>, cx: &TestAppContext) -> AnyWindowHandle {
    main_handle(shell, cx).into()
}

fn push(cx: &mut TestAppContext, t: NewToast) -> u64 {
    let id = cx.update(|cx| toasts(cx).update(cx, |s, cx| s.push_new(t, cx)));
    cx.run_until_parked();
    id
}

fn note(kind: ToastKind, text: &str, source: Option<&str>) -> NewToast {
    NewToast::new(kind, text.to_string()).source(source.map(|s| s.to_string().into()))
}

fn cover(cx: &TestAppContext) -> Cover {
    cx.read(app_settings::cover)
}

fn center(shell: &Entity<Shell>, cx: &TestAppContext) -> Entity<NotificationsView> {
    let main = main_handle(shell, cx);
    cx.read(|cx| main.read(cx).unwrap().notifications_view()).expect("the center was opened")
}

fn rows(shell: &Entity<Shell>, cx: &TestAppContext) -> Vec<u64> {
    let c = center(shell, cx);
    cx.read(|cx| c.read(cx).rows().to_vec())
}

fn unread(cx: &mut TestAppContext) -> usize {
    cx.update(|cx| toasts(cx).read(cx).notes.unread())
}

fn badge(cx: &mut TestAppContext) -> Badge {
    cx.update(notifications::badge)
}

/// Draws `window` and says whether `selector` is in it.
fn drawn(window: AnyWindowHandle, selector: &'static str, cx: &mut TestAppContext) -> bool {
    let mut vcx = VisualTestContext::from_window(window, cx);
    vcx.run_until_parked();
    vcx.debug_bounds(selector).is_some()
}

fn click(window: AnyWindowHandle, selector: String, cx: &mut TestAppContext) {
    let mut vcx = VisualTestContext::from_window(window, cx);
    vcx.run_until_parked();
    let b = vcx.debug_bounds(Box::leak(selector.clone().into_boxed_str())).unwrap_or_else(|| panic!("{selector} drawn"));
    vcx.simulate_click(b.center(), Modifiers::none());
    vcx.run_until_parked();
}

#[gpui::test]
fn the_bell_has_three_states_and_red_counts_errors_only(cx: &mut TestAppContext) {
    let shell = start(cx, "bell");
    let main = main_window(&shell, cx);
    assert_eq!(badge(cx), Badge::Clear);
    assert!(drawn(main, "bell", cx));
    assert!(!drawn(main, "bell-dot", cx) && !drawn(main, "bell-count", cx), "nothing unread: the bell alone");
    push(cx, note(ToastKind::Info, "built", None));
    push(cx, note(ToastKind::Warn, "refused", None));
    assert_eq!(badge(cx), Badge::Unread(2), "a warning never counts red");
    assert!(drawn(main, "bell-dot", cx));
    assert!(!drawn(main, "bell-count", cx));
    let e1 = push(cx, note(ToastKind::Err, "failed", Some("work")));
    push(cx, note(ToastKind::Err, "failed again", Some("personal")));
    assert_eq!(badge(cx), Badge::Errors { errors: 2, unread: 4 });
    assert!(drawn(main, "bell-count", cx), "red wins");
    assert!(!drawn(main, "bell-dot", cx));
    assert_eq!(notifications::badge_words(badge(cx)), "notifications: 4 unread, 2 errors");
    cx.update(|cx| toasts(cx).update(cx, |t, cx| t.close(e1, cx)));
    assert_eq!(badge(cx), Badge::Errors { errors: 1, unread: 3 }, "closing its toast reads it");
}

#[gpui::test]
fn the_center_opens_closes_and_excludes_settings(cx: &mut TestAppContext) {
    let shell = start(cx, "cover");
    let main = main_window(&shell, cx);
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    let panes = |cx: &TestAppContext| cx.read(|cx| deck.read(cx).layout().panes_in(devdeck_core::layout::MAIN_WINDOW).unwrap().len());
    let before = panes(cx);
    cx.simulate_keystrokes(main, "ctrl-shift-n");
    assert_eq!(cover(cx), Cover::Notifications);
    assert!(drawn(main, "notifications", cx));
    cx.simulate_keystrokes(main, "ctrl-\\");
    assert_eq!(panes(cx), before, "pane keys do nothing behind the center");
    cx.simulate_keystrokes(main, "ctrl-,");
    assert_eq!(cover(cx), Cover::Settings, "settings replace the center");
    assert!(!drawn(main, "notifications", cx));
    cx.simulate_keystrokes(main, "ctrl-shift-n");
    assert_eq!(cover(cx), Cover::Notifications, "and the center replaces settings");
    cx.simulate_keystrokes(main, "escape");
    assert_eq!(cover(cx), Cover::None);
    cx.simulate_keystrokes(main, "ctrl-\\");
    assert_eq!(panes(cx), before + 1, "focus is back in the panes");
    // The bell opens and closes it too.
    click(main, "bell".into(), cx);
    assert_eq!(cover(cx), Cover::Notifications);
    click(main, "bell".into(), cx);
    assert_eq!(cover(cx), Cover::None);
}

#[gpui::test]
fn shown_rows_are_read_and_filters_pick_them(cx: &mut TestAppContext) {
    let shell = start(cx, "filters");
    let main = main_window(&shell, cx);
    let w = push(cx, note(ToastKind::Info, "work's", Some("work")));
    let p = push(cx, note(ToastKind::Err, "personal's", Some("personal")));
    let a = push(cx, note(ToastKind::Warn, "the app's", None));
    cx.simulate_keystrokes(main, "ctrl-shift-n");
    cx.run_until_parked();
    assert_eq!(rows(&shell, cx), [a, p, w], "newest first");
    assert_eq!(unread(cx), 0, "shown: read");
    assert_eq!(badge(cx), Badge::Clear);
    let c = center(&shell, cx);
    cx.update(|cx| c.update(cx, |v, cx| v.set_source(Source::App, cx)));
    assert_eq!(rows(&shell, cx), [a], "\"app\": no workspace");
    cx.update(|cx| c.update(cx, |v, cx| v.set_source(Source::Workspace("personal".into()), cx)));
    assert_eq!(rows(&shell, cx), [p]);
    cx.update(|cx| c.update(cx, |v, cx| v.set_source(Source::All, cx)));
    // Unread: what was unread when shown this visit stays.
    cx.simulate_keystrokes(main, "u");
    assert_eq!(rows(&shell, cx), [a, p, w], "read while shown this visit: kept");
    cx.simulate_keystrokes(main, "escape");
    let n = push(cx, note(ToastKind::Info, "newer", Some("work")));
    cx.simulate_keystrokes(main, "ctrl-shift-n");
    cx.run_until_parked();
    assert_eq!(rows(&shell, cx), [n], "another visit: only the one unread");
    cx.simulate_keystrokes(main, "u");
    assert_eq!(rows(&shell, cx), [n, a, p, w]);
}

#[gpui::test]
fn mark_all_read_reaches_rows_not_shown(cx: &mut TestAppContext) {
    let shell = start(cx, "read-all");
    let main = main_window(&shell, cx);
    cx.simulate_window_resize(main, size(logical(900.), logical(400.)));
    for i in 0..60 {
        push(cx, note(ToastKind::Info, &format!("note {i}"), Some("work")));
    }
    cx.simulate_keystrokes(main, "ctrl-shift-n");
    cx.run_until_parked();
    let left = unread(cx);
    assert!(left > 0 && left < 60, "only the rows drawn are read: {left} left");
    // Read: only rows in view, not those drawn ahead below it.
    let c = center(&shell, cx);
    let ids = cx.read(|cx| c.read(cx).rows().to_vec());
    let mut vcx = VisualTestContext::from_window(main, cx);
    vcx.run_until_parked();
    for id in ids {
        let read = vcx.update(|_, cx| toasts(cx).read(cx).notes.get(id).map(|n| n.read)).unwrap();
        let below = vcx.debug_bounds(Box::leak(format!("note-{id}").into_boxed_str())).is_none_or(|b| b.top() >= logical(400.));
        assert!(!(read && below), "note {id} read without being in view");
    }
    cx.simulate_keystrokes(main, "r");
    cx.run_until_parked();
    assert_eq!(unread(cx), 0);
}

/// Opens the center scrolled down, with the selection at the bottom of the view.
fn scrolled(cx: &mut TestAppContext, tag: &str) -> (Entity<Shell>, AnyWindowHandle) {
    let shell = start(cx, tag);
    let main = main_window(&shell, cx);
    cx.simulate_window_resize(main, size(logical(900.), logical(400.)));
    for i in 0..40 {
        push(cx, note(ToastKind::Info, &format!("note {i}"), None));
    }
    cx.simulate_keystrokes(main, "ctrl-shift-n");
    cx.run_until_parked();
    cx.simulate_keystrokes(main, &["down"; 20].join(" "));
    cx.run_until_parked();
    (shell, main)
}

fn top(shell: &Entity<Shell>, cx: &mut TestAppContext) -> Option<u64> {
    let c = center(shell, cx);
    let main = main_window(shell, cx);
    VisualTestContext::from_window(main, cx).run_until_parked();
    cx.read(|cx| c.read(cx).top_row())
}

#[gpui::test]
fn a_notification_coming_keeps_the_scroll(cx: &mut TestAppContext) {
    let (shell, _) = scrolled(cx, "scroll-come");
    let before = top(&shell, cx);
    assert!(before.is_some() && before != rows(&shell, cx).first().copied(), "scrolled down");
    push(cx, note(ToastKind::Info, "newer", None));
    assert_eq!(top(&shell, cx), before, "the same row at the top");
}

#[gpui::test]
fn a_delete_keeps_the_scroll(cx: &mut TestAppContext) {
    let (shell, main) = scrolled(cx, "scroll-delete");
    let before = top(&shell, cx);
    cx.simulate_keystrokes(main, "delete");
    assert_eq!(top(&shell, cx), before, "the same row at the top");
}

/// Where the keys are in `deck`'s windows: the views shown in the panes that have them.
fn keys_on(shell: &Entity<Shell>, cx: &mut TestAppContext) -> Vec<Option<ViewRef>> {
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    cx.update(|cx| devdeck_app::window_view::keys(&deck, cx)).unwrap_or_default().into_iter().map(|h| h.view).collect()
}

#[gpui::test]
fn what_an_action_opens_takes_the_keys(cx: &mut TestAppContext) {
    let shell = start(cx, "focus");
    let main = main_window(&shell, cx);
    let problems = ViewAddr::Problems.to_ref();
    push(cx, note(ToastKind::Warn, "problems", None).note(notifications::problems_action("View")));
    cx.simulate_keystrokes(main, "ctrl-2");
    cx.simulate_keystrokes(main, "ctrl-shift-n");
    cx.simulate_keystrokes(main, "enter");
    cx.run_until_parked();
    assert_eq!(cover(cx), Cover::None);
    assert_eq!(keys_on(&shell, cx), [Some(problems.clone())], "the keys are in the opened view, not back in the sidebar");
    // In a workspace not shown.
    push(cx, note(ToastKind::Err, "personal's", Some("personal")).note(notifications::open_action("View", "personal", &problems)));
    cx.simulate_keystrokes(main, "ctrl-shift-n");
    cx.simulate_keystrokes(main, "enter");
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| shell.read(cx).workspaces[shell.read(cx).active].id.clone()), "personal");
    assert_eq!(cover(cx), Cover::None);
    assert_eq!(keys_on(&shell, cx), [Some(problems)], "there too");
}

#[gpui::test]
fn an_action_whose_workspace_is_gone_says_so_and_leaves_the_center(cx: &mut TestAppContext) {
    let shell = start(cx, "gone");
    let main = main_window(&shell, cx);
    let id = push(cx, note(ToastKind::Err, "elsewhere", Some("gone")).note(notifications::open_action("View", "gone", &ViewAddr::Problems.to_ref())));
    cx.simulate_keystrokes(main, "ctrl-shift-n");
    cx.simulate_keystrokes(main, "enter");
    cx.run_until_parked();
    assert_eq!(cover(cx), Cover::Notifications, "nothing to make way for");
    let said = cx.update(|cx| toasts(cx).read(cx).queue.items.iter().any(|t| t.text.as_ref() == "View: the workspace gone is gone"));
    assert!(said);
    assert_eq!(cx.update(|cx| toasts(cx).read(cx).notes.get(id).map(|n| n.read)), Some(true));
}

#[gpui::test]
fn every_action_of_a_row_is_reachable_by_keys(cx: &mut TestAppContext) {
    let shell = start(cx, "actions");
    let main = main_window(&shell, cx);
    let gone = notifications::open_action("Gone", "gone", &ViewAddr::Problems.to_ref());
    push(cx, note(ToastKind::Warn, "two", None).note(gone).note(notifications::problems_action("View")));
    cx.simulate_keystrokes(main, "ctrl-shift-n");
    let c = center(&shell, cx);
    cx.simulate_keystrokes(main, "right right");
    assert_eq!(cx.read(|cx| c.read(cx).action), 1, "stops at the last");
    cx.simulate_keystrokes(main, "left");
    assert_eq!(cx.read(|cx| c.read(cx).action), 0);
    cx.simulate_keystrokes(main, "right enter");
    cx.run_until_parked();
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    assert!(cx.read(|cx| deck.read(cx).layout().find_view(&ViewAddr::Problems.to_ref()).is_some()), "the second action ran");
    assert_eq!(cover(cx), Cover::None);
}

#[gpui::test]
fn keys_move_run_and_remove(cx: &mut TestAppContext) {
    let shell = start(cx, "keys");
    let main = main_window(&shell, cx);
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    let first = push(cx, note(ToastKind::Warn, "problems", None).note(notifications::problems_action("View")));
    let second = push(cx, note(ToastKind::Info, "plain", None));
    let third = push(cx, note(ToastKind::Info, "plain too", None));
    cx.simulate_keystrokes(main, "ctrl-shift-n");
    cx.run_until_parked();
    let c = center(&shell, cx);
    let selected = |cx: &TestAppContext| cx.read(|cx| c.read(cx).selected);
    assert_eq!(selected(cx), Some(third), "the newest");
    cx.simulate_keystrokes(main, "down down down");
    assert_eq!(selected(cx), Some(first), "stops at the last");
    cx.simulate_keystrokes(main, "up");
    assert_eq!(selected(cx), Some(second));
    cx.simulate_keystrokes(main, "delete");
    assert_eq!(rows(&shell, cx), [third, first]);
    assert_eq!(selected(cx), Some(first), "the next one is selected");
    cx.simulate_keystrokes(main, "enter");
    cx.run_until_parked();
    let problems = ViewAddr::Problems.to_ref();
    assert!(cx.read(|cx| deck.read(cx).layout().find_view(&problems).is_some()), "Enter ran its first action");
    assert_eq!(cover(cx), Cover::None, "and the center made way for what it opened");
    // Clear: what the filters show, the store only.
    cx.simulate_keystrokes(main, "ctrl-shift-n");
    cx.simulate_keystrokes(main, "shift-delete");
    assert!(cx.update(|cx| toasts(cx).read(cx).notes.is_empty()));
    assert_eq!(cx.update(|cx| toasts(cx).read(cx).queue.history.len()), 3, "the history stays");
}

#[gpui::test]
fn a_click_on_a_row_selects_it_and_its_action_runs(cx: &mut TestAppContext) {
    let shell = start(cx, "row-click");
    let main = main_window(&shell, cx);
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    let id = push(cx, note(ToastKind::Warn, "problems", None).note(notifications::problems_action("View")));
    push(cx, note(ToastKind::Info, "newer", None));
    cx.simulate_keystrokes(main, "ctrl-shift-n");
    cx.run_until_parked();
    click(main, format!("note-{id}"), cx);
    let c = center(&shell, cx);
    assert_eq!(cx.read(|cx| c.read(cx).selected), Some(id));
    click(main, format!("note-action-{id}-0"), cx);
    assert!(cx.read(|cx| deck.read(cx).layout().find_view(&ViewAddr::Problems.to_ref()).is_some()));
}

#[gpui::test]
fn a_toasts_text_opens_the_center_on_it(cx: &mut TestAppContext) {
    let shell = start(cx, "toast-text");
    let main = main_window(&shell, cx);
    let older = push(cx, note(ToastKind::Info, "older", None));
    let id = push(cx, note(ToastKind::Err, &"a long failure ".repeat(30), None));
    click(main, format!("toast-text-{id}"), cx);
    assert_eq!(cover(cx), Cover::Notifications);
    let c = center(&shell, cx);
    assert_eq!(cx.read(|cx| c.read(cx).selected), Some(id), "on it, not on the newest");
    assert!(rows(&shell, cx).contains(&older));
}

#[gpui::test]
fn a_detached_windows_bell_opens_the_center_in_the_main_window(cx: &mut TestAppContext) {
    let shell = start(cx, "detached");
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    let detached = cx.update(|cx| {
        deck.update(cx, |d, cx| {
            d.apply(cx, |l| {
                let (p, i) = l.find_view(&ViewRef::new("demo/notes")).expect("notes open");
                l.detach(p, i)
            })
        })
    });
    assert!(detached.is_some());
    cx.run_until_parked();
    let main = main_window(&shell, cx);
    let other = cx.windows().into_iter().find(|w| *w != main).expect("a detached window");
    push(cx, note(ToastKind::Err, "failed", None));
    assert!(drawn(other, "bell-count", cx), "the bell is in every window");
    click(other, "bell".into(), cx);
    assert_eq!(cover(cx), Cover::Notifications, "opened in the main window");
    assert!(drawn(main, "notifications", cx));
    assert!(!drawn(other, "notifications", cx));
}

fn agent() -> Who {
    Who::Terminal { workspace: "work".into(), n: 3, uid: "uid-3".into(), shell: 3, shell_start: 1, starts: 1 }
}

fn ui_toasts(cx: &mut TestAppContext) -> Toasts {
    let who = agent();
    match cx.update(|cx| dispatch::answer(&CallCx { who: &who, instance: "i", state: "demo", conn: None }, "ui.toasts", json!({}), cx)) {
        Answer::Now(r) => serde_json::from_value::<Toasts>(r.expect("answered")).unwrap(),
        Answer::Later(_) => panic!("ui.toasts answers at once"),
    }
}

#[gpui::test]
fn the_apis_toasts_are_unchanged_by_the_center_and_carry_the_title(cx: &mut TestAppContext) {
    let shell = start(cx, "api");
    let main = main_window(&shell, cx);
    let plain = push(cx, note(ToastKind::Info, "plain", None));
    let titled = push(cx, note(ToastKind::Ok, "rolled out", Some("work")).title("graphservice on dev35"));
    let before = ui_toasts(cx);
    let t = before.toasts.iter().find(|t| t.id == titled).expect("listed");
    assert_eq!(t.title.as_ref().map(|t| t.as_str()), Some("graphservice on dev35"));
    let p = before.toasts.iter().find(|t| t.id == plain).expect("listed");
    assert_eq!(p.title, None);
    let raw = serde_json::to_value(p).unwrap();
    assert_eq!(raw, json!({"id": plain, "kind": "info", "text": "plain"}), "no title: the item as it always was");
    cx.simulate_keystrokes(main, "ctrl-shift-n");
    cx.simulate_keystrokes(main, "shift-delete");
    assert!(cx.update(|cx| toasts(cx).read(cx).notes.is_empty()), "the center is empty");
    assert_eq!(ui_toasts(cx), before, "ui.toasts as it was");
}
