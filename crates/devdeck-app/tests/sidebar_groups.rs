//! Sidebar organisation (R4.7–R4.9): folded sections, groups the user makes, and the
//! sidebar's width, on GPUI's test platform with the demo workspaces.

use devdeck_app::cluster::LOAD_DELAY;
use devdeck_app::name_prompt::NamePrompt;
use devdeck_app::shell::Shell;
use devdeck_app::sidebar::{Activity, Effect, Moving, Sidebar, Target};
use devdeck_core::list::{Groups, Row, Section, group_row_path};
use devdeck_core::state::{AppState, Loaded, SidebarState, StateStore};
use devdeck_ui::logical;
use devdeck_ui::overlay::layer_state;
use gpui::{AnyWindowHandle, AppContext, Entity, Modifiers, MouseButton, TestAppContext, VisualTestContext, point};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

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

/// The rows as text: `[All 12]`, `[All 12 folded]`, `#group ▸2`, items by key, indented.
fn rows(s: &Entity<Sidebar>, cx: &TestAppContext) -> Vec<String> {
    cx.read(|cx| {
        let (items, rows) = s.read(cx).rows(cx);
        rows.iter()
            .map(|r| {
                let pad = "  ".repeat(r.depth());
                match r {
                    Row::Header { section, open, count } => format!("[{section:?} {count}{}]", if *open { "" } else { " folded" }),
                    Row::Empty(s) => format!("({s:?} empty)"),
                    Row::Item { index, .. } => format!("{pad}{}", items[*index].key),
                    Row::Folder { name, kids, group, .. } => {
                        format!("{pad}{}{name}{} {}{}", if *group { "#" } else { "" }, if *group { "" } else { "/" }, if kids.open { "▾" } else { "▸" }, kids.count)
                    }
                }
            })
            .collect()
    })
}

fn group_of(s: &Entity<Sidebar>, node: &str, cx: &TestAppContext) -> Option<String> {
    cx.read(|cx| s.read(cx).groups().and_then(|g| g.group_of(node)).map(String::from))
}

