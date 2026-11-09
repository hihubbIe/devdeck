//! The sidebar's Notes list (R12.3): one level of projects holding their notes, then the
//! notes without a project; made, opened, pinned, renamed and deleted from there.

use devdeck_app::deck::Deck;
use devdeck_app::notes::Notes;
use devdeck_app::shell::Shell;
use devdeck_app::sidebar::{Activity, Effect, Sidebar, Target};
use devdeck_core::layout::{Layout, ViewRef};
use devdeck_core::list::Row;
use devdeck_core::notes::{NotesStore, project_node};
use devdeck_core::view_addr::ViewAddr;
use gpui::{Entity, Modifiers, MouseButton, TestAppContext, VisualTestContext};

struct Setup {
    deck: Entity<Deck>,
    notes: Entity<Notes>,
    sidebar: Entity<Sidebar>,
    store: NotesStore,
}

fn setup(cx: &mut TestAppContext) -> Setup {
    let dir = cx.update(|cx| devdeck_app::test_dir("notes-sidebar", cx));
    cx.update(|cx| devdeck_app::init_with_config_dir(&dir, cx));
    let shell = cx.update(|cx| Shell::start_single(Layout::new, cx));
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    let sidebar = cx.read(|cx| shell.read(cx).active_sidebar());
    let notes = cx.read(|cx| deck.read(cx).services().unwrap().read(cx).notes.clone());
    let store = NotesStore::new(dir.join("data/notes"));
    let s = store.clone();
    cx.update(|cx| notes.update(cx, |n, cx| n.set_store(s, cx)));
    cx.update(|cx| sidebar.update(cx, |s, cx| s.show(Activity::Notes, cx)));
    cx.run_until_parked();
    Setup { deck, notes, sidebar, store }
}

fn window(cx: &mut TestAppContext) -> VisualTestContext {
    let w = cx.read(|cx| cx.windows()[0]);
    let vcx = VisualTestContext::from_window(w, cx);
    vcx.run_until_parked();
    vcx
}

fn make(s: &Setup, app: &str, name: &str, cx: &mut TestAppContext) -> String {
    cx.update(|cx| s.notes.update(cx, |n, cx| n.create(app, name, cx))).expect("made")
}

fn note_ref(id: &str) -> ViewRef {
    ViewAddr::Note { id: id.into() }.to_ref()
}

/// The rows as text: notes by name, projects as `name ▸2`, indented.
fn rows(s: &Setup, cx: &TestAppContext) -> Vec<String> {
    cx.read(|cx| {
        let (items, rows) = s.sidebar.read(cx).rows(cx);
        rows.iter()
            .map(|r| {
                let pad = "  ".repeat(r.depth());
                match r {
                    Row::Header { section, count, .. } => format!("[{section:?} {count}]"),
                    Row::Empty(s) => format!("({s:?} empty)"),
                    Row::Item { index, .. } => format!("{pad}{}", items[*index].label),
                    Row::Folder { name, kids, .. } => format!("{pad}{name} {}{}", if kids.open { "▾" } else { "▸" }, kids.count),
                }
            })
            .collect()
    })
}

#[gpui::test]
fn projects_first_one_level_then_notes_without_one(cx: &mut TestAppContext) {
    let s = setup(cx);
    assert_eq!(rows(&s, cx), ["[Pinned 0]", "(Pinned empty)", "[All 0]", "(All empty)"], "no notes: no project shows");
    make(&s, "graphservice", "todo", cx);
    make(&s, "front-end/mastertadaapp", "bugs", cx);
    make(&s, "", "zeta", cx);
    make(&s, "", "Alpha", cx);
    make(&s, "old/gone", "kept", cx);
    // Nested projects show at one level, by their app's name (the demo names this one by
    // its path); a gone one by its id.
    assert_eq!(rows(&s, cx), ["[Pinned 0]", "(Pinned empty)", "[All 5]", "front-end/mastertadaapp ▸1", "graphservice ▸1", "old/gone ▸1", "Alpha", "zeta"]);
    let tree = cx.read(|cx| s.sidebar.read(cx).notes_tree(cx));
    let gone = project_node("old/gone");
    assert!(tree.gone.contains(&gone));
    assert_eq!(tree.hints[&gone], "old/gone · project not found");
    assert_eq!(tree.hints[&project_node("front-end/mastertadaapp")], "front-end/mastertadaapp");
    // A project opens and closes like a folder.
    let node = project_node("graphservice");
    cx.update(|cx| s.sidebar.update(cx, |sb, cx| sb.toggle_open(&node, cx)));
    assert_eq!(rows(&s, cx)[4..6], ["graphservice ▾1", "  todo"]);
    // A filter shows the matches inside their project.
    cx.update(|cx| s.sidebar.update(cx, |sb, cx| sb.filter.update(cx, |f, cx| f.set_text("bug", cx))));
    assert_eq!(rows(&s, cx), ["[Pinned 0]", "(Pinned empty)", "[All 1]", "front-end/mastertadaapp ▾1", "  bugs"]);
}

