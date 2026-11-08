//! Notes (R12.3, R12.4): made from the app view, typed into, saved by themselves; kept
//! when their tab closes; renamed and deleted from their row; app keys stay out of the
//! note editor while pane keys still work.

use devdeck_app::deck::Deck;
use devdeck_app::notes::{Notes, SAVE_AFTER};
use devdeck_app::shell::Shell;
use devdeck_app::views::app_view::AppView;
use devdeck_app::views::note_view::NoteView;
use devdeck_core::layout::{Layout, ViewRef};
use devdeck_core::notes::NotesStore;
use devdeck_core::view_addr::ViewAddr;
use gpui::{Entity, Focusable, Modifiers, TestAppContext, VisualTestContext};
use std::path::PathBuf;

const APP: &str = "entityservice";

struct Setup {
    shell: Entity<Shell>,
    deck: Entity<Deck>,
    notes: Entity<Notes>,
    store: NotesStore,
}

fn app_layout() -> Layout {
    let mut l = Layout::new();
    let a = l.main().focused;
    l.open(ViewAddr::App { app: APP.into() }.to_ref(), a).expect("open the app");
    l
}

fn setup(cx: &mut TestAppContext) -> Setup {
    let dir = cx.update(|cx| devdeck_app::test_dir("notes", cx));
    cx.update(|cx| devdeck_app::init_with_config_dir(&dir, cx));
    let shell = cx.update(|cx| Shell::start_single(app_layout, cx));
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    let notes = cx.read(|cx| deck.read(cx).services().unwrap().read(cx).notes.clone());
    let store = NotesStore::new(dir.join("data/notes"));
    let s = store.clone();
    cx.update(|cx| notes.update(cx, |n, cx| n.set_store(s, cx)));
    cx.run_until_parked();
    Setup { shell, deck, notes, store }
}

fn window(cx: &mut TestAppContext) -> VisualTestContext {
    let w = cx.read(|cx| cx.windows()[0]);
    let vcx = VisualTestContext::from_window(w, cx);
    vcx.run_until_parked();
    vcx
}

fn note_ref(id: &str) -> ViewRef {
    ViewAddr::Note { id: id.into() }.to_ref()
}

fn app_view(s: &Setup, vcx: &mut VisualTestContext) -> Entity<AppView> {
    vcx.read(|cx| s.deck.read(cx).view(&ViewAddr::App { app: APP.into() }.to_ref()).unwrap().downcast::<AppView>().unwrap())
}

fn note_view(s: &Setup, id: &str, vcx: &mut VisualTestContext) -> Entity<NoteView> {
    vcx.read(|cx| s.deck.read(cx).view(&note_ref(id)).expect("open").downcast::<NoteView>().unwrap())
}

fn only_note(s: &Setup, vcx: &mut VisualTestContext) -> String {
    let notes = vcx.read(|cx| s.notes.read(cx).of(APP));
    assert_eq!(notes.len(), 1, "{notes:?}");
    notes[0].id.clone()
}

fn file(s: &Setup, id: &str) -> PathBuf {
    s.store.file(id)
}

/// Makes a note named `name` with the app view's button and the name prompt.
fn make(s: &Setup, name: &str, vcx: &mut VisualTestContext) -> String {
    let button = vcx.debug_bounds("new-note").expect("the button");
    vcx.simulate_click(button.center(), Modifiers::none());
    vcx.simulate_input(name);
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    only_note(s, vcx)
}

fn editor_focused(v: &Entity<NoteView>, vcx: &mut VisualTestContext) -> bool {
    vcx.update(|window, cx| v.read(cx).code.as_ref().is_some_and(|c| c.read(cx).focus_handle(cx).is_focused(window)))
}

#[gpui::test]
fn a_new_note_opens_as_source_and_saves_by_itself(cx: &mut TestAppContext) {
    let s = setup(cx);
    let mut vcx = window(cx);
    let id = make(&s, "todo", &mut vcx);
    let v = note_view(&s, &id, &mut vcx);
    assert!(!vcx.read(|cx| v.read(cx).rendered), "empty: the source");
    assert!(editor_focused(&v, &mut vcx), "ready to type");
    assert_eq!(vcx.read(|cx| s.deck.read(cx).title(&note_ref(&id), cx)), "entityservice · todo");
    vcx.simulate_input("- [ ] fix the zoom");
    vcx.run_until_parked();
    assert_eq!(std::fs::read_to_string(file(&s, &id)).unwrap(), "", "not yet");
    vcx.executor().advance_clock(SAVE_AFTER);
    vcx.run_until_parked();
    assert_eq!(std::fs::read_to_string(file(&s, &id)).unwrap(), "- [ ] fix the zoom");
}

