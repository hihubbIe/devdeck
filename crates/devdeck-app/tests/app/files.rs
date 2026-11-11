//! Files (R12.1, R12.2, R12.4): dropped from outside onto a pane, they open there, view
//! only; problems say why; a file follows its changes while shown; a Markdown file
//! switches between source and rendered from its header, its tab's menu and a key.

use devdeck_app::deck::Deck;
use devdeck_app::shell::Shell;
use devdeck_app::views::file_view::{FileView, RELOAD_EVERY, Shown};
use devdeck_core::layout::{Layout, PaneId, ViewRef};
use devdeck_core::view_addr::ViewAddr;
use devdeck_ui::logical;
use gpui::{Entity, ExternalPaths, FileDropEvent, Modifiers, Pixels, Point, TestAppContext, VisualTestContext, point};
use std::path::{Path, PathBuf};

fn setup(cx: &mut TestAppContext) -> (Entity<Deck>, PathBuf) {
    let dir = cx.update(|cx| devdeck_app::test_dir("files", cx));
    cx.update(|cx| devdeck_app::init_with_config_dir(&dir, cx));
    let shell = cx.update(|cx| Shell::start_single(one_pane, cx));
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    cx.run_until_parked();
    let files = dir.join("files");
    std::fs::create_dir_all(&files).unwrap();
    (deck, files)
}

/// One pane showing the demo's notes.
fn one_pane() -> Layout {
    let mut l = Layout::new();
    let a = l.main().focused;
    l.open(ViewRef::new("demo/notes"), a).expect("open notes");
    l
}

fn window(cx: &mut TestAppContext) -> VisualTestContext {
    let w = cx.read(|cx| cx.windows()[0]);
    let vcx = VisualTestContext::from_window(w, cx);
    vcx.run_until_parked();
    vcx
}

fn file_ref(path: &Path) -> ViewRef {
    ViewAddr::File { path: path.to_str().unwrap().into() }.to_ref()
}

fn the_pane(deck: &Entity<Deck>, cx: &mut VisualTestContext) -> PaneId {
    cx.read(|cx| deck.read(cx).layout().main().focused)
}

fn body(vcx: &mut VisualTestContext, pane: PaneId) -> gpui::Bounds<Pixels> {
    vcx.debug_bounds(Box::leak(format!("pane-body-{}", pane.0).into_boxed_str())).expect("the pane's body")
}

/// Drops `paths` from outside at `at`, as the platform does.
fn drop_files(vcx: &mut VisualTestContext, paths: &[PathBuf], at: Point<Pixels>) {
    vcx.simulate_event(FileDropEvent::Entered { position: at, paths: ExternalPaths(paths.iter().cloned().collect()) });
    vcx.run_until_parked();
    vcx.simulate_event(FileDropEvent::Pending { position: at });
    vcx.run_until_parked();
    vcx.simulate_event(FileDropEvent::Submit { position: at });
    vcx.run_until_parked();
}

fn view(deck: &Entity<Deck>, path: &Path, vcx: &mut VisualTestContext) -> Entity<FileView> {
    vcx.read(|cx| deck.read(cx).view(&file_ref(path)).expect("open").downcast::<FileView>().expect("a file view"))
}

fn text(v: &Entity<FileView>, vcx: &mut VisualTestContext) -> String {
    vcx.read(|cx| v.read(cx).code.as_ref().map(|c| c.read(cx).text(cx).to_string()).unwrap_or_default())
}

fn toasts(vcx: &mut VisualTestContext) -> Vec<String> {
    vcx.update(|_, cx| devdeck_ui::toasts(cx).read(cx).queue.items.iter().map(|t| t.text.to_string()).collect())
}

/// Tells the workspace `path` changed, as the watcher does.
fn changed(deck: &Entity<Deck>, path: &Path, vcx: &mut VisualTestContext) {
    let services = vcx.read(|cx| deck.read(cx).services().expect("a workspace"));
    vcx.update(|_, cx| services.update(cx, |s, cx| s.files_changed(&[path.to_path_buf()], cx)));
    vcx.run_until_parked();
}

