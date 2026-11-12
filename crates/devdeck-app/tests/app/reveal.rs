//! Notifications step B: `Shell::reveal` brings a view on screen where it is (the
//! workspace, the layout, the tab, a maximized or collapsed pane, a detached window),
//! flashes it, or opens it here when it's open nowhere (R2.2, R3.8, R5.1, R1.3); nothing
//! when its workspace or its target is gone, or while a dialog is open.

use devdeck_app::app_settings::{self, Dialog};
use devdeck_app::deck::{Deck, FLASH, NotRevealed, Revealed};
use devdeck_app::notifications;
use devdeck_app::shell::Shell;
use devdeck_app::window_view::WindowView;
use devdeck_core::layout::{Layout, PaneId, ViewRef, WindowId};
use devdeck_core::view_addr::ViewAddr;
use devdeck_ui::overlay::{layer_state, pop_modal, push_modal};
use devdeck_ui::{MenuItem, logical, open_menu, toasts};
use gpui::{AnyWindowHandle, AppContext, Entity, TestAppContext, point};

fn start(cx: &mut TestAppContext, tag: &str) -> Entity<Shell> {
    let config = cx.update(|cx| devdeck_app::test_dir(&format!("{tag}-config"), cx));
    cx.update(|cx| devdeck_app::init_with_config_dir(&config, cx));
    let shell = cx.update(|cx| Shell::start(devdeck_app::demo_workspaces(), "work", None, cx));
    cx.update(|cx| shell.update(cx, |s, _| s.quit_for_real = false));
    cx.run_until_parked();
    shell
}

fn deck(shell: &Entity<Shell>, cx: &TestAppContext, id: &str) -> Entity<Deck> {
    cx.read(|cx| shell.read(cx).workspaces.iter().find(|w| w.id == id).expect("workspace").deck.clone())
}

fn active_id(shell: &Entity<Shell>, cx: &TestAppContext) -> String {
    cx.read(|cx| shell.read(cx).workspaces[shell.read(cx).active].id.clone())
}

fn reveal(shell: &Entity<Shell>, cx: &mut TestAppContext, ws: &str, view: &str) -> Result<Revealed, NotRevealed> {
    let out = cx.update(|cx| shell.update(cx, |s, cx| s.reveal(ws, &ViewRef::new(view), cx)));
    cx.run_until_parked();
    out
}

fn layout(deck: &Entity<Deck>, cx: &TestAppContext) -> Layout {
    cx.read(|cx| deck.read(cx).layout().clone())
}

fn pane_of(deck: &Entity<Deck>, cx: &TestAppContext, view: &str) -> PaneId {
    layout(deck, cx).find_view(&ViewRef::new(view)).expect("open").0
}

fn change(deck: &Entity<Deck>, cx: &mut TestAppContext, op: impl FnOnce(&mut Layout) -> devdeck_core::layout::Result<()>) {
    cx.update(|cx| deck.update(cx, |d, cx| d.apply(cx, op))).expect("applied");
    cx.run_until_parked();
}

/// The view is its pane's active tab, the pane is its window's focused one, and the
/// view is on screen.
fn shown(deck: &Entity<Deck>, cx: &TestAppContext, view: &str) -> bool {
    let r = ViewRef::new(view);
    let l = layout(deck, cx);
    let Some((p, _)) = l.find_view(&r) else { return false };
    let w = l.window_of(p).unwrap();
    l.pane(p).unwrap().active_view() == Some(&r) && l.window(w).unwrap().focused == p && cx.read(|cx| deck.read(cx).is_visible(&r))
}

/// The views the keys are in, in `deck`'s windows.
fn keys_on(deck: &Entity<Deck>, cx: &mut TestAppContext) -> Vec<Option<ViewRef>> {
    cx.update(|cx| devdeck_app::window_view::keys(deck, cx)).unwrap_or_default().into_iter().map(|h| h.view).collect()
}

fn flashing(deck: &Entity<Deck>, cx: &TestAppContext) -> Option<ViewRef> {
    cx.read(|cx| deck.read(cx).flash.as_ref().map(|f| f.view.clone()))
}

fn main_window(shell: &Entity<Shell>, cx: &TestAppContext) -> AnyWindowHandle {
    cx.read(|cx| shell.read(cx).main_window().expect("main").into())
}

