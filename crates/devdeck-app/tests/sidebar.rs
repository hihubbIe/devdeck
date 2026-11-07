//! M1 step 4: activity bar and sidebar lists (R4.1–R4.3, R2.3, R5.1, R5.2), on GPUI's
//! test platform with the demo workspaces (`work` has a cluster, `personal` doesn't).
//! M2 step 3: the Apps list as a tree of folders (apps added to the demo's).

use devdeck_app::cluster::LOAD_DELAY;
use devdeck_app::shell::Shell;
use devdeck_app::sidebar::{Activity, Sidebar, open_at};
use devdeck_core::layout::{Drop, Edge, Layout, ViewRef};
use devdeck_core::list::Row;
use devdeck_core::state::{Loaded, SidebarState, StateStore};
use devdeck_ui::overlay::layer_state;
use devdeck_ui::{logical, toasts};
use gpui::{AnyWindowHandle, AppContext, Entity, Modifiers, MouseButton, MouseDownEvent, TestAppContext, VisualTestContext, point};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

/// A folder kept as long as the test's app.
fn temp(cx: &mut TestAppContext, tag: &str) -> PathBuf {
    cx.update(|cx| devdeck_app::test_dir(tag, cx))
}

fn start(cx: &mut TestAppContext, store: Option<StateStore>, tag: &str) -> Entity<Shell> {
    let config = temp(cx, &format!("{tag}-config"));
    cx.update(|cx| devdeck_app::init_with_config_dir(&config, cx));
    let shell = cx.update(|cx| Shell::start(devdeck_app::demo_workspaces(), "work", store, cx));
    cx.update(|cx| shell.update(cx, |s, _| s.quit_for_real = false));
    cx.run_until_parked();
    shell
}

fn main_window(shell: &Entity<Shell>, cx: &TestAppContext) -> AnyWindowHandle {
    cx.read(|cx| shell.read(cx).main_window().expect("main").into())
}

fn sidebar(shell: &Entity<Shell>, cx: &TestAppContext) -> Entity<Sidebar> {
    cx.read(|cx| shell.read(cx).active_sidebar())
}

fn switch_to(shell: &Entity<Shell>, cx: &mut TestAppContext, id: &str) {
    let i = cx.read(|cx| shell.read(cx).workspaces.iter().position(|w| w.id == id).unwrap());
    cx.update(|cx| shell.update(cx, |s, cx| s.switch(i, cx)));
    cx.run_until_parked();
}

fn selected(s: &Entity<Sidebar>, cx: &TestAppContext) -> Option<String> {
    cx.read(|cx| s.read(cx).selected.clone())
}

fn pinned(s: &Entity<Sidebar>, cx: &TestAppContext) -> Vec<String> {
    cx.read(|cx| {
        let (items, rows) = s.read(cx).rows(cx);
        rows.iter()
            .filter_map(|r| match r {
                Row::Item { index, pinned: true, .. } => Some(items[*index].key.clone()),
                _ => None,
            })
            .collect()
    })
}

fn list_focused(s: &Entity<Sidebar>, w: AnyWindowHandle, cx: &mut TestAppContext) -> bool {
    let h = cx.read(|cx| s.read(cx).list_focus.clone());
    cx.update_window(w, |_, window, _| h.is_focused(window)).unwrap()
}

#[gpui::test]
fn cluster_and_env_only_where_configured(cx: &mut TestAppContext) {
    let shell = start(cx, None, "r23");
    let work = sidebar(&shell, cx);
    assert_eq!(cx.read(|cx| work.read(cx).activities(cx)), [Activity::Apps, Activity::Cluster, Activity::Ports, Activity::Layouts]);
    switch_to(&shell, cx, "personal");
    let personal = sidebar(&shell, cx);
    assert_eq!(cx.read(|cx| personal.read(cx).activities(cx)), [Activity::Apps, Activity::Ports, Activity::Layouts]);
    assert!(cx.read(|cx| personal.read(cx).env(cx).is_none()), "no env button without envs");
    let main = main_window(&shell, cx);
    cx.simulate_keystrokes(main, "ctrl-2");
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| personal.read(cx).activity()), Activity::Apps, "ctrl+2 does nothing without a cluster");
    cx.simulate_keystrokes(main, "ctrl-e");
    cx.run_until_parked();
    assert!(!cx.update(|cx| layer_state(main, cx).0), "no env menu without envs");
}