#[gpui::test]
fn a_dropped_file_opens_in_the_pane(cx: &mut TestAppContext) {
    let (deck, files) = setup(cx);
    let a = files.join("main.rs");
    std::fs::write(&a, "fn main() {}\n").unwrap();
    let mut vcx = window(cx);
    let pane = the_pane(&deck, &mut vcx);
    let at = body(&mut vcx, pane).center();
    drop_files(&mut vcx, std::slice::from_ref(&a), at);
    let tabs = vcx.read(|cx| deck.read(cx).layout().pane(pane).unwrap().tabs.clone());
    assert!(tabs.contains(&file_ref(&a)), "{tabs:?}");
    let v = view(&deck, &a, &mut vcx);
    assert_eq!(vcx.read(|cx| v.read(cx).shown.clone()), Shown::Text { plain: false });
    assert_eq!(text(&v, &mut vcx), "fn main() {}\n");
    let title = vcx.read(|cx| deck.read(cx).title(&file_ref(&a), cx));
    assert_eq!(title, "main.rs");
}

#[gpui::test]
fn files_dropped_on_an_edge_share_one_new_pane(cx: &mut TestAppContext) {
    let (deck, files) = setup(cx);
    let (a, b) = (files.join("a.rs"), files.join("b.rs"));
    std::fs::write(&a, "").unwrap();
    std::fs::write(&b, "").unwrap();
    let mut vcx = window(cx);
    let pane = the_pane(&deck, &mut vcx);
    let bounds = body(&mut vcx, pane);
    let right_edge = point(bounds.right() - logical(10.), bounds.center().y);
    drop_files(&mut vcx, &[a.clone(), b.clone()], right_edge);
    let l = vcx.read(|cx| deck.read(cx).layout().clone());
    assert_eq!(l.panes.len(), 2, "one split");
    assert_eq!(l.find_view(&file_ref(&a)).unwrap().0, l.find_view(&file_ref(&b)).unwrap().0);
    assert_ne!(l.find_view(&file_ref(&a)).unwrap().0, pane);
}

#[gpui::test]
fn folders_are_refused_and_named(cx: &mut TestAppContext) {
    let (deck, files) = setup(cx);
    let a = files.join("a.md");
    std::fs::write(&a, "# a\n").unwrap();
    let mut vcx = window(cx);
    let pane = the_pane(&deck, &mut vcx);
    let at = body(&mut vcx, pane).center();
    drop_files(&mut vcx, &[files.clone(), a.clone()], at);
    assert!(vcx.read(|cx| deck.read(cx).layout().find_view(&file_ref(&a)).is_some()), "the file opens");
    assert!(toasts(&mut vcx).iter().any(|t| t.contains("files (a folder)")), "{:?}", toasts(&mut vcx));
}

#[gpui::test]
fn a_file_already_open_moves_where_it_is_dropped(cx: &mut TestAppContext) {
    let (deck, files) = setup(cx);
    let a = files.join("a.rs");
    std::fs::write(&a, "").unwrap();
    let mut vcx = window(cx);
    let pane = the_pane(&deck, &mut vcx);
    let at = body(&mut vcx, pane).center();
    drop_files(&mut vcx, std::slice::from_ref(&a), at);
    let bounds = body(&mut vcx, pane);
    drop_files(&mut vcx, std::slice::from_ref(&a), point(bounds.left() + logical(10.), bounds.center().y));
    let l = vcx.read(|cx| deck.read(cx).layout().clone());
    let count = l.panes.values().flat_map(|p| &p.tabs).filter(|r| **r == file_ref(&a)).count();
    assert_eq!(count, 1, "open once");
    assert_ne!(l.find_view(&file_ref(&a)).unwrap().0, pane, "moved to the new left pane");
}