#[gpui::test]
fn closing_the_tab_before_the_write_loses_nothing(cx: &mut TestAppContext) {
    let s = setup(cx);
    let mut vcx = window(cx);
    let id = make(&s, "todo", &mut vcx);
    vcx.simulate_input("typed fast");
    let r = note_ref(&id);
    vcx.update(|_, cx| s.deck.update(cx, |d, cx| d.apply(cx, |l| l.close_view(&r))));
    vcx.run_until_parked();
    assert!(vcx.read(|cx| s.deck.read(cx).view(&note_ref(&id)).is_none()), "the tab is gone");
    // Open it again from its row.
    let row = vcx.debug_bounds(Box::leak(format!("note-{id}").into_boxed_str())).expect("its row");
    vcx.simulate_click(row.center(), Modifiers::none());
    vcx.run_until_parked();
    let v = note_view(&s, &id, &mut vcx);
    assert!(vcx.read(|cx| v.read(cx).rendered), "it has text: rendered");
    vcx.executor().advance_clock(SAVE_AFTER);
    vcx.run_until_parked();
    assert_eq!(std::fs::read_to_string(file(&s, &id)).unwrap(), "typed fast");
}

#[gpui::test]
fn the_shells_flush_writes_at_once(cx: &mut TestAppContext) {
    let s = setup(cx);
    let mut vcx = window(cx);
    let id = make(&s, "todo", &mut vcx);
    vcx.simulate_input("before quitting");
    vcx.update(|_, cx| s.shell.update(cx, |sh, cx| sh.flush(cx)));
    assert_eq!(std::fs::read_to_string(file(&s, &id)).unwrap(), "before quitting");
}

#[gpui::test]
fn rename_and_delete_from_the_row(cx: &mut TestAppContext) {
    let s = setup(cx);
    let mut vcx = window(cx);
    let id = make(&s, "todo", &mut vcx);
    // The app view comes back in front of the note.
    let app = ViewAddr::App { app: APP.into() }.to_ref();
    vcx.update(|_, cx| s.deck.update(cx, |d, cx| d.apply(cx, |l| { let (p, i) = l.find_view(&app).unwrap(); l.activate(p, i) })));
    vcx.run_until_parked();
    let w = vcx.update(|window, _| window.window_handle());
    let open_row_menu = |vcx: &mut VisualTestContext| {
        let row = vcx.debug_bounds(Box::leak(format!("note-{id}").into_boxed_str())).expect("its row");
        vcx.simulate_mouse_down(row.center(), gpui::MouseButton::Right, Modifiers::none());
        vcx.run_until_parked();
        vcx.update(|_, cx| devdeck_ui::overlay::layer(w, cx).read(cx).menu.clone()).expect("a menu")
    };
    let menu = open_row_menu(&mut vcx);
    let labels: Vec<String> = vcx.read(|cx| menu.read(cx).items().iter().map(|i| i.label.to_string()).collect());
    assert_eq!(labels, ["Open", "Rename…", "Delete…"]);
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    assert!(vcx.update(|_, cx| devdeck_ui::overlay::layer(w, cx).read(cx).menu.is_none()), "the menu closed");

    vcx.update(|_, cx| s.notes.update(cx, |n, cx| n.rename(&id, "bugs", cx)));
    vcx.run_until_parked();
    assert_eq!(vcx.read(|cx| s.deck.read(cx).title(&note_ref(&id), cx)), "entityservice · bugs", "the tab follows");
    assert_eq!(s.store.list().unwrap()[0].name, "bugs");

    // Delete asks first.
    let id2 = id.clone();
    vcx.update(|window, cx| devdeck_app::views::note_view::delete_note(&s.notes, &id2, window, cx));
    vcx.run_until_parked();
    assert!(file(&s, &id).exists(), "not before the answer");
    let confirm = vcx.debug_bounds("delete-note-confirm").expect("asks");
    vcx.simulate_click(confirm.center(), Modifiers::none());
    vcx.run_until_parked();
    assert!(!file(&s, &id).exists());
    assert!(vcx.read(|cx| s.notes.read(cx).of(APP)).is_empty());
    assert!(vcx.read(|cx| s.deck.read(cx).layout().find_view(&note_ref(&id)).is_none()), "its tab closed");
}