#[gpui::test]
fn each_workspace_keeps_its_own_sidebar(cx: &mut TestAppContext) {
    let shell = start(cx, None, "per-ws");
    let main = main_window(&shell, cx);
    cx.simulate_keystrokes(main, "ctrl-3");
    cx.run_until_parked();
    let work = sidebar(&shell, cx);
    assert_eq!(cx.read(|cx| work.read(cx).activity()), Activity::Ports);
    assert!(list_focused(&work, main, cx), "the activity key focuses its list");
    switch_to(&shell, cx, "personal");
    assert_eq!(cx.read(|cx| shell.read(cx).active_sidebar().read(cx).activity()), Activity::Apps);
    switch_to(&shell, cx, "work");
    assert_eq!(cx.read(|cx| shell.read(cx).active_sidebar().read(cx).activity()), Activity::Ports);
    assert!(list_focused(&work, main, cx), "focus comes back to the list");
}

#[gpui::test]
fn list_keys_select_pin_and_open(cx: &mut TestAppContext) {
    let shell = start(cx, None, "keys");
    let main = main_window(&shell, cx);
    let s = sidebar(&shell, cx);
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    cx.simulate_keystrokes(main, "ctrl-1 down");
    cx.run_until_parked();
    let first = "a-service-with-a-name-long-enough-to-truncate-in-the-sidebar".to_string();
    assert_eq!(selected(&s, cx), Some(first.clone()), "alphabetical first, headers skipped");
    cx.simulate_keystrokes(main, "down down up");
    assert_eq!(selected(&s, cx).as_deref(), Some("boardservice"));
    cx.simulate_keystrokes(main, "p");
    assert_eq!(pinned(&s, cx), ["boardservice"]);
    assert_eq!(selected(&s, cx).as_deref(), Some("boardservice"), "selection follows the item into Pinned");
    cx.simulate_keystrokes(main, "end");
    assert_eq!(selected(&s, cx).as_deref(), Some("visualizerservice"));
    cx.simulate_keystrokes(main, "enter enter");
    cx.run_until_parked();
    let view = ViewRef::new("app/visualizerservice");
    let tabs = cx.read(|cx| deck.read(cx).layout().panes.values().flat_map(|p| p.tabs.iter()).filter(|r| **r == view).count());
    assert_eq!(tabs, 1, "opened once, then activated rather than duplicated");
    assert!(list_focused(&s, main, cx), "focus stays in the list");
    cx.simulate_keystrokes(main, "r");
    let (items, rows) = cx.read(|cx| s.read(cx).rows(cx));
    assert!(rows.iter().all(|r| !matches!(r, Row::Item { index, .. } if !items[*index].running)), "r: running only");
}

