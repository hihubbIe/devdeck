//! M2 step 2 (builder F): a run's terminal takes keys for its process, except `alt+…` and
//! `ctrl+shift+…`; copy and paste; replies with the theme's colors (R6.7, R5.1). A
//! recording driver stands in for processes, on GPUI's test platform.

use devdeck_app::registry;
use devdeck_app::runner::{Launch, RunControl, RunDriver, RunId, RunKey, Runner, Started};
use devdeck_app::shell::Shell;
use devdeck_app::views::run_view::RunView;
use devdeck_core::runs::RunState;
use devdeck_core::terminal::TermSize;
use devdeck_core::view_addr::ViewAddr;
use gpui::{AnyWindowHandle, AppContext, ClipboardItem, Entity, TestAppContext};
use std::cell::RefCell;
use std::rc::Rc;

/// What the processes were sent, and the starts.
#[derive(Clone, Default)]
struct Recorded {
    input: Rc<RefCell<Vec<u8>>>,
    starts: Rc<RefCell<Vec<RunId>>>,
}

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
        self.starts.borrow_mut().push(launch.id);
        Ok(Started { control: Box::new(Control(self.input.clone())), task: gpui::Task::ready(()) })
    }
}

struct Setup {
    window: AnyWindowHandle,
    runner: Entity<Runner>,
    view: Entity<RunView>,
    rec: Recorded,
    key: RunKey,
    deck: Entity<devdeck_app::deck::Deck>,
}

impl Setup {
    fn input(&self) -> String {
        String::from_utf8(std::mem::take(&mut *self.rec.input.borrow_mut())).unwrap()
    }

    fn output(&self, cx: &mut TestAppContext, bytes: &[u8]) {
        let id = self.rec.starts.borrow().last().cloned().unwrap();
        cx.update(|cx| self.runner.update(cx, |r, cx| r.output(&id, bytes, cx)));
        cx.run_until_parked();
    }

    fn state(&self, cx: &TestAppContext) -> Option<RunState> {
        cx.read(|cx| self.runner.read(cx).state(&self.key))
    }
}

/// The work workspace with a recording runner, eventservice's server running and its
/// output open in the focused pane (the run view has the keys).
fn setup(cx: &mut TestAppContext, tag: &str) -> Setup {
    let config = cx.update(|cx| devdeck_app::test_dir(tag, cx));
    cx.update(|cx| devdeck_app::init_with_config_dir(&config, cx));
    let shell = cx.update(|cx| Shell::start(devdeck_app::demo_workspaces(), "work", None, cx));
    cx.update(|cx| shell.update(cx, |s, _| s.quit_for_real = false));
    cx.run_until_parked();
    let rec = Recorded::default();
    let services = cx.read(|cx| shell.read(cx).workspaces.iter().find(|w| w.id == "work").unwrap().services.clone());
    let catalog = cx.read(|cx| services.read(cx).catalog.clone());
    let runner = cx.new(|_| Runner::new("work".into(), catalog, Rc::new(rec.clone())));
    cx.update(|cx| services.update(cx, |s, _| s.runner = runner.clone()));
    let key = RunKey::new("eventservice", "go:run");
    cx.update(|cx| runner.update(cx, |r, cx| r.start(&key, None, cx))).unwrap();
    let addr = ViewAddr::Run { app: key.app.clone(), cmd: key.cmd.clone() };
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    cx.update(|cx| deck.update(cx, |d, cx| d.apply(cx, |l| l.open(addr.to_ref(), l.main().focused).map(|_| ()))));
    cx.run_until_parked();
    let view = cx.read(|cx| deck.read(cx).view(&addr.to_ref()).unwrap().downcast::<RunView>().unwrap());
    let window = cx.read(|cx| shell.read(cx).main_window().expect("main").into());
    let s = Setup { window, runner, view, rec, key, deck };
    s.input();
    s
}