#[gpui::test]
fn in_the_current_layout_its_tab_is_activated_focused_and_flashes_once(cx: &mut TestAppContext) {
    let shell = start(cx, "reveal-here");
    let work = deck(&shell, cx, "work");
    // The demo's first pane shows logs; gallery is a tab behind it, matrix's pane focused.
    let matrix = pane_of(&work, cx, "matrix");
    change(&work, cx, |l| l.focus(matrix));
    assert_eq!(reveal(&shell, cx, "work", "demo/gallery"), Ok(Revealed::Shown));
    assert!(shown(&work, cx, "demo/gallery"));
    assert_eq!(keys_on(&work, cx), [Some(ViewRef::new("demo/gallery"))], "the keys are in it");
    assert_eq!(flashing(&work, cx), Some(ViewRef::new("demo/gallery")));
    let first = cx.read(|cx| work.read(cx).flash.as_ref().unwrap().n);
    // A second reveal flashes again, under a new number.
    reveal(&shell, cx, "work", "demo/gallery").unwrap();
    assert!(cx.read(|cx| work.read(cx).flash.as_ref().unwrap().n) > first, "a fresh id per flash");
    cx.executor().advance_clock(FLASH / 2);
    cx.run_until_parked();
    assert!(flashing(&work, cx).is_some(), "still flashing");
    cx.executor().advance_clock(FLASH);
    cx.run_until_parked();
    assert_eq!(flashing(&work, cx), None, "over after its timer: nothing left running");
}

#[gpui::test]
fn in_another_layout_it_switches_there_and_the_layout_left_loses_nothing(cx: &mut TestAppContext) {
    let shell = start(cx, "reveal-layout");
    let work = deck(&shell, cx, "work");
    // "default" holds the demo's tabs; "full" a copy of it; "sparse" only the notes.
    let full = cx.update(|cx| work.update(cx, |d, cx| d.save_layout_as("full", cx))).unwrap();
    let sparse = cx.update(|cx| work.update(cx, |d, cx| d.save_layout_as("sparse", cx))).unwrap();
    change(&work, cx, |l| {
        for v in ["demo/logs", "demo/long", "matrix", "demo/gallery"] {
            l.close_view(&ViewRef::new(v))?;
        }
        Ok(())
    });
    let left = layout(&work, cx);
    assert!(cx.read(|cx| shell.read(cx).holds("work", &ViewRef::new("demo/gallery"), cx)), "held by the saved layout");
    assert_eq!(reveal(&shell, cx, "work", "demo/gallery"), Ok(Revealed::Shown));
    let current = cx.read(|cx| work.read(cx).book.current.clone());
    assert_eq!(current, devdeck_core::layout_book::DEFAULT_ID, "the first layout holding it by name");
    assert_ne!(current, full);
    assert!(shown(&work, cx, "demo/gallery"));
    // Back to the layout left: its tabs as they were.
    cx.update(|cx| work.update(cx, |d, cx| d.switch_layout(&sparse, cx))).unwrap();
    cx.run_until_parked();
    assert_eq!(layout(&work, cx), left);
}

#[gpui::test]
fn in_another_workspace_it_shows_that_workspace(cx: &mut TestAppContext) {
    let shell = start(cx, "reveal-workspace");
    assert_eq!(active_id(&shell, cx), "work");
    let personal = deck(&shell, cx, "personal");
    assert_eq!(reveal(&shell, cx, "personal", "demo/notes"), Ok(Revealed::Shown));
    assert_eq!(active_id(&shell, cx), "personal");
    assert!(shown(&personal, cx, "demo/notes"));
    assert_eq!(flashing(&personal, cx), Some(ViewRef::new("demo/notes")));
}

#[gpui::test]
fn a_maximized_pane_hiding_it_is_restored(cx: &mut TestAppContext) {
    let shell = start(cx, "reveal-maximized");
    let work = deck(&shell, cx, "work");
    let matrix = pane_of(&work, cx, "matrix");
    change(&work, cx, |l| l.toggle_maximize(matrix));
    assert!(!cx.read(|cx| work.read(cx).is_visible(&ViewRef::new("demo/logs"))));
    reveal(&shell, cx, "work", "demo/gallery").unwrap();
    assert_eq!(layout(&work, cx).main().maximized, None);
    assert!(shown(&work, cx, "demo/gallery"));
}

#[gpui::test]
fn a_collapsed_pane_is_expanded(cx: &mut TestAppContext) {
    let shell = start(cx, "reveal-collapsed");
    let work = deck(&shell, cx, "work");
    let logs = pane_of(&work, cx, "demo/logs");
    change(&work, cx, |l| l.toggle_collapse(logs));
    reveal(&shell, cx, "work", "demo/gallery").unwrap();
    assert!(!layout(&work, cx).pane(logs).unwrap().collapsed);
    assert!(shown(&work, cx, "demo/gallery"));
}