#[gpui::test]
fn problems_say_why(cx: &mut TestAppContext) {
    let (deck, files) = setup(cx);
    let (bin, big, wide) = (files.join("x.bin"), files.join("big.log"), files.join("min.js"));
    std::fs::write(&bin, b"ab\0cd").unwrap();
    std::fs::File::create(&big).unwrap().set_len(devdeck_core::files::MAX_BYTES + 1).unwrap();
    std::fs::write(&wide, "x".repeat(devdeck_ui::code::HIGHLIGHT_MAX_LINE)).unwrap();
    let mut vcx = window(cx);
    let pane = the_pane(&deck, &mut vcx);
    let at = body(&mut vcx, pane).center();
    drop_files(&mut vcx, &[bin.clone(), big.clone(), wide.clone()], at);
    let shown = |p: &Path, vcx: &mut VisualTestContext| {
        let v = view(&deck, p, vcx);
        vcx.read(|cx| v.read(cx).shown.clone())
    };
    assert_eq!(shown(&bin, &mut vcx), Shown::Problem("x.bin isn't text.".into()));
    assert_eq!(shown(&big, &mut vcx), Shown::Problem("big.log is 10 MB: files over 10 MB aren't shown.".into()));
    assert_eq!(shown(&wide, &mut vcx), Shown::Text { plain: true });
    let r = file_ref(&wide);
    vcx.update(|_, cx| deck.update(cx, |d, cx| d.apply(cx, |l| { let (p, i) = l.find_view(&r).unwrap(); l.activate(p, i) })));
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("file-plain").is_some(), "says why it's plain");
}

#[gpui::test]
fn it_follows_the_file_while_shown(cx: &mut TestAppContext) {
    let (deck, files) = setup(cx);
    let a = files.join("a.rs");
    std::fs::write(&a, "one\n").unwrap();
    let mut vcx = window(cx);
    let pane = the_pane(&deck, &mut vcx);
    let at = body(&mut vcx, pane).center();
    drop_files(&mut vcx, std::slice::from_ref(&a), at);
    let v = view(&deck, &a, &mut vcx);
    std::fs::write(&a, "two\n").unwrap();
    changed(&deck, &a, &mut vcx);
    vcx.executor().advance_clock(RELOAD_EVERY);
    vcx.run_until_parked();
    assert_eq!(text(&v, &mut vcx), "two\n", "read again, once a second at most");

    // Hidden behind another tab: not read until it shows again.
    let notes = ViewRef::new("demo/notes");
    vcx.update(|_, cx| deck.update(cx, |d, cx| d.apply(cx, |l| { let (p, i) = l.find_view(&notes).unwrap(); l.activate(p, i) })));
    vcx.run_until_parked();
    std::fs::write(&a, "three\n").unwrap();
    changed(&deck, &a, &mut vcx);
    vcx.executor().advance_clock(RELOAD_EVERY * 2);
    vcx.run_until_parked();
    assert_eq!(vcx.read(|cx| v.read(cx).code.as_ref().map(|c| c.read(cx).text(cx).to_string())).unwrap(), "two\n", "hidden: not read");
    let r = file_ref(&a);
    vcx.update(|_, cx| deck.update(cx, |d, cx| d.apply(cx, |l| { let (p, i) = l.find_view(&r).unwrap(); l.activate(p, i) })));
    vcx.run_until_parked();
    assert_eq!(text(&v, &mut vcx), "three\n", "read on showing");

    // Deleted, then back.
    std::fs::remove_file(&a).unwrap();
    changed(&deck, &a, &mut vcx);
    vcx.executor().advance_clock(RELOAD_EVERY);
    vcx.run_until_parked();
    assert_eq!(vcx.read(|cx| v.read(cx).shown.clone()), Shown::Problem("a.rs isn't there anymore.".into()));
    std::fs::write(&a, "four\n").unwrap();
    changed(&deck, &a, &mut vcx);
    vcx.executor().advance_clock(RELOAD_EVERY);
    vcx.run_until_parked();
    assert_eq!(text(&v, &mut vcx), "four\n");
}

