//! The right-click menu (R5.7): a terminal's selection offers Copy and Share with, which
//! pastes it as it is into another terminal of the workspace that takes pastes safely,
//! and shows that terminal. A recording driver stands in for the shells.

use devdeck_app::context_menu::{self, Content, Origin};
use devdeck_app::deck::Deck;
use devdeck_app::places::Place;
use devdeck_app::runner::{Launch, RunControl, RunDriver, RunId, RunKey, Runner, Started};
use devdeck_app::shell::Shell;
use devdeck_app::views::run_view::RunView;
use devdeck_core::layout::{MAIN_WINDOW, ViewRef};
use devdeck_core::terminal::TermSize;
use devdeck_core::view_addr::ViewAddr;
use devdeck_ui::MenuItem;
use devdeck_ui::overlay::layer_state;
use gpui::{AnyWindowHandle, AppContext, Entity, Modifiers, MouseButton, TestAppContext, VisualTestContext};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

/// What a process was sent.
type Input = Rc<RefCell<Vec<u8>>>;

/// Each start: its id, folder and input.
#[derive(Clone, Default)]
struct Recorded(Rc<RefCell<Vec<(RunId, PathBuf, Input)>>>);

struct Control(Rc<RefCell<Vec<u8>>>);

impl RunControl for Control {
    fn write(&self, bytes: &[u8]) {
        self.0.borrow_mut().extend_from_slice(bytes);
    }
    fn resize(&self, _: TermSize) {}
    fn stop(&self) {}
}

impl RunDriver for Recorded {
    fn spawn(&self, launch: Launch, _: &mut gpui::Context<Runner>) -> Result<Started, String> {
        let input = Rc::new(RefCell::new(Vec::new()));
        self.0.borrow_mut().push((launch.id, launch.app.dir.clone(), input.clone()));
        Ok(Started { control: Box::new(Control(input)), task: gpui::Task::ready(()) })
    }
}

struct Setup {
    window: AnyWindowHandle,
    runner: Entity<Runner>,
    deck: Entity<Deck>,
    services: Entity<devdeck_app::services::Services>,
    rec: Recorded,
    /// The terminals opened, in order: (key, view).
    terms: Vec<(RunKey, ViewRef)>,
}

impl Setup {
    /// What terminal `i`'s shell was sent, taken.
    fn input(&self, i: usize) -> String {
        String::from_utf8(std::mem::take(&mut *self.rec.0.borrow()[i].2.borrow_mut())).unwrap()
    }

    /// Terminal `i`'s program turns bracketed paste on, as bash and zsh do.
    fn takes_pastes(&self, i: usize, cx: &mut TestAppContext) {
        let id = self.rec.0.borrow()[i].0.clone();
        cx.update(|cx| self.runner.update(cx, |r, cx| r.output(&id, b"\x1b[?2004h$ ", cx)));
        cx.run_until_parked();
    }

    fn origin(&self, source: usize) -> Origin {
        Origin { deck: self.deck.downgrade(), services: self.services.clone(), window: MAIN_WINDOW, source: Some(self.terms[source].0.clone()) }
    }
}