#[gpui::test]
fn a_detached_window_comes_to_the_front(cx: &mut TestAppContext) {
    let shell = start(cx, "reveal-detached");
    let work = deck(&shell, cx, "work");
    let notes = pane_of(&work, cx, "demo/notes");
    let w: WindowId = cx.update(|cx| work.update(cx, |d, cx| d.apply(cx, |l| l.detach(notes, 0)))).expect("detached");
    cx.run_until_parked();
    let detached: AnyWindowHandle = cx.read(|cx| work.read(cx).os_window(w)).expect("its window opened").into();
    let main = main_window(&shell, cx);
    cx.update_window(main, |_, window, _| window.activate_window()).unwrap();
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| cx.active_window()), Some(main));
    assert_eq!(reveal(&shell, cx, "work", "demo/notes"), Ok(Revealed::Shown));
    assert_eq!(cx.update(|cx| cx.active_window()), Some(detached), "the detached window is in front");
    assert!(shown(&work, cx, "demo/notes"));
}

#[gpui::test]
fn open_nowhere_it_opens_in_the_focused_pane(cx: &mut TestAppContext) {
    let shell = start(cx, "reveal-nowhere");
    let work = deck(&shell, cx, "work");
    let long = ViewRef::new("demo/long");
    let label = |cx: &mut TestAppContext| cx.update(|cx| notifications::reveal_action("work", &long, cx).label.to_string());
    assert_eq!(label(cx), "Open");
    change(&work, cx, |l| l.close_view(&ViewRef::new("demo/long")));
    assert_eq!(label(cx), "Open here");
    let focused = layout(&work, cx).main().focused;
    assert_eq!(reveal(&shell, cx, "work", "demo/long"), Ok(Revealed::OpenedHere));
    assert_eq!(pane_of(&work, cx, "demo/long"), focused);
    assert!(shown(&work, cx, "demo/long"));
    assert_eq!(flashing(&work, cx), Some(ViewRef::new("demo/long")));
}

#[gpui::test]
fn a_workspace_gone_does_nothing(cx: &mut TestAppContext) {
    let shell = start(cx, "reveal-gone");
    let work = deck(&shell, cx, "work");
    let before = layout(&work, cx);
    assert_eq!(reveal(&shell, cx, "gone", "demo/notes"), Err(NotRevealed::WorkspaceGone));
    assert_eq!(active_id(&shell, cx), "work");
    assert_eq!(layout(&work, cx), before);
    assert_eq!(flashing(&work, cx), None);
}

fn main_handle(shell: &Entity<Shell>, cx: &TestAppContext) -> gpui::WindowHandle<WindowView> {
    cx.read(|cx| shell.read(cx).main_window().expect("main"))
}

/// What has the keys in the main window: its focus handle, compared before and after.
fn focused(shell: &Entity<Shell>, cx: &mut TestAppContext) -> Option<gpui::FocusHandle> {
    let main = main_window(shell, cx);
    cx.update_window(main, |_, window, cx| window.focused(cx)).unwrap()
}

#[gpui::test]
fn the_keys_reach_the_view_after_a_workspace_switch(cx: &mut TestAppContext) {
    let shell = start(cx, "reveal-keys-ws");
    let main = main_window(&shell, cx);
    cx.simulate_keystrokes(main, "ctrl-2");
    reveal(&shell, cx, "personal", "demo/gallery").unwrap();
    let personal = deck(&shell, cx, "personal");
    assert_eq!(keys_on(&personal, cx), [Some(ViewRef::new("demo/gallery"))], "not back where the keys were in personal");
}

#[gpui::test]
fn the_keys_reach_a_view_opened_here(cx: &mut TestAppContext) {
    let shell = start(cx, "reveal-keys-here");
    let work = deck(&shell, cx, "work");
    change(&work, cx, |l| l.close_view(&ViewRef::new("demo/long")));
    cx.simulate_keystrokes(main_window(&shell, cx), "ctrl-2");
    assert_eq!(reveal(&shell, cx, "work", "demo/long"), Ok(Revealed::OpenedHere));
    assert_eq!(keys_on(&work, cx), [Some(ViewRef::new("demo/long"))]);
}