#[gpui::test]
fn markdown_switches_from_the_header_the_menu_and_a_key(cx: &mut TestAppContext) {
    let (deck, files) = setup(cx);
    let a = files.join("README.md");
    std::fs::write(&a, "# Title\n\n- [ ] a task\n").unwrap();
    let mut vcx = window(cx);
    let pane = the_pane(&deck, &mut vcx);
    let at = body(&mut vcx, pane).center();
    drop_files(&mut vcx, std::slice::from_ref(&a), at);
    let v = view(&deck, &a, &mut vcx);
    let pane = vcx.read(|cx| deck.read(cx).layout().find_view(&file_ref(&a)).unwrap().0);
    let rendered = |vcx: &mut VisualTestContext| vcx.read(|cx| v.read(cx).rendered);
    assert!(rendered(&mut vcx), "a Markdown file opens rendered");
    assert!(vcx.debug_bounds("file-rendered").is_some());
    let actions = vcx.read(|cx| deck.read(cx).view_actions(&file_ref(&a), cx));
    assert_eq!(actions.len(), 1);
    assert!(actions[0].on);

    // The header's button.
    let button = vcx.debug_bounds("tool-view.toggle_rendered").expect("in the header");
    vcx.simulate_click(button.center(), Modifiers::none());
    vcx.run_until_parked();
    assert!(!rendered(&mut vcx), "the button shows the source");
    assert!(!vcx.read(|cx| deck.read(cx).view_actions(&file_ref(&a), cx))[0].on, "the header follows");

    // The key, with the focus in the text.
    vcx.simulate_keystrokes("ctrl-shift-e");
    vcx.run_until_parked();
    assert!(rendered(&mut vcx), "the key shows it rendered");

    // The tab's menu lists it first.
    let i = vcx.read(|cx| deck.read(cx).layout().find_view(&file_ref(&a)).unwrap().1);
    let header = vcx.debug_bounds(Box::leak(format!("tab-{}-{i}", pane.0).into_boxed_str())).expect("its tab");
    vcx.simulate_mouse_down(header.center(), gpui::MouseButton::Right, Modifiers::none());
    vcx.run_until_parked();
    let w = vcx.update(|window, _| window.window_handle());
    let menu = vcx.update(|_, cx| devdeck_ui::overlay::layer(w, cx).read(cx).menu.clone()).expect("a menu");
    let first = vcx.read(|cx| menu.read(cx).items()[0].label.to_string());
    assert_eq!(first, "source or rendered");
}

#[gpui::test]
fn what_is_shown_survives_a_restart(cx: &mut TestAppContext) {
    use devdeck_app::views::KeepsState;
    let (deck, files) = setup(cx);
    let a = files.join("a.md");
    std::fs::write(&a, "# a\n").unwrap();
    let mut vcx = window(cx);
    let pane = the_pane(&deck, &mut vcx);
    let at = body(&mut vcx, pane).center();
    drop_files(&mut vcx, std::slice::from_ref(&a), at);
    let v = view(&deck, &a, &mut vcx);
    vcx.update(|window, cx| v.update(cx, |v, cx| v.toggle_rendered(window, cx)));
    let saved = vcx.read(|cx| v.read(cx).save(cx));
    assert_eq!(saved["rendered"], false);
    vcx.update(|_, cx| v.update(cx, |v, cx| {
        v.rendered = true;
        v.restore(&saved, cx);
    }));
    assert!(!vcx.read(|cx| v.read(cx).rendered));
}