/// `work` with a recording runner and a terminal open in each of `dirs` (apps' folders).
fn setup(cx: &mut TestAppContext, tag: &str, apps: &[&str]) -> Setup {
    let config = cx.update(|cx| devdeck_app::test_dir(tag, cx));
    cx.update(|cx| devdeck_app::init_with_config_dir(&config, cx));
    let shell = cx.update(|cx| Shell::start(devdeck_app::demo_workspaces(), "work", None, cx));
    cx.update(|cx| shell.update(cx, |s, _| s.quit_for_real = false));
    cx.run_until_parked();
    let rec = Recorded::default();
    let services = cx.read(|cx| shell.read(cx).workspaces.iter().find(|w| w.id == "work").unwrap().services.clone());
    let catalog = cx.read(|cx| services.read(cx).catalog.clone());
    let runner = cx.new(|_| Runner::new("work".into(), catalog.clone(), Rc::new(rec.clone())));
    cx.update(|cx| services.update(cx, |s, _| s.runner = runner.clone()));
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    let window = cx.read(|cx| shell.read(cx).main_window().unwrap().into());
    let mut terms = Vec::new();
    for app in apps {
        let dir = catalog.app(app).unwrap().dir.clone();
        let focused = cx.read(|cx| deck.read(cx).layout().main().focused);
        cx.update(|cx| deck.update(cx, |d, cx| d.open_terminal(&dir, Place::Pane(focused), cx)));
        cx.run_until_parked();
        // The newest terminal: the highest number.
        let (n, view) = cx
            .read(|cx| {
                deck.read(cx).layout().views().into_iter().filter_map(|r| match ViewAddr::parse(&r) {
                    ViewAddr::Terminal { n, .. } => Some((n, r)),
                    _ => None,
                })
                .max_by_key(|(n, _)| *n)
            })
            .unwrap();
        terms.push((RunKey::terminal(n), view));
    }
    assert_eq!(rec.0.borrow().len(), apps.len(), "each shell started");
    Setup { window, runner, deck, services, rec, terms }
}

fn labels(items: &[MenuItem]) -> Vec<String> {
    items.iter().map(|i| format!("{}{}", i.label, i.disabled.as_ref().map(|w| format!(" ({w})")).unwrap_or_default())).collect()
}

#[gpui::test]
fn share_with_offers_the_other_terminals_by_short_name_and_pastes_the_text_as_it_is(cx: &mut TestAppContext) {
    let s = setup(cx, "share", &["graphservice", "eventservice", "eventservice"]);
    s.takes_pastes(1, cx);
    let origin = s.origin(0);
    let text = Content::Text("$ make\nerror: x\x03 failed".into());
    let items = cx.read(|cx| context_menu::items(Vec::new(), Some(&text), &origin, cx));
    assert_eq!(labels(&items), ["Copy", "Share with"]);
    let name = |i: usize| devdeck_app::views::terminal_name(&s.rec.0.borrow()[i].1.display().to_string());
    let (n1, n2) = (s.terms[1].0.terminal_number().unwrap(), s.terms[2].0.terminal_number().unwrap());
    assert_eq!(
        labels(&items[1].submenu),
        [format!("{} · {n1}", name(1)), format!("{} · {n2} (doesn't take pastes safely)", name(2))],
        "the source left out, short names, numbered when two share one, a shell without bracketed paste refused"
    );
    // Its own click: the first that takes text. It pastes, no Enter, control characters out.
    let run = items[1].run.clone().unwrap();
    cx.update_window(s.window, |_, window, cx| run(window, cx)).unwrap();
    cx.run_until_parked();
    assert_eq!(s.input(1), "\x1b[200~$ make\rerror: x failed\x1b[201~");
    assert_eq!(s.input(2), "", "nothing to the refusing one");
    // The target is shown, keys going to it.
    let (pane, active) = cx.read(|cx| {
        let l = s.deck.read(cx).layout();
        let p = l.main().focused;
        (p, l.pane(p).unwrap().active_view().cloned())
    });
    assert_eq!(active.as_ref(), Some(&s.terms[1].1), "its tab in front in pane {pane:?}");
    let view = cx.read(|cx| s.deck.read(cx).view(&s.terms[1].1).unwrap().downcast::<RunView>().unwrap());
    assert!(cx.update_window(s.window, |_, window, cx| view.read(cx).terminal_focused(window)).unwrap());
    // Once the third takes pastes, text sent there from the list makes it the own
    // click's (not the first any more).
    s.takes_pastes(2, cx);
    let share = |cx: &mut TestAppContext, pick: Option<usize>| {
        let items = cx.read(|cx| context_menu::items(Vec::new(), Some(&text), &origin, cx));
        let run = match pick {
            Some(i) => items[1].submenu[i].run.clone().unwrap(),
            None => items[1].run.clone().unwrap(),
        };
        cx.update_window(s.window, |_, window, cx| run(window, cx)).unwrap();
        cx.run_until_parked();
    };
    share(cx, Some(1));
    assert!(s.input(2).contains("make") && s.input(1).is_empty(), "the list's pick");
    share(cx, None);
    assert!(s.input(2).contains("make") && s.input(1).is_empty(), "where text went last");
}