#[gpui::test]
fn n_makes_a_note_without_a_project_and_opens_it(cx: &mut TestAppContext) {
    let s = setup(cx);
    let mut vcx = window(cx);
    vcx.update(|window, cx| s.sidebar.read(cx).list_focus.clone().focus(window, cx));
    vcx.simulate_keystrokes("n");
    vcx.run_until_parked();
    vcx.simulate_input("todo");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    let all = s.store.list().unwrap();
    assert_eq!((all.len(), all[0].app.as_str(), all[0].name.as_str()), (1, "", "todo"));
    let r = note_ref(&all[0].id);
    assert!(vcx.read(|cx| s.deck.read(cx).layout().find_view(&r).is_some()), "opened");
    assert_eq!(vcx.read(|cx| s.deck.read(cx).title(&r, cx)), "todo", "no project in its title");
    assert_eq!(rows(&s, cx)[3..], ["todo"]);
}

#[gpui::test]
fn the_new_note_button_and_a_projects_menu(cx: &mut TestAppContext) {
    let s = setup(cx);
    make(&s, "graphservice", "todo", cx);
    let mut vcx = window(cx);
    let button = vcx.debug_bounds("sidebar-new-note").expect("the button");
    vcx.simulate_click(button.center(), Modifiers::none());
    vcx.run_until_parked();
    vcx.simulate_input("global");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert!(s.store.list().unwrap().iter().any(|n| n.name == "global" && n.app.is_empty()));

    // Right-click on the project: a note for it.
    let w = vcx.update(|window, _| window.window_handle());
    let row = vcx.debug_bounds("row-graphservice/").expect("the project's row");
    vcx.simulate_mouse_down(row.center(), MouseButton::Right, Modifiers::none());
    vcx.run_until_parked();
    let menu = vcx.update(|_, cx| devdeck_ui::overlay::layer(w, cx).read(cx).menu.clone()).expect("a menu");
    let (labels, run) = vcx.read(|cx| {
        let items = menu.read(cx).items();
        (items.iter().map(|i| i.label.to_string()).collect::<Vec<_>>(), items[0].run.clone().unwrap())
    });
    assert_eq!(labels, ["New note…"]);
    let by_key = vcx.update(|_, cx| Sidebar::item_menu(&s.sidebar, &format!("{}/", project_node("graphservice")), false, cx).len());
    assert_eq!(by_key, 1, "the menu key on the project's row: the same menu");
    vcx.simulate_keystrokes("escape");
    vcx.update(|window, cx| run(window, cx));
    vcx.run_until_parked();
    vcx.simulate_input("bugs");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert!(s.store.list().unwrap().iter().any(|n| n.name == "bugs" && n.app == "graphservice"));
}

#[gpui::test]
fn open_pin_rename_delete_from_the_list(cx: &mut TestAppContext) {
    let s = setup(cx);
    let id = make(&s, "", "todo", cx);
    let menu = cx.update(|cx| Sidebar::item_menu(&s.sidebar, &id, false, cx).iter().map(|i| i.label.to_string()).collect::<Vec<_>>());
    assert_eq!(menu, ["Open", "Pin", "Rename…", "Delete…"], "no groups in Notes");

    // ⏎ opens the selected note.
    let mut vcx = window(cx);
    vcx.update(|window, cx| s.sidebar.read(cx).list_focus.clone().focus(window, cx));
    vcx.simulate_keystrokes("down enter");
    vcx.run_until_parked();
    assert!(vcx.read(|cx| s.deck.read(cx).layout().find_view(&note_ref(&id)).is_some()));

    // Pinned by key, flat; a drop on All unpins it, one on a row of All does nothing more.
    vcx.update(|window, cx| s.sidebar.read(cx).list_focus.clone().focus(window, cx));
    vcx.simulate_keystrokes("p");
    vcx.run_until_parked();
    assert_eq!(rows(&s, cx), ["[Pinned 1]", "todo", "[All 0]", "(All empty)"]);
    let drop = |pinned, target: Target, cx: &TestAppContext| cx.read(|cx| s.sidebar.read(cx).item_drop(&id, pinned, &target));
    assert_eq!(drop(true, Target::AllHeader, cx), Some(Effect::TogglePin(id.clone())));
    assert_eq!(drop(true, Target::All(None), cx), Some(Effect::TogglePin(id.clone())));
    assert_eq!(drop(false, Target::All(None), cx), None);
    assert_eq!(drop(false, Target::Pinned, cx), Some(Effect::TogglePin(id.clone())));
    assert!(cx.update(|cx| s.sidebar.update(cx, |sb, cx| sb.create_group(None, "mine", cx))).is_err(), "no groups");

    // A rename shows in the list; a delete takes it away.
    cx.update(|cx| s.notes.update(cx, |n, cx| n.rename(&id, "bugs", cx)));
    assert_eq!(rows(&s, cx)[1], "bugs");
    cx.update(|cx| s.notes.update(cx, |n, cx| n.delete(&id, cx)));
    assert_eq!(rows(&s, cx), ["[Pinned 0]", "(Pinned empty)", "[All 0]", "(All empty)"]);
}

#[gpui::test]
fn ctrl_6_shows_the_notes(cx: &mut TestAppContext) {
    let s = setup(cx);
    cx.update(|cx| s.sidebar.update(cx, |sb, cx| sb.show(Activity::Apps, cx)));
    let w = cx.read(|cx| cx.windows()[0]);
    cx.simulate_keystrokes(w, "ctrl-6");
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| s.sidebar.read(cx).activity()), Activity::Notes);
}