#[gpui::test]
fn app_keys_stay_out_of_the_note_but_pane_keys_work(cx: &mut TestAppContext) {
    let s = setup(cx);
    let mut vcx = window(cx);
    let id = make(&s, "todo", &mut vcx);
    vcx.simulate_keystrokes("? n s r");
    vcx.run_until_parked();
    assert_eq!(vcx.update(|_, cx| s.notes.update(cx, |n, _| n.text(&id))), "?nsr", "typed, not run");
    // Esc gives the keys back to the view.
    let v = note_view(&s, &id, &mut vcx);
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    assert!(!editor_focused(&v, &mut vcx), "left the editor");
    // ctrl+w still closes the tab from inside the editor.
    vcx.update(|window, cx| {
        let f = v.read(cx).focus_handle(cx);
        f.focus(window, cx);
    });
    vcx.run_until_parked();
    vcx.simulate_keystrokes("ctrl-w");
    vcx.run_until_parked();
    assert!(vcx.read(|cx| s.deck.read(cx).layout().find_view(&note_ref(&id)).is_none()), "ctrl+w closed it");
}

#[gpui::test]
fn up_and_down_reach_the_notes(cx: &mut TestAppContext) {
    let s = setup(cx);
    let mut vcx = window(cx);
    let id = vcx.update(|_, cx| s.notes.update(cx, |n, cx| n.create(APP, "todo", cx))).unwrap();
    let v = app_view(&s, &mut vcx);
    vcx.update(|window, cx| {
        let f = v.read(cx).focus_handle(cx);
        f.focus(window, cx);
    });
    vcx.run_until_parked();
    for _ in 0..50 {
        vcx.simulate_keystrokes("down");
    }
    vcx.run_until_parked();
    assert_eq!(vcx.read(|cx| v.read(cx).selected.clone()), Some(format!("note:{id}")), "the last row is the note");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert!(vcx.read(|cx| s.deck.read(cx).view(&note_ref(&id)).is_some()), "⏎ opens it");
}

#[gpui::test]
fn the_source_or_rendered_switch(cx: &mut TestAppContext) {
    let s = setup(cx);
    let mut vcx = window(cx);
    let id = make(&s, "todo", &mut vcx);
    vcx.simulate_input("# Title");
    let v = note_view(&s, &id, &mut vcx);
    vcx.simulate_keystrokes("ctrl-shift-e");
    vcx.run_until_parked();
    assert!(vcx.read(|cx| v.read(cx).rendered));
    assert!(vcx.debug_bounds("note-rendered").is_some());
    vcx.simulate_keystrokes("ctrl-shift-e");
    vcx.run_until_parked();
    assert!(!vcx.read(|cx| v.read(cx).rendered));
    assert!(editor_focused(&v, &mut vcx), "back in the text");
}

#[gpui::test]
fn a_deleted_note_in_a_saved_layout_says_so(cx: &mut TestAppContext) {
    let s = setup(cx);
    let mut vcx = window(cx);
    let r = note_ref("not-a-note");
    vcx.update(|_, cx| s.deck.update(cx, |d, cx| d.apply(cx, |l| { let p = l.main().focused; l.open(r.clone(), p).map(|_| ()) })));
    vcx.run_until_parked();
    let v = vcx.read(|cx| s.deck.read(cx).view(&note_ref("not-a-note")).unwrap());
    assert!(v.downcast::<NoteView>().is_err(), "a message stands in");
}

#[gpui::test]
fn a_failed_write_keeps_the_text_and_says_so(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt;
    let s = setup(cx);
    let mut vcx = window(cx);
    let id = make(&s, "todo", &mut vcx);
    let dir = s.store.dir.clone();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    vcx.simulate_input("kept");
    vcx.executor().advance_clock(SAVE_AFTER);
    vcx.run_until_parked();
    let unsaved = vcx.read(|cx| s.notes.read(cx).is_unsaved(&id));
    let toasts: Vec<String> = vcx.update(|_, cx| devdeck_ui::toasts(cx).read(cx).queue.items.iter().map(|t| t.text.to_string()).collect());
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(unsaved, "still to write");
    assert!(toasts.iter().any(|t| t.contains("couldn't be saved")), "{toasts:?}");
    assert_eq!(vcx.update(|_, cx| s.notes.update(cx, |n, _| n.text(&id))), "kept");
    vcx.update(|_, cx| s.notes.update(cx, |n, cx| n.flush(cx)));
    assert_eq!(std::fs::read_to_string(file(&s, &id)).unwrap(), "kept", "written once it can be");
}