#[gpui::test]
fn filter_typing_never_triggers_row_keys(cx: &mut TestAppContext) {
    let shell = start(cx, None, "filter");
    let main = main_window(&shell, cx);
    let s = sidebar(&shell, cx);
    cx.simulate_keystrokes(main, "ctrl-1 /");
    cx.simulate_input(main, "proxp");
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| s.read(cx).filter.read(cx).text().to_string()), "proxp");
    assert!(pinned(&s, cx).is_empty(), "p typed in the filter does not pin");
    cx.simulate_keystrokes(main, "backspace enter");
    cx.run_until_parked();
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    assert!(cx.read(|cx| deck.read(cx).layout().find_view(&ViewRef::new("app/proxyapi")).is_some()), "enter opens the match");
    cx.simulate_keystrokes(main, "escape");
    assert_eq!(cx.read(|cx| s.read(cx).filter.read(cx).text().to_string()), "", "esc clears the filter");
    assert!(list_focused(&s, main, cx));
    cx.simulate_keystrokes(main, "escape");
    assert!(!list_focused(&s, main, cx), "second esc goes back to the panes");
    cx.simulate_keystrokes(main, "ctrl-b");
    assert!(!cx.read(|cx| s.read(cx).state.visible));
    cx.simulate_keystrokes(main, "ctrl-b");
    assert!(cx.read(|cx| s.read(cx).state.visible));
    assert!(list_focused(&s, main, cx), "showing the sidebar focuses its list");
}

#[gpui::test]
fn hiding_a_focused_sidebar_returns_focus_to_the_panes(cx: &mut TestAppContext) {
    let shell = start(cx, None, "hide");
    let main = main_window(&shell, cx);
    let s = sidebar(&shell, cx);
    cx.simulate_keystrokes(main, "ctrl-1 ctrl-b");
    cx.run_until_parked();
    assert!(!list_focused(&s, main, cx));
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    let panes = cx.read(|cx| deck.read(cx).layout().panes.len());
    cx.simulate_keystrokes(main, "ctrl-\\");
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| deck.read(cx).layout().panes.len()), panes + 1, "pane keys work again");
}