/// Detaches `view` of `deck` to its own window; returns it and its OS window.
fn detach(deck: &Entity<Deck>, cx: &mut TestAppContext, view: &str) -> (WindowId, AnyWindowHandle) {
    let p = pane_of(deck, cx, view);
    let i = layout(deck, cx).pane(p).unwrap().tabs.iter().position(|t| t.as_str() == view).unwrap();
    let w: WindowId = cx.update(|cx| deck.update(cx, |d, cx| d.apply(cx, |l| l.detach(p, i)))).expect("detached");
    cx.run_until_parked();
    let h: AnyWindowHandle = cx.read(|cx| deck.read(cx).os_window(w)).expect("its window opened").into();
    (w, h)
}

#[gpui::test]
fn the_keys_reach_the_view_in_a_detached_window(cx: &mut TestAppContext) {
    let shell = start(cx, "reveal-keys-detached");
    let work = deck(&shell, cx, "work");
    let (w, h) = detach(&work, cx, "demo/notes");
    cx.simulate_keystrokes(main_window(&shell, cx), "ctrl-2");
    cx.update_window(h, |_, window, cx| window.blur(cx)).unwrap();
    reveal(&shell, cx, "work", "demo/notes").unwrap();
    let held = cx.update(|cx| devdeck_app::window_view::keys(&work, cx)).unwrap();
    assert!(held.iter().any(|k| k.window == w && k.view == Some(ViewRef::new("demo/notes"))), "{held:?}");
}

#[gpui::test]
fn a_detached_window_of_another_layout_opens_in_front(cx: &mut TestAppContext) {
    let shell = start(cx, "reveal-layout-detached");
    let work = deck(&shell, cx, "work");
    detach(&work, cx, "demo/notes");
    cx.update(|cx| work.update(cx, |d, cx| d.save_layout_as("with notes", cx))).unwrap();
    let bare = cx.update(|cx| work.update(cx, |d, cx| d.save_layout_as("bare", cx))).unwrap();
    // "bare": its notes window closed; "default" and "with notes" keep theirs.
    let w = layout(&work, cx).windows[1].id;
    cx.update(|cx| work.update(cx, |d, cx| d.close_window(w, cx)));
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| work.read(cx).book.current.clone()), bare);
    assert_eq!(cx.update(|cx| cx.windows().len()), 1, "the main window only");
    assert_eq!(reveal(&shell, cx, "work", "demo/notes"), Ok(Revealed::Shown));
    let l = layout(&work, cx);
    let w = l.window_of(pane_of(&work, cx, "demo/notes")).unwrap();
    assert!(l.is_detached(w), "in its window, as that layout has it");
    let h: AnyWindowHandle = cx.read(|cx| work.read(cx).os_window(w)).expect("opened").into();
    assert_eq!(cx.update(|cx| cx.active_window()), Some(h), "in front");
    assert!(shown(&work, cx, "demo/notes"));
}

#[gpui::test]
fn the_center_or_settings_make_way(cx: &mut TestAppContext) {
    let shell = start(cx, "reveal-cover");
    let main = main_window(&shell, cx);
    let work = deck(&shell, cx, "work");
    let h = main_handle(&shell, cx);
    cx.update(|cx| h.update(cx, |w, window, cx| w.open_notifications(None, window, cx))).unwrap();
    cx.run_until_parked();
    assert_eq!(cx.read(app_settings::dialog), Dialog::Notifications);
    reveal(&shell, cx, "work", "demo/gallery").unwrap();
    assert_eq!(cx.read(app_settings::dialog), Dialog::None);
    assert!(shown(&work, cx, "demo/gallery"));
    cx.simulate_keystrokes(main, "ctrl-,");
    assert_eq!(cx.read(app_settings::dialog), Dialog::Settings);
    reveal(&shell, cx, "work", "demo/long").unwrap();
    assert_eq!(cx.read(app_settings::dialog), Dialog::None);
    assert!(shown(&work, cx, "demo/long"));
    assert_eq!(keys_on(&work, cx), [Some(ViewRef::new("demo/long"))]);
}