#[gpui::test]
fn when_no_terminal_takes_pastes_the_list_still_says_why(cx: &mut TestAppContext) {
    let s = setup(cx, "share-refused", &["eventservice", "graphservice"]);
    let text = Content::Text("x".into());
    let items = cx.read(|cx| context_menu::items(Vec::new(), Some(&text), &s.origin(0), cx));
    assert_eq!(labels(&items), ["Copy", "Share with"], "open, so its list shows");
    assert_eq!(labels(&items[1].submenu), ["graphservice (doesn't take pastes safely)"]);
    let run = items[1].run.clone().unwrap();
    cx.update_window(s.window, |_, window, cx| run(window, cx)).unwrap();
    assert_eq!(s.input(1), "", "its own click sends nothing");
}

#[gpui::test]
fn sharing_from_a_detached_window_focuses_the_target_in_the_main_one(cx: &mut TestAppContext) {
    let s = setup(cx, "share-detached", &["eventservice", "graphservice"]);
    s.takes_pastes(1, cx);
    let source = s.terms[0].1.clone();
    cx.update(|cx| {
        s.deck.update(cx, |d, cx| {
            d.apply(cx, |l| {
                let (p, i) = l.find_view(&source).unwrap();
                l.detach(p, i).map(drop)
            })
        })
    });
    cx.run_until_parked();
    let detached = cx.read(|cx| {
        let l = s.deck.read(cx).layout();
        l.window_of(l.find_view(&source).unwrap().0).unwrap()
    });
    assert_ne!(detached, MAIN_WINDOW);
    let origin = Origin { window: detached, ..s.origin(0) };
    let text = Content::Text("x".into());
    let items = cx.read(|cx| context_menu::items(Vec::new(), Some(&text), &origin, cx));
    let run = items[1].run.clone().unwrap();
    let other = cx.read(|cx| s.deck.read(cx).os_window(detached)).expect("its window opened");
    let view = cx.read(|cx| s.deck.read(cx).view(&s.terms[1].1).unwrap().downcast::<RunView>().unwrap());
    cx.update_window(s.window, |_, window, cx| window.blur(cx)).unwrap();
    assert!(!cx.update_window(s.window, |_, window, cx| view.read(cx).terminal_focused(window)).unwrap());
    cx.update_window(other.into(), |_, window, cx| run(window, cx)).unwrap();
    cx.run_until_parked();
    assert!(s.input(1).contains('x'));
    assert!(cx.update_window(s.window, |_, window, cx| view.read(cx).terminal_focused(window)).unwrap(), "keys go to it in the main window");
}

#[gpui::test]
fn sharing_into_a_terminal_behind_a_maximized_pane_brings_it_forward(cx: &mut TestAppContext) {
    let s = setup(cx, "share-max", &["eventservice", "graphservice"]);
    s.takes_pastes(1, cx);
    // The target's tab moves to another pane, which then isn't drawn: the first is maximized.
    let (other, first) = cx.read(|cx| {
        let l = s.deck.read(cx).layout();
        let first = l.main().focused;
        (l.panes_in(MAIN_WINDOW).unwrap().into_iter().find(|p| *p != first).unwrap(), first)
    });
    let target = s.terms[1].1.clone();
    cx.update(|cx| {
        s.deck.update(cx, |d, cx| {
            d.apply(cx, |l| {
                devdeck_app::places::open(l, target.clone(), Place::Pane(other))?;
                l.toggle_maximize(first)
            })
        })
    });
    cx.run_until_parked();
    let text = Content::Text("x".into());
    let items = cx.read(|cx| context_menu::items(Vec::new(), Some(&text), &s.origin(0), cx));
    let run = items[1].run.clone().unwrap();
    cx.update_window(s.window, |_, window, cx| run(window, cx)).unwrap();
    cx.run_until_parked();
    let (maximized, focused) = cx.read(|cx| (s.deck.read(cx).layout().main().maximized, s.deck.read(cx).layout().main().focused));
    assert_eq!((maximized, focused), (None, other), "restored, the target's pane focused");
}