#[gpui::test]
fn the_tab_menu_runs_the_action_on_a_tab_behind(cx: &mut TestAppContext) {
    let (deck, files) = setup(cx);
    let a = files.join("a.md");
    std::fs::write(&a, "# a\n").unwrap();
    let mut vcx = window(cx);
    let pane = the_pane(&deck, &mut vcx);
    let at = body(&mut vcx, pane).center();
    drop_files(&mut vcx, std::slice::from_ref(&a), at);
    let v = view(&deck, &a, &mut vcx);
    // The notes tab comes in front; the file's is behind.
    let notes = ViewRef::new("demo/notes");
    vcx.update(|_, cx| deck.update(cx, |d, cx| d.apply(cx, |l| { let (p, i) = l.find_view(&notes).unwrap(); l.activate(p, i) })));
    vcx.run_until_parked();
    let i = vcx.read(|cx| deck.read(cx).layout().find_view(&file_ref(&a)).unwrap().1);
    let tab = vcx.debug_bounds(Box::leak(format!("tab-{}-{i}", pane.0).into_boxed_str())).expect("its tab");
    vcx.simulate_mouse_down(tab.center(), gpui::MouseButton::Right, Modifiers::none());
    vcx.run_until_parked();
    let w = vcx.update(|window, _| window.window_handle());
    let menu = vcx.update(|_, cx| devdeck_ui::overlay::layer(w, cx).read(cx).menu.clone()).expect("a menu");
    let run = vcx.read(|cx| menu.read(cx).items()[0].run.clone()).expect("runs");
    vcx.update(|window, cx| run(window, cx));
    vcx.run_until_parked();
    assert!(!vcx.read(|cx| v.read(cx).rendered), "switched to the source");
    let front = vcx.read(|cx| deck.read(cx).layout().pane(pane).unwrap().active_view().cloned());
    assert_eq!(front, Some(file_ref(&a)), "and brought forward");
}

#[gpui::test]
fn a_closed_files_folder_stops_being_watched(cx: &mut TestAppContext) {
    let (deck, files) = setup(cx);
    let a = files.join("a.rs");
    std::fs::write(&a, "").unwrap();
    let mut vcx = window(cx);
    let pane = the_pane(&deck, &mut vcx);
    let at = body(&mut vcx, pane).center();
    drop_files(&mut vcx, std::slice::from_ref(&a), at);
    let services = vcx.read(|cx| deck.read(cx).services().unwrap());
    assert_eq!(vcx.read(|cx| services.read(cx).file_watch(cx)), vec![files.clone()]);
    let r = file_ref(&a);
    vcx.update(|_, cx| deck.update(cx, |d, cx| d.apply(cx, |l| l.close_view(&r))));
    vcx.run_until_parked();
    assert!(vcx.read(|cx| services.read(cx).file_watch(cx)).is_empty());
}

/// A file shows a line asked for (the control API's `file.open`): once its text is read
/// and the editor drawn, even if asked while hidden, and never moving the focus.
#[gpui::test]
fn a_file_goes_to_a_line_without_taking_the_focus(cx: &mut TestAppContext) {
    let (deck, files) = setup(cx);
    let a = files.join("long.rs");
    std::fs::write(&a, (1..=80).map(|i| format!("line {i}\n")).collect::<String>()).unwrap();
    let mut vcx = window(cx);
    let pane = the_pane(&deck, &mut vcx);
    let at = body(&mut vcx, pane).center();
    drop_files(&mut vcx, std::slice::from_ref(&a), at);
    let v = view(&deck, &a, &mut vcx);
    let caret = |v: &Entity<FileView>, vcx: &mut VisualTestContext| vcx.read(|cx| v.read(cx).code.as_ref().map(|c| c.read(cx).caret(cx)));
    // Hidden behind another tab when asked: it goes there once shown.
    let notes = ViewRef::new("demo/notes");
    vcx.update(|_, cx| deck.update(cx, |d, cx| d.apply(cx, |l| { let (p, i) = l.find_view(&notes).unwrap(); l.activate(p, i) })));
    vcx.run_until_parked();
    let focused = vcx.update(|window, cx| window.focused(cx));
    vcx.update(|_, cx| v.update(cx, |v, cx| v.go_to_line(60, cx)));
    vcx.run_until_parked();
    assert_eq!(vcx.update(|window, cx| window.focused(cx)), focused, "the focus didn't move");
    let r = file_ref(&a);
    vcx.update(|_, cx| deck.update(cx, |d, cx| d.apply(cx, |l| { let (p, i) = l.find_view(&r).unwrap(); l.activate(p, i) })));
    vcx.run_until_parked();
    assert_eq!(caret(&v, &mut vcx), Some((60, 1)));
    // Past the end: the last line.
    vcx.update(|_, cx| v.update(cx, |v, cx| v.go_to_line(500, cx)));
    vcx.run_until_parked();
    assert_eq!(caret(&v, &mut vcx), Some((81, 1)), "the empty line after the last break");
}