#[gpui::test]
fn keys_in_the_terminal_go_to_the_process_not_to_devdeck(cx: &mut TestAppContext) {
    let s = setup(cx, "keys");
    cx.simulate_keystrokes(s.window, "i");
    cx.run_until_parked();
    assert!(cx.update_window(s.window, |_, w, cx| s.view.read(cx).terminal_focused(w)).unwrap(), "i gives the keys to the terminal");
    assert_eq!(s.input(), "", "i itself isn't typed");
    // Run-view keys and app keys (ctrl+w closes a tab, ctrl+k opens the palette) are the process's.
    cx.simulate_keystrokes(s.window, "s r enter ctrl-c ctrl-w ctrl-k shift-a up f5");
    cx.run_until_parked();
    assert_eq!(s.input(), "sr\r\x03\x17\x0bA\x1b[A\x1b[15~");
    assert_eq!(s.state(cx), Some(RunState::Running), "s didn't stop it");
    let tab = ViewAddr::Run { app: s.key.app.clone(), cmd: s.key.cmd.clone() }.to_ref();
    assert!(cx.read(|cx| s.deck.read(cx).view(&tab).is_some()), "ctrl+w didn't close the tab");
    // Application cursor mode changes what arrows send.
    s.output(cx, b"\x1b[?1h");
    cx.simulate_keystrokes(s.window, "up");
    cx.run_until_parked();
    assert_eq!(s.input(), "\x1bOA");
    // ctrl+shift+… stays devdeck's.
    cx.simulate_keystrokes(s.window, "ctrl-shift-x");
    cx.run_until_parked();
    assert_eq!(s.state(cx), Some(RunState::Stopping), "ctrl+shift+x stops");
    assert_eq!(s.input(), "");
}

#[gpui::test]
fn leaving_the_terminal_gives_the_keys_back_to_the_run_view(cx: &mut TestAppContext) {
    let s = setup(cx, "leave");
    cx.simulate_keystrokes(s.window, "i ctrl-shift-escape s");
    cx.run_until_parked();
    assert_eq!(s.input(), "");
    assert_eq!(s.state(cx), Some(RunState::Stopping), "s stops again");
}

#[gpui::test]
fn once_the_process_ended_the_run_views_keys_work_again(cx: &mut TestAppContext) {
    let s = setup(cx, "ended");
    cx.simulate_keystrokes(s.window, "i");
    cx.run_until_parked();
    let id = s.rec.starts.borrow().last().cloned().unwrap();
    cx.update(|cx| s.runner.update(cx, |r, cx| r.exited(&id, devdeck_core::runs::Exit::Code(0), cx)));
    cx.run_until_parked();
    // The terminal still has focus, but nothing takes keys there: ⏎ runs it again.
    cx.simulate_keystrokes(s.window, "enter");
    cx.run_until_parked();
    assert_eq!(s.input(), "");
    assert_eq!(s.rec.starts.borrow().len(), 2, "⏎ ran it again");
    assert_eq!(s.state(cx), Some(RunState::Running));
}

#[gpui::test]
fn copy_takes_the_selection_and_paste_honours_bracketed_paste(cx: &mut TestAppContext) {
    let s = setup(cx, "clipboard");
    s.output(cx, b"hello world\r\nsecond line\r\n");
    let rows = cx.read(|cx| s.runner.read(cx).run(&s.key).unwrap().terminal.lines());
    let first = rows.iter().position(|l| l == "hello world").unwrap();
    cx.update(|cx| s.view.update(cx, |v, cx| v.select((first, 6), (first + 1, 5), cx)));
    cx.simulate_keystrokes(s.window, "i ctrl-shift-c");
    cx.run_until_parked();
    assert_eq!(cx.read_from_clipboard().and_then(|c| c.text()).as_deref(), Some("world\nsecond"));
    cx.write_to_clipboard(ClipboardItem::new_string("a\nb".into()));
    cx.simulate_keystrokes(s.window, "ctrl-shift-v");
    cx.run_until_parked();
    assert_eq!(s.input(), "a\rb");
    s.output(cx, b"\x1b[?2004h");
    cx.simulate_keystrokes(s.window, "ctrl-shift-v");
    cx.run_until_parked();
    assert_eq!(s.input(), "\x1b[200~a\rb\x1b[201~");
}

#[gpui::test]
fn colour_queries_are_answered_with_the_theme(cx: &mut TestAppContext) {
    let s = setup(cx, "colours");
    // The theme's background is #0f1012.
    s.output(cx, b"\x1b]11;?\x07");
    assert_eq!(s.input(), "\x1b]11;rgb:0f0f/1010/1212\x07");
}

#[test]
fn bindings_a_terminal_takes_are_off_inside_terminals() {
    for key in ["enter", "s", "ctrl-k", "ctrl-w", "ctrl-q", "?", "shift-f10"] {
        assert!(registry::terminal_takes(key), "{key}");
    }
    for key in ["alt-f", "ctrl-shift-p", "ctrl-shift-x", "ctrl-1", "ctrl-tab", "ctrl-alt-1"] {
        assert!(!registry::terminal_takes(key), "{key}");
    }
    // Every action stays reachable from a terminal: the palette has a key there.
    let palette = registry::by_id("app.palette").unwrap();
    assert!(palette.keys.iter().any(|k| !registry::terminal_takes(k)));
}