#[gpui::test]
fn a_dialog_open_refuses_it_and_keeps_everything(cx: &mut TestAppContext) {
    let shell = start(cx, "reveal-modal");
    let main = main_window(&shell, cx);
    let work = deck(&shell, cx, "work");
    cx.update_window(main, |_, window, cx| {
        let dialog = cx.new(|_| devdeck_ui::Empty);
        push_modal(dialog.into(), window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    let (before, keys) = (layout(&work, cx), focused(&shell, cx));
    assert_eq!(reveal(&shell, cx, "personal", "demo/gallery"), Err(NotRevealed::Modal));
    assert_eq!(active_id(&shell, cx), "work", "no workspace switch");
    assert_eq!(reveal(&shell, cx, "work", "demo/gallery"), Err(NotRevealed::Modal));
    assert_eq!(layout(&work, cx), before);
    assert_eq!(focused(&shell, cx), keys, "the dialog keeps the keys");
    assert_eq!(flashing(&work, cx), None);
    // Through a notification: refused (the toast stays), and the user is told.
    toasts_clear(cx);
    let open = notifications::open_action("Open", "work", &ViewRef::new("demo/gallery"));
    let done = cx.update_window(main, |_, window, cx| notifications::resolve(&open, window, cx)).unwrap();
    cx.run_until_parked();
    assert!(!done, "refused");
    assert!(said(cx, "Open: close the dialog first"));
    cx.update_window(main, |_, window, cx| pop_modal(window, cx)).unwrap();
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| layer_state(main, cx)).1, 0);
    assert_eq!(reveal(&shell, cx, "work", "demo/gallery"), Ok(Revealed::Shown));
}

fn toasts_clear(cx: &mut TestAppContext) {
    cx.update(|cx| {
        toasts(cx).update(cx, |t, cx| {
            t.queue.items.clear();
            cx.notify();
        })
    });
}

fn said(cx: &mut TestAppContext, text: &str) -> bool {
    cx.update(|cx| toasts(cx).read(cx).queue.items.iter().any(|t| t.text.as_ref() == text))
}

#[gpui::test]
fn a_menu_open_doesnt_stop_it(cx: &mut TestAppContext) {
    let shell = start(cx, "reveal-menu");
    let main = main_window(&shell, cx);
    let work = deck(&shell, cx, "work");
    cx.update_window(main, |_, window, cx| open_menu(point(logical(100.), logical(100.)), vec![MenuItem::new("one", |_, _| {})], window, cx)).unwrap();
    cx.run_until_parked();
    assert!(cx.update(|cx| layer_state(main, cx)).0, "the menu is open");
    assert_eq!(reveal(&shell, cx, "work", "demo/gallery"), Ok(Revealed::Shown));
    assert!(shown(&work, cx, "demo/gallery"));
}

#[gpui::test]
fn a_view_whose_target_is_gone_does_nothing(cx: &mut TestAppContext) {
    let shell = start(cx, "reveal-view-gone");
    let work = deck(&shell, cx, "work");
    cx.update(|cx| shell.update(cx, |s, cx| s.switch(0, cx)));
    cx.run_until_parked();
    assert_eq!(active_id(&shell, cx), "personal");
    let before = layout(&work, cx);
    let gone = ViewAddr::Run { app: "nosuchapp".into(), cmd: "run".into() }.to_ref();
    let out = cx.update(|cx| shell.update(cx, |s, cx| s.reveal("work", &gone, cx)));
    cx.run_until_parked();
    assert!(matches!(out, Err(NotRevealed::ViewGone(_))), "{out:?}");
    assert_eq!(active_id(&shell, cx), "personal", "no switch");
    assert_eq!(layout(&work, cx), before, "nothing opened");
    // Through a notification: nothing said beyond the log, the notification used.
    toasts_clear(cx);
    let main = main_window(&shell, cx);
    let open = notifications::open_action("Open", "work", &gone);
    let done = cx.update_window(main, |_, window, cx| notifications::resolve(&open, window, cx)).unwrap();
    cx.run_until_parked();
    assert!(done);
    assert!(cx.update(|cx| toasts(cx).read(cx).queue.items.is_empty()), "no \"workspace gone\" toast");
}

#[gpui::test]
fn the_problems_action_reveals_the_problems_of_the_workspace_shown(cx: &mut TestAppContext) {
    let shell = start(cx, "reveal-problems");
    let main = main_window(&shell, cx);
    cx.update(|cx| shell.update(cx, |s, cx| s.switch(0, cx)));
    cx.run_until_parked();
    let personal = deck(&shell, cx, "personal");
    let action = notifications::problems_action("View");
    let done = cx.update_window(main, |_, window, cx| notifications::resolve(&action, window, cx)).unwrap();
    cx.run_until_parked();
    assert!(done);
    let problems = ViewAddr::Problems.to_ref();
    assert!(shown(&personal, cx, problems.as_str()));
    assert_eq!(flashing(&personal, cx), Some(problems));
}