#[gpui::test]
fn mouse_right_click_menu_and_drop_on_header(cx: &mut TestAppContext) {
    let shell = start(cx, None, "mouse");
    let main = main_window(&shell, cx);
    let s = sidebar(&shell, cx);
    let mut vcx = VisualTestContext::from_window(main, cx);
    let row = vcx.debug_bounds("row-graphservice").expect("row drawn");
    vcx.simulate_mouse_down(row.center(), MouseButton::Right, Modifiers::none());
    vcx.run_until_parked();
    assert!(vcx.update(|_, cx| layer_state(main, cx).0), "right-click opens the row menu");
    vcx.simulate_keystrokes("down enter");
    vcx.run_until_parked();
    assert_eq!(pinned(&s, &vcx), ["graphservice"], "menu: Pin");
    // Drag a row of All onto the Pinned header.
    let row = vcx.debug_bounds("row-boardservice").expect("row drawn");
    let header = vcx.debug_bounds("header-pinned").expect("header drawn");
    vcx.simulate_mouse_down(row.center(), MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_move(row.center() + point(logical(20.), logical(0.)), MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_move(header.center(), MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_up(header.center(), MouseButton::Left, Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(pinned(&s, &vcx), ["boardservice", "graphservice"], "dropped on Pinned: pinned, alphabetical");
}

#[gpui::test]
fn env_menu_changes_the_cluster_list(cx: &mut TestAppContext) {
    let shell = start(cx, None, "env");
    let main = main_window(&shell, cx);
    let s = sidebar(&shell, cx);
    cx.simulate_keystrokes(main, "ctrl-2 ctrl-e");
    cx.run_until_parked();
    assert!(cx.update(|cx| layer_state(main, cx).0), "ctrl+e opens the env menu");
    cx.simulate_keystrokes(main, "escape");
    assert_eq!(cx.read(|cx| s.read(cx).status(cx)).as_deref(), Some("loading…"));
    cx.executor().advance_clock(LOAD_DELAY);
    cx.run_until_parked();
    let before = cx.read(|cx| s.read(cx).items(cx));
    assert!(!before.is_empty());
    cx.update(|cx| s.update(cx, |s, cx| s.set_env("dev31", cx)));
    cx.executor().advance_clock(LOAD_DELAY);
    cx.run_until_parked();
    let after = cx.read(|cx| s.read(cx).items(cx));
    assert_eq!(cx.read(|cx| s.read(cx).state.env.clone()).as_deref(), Some("dev31"));
    assert_ne!(before, after, "another env, other tags");
    assert_eq!(cx.read(|cx| s.read(cx).view_ref("tadawebapi", cx)), Some(ViewRef::new("svc/dev31/tadawebapi")));
}

#[gpui::test]
fn pane_neighbours_ignore_the_sidebar_width(cx: &mut TestAppContext) {
    let shell = start(cx, None, "geometry");
    let main = main_window(&shell, cx);
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    let focused = |cx: &TestAppContext| cx.read(|cx| deck.read(cx).layout().main().focused);
    let logs = cx.read(|cx| deck.read(cx).layout().find_view(&ViewRef::new("demo/logs")).unwrap().0);
    let right = cx.read(|cx| deck.read(cx).layout().find_view(&ViewRef::new("demo/deployments")).unwrap().0);
    for visible in [true, false] {
        let s = sidebar(&shell, cx);
        cx.update(|cx| s.update(cx, |s, cx| s.set_visible(visible, cx)));
        cx.update(|cx| deck.update(cx, |d, cx| d.apply(cx, |l| l.focus(logs))));
        cx.run_until_parked();
        cx.simulate_keystrokes(main, "alt-right");
        assert_eq!(focused(cx), right, "alt+right from logs (sidebar shown: {visible})");
        cx.simulate_keystrokes(main, "alt-left");
        assert_eq!(focused(cx), logs);
    }
}

#[gpui::test]
fn sidebar_state_is_saved_and_restored(cx: &mut TestAppContext) {
    let dir = temp(cx, "persist");
    let store = StateStore::new(dir.clone());
    let saved = SidebarState {
        activity: "cluster".into(),
        visible: true,
        env: Some("dev33".into()),
        pins: BTreeMap::from([("deployments".into(), BTreeSet::from(["vesselservice".into(), "gone-service".into()]))]),
        open: BTreeMap::new(),
    };
    store.save_sidebar("work", &saved).unwrap();
    std::fs::create_dir_all(dir.join("workspaces/personal")).unwrap();
    std::fs::write(dir.join("workspaces/personal/sidebar.json"), "{").unwrap();
    let shell = start(cx, Some(store), "persist");
    cx.executor().advance_clock(LOAD_DELAY);
    cx.run_until_parked();
    let s = sidebar(&shell, cx);
    assert_eq!(cx.read(|cx| s.read(cx).activity()), Activity::Cluster);
    assert_eq!(cx.read(|cx| s.read(cx).state.env.clone()).as_deref(), Some("dev33"));
    assert_eq!(pinned(&s, cx), ["vesselservice"], "unknown pins are kept but not shown");
    assert!(dir.join("workspaces/personal/sidebar.json.bad").exists(), "broken file kept aside");
    assert_eq!(cx.update(|cx| toasts(cx).read(cx).queue.items.len()), 1, "and reported");
    // A pin is written in the background.
    cx.update(|cx| s.update(cx, |s, cx| s.toggle_pin("proxyapi", cx)));
    cx.run_until_parked();
    let Loaded::Ok(back) = StateStore::new(dir).load_sidebar("work") else { panic!("saved") };
    assert!(back.pins["deployments"].contains("proxyapi") && back.pins["deployments"].contains("gone-service"));
}

#[test]
fn rows_dropped_on_a_pane_open_there() {
    let mut l = Layout::new();
    let p = l.main().focused;
    l.open(ViewRef::new("demo/notes"), p).unwrap();
    open_at(&mut l, ViewRef::new("app/entityservice"), Drop::Edge(p, Edge::Right)).unwrap();
    assert_eq!(l.panes_in(devdeck_core::layout::MAIN_WINDOW).unwrap().len(), 2, "split at the edge");
    let (q, _) = l.find_view(&ViewRef::new("app/entityservice")).unwrap();
    assert_ne!(q, p);
    open_at(&mut l, ViewRef::new("app/entityservice"), Drop::Into(p)).unwrap();
    assert_eq!(l.find_view(&ViewRef::new("app/entityservice")).unwrap().0, p, "an open item moves instead of duplicating");
}

/// Adds a monorepo to `work`'s apps: `mono` holding `apps/shell` and the library
/// `packages/ui`.
fn with_tree(shell: &Entity<Shell>, cx: &mut TestAppContext) {
    let services = cx.read(|cx| shell.read(cx).workspaces.iter().find(|w| w.id == "work").unwrap().services.clone());
    let mut catalog = cx.read(|cx| (*services.read(cx).catalog).clone());
    let base = catalog.apps[0].clone();
    for (id, name, library) in [("mono", "mono", false), ("mono/apps/shell", "shell", false), ("mono/packages/ui", "ui", true)] {
        catalog.apps.push(devdeck_core::apps::App { id: id.into(), name: name.into(), library, ..base.clone() });
    }
    cx.update(|cx| services.update(cx, |s, cx| s.set_catalog(catalog, cx)));
    cx.run_until_parked();
}

/// The rows from `mono` down: indented labels, folders with `/`, `▸`/`▾` when they hold.
fn tree(s: &Entity<Sidebar>, cx: &TestAppContext) -> Vec<String> {
    cx.read(|cx| {
        let (items, rows) = s.read(cx).rows(cx);
        let mark = |k: Option<devdeck_core::list::Kids>| k.map_or("", |k| if k.open { " ▾" } else { " ▸" });
        rows.iter()
            .filter_map(|r| match r {
                Row::Item { index, depth, kids, .. } => Some(format!("{}{}{}", "  ".repeat(*depth), items[*index].label, mark(*kids))),
                Row::Folder { name, depth, kids, .. } => Some(format!("{}{name}/{}", "  ".repeat(*depth), mark(Some(*kids)))),
                _ => None,
            })
            .skip_while(|l| !l.starts_with("mono"))
            .take_while(|l| l.starts_with("mono") || l.starts_with(' '))
            .collect()
    })
}

#[gpui::test]
fn apps_are_a_tree_that_arrows_open_and_close(cx: &mut TestAppContext) {
    let shell = start(cx, None, "tree-keys");
    with_tree(&shell, cx);
    let main = main_window(&shell, cx);
    let s = sidebar(&shell, cx);
    assert_eq!(tree(&s, cx), ["mono ▸"], "closed at first");
    cx.simulate_keystrokes(main, "ctrl-1");
    cx.update(|cx| s.update(cx, |s, cx| {
        s.selected = Some("mono".into());
        cx.notify();
    }));
    cx.simulate_keystrokes(main, "right");
    assert_eq!(tree(&s, cx), ["mono ▾", "  apps/ ▸", "  packages/ ▸"], "→ opens");
    cx.simulate_keystrokes(main, "right");
    assert_eq!(selected(&s, cx).as_deref(), Some("mono/apps/"), "→ again: the first child");
    cx.simulate_keystrokes(main, "enter");
    assert_eq!(tree(&s, cx), ["mono ▾", "  apps/ ▾", "    shell", "  packages/ ▸"], "enter opens a folder");
    cx.simulate_keystrokes(main, "down");
    assert_eq!(selected(&s, cx).as_deref(), Some("mono/apps/shell"));
    cx.simulate_keystrokes(main, "left");
    assert_eq!(selected(&s, cx).as_deref(), Some("mono/apps/"), "← on a leaf: its parent");
    cx.simulate_keystrokes(main, "left");
    assert_eq!(tree(&s, cx), ["mono ▾", "  apps/ ▸", "  packages/ ▸"], "← closes");
    cx.simulate_keystrokes(main, "left");
    assert_eq!(selected(&s, cx).as_deref(), Some("mono"), "← on a closed one: its parent");
    cx.simulate_keystrokes(main, "p");
    assert!(pinned(&s, cx).contains(&"mono".to_string()), "an app holding apps pins");
    let folder = cx.read(|cx| {
        let (_, rows) = s.read(cx).rows(cx);
        rows.iter().any(|r| matches!(r, Row::Folder { path, depth: 0, .. } if path == "mono"))
    });
    assert!(folder, "what it held stays in All, under a folder row");
    cx.update(|cx| s.update(cx, |s, cx| {
        s.selected = Some("mono/".into());
        cx.notify();
    }));
    cx.simulate_keystrokes(main, "p");
    assert_eq!(pinned(&s, cx), ["mono"], "folders don't pin");
}

#[gpui::test]
fn the_chevron_and_a_double_click_toggle_and_a_closing_folder_takes_the_selection(cx: &mut TestAppContext) {
    let shell = start(cx, None, "tree-mouse");
    with_tree(&shell, cx);
    let main = main_window(&shell, cx);
    let s = sidebar(&shell, cx);
    let mut vcx = VisualTestContext::from_window(main, cx);
    let chevron = vcx.debug_bounds("row-mono-chevron").expect("drawn");
    vcx.simulate_click(chevron.center(), Modifiers::none());
    assert_eq!(tree(&s, &vcx), ["mono ▾", "  apps/ ▸", "  packages/ ▸"]);
    assert_eq!(selected(&s, &vcx), None, "the chevron doesn't select");
    let folder = vcx.debug_bounds("row-mono/packages/").expect("drawn");
    vcx.simulate_click(folder.center(), Modifiers::none());
    vcx.simulate_event(MouseDownEvent { button: MouseButton::Left, position: folder.center(), modifiers: Modifiers::none(), click_count: 2, first_mouse: false });
    vcx.simulate_mouse_up(folder.center(), MouseButton::Left, Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(tree(&s, &vcx), ["mono ▾", "  apps/ ▸", "  packages/ ▾", "    ui"], "double click opens a folder");
    vcx.update(|_, cx| s.update(cx, |s, cx| {
        s.selected = Some("mono/packages/ui".into());
        cx.notify();
    }));
    let chevron = vcx.debug_bounds("row-mono-chevron").expect("drawn");
    vcx.simulate_click(chevron.center(), Modifiers::none());
    assert_eq!(tree(&s, &vcx), ["mono ▸"]);
    assert_eq!(selected(&s, &vcx).as_deref(), Some("mono"), "the selection moves to the row that closed");
}

#[gpui::test]
fn whats_open_is_saved_and_a_filter_leaves_it_as_it_was(cx: &mut TestAppContext) {
    let dir = temp(cx, "tree-save");
    let shell = start(cx, Some(StateStore::new(dir.clone())), "tree-save");
    with_tree(&shell, cx);
    let s = sidebar(&shell, cx);
    cx.update(|cx| s.update(cx, |s, cx| s.toggle_open("mono", cx)));
    cx.run_until_parked();
    let Loaded::Ok(saved) = StateStore::new(dir).load_sidebar("work") else { panic!("saved") };
    assert_eq!(saved.open["apps"], BTreeSet::from(["mono".to_string()]));
    cx.update(|cx| s.update(cx, |s, cx| s.filter.update(cx, |f, cx| f.set_text("shell", cx))));
    cx.run_until_parked();
    assert_eq!(tree(&s, cx), ["mono ▾", "  apps/ ▾", "    shell"], "the way to the match, open");
    cx.update(|cx| s.update(cx, |s, cx| s.toggle_open("mono", cx)));
    assert_eq!(cx.read(|cx| s.read(cx).state.open["apps"].clone()), BTreeSet::from(["mono".to_string()]), "nothing to toggle while filtering");
    cx.update(|cx| s.update(cx, |s, cx| s.filter.update(cx, |f, cx| f.set_text("", cx))));
    cx.run_until_parked();
    assert_eq!(tree(&s, cx), ["mono ▾", "  apps/ ▸", "  packages/ ▸"], "as it was");
}