#[gpui::test]
fn share_with_says_why_when_there_is_nowhere_to_paste(cx: &mut TestAppContext) {
    let s = setup(cx, "share-none", &["eventservice"]);
    let text = Content::Text("x".into());
    let items = cx.read(|cx| context_menu::items(Vec::new(), Some(&text), &s.origin(0), cx));
    assert_eq!(labels(&items), ["Copy", "Share with (no other terminal open)"]);
}

#[gpui::test]
fn a_right_click_opens_the_menu_with_what_applies(cx: &mut TestAppContext) {
    let s = setup(cx, "right-click", &["eventservice"]);
    let view = cx.read(|cx| s.deck.read(cx).view(&s.terms[0].1).unwrap().downcast::<RunView>().unwrap());
    s.takes_pastes(0, cx);
    let mut vcx = VisualTestContext::from_window(s.window, cx);
    vcx.run_until_parked();
    let at = view.read_with(&vcx, |v, _| v.cell_center(0, 0)).expect("drawn");
    // Nothing selected: Paste only (the shell runs).
    vcx.simulate_mouse_down(at, MouseButton::Right, Modifiers::none());
    vcx.run_until_parked();
    assert!(vcx.update(|_, cx| layer_state(s.window, cx).0), "the menu");
    let menu = vcx.update(|_, cx| devdeck_ui::overlay::layer(s.window, cx).read(cx).menu.clone().unwrap());
    assert_eq!(vcx.read(|cx| labels(menu.read(cx).items())), ["Paste"]);
    vcx.simulate_keystrokes("escape");
    // A selection: Copy and Share with too.
    vcx.update(|_, cx| view.update(cx, |v, cx| v.select((0, 0), (0, 1), cx)));
    vcx.simulate_mouse_down(at, MouseButton::Right, Modifiers::none());
    vcx.run_until_parked();
    let menu = vcx.update(|_, cx| devdeck_ui::overlay::layer(s.window, cx).read(cx).menu.clone().unwrap());
    assert_eq!(vcx.read(|cx| labels(menu.read(cx).items())), ["Paste", "", "Copy", "Share with (no other terminal open)"]);
    vcx.simulate_keystrokes("escape");
    // The menu key: the same menu at the selection; without one, the tab menu.
    vcx.update(|window, cx| view.update(cx, |v, cx| v.focus_terminal(window, cx)));
    vcx.update(|window, cx| window.dispatch_action(Box::new(devdeck_app::actions::ContextMenu), cx));
    vcx.run_until_parked();
    let menu = vcx.update(|_, cx| devdeck_ui::overlay::layer(s.window, cx).read(cx).menu.clone().unwrap());
    assert_eq!(vcx.read(|cx| labels(menu.read(cx).items()))[2], "Copy");
    vcx.simulate_keystrokes("escape");
    vcx.update(|_, cx| view.update(cx, |v, cx| v.clear_selection(cx)));
    vcx.update(|window, cx| window.dispatch_action(Box::new(devdeck_app::actions::ContextMenu), cx));
    vcx.run_until_parked();
    let menu = vcx.update(|_, cx| devdeck_ui::overlay::layer(s.window, cx).read(cx).menu.clone().unwrap());
    assert!(vcx.read(|cx| labels(menu.read(cx).items())).contains(&"Close".to_string()), "the tab menu");
}