fn drag(vcx: &mut VisualTestContext, from: &'static str, to: &'static str) {
    let a = vcx.debug_bounds(from).unwrap_or_else(|| panic!("{from} drawn"));
    let b = vcx.debug_bounds(to).unwrap_or_else(|| panic!("{to} drawn"));
    vcx.simulate_mouse_down(a.center(), MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_move(a.center() + point(logical(20.), logical(0.)), MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_move(b.center(), MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_up(b.center(), MouseButton::Left, Modifiers::none());
    vcx.run_until_parked();
}

#[gpui::test]
fn sections_fold_from_their_header_and_a_filter_shows_their_matches(cx: &mut TestAppContext) {
    let dir = temp(cx, "fold");
    let shell = start(cx, Some(StateStore::new(dir.clone())), "fold");
    let main = main_window(&shell, cx);
    let s = sidebar(&shell, cx);
    let before = rows(&s, cx);
    let all = before.iter().find(|r| r.starts_with("[All")).unwrap().clone();
    let mut vcx = VisualTestContext::from_window(main, cx);
    let header = vcx.debug_bounds("header-all").expect("header drawn");
    vcx.simulate_mouse_down(header.center(), MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_up(header.center(), MouseButton::Left, Modifiers::none());
    vcx.run_until_parked();
    let folded = rows(&s, &vcx);
    assert_eq!(folded.last().unwrap(), &all.replace(']', " folded]"), "only the header, with its count");
    cx.update(|cx| s.update(cx, |s, cx| s.filter.update(cx, |f, cx| f.set_text("graph", cx))));
    assert!(rows(&s, cx).contains(&"graphservice".to_string()), "a filter shows the matches of a folded section");
    cx.update(|cx| s.update(cx, |s, cx| s.toggle_section(Section::Pinned, cx)));
    assert!(!rows(&s, cx).iter().any(|r| r.contains("folded")), "folding does nothing while filtering");
    cx.update(|cx| s.update(cx, |s, cx| s.filter.update(cx, |f, cx| f.set_text("", cx))));
    assert_eq!(rows(&s, cx), folded, "what's folded stays as it was");
    cx.run_until_parked();
    let Loaded::Ok(saved) = StateStore::new(dir).load_sidebar("work") else { panic!("saved") };
    assert_eq!(saved.collapsed["apps"], BTreeSet::from(["all".to_string()]));
}

#[gpui::test]
fn folded_sections_and_groups_come_back(cx: &mut TestAppContext) {
    let dir = temp(cx, "restore");
    let store = StateStore::new(dir.clone());
    let groups = Groups { folders: BTreeSet::from(["back".into()]), members: BTreeMap::from([("vesselservice".into(), "back".into())]) };
    let saved = SidebarState {
        activity: "cluster".into(),
        collapsed: BTreeMap::from([("deployments".into(), BTreeSet::from(["pinned".into()]))]),
        groups: BTreeMap::from([("deployments".into(), groups)]),
        open: BTreeMap::from([("deployments".into(), BTreeSet::from([group_row_path("back")]))]),
        ..Default::default()
    };
    store.save_sidebar("work", &saved).unwrap();
    let shell = start(cx, Some(store), "restore");
    cx.executor().advance_clock(LOAD_DELAY);
    cx.run_until_parked();
    let s = sidebar(&shell, cx);
    let r = rows(&s, cx);
    assert_eq!(r[0], "[Pinned 0 folded]");
    assert_eq!(&r[2..4], ["#back ▾1", "  vesselservice"], "groups first, open as saved");
    // Shared by every env, as pins are.
    cx.update(|cx| s.update(cx, |s, cx| s.set_env("dev31", cx))).unwrap();
    cx.executor().advance_clock(LOAD_DELAY);
    cx.run_until_parked();
    assert_eq!(&rows(&s, cx)[2..4], ["#back ▾1", "  vesselservice"]);
}

#[gpui::test]
fn groups_are_made_through_the_prompt_renamed_and_deleted_without_losing_anything(cx: &mut TestAppContext) {
    let shell = start(cx, None, "make");
    let main = main_window(&shell, cx);
    let s = sidebar(&shell, cx);
    let mut vcx = VisualTestContext::from_window(main, cx);
    // The All header's menu: New group…
    let header = vcx.debug_bounds("header-all").expect("header drawn");
    vcx.simulate_mouse_down(header.center(), MouseButton::Right, Modifiers::none());
    vcx.run_until_parked();
    assert!(vcx.update(|_, cx| layer_state(main, cx).0), "the header's menu");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert_eq!(vcx.update(|_, cx| layer_state(main, cx).1), 1, "the name prompt");
    vcx.simulate_input("back");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert_eq!(vcx.update(|_, cx| layer_state(main, cx).1), 0, "made, closed");
    assert!(rows(&s, &vcx).contains(&"#back ▾0".to_string()), "a new group shows open");
    // A name taken stays in the prompt with the reason.
    let header = vcx.debug_bounds("header-all").expect("header drawn");
    vcx.simulate_mouse_down(header.center(), MouseButton::Right, Modifiers::none());
    vcx.run_until_parked();
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    vcx.simulate_input("Back");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    let prompt = vcx.update(|_, cx| devdeck_ui::overlay::layer(main, cx).read(cx).modals.last().cloned().unwrap().downcast::<NamePrompt>().unwrap());
    assert_eq!(vcx.read(|cx| prompt.read(cx).error.clone()).as_deref(), Some("a group called back exists here"));
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    // Inside, rename, delete: what it holds moves up.
    vcx.update(|_, cx| {
        s.update(cx, |s, cx| {
            s.create_group(s.activity(), Some("back"), "api", cx).unwrap();
            s.put(s.activity(), "tadawebapi", Some("back/api"), cx);
            s.put(s.activity(), "graphservice", Some("back"), cx);
            assert_eq!(s.rename_group(s.activity(), "back", "BE", cx), Ok("BE".into()));
        })
    });
    assert_eq!((group_of(&s, "tadawebapi", &vcx).as_deref(), group_of(&s, "graphservice", &vcx).as_deref()), (Some("BE/api"), Some("BE")));
    vcx.update(|_, cx| s.update(cx, |s, cx| s.delete_group(s.activity(), "BE", cx)));
    assert_eq!((group_of(&s, "tadawebapi", &vcx).as_deref(), group_of(&s, "graphservice", &vcx)), (Some("api"), None));
    assert_eq!(vcx.read(|cx| s.read(cx).groups().unwrap().paths()), ["api"]);
}

#[gpui::test]
fn rows_dropped_on_a_group_go_in_it_and_back_to_the_top_on_the_all_header(cx: &mut TestAppContext) {
    let shell = start(cx, None, "drop");
    let main = main_window(&shell, cx);
    let s = sidebar(&shell, cx);
    cx.update(|cx| s.update(cx, |s, cx| s.create_group(s.activity(), None, "back", cx).map(drop))).unwrap();
    cx.run_until_parked();
    let mut vcx = VisualTestContext::from_window(main, cx);
    drag(&mut vcx, "row-boardservice", "row-\u{1}back/");
    assert_eq!(group_of(&s, "boardservice", &vcx).as_deref(), Some("back"));
    assert!(rows(&s, &vcx).contains(&"  boardservice".to_string()), "shown inside");
    drag(&mut vcx, "row-boardservice", "header-all");
    assert_eq!(group_of(&s, "boardservice", &vcx), None, "the All header: back to the top");
    // A pinned item dropped on a group is unpinned into it.
    vcx.update(|_, cx| s.update(cx, |s, cx| s.toggle_pin("graphservice", cx)));
    vcx.run_until_parked();
    drag(&mut vcx, "row-graphservice", "row-\u{1}back/");
    assert_eq!(group_of(&s, "graphservice", &vcx).as_deref(), Some("back"));
    assert!(vcx.read(|cx| !s.read(cx).state.pins["apps"].contains("graphservice")));
}

#[gpui::test]
fn what_a_drop_does_depends_on_where(cx: &mut TestAppContext) {
    let shell = start(cx, None, "effects");
    let s = sidebar(&shell, cx);
    cx.update(|cx| {
        s.update(cx, |s, cx| {
            s.create_group(s.activity(), None, "a", cx).unwrap();
            s.create_group(s.activity(), Some("a"), "b", cx).unwrap();
            s.put(s.activity(), "graphservice", Some("a"), cx);
        })
    });
    let all = |g: Option<&str>| Target::All(g.map(String::from));
    cx.read(|cx| {
        let s = s.read(cx);
        assert_eq!(s.item_drop("graphservice", false, &Target::Pinned), Some(Effect::TogglePin("graphservice".into())));
        assert_eq!(s.item_drop("graphservice", false, &all(Some("a"))), None, "already there");
        assert_eq!(s.item_drop("graphservice", false, &all(None)), Some(Effect::Put("graphservice".into(), None)));
        assert_eq!(s.item_drop("graphservice", true, &all(None)), Some(Effect::TogglePin("graphservice".into())), "unpinned, it goes back to its group");
        assert_eq!(s.item_drop("graphservice", true, &Target::Pinned), None);
        let a = group_row_path("a");
        assert_eq!(s.folder_drop(&a, &all(Some("a/b"))), None, "never into itself");
        assert_eq!(s.folder_drop(&a, &all(None)), None, "already at the top");
        assert_eq!(s.folder_drop(&group_row_path("a/b"), &all(None)), Some(Effect::MoveGroup("a/b".into(), None)));
        assert_eq!(s.folder_drop(&a, &Target::Pinned), None, "groups aren't pinned");
    });
    // The move menu: the top level and every group but itself and what's inside it.
    let labels = |what: Moving, cx: &mut TestAppContext| cx.update(|cx| Sidebar::move_menu(&s, what, cx).submenu.iter().map(|i| i.label.to_string()).collect::<Vec<_>>());
    assert_eq!(labels(Moving::Node("graphservice".into()), cx), ["Top level", "a", "a/b"]);
    assert_eq!(labels(Moving::Group("a".into()), cx), ["Top level"]);
    // Terminals hold no groups.
    cx.update(|cx| s.update(cx, |s, cx| s.show(Activity::Terminals, cx)));
    cx.read(|cx| assert_eq!(s.read(cx).item_drop("1", false, &all(Some("a"))), None));
}

#[gpui::test]
fn the_sidebar_edge_drags_its_width_and_a_double_click_resets_it(cx: &mut TestAppContext) {
    let dir = temp(cx, "width");
    let shell = start(cx, Some(StateStore::new(dir.clone())), "width");
    let main = main_window(&shell, cx);
    let mut vcx = VisualTestContext::from_window(main, cx);
    let width = |vcx: &mut VisualTestContext| vcx.debug_bounds("sidebar-filter").map(|b| f32::from(b.size.width)).unwrap();
    let before = width(&mut vcx);
    let edge = vcx.debug_bounds("sidebar-edge").expect("edge drawn");
    vcx.simulate_mouse_down(edge.center(), MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_move(edge.center() + point(logical(40.), logical(0.)), MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_move(edge.center() + point(logical(100.), logical(0.)), MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_up(edge.center() + point(logical(100.), logical(0.)), MouseButton::Left, Modifiers::none());
    vcx.run_until_parked();
    let saved = vcx.read(|cx| shell.read(cx).sidebar_w).expect("a width");
    assert!((width(&mut vcx) - before - 100.).abs() < 2., "wider by the drag");
    // Kept to half the window, without changing what's saved.
    let half = f32::from(vcx.update(|window, _| window.viewport_size().width)) / 2.;
    vcx.update(|_, cx| shell.update(cx, |s, _| s.sidebar_w = Some(5000.)));
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    assert!(width(&mut vcx) <= half, "at most half the window");
    assert_eq!(vcx.read(|cx| shell.read(cx).sidebar_w), Some(5000.));
    vcx.update(|_, cx| shell.update(cx, |s, _| s.sidebar_w = Some(saved)));
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    // Saved with the window size.
    vcx.update(|_, cx| shell.update(cx, |s, cx| s.flush(cx)));
    let Loaded::Ok(AppState { sidebar_w, .. }) = StateStore::new(dir).load_app() else { panic!("saved") };
    assert_eq!(sidebar_w, Some(saved));
    // A double click goes back to the theme's.
    let edge = vcx.debug_bounds("sidebar-edge").expect("edge drawn");
    vcx.simulate_event(gpui::MouseDownEvent { position: edge.center(), button: MouseButton::Left, modifiers: Modifiers::none(), click_count: 2, first_mouse: false });
    vcx.simulate_event(gpui::MouseUpEvent { position: edge.center(), button: MouseButton::Left, modifiers: Modifiers::none(), click_count: 2 });
    vcx.run_until_parked();
    assert_eq!(vcx.read(|cx| shell.read(cx).sidebar_w), None);
    assert!((width(&mut vcx) - before).abs() < 1.);
}

#[gpui::test]
fn the_pane_area_is_what_the_sidebar_leaves(cx: &mut TestAppContext) {
    let shell = start(cx, None, "wide");
    let main = cx.read(|cx| shell.read(cx).main_window().expect("main"));
    for saved in [None, Some(560.), Some(5000.)] {
        cx.update(|cx| shell.update(cx, |s, _| s.sidebar_w = saved));
        cx.update_window(main.into(), |_, window, _| window.refresh()).unwrap();
        cx.run_until_parked();
        let mut vcx = VisualTestContext::from_window(main.into(), cx);
        let drawn = vcx.debug_bounds("panes").expect("panes drawn");
        let (w, h) = main.update(&mut vcx, |v, window, cx| v.pane_area(window, cx)).unwrap();
        assert!((w - f32::from(drawn.size.width)).abs() < 1. && (h - f32::from(drawn.size.height)).abs() < 1., "{saved:?}: {w}x{h} vs {drawn:?}");
    }
}

/// Adds a monorepo to `work`'s apps: `mono` holding `packages/ui` and `packages/kit`.
fn with_mono(shell: &Entity<Shell>, cx: &mut TestAppContext) {
    let services = cx.read(|cx| shell.read(cx).workspaces.iter().find(|w| w.id == "work").unwrap().services.clone());
    let mut catalog = cx.read(|cx| (*services.read(cx).catalog).clone());
    let base = catalog.apps[0].clone();
    for (id, name) in [("mono", "mono"), ("mono/packages/ui", "ui"), ("mono/packages/kit", "kit")] {
        catalog.apps.push(devdeck_core::apps::App { id: id.into(), name: name.into(), ..base.clone() });
    }
    cx.update(|cx| services.update(cx, |s, cx| s.set_catalog(catalog, cx)));
    cx.run_until_parked();
}

#[gpui::test]
fn what_sits_in_a_moved_folder_stays_put_on_its_rows_and_goes_to_the_top_on_the_header(cx: &mut TestAppContext) {
    let shell = start(cx, None, "nested");
    with_mono(&shell, cx);
    let s = sidebar(&shell, cx);
    cx.update(|cx| {
        s.update(cx, |s, cx| {
            s.create_group(s.activity(), None, "design", cx).unwrap();
            s.put(s.activity(), "mono/packages", Some("design"), cx);
        })
    });
    let all = |g: Option<&str>| Target::All(g.map(String::from));
    cx.read(|cx| {
        let s = s.read(cx);
        assert_eq!(s.item_drop("mono/packages/ui", false, &all(Some("design"))), None, "a drop beside it changes nothing");
        assert_eq!(s.item_drop("mono/packages/ui", false, &Target::AllHeader), Some(Effect::Put("mono/packages/ui".into(), None)));
    });
    // The move menu says where it shows, and its Top level takes it out.
    let menu = cx.update(|cx| Sidebar::move_menu(&s, Moving::Node("mono/packages/ui".into()), cx));
    assert!(menu.submenu.iter().any(|i| i.label.as_ref() == "design" && i.checked) && !menu.submenu[0].checked);
    let main = main_window(&shell, cx);
    let top = menu.submenu[0].run.clone().expect("runs");
    cx.update_window(main, |_, window, cx| top(window, cx)).unwrap();
    assert_eq!(cx.read(|cx| s.read(cx).groups().unwrap().shown_in("mono/packages/ui").map(String::from)), None);
    assert!(rows(&s, cx).iter().any(|r| r == "mono/packages/ui"), "at the top of All");
    assert_eq!(cx.read(|cx| s.read(cx).groups().unwrap().shown_in("mono/packages/kit").map(String::from)).as_deref(), Some("design"), "its sibling stays");
    // A pinned item dropped on the All header: unpinned to the top.
    cx.update(|cx| {
        s.update(cx, |s, cx| {
            s.put(s.activity(), "graphservice", Some("design"), cx);
            s.toggle_pin("graphservice", cx);
        })
    });
    cx.read(|cx| assert_eq!(s.read(cx).item_drop("graphservice", true, &Target::AllHeader), Some(Effect::Put("graphservice".into(), None))));
}

#[gpui::test]
fn the_selection_follows_a_group_when_what_holds_it_changes(cx: &mut TestAppContext) {
    let shell = start(cx, None, "follow");
    let s = sidebar(&shell, cx);
    let key = |p: &str| Some(devdeck_core::list::folder_key(&group_row_path(p)));
    cx.update(|cx| {
        s.update(cx, |s, cx| {
            s.create_group(s.activity(), None, "front", cx).unwrap();
            s.create_group(s.activity(), Some("front"), "web", cx).unwrap();
            s.selected = key("front/web");
            s.rename_group(s.activity(), "front", "FE", cx).unwrap();
        })
    });
    assert_eq!(cx.read(|cx| s.read(cx).selected.clone()), key("FE/web"), "renamed above it");
    cx.update(|cx| s.update(cx, |s, cx| s.delete_group(s.activity(), "FE", cx)));
    assert_eq!(cx.read(|cx| s.read(cx).selected.clone()), key("web"), "moved up by a delete");
}

/// The control API changes a list by name, whichever the sidebar shows (R13.1), and says
/// the state it wants: asking twice changes nothing more.
#[gpui::test]
fn pins_and_groups_change_the_list_named_not_the_one_shown(cx: &mut TestAppContext) {
    let shell = start(cx, None, "list-by-name");
    let s = sidebar(&shell, cx);
    assert_eq!(cx.read(|cx| s.read(cx).activity()), Activity::Apps);
    let pins = |cx: &TestAppContext, list: &str| cx.read(|cx| s.read(cx).state.pins.get(list).cloned().unwrap_or_default());
    cx.update(|cx| {
        s.update(cx, |s, cx| {
            s.set_pinned(Activity::Layouts, "default", true, cx).unwrap();
            s.set_pinned(Activity::Layouts, "default", true, cx).unwrap();
            assert!(s.set_pinned(Activity::Terminals, "1", true, cx).is_err());
            s.create_group(Activity::Cluster, None, "front", cx).unwrap();
            assert!(s.create_group(Activity::Notes, None, "mine", cx).is_err(), "notes go by project");
            assert_eq!(s.set_env("nope", cx), Err("no env nope".into()), "an unknown env is refused");
        })
    });
    assert_eq!(pins(cx, "layouts"), BTreeSet::from(["default".to_string()]), "pinned once");
    assert!(pins(cx, "apps").is_empty(), "the shown list is untouched");
    let groups = |cx: &TestAppContext, list: &str| cx.read(|cx| s.read(cx).state.groups.get(list).map(|g| g.paths()).unwrap_or_default());
    assert_eq!(groups(cx, "deployments"), ["front"]);
    assert!(groups(cx, "apps").is_empty());
    cx.update(|cx| s.update(cx, |s, cx| s.set_pinned(Activity::Layouts, "default", false, cx).unwrap()));
    assert!(pins(cx, "layouts").is_empty());
}
