//! Code and notes (R12): the read-only code view refuses edits and still copies; the note
//! editor reports the user's edits only; a gesture scrolls one axis (R9.6); right-click
//! opens our menu (R5.2); our theme reaches gpui-component's; the note editor's own keys
//! are all known to devdeck's key rule.

use crate::common;

use common::host;
use devdeck_ui::code::{self, Code, CodeEvent, EDITOR_KEYS, editor_takes};
use devdeck_ui::overlay::layer;
use devdeck_ui::{Theme, logical, theme};
use gpui::{
    AnyWindowHandle, AppContext, Entity, Focusable, IntoElement, Keystroke, Modifiers, MouseButton, ParentElement, ScrollDelta, ScrollEvent, Styled,
    TestAppContext, TouchPhase, VisualTestContext, div, point,
};
use std::cell::RefCell;
use std::rc::Rc;

/// A `Code` 300×120 at the window's top left, made by `make` once the window exists.
fn code_in_window(cx: &mut TestAppContext, make: fn(&mut gpui::Window, &mut gpui::Context<Code>) -> Code, text: &str, language: &'static str) -> (AnyWindowHandle, Entity<Code>) {
    let slot: Rc<RefCell<Option<Entity<Code>>>> = Rc::default();
    let s = slot.clone();
    let text = text.to_string();
    let window = host(cx, move |window, cx| {
        let entity = s
            .borrow_mut()
            .get_or_insert_with(|| {
                let e = cx.new(|cx| make(window, cx));
                e.update(cx, |c, cx| c.set_text(&text, language, window, cx));
                e
            })
            .clone();
        div().w(logical(300.)).h(logical(120.)).child(entity).into_any_element()
    });
    let vcx = VisualTestContext::from_window(window, cx);
    vcx.run_until_parked();
    let entity = slot.borrow().clone().expect("built on first render");
    (window, entity)
}

fn focus(cx: &mut VisualTestContext, code: &Entity<Code>) {
    cx.update(|window, cx| {
        let f = code.read(cx).focus_handle(cx);
        f.focus(window, cx);
    });
    cx.run_until_parked();
}

const LONG: &str = "fn main() {\n    let a_very_long_name_that_goes_far_to_the_right_of_the_pane_for_sure = 1;\n}\n";

fn many_lines() -> String {
    (0..200).map(|i| format!("let line_{i} = \"{}\";\n", "x".repeat(120))).collect()
}

#[gpui::test]
fn the_viewer_refuses_typing_and_paste(cx: &mut TestAppContext) {
    let (window, code) = code_in_window(cx, Code::viewer, LONG, "rust");
    let mut vcx = VisualTestContext::from_window(window, cx);
    focus(&mut vcx, &code);
    vcx.simulate_input("zzz");
    vcx.update(|_, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string("pasted".into())));
    vcx.update(|window, cx| window.dispatch_action(Box::new(gpui_component::input::Paste), cx));
    vcx.update(|window, cx| window.dispatch_action(Box::new(gpui_component::input::Cut), cx));
    vcx.run_until_parked();
    assert_eq!(vcx.update(|_, cx| code.read(cx).text(cx).to_string()), LONG);
}

#[gpui::test]
fn the_viewer_selects_and_copies(cx: &mut TestAppContext) {
    let (window, code) = code_in_window(cx, Code::viewer, LONG, "rust");
    let mut vcx = VisualTestContext::from_window(window, cx);
    focus(&mut vcx, &code);
    vcx.update(|window, cx| window.dispatch_action(Box::new(gpui_component::input::SelectAll), cx));
    vcx.run_until_parked();
    assert_eq!(vcx.update(|_, cx| code.read(cx).selected_text(cx)), Some(LONG.to_string()));
}

#[gpui::test]
fn the_note_editor_reports_typing_but_not_set_text(cx: &mut TestAppContext) {
    let (window, code) = code_in_window(cx, Code::note, "# notes\n", "markdown");
    let mut vcx = VisualTestContext::from_window(window, cx);
    let seen = Rc::new(RefCell::new(0));
    let s = seen.clone();
    let _sub = vcx.update(|_, cx| cx.subscribe(&code, move |_, e: &CodeEvent, _| {
        assert_eq!(e, &CodeEvent::Changed);
        *s.borrow_mut() += 1;
    }));
    vcx.update(|window, cx| code.update(cx, |c, cx| c.set_text("# other\n", "markdown", window, cx)));
    vcx.run_until_parked();
    assert_eq!(*seen.borrow(), 0, "set_text isn't the user's edit");
    focus(&mut vcx, &code);
    vcx.simulate_input("hi");
    vcx.run_until_parked();
    assert!(*seen.borrow() > 0);
    assert!(vcx.update(|_, cx| code.read(cx).text(cx).contains("hi")));
}

#[gpui::test]
fn set_text_keeps_the_scroll(cx: &mut TestAppContext) {
    let text = many_lines();
    let (window, code) = code_in_window(cx, Code::viewer, &text, "rust");
    let mut vcx = VisualTestContext::from_window(window, cx);
    vcx.update(|_, cx| code.update(cx, |c, cx| c.set_scroll_offset(point(logical(0.), logical(-400.)), cx)));
    vcx.run_until_parked();
    let before = vcx.update(|_, cx| code.read(cx).scroll_offset(cx));
    assert!(before.y < logical(0.), "{before:?}");
    let changed = format!("{text}// one more line\n");
    vcx.update(|window, cx| code.update(cx, |c, cx| c.set_text(&changed, "rust", window, cx)));
    vcx.run_until_parked();
    assert_eq!(vcx.update(|_, cx| code.read(cx).scroll_offset(cx)), before);
}

#[gpui::test]
fn a_sideways_swipe_never_moves_up_or_down(cx: &mut TestAppContext) {
    let (window, code) = code_in_window(cx, Code::viewer, &many_lines(), "rust");
    let mut vcx = VisualTestContext::from_window(window, cx);
    let position = point(logical(150.), logical(60.));
    for (x, y) in [(-10., -1.), (-2., -3.), (-8., -2.)] {
        let delta = ScrollDelta::Pixels(point(logical(x), logical(y)));
        vcx.simulate_event(ScrollEvent { position, delta, modifiers: Modifiers::none(), touch_phase: TouchPhase::Moved, source: Default::default() });
    }
    vcx.run_until_parked();
    let offset = vcx.update(|_, cx| code.read(cx).scroll_offset(cx));
    assert_eq!(offset.y, logical(0.), "{offset:?}");
    assert!(offset.x < logical(0.), "{offset:?}");
}

#[gpui::test]
fn right_click_opens_our_menu(cx: &mut TestAppContext) {
    let (window, _code) = code_in_window(cx, Code::note, "# notes\n", "markdown");
    let mut vcx = VisualTestContext::from_window(window, cx);
    vcx.simulate_click(point(logical(100.), logical(30.)), Modifiers::none());
    vcx.simulate_mouse_down(point(logical(100.), logical(30.)), MouseButton::Right, Modifiers::none());
    vcx.run_until_parked();
    let open = vcx.update(|window, cx| layer(window.window_handle(), cx).read(cx).menu.is_some());
    assert!(open, "our menu opens on right-click");
}

#[gpui::test]
fn our_theme_reaches_the_editor(cx: &mut TestAppContext) {
    let _ = host(cx, |_, _| div().into_any_element());
    cx.update(|cx| {
        let mut t = Theme::devdeck_dark(1.5);
        t.bg = gpui::red();
        cx.set_global(t);
        code::sync_theme(cx);
        let ours = theme(cx).clone();
        let theirs = gpui_component::Theme::global(cx);
        assert_eq!(theirs.colors.background, gpui::red());
        assert_eq!(theirs.mono_font_family, ours.font);
        assert_eq!(theirs.mono_font_size, ours.text);
        assert_eq!(theirs.highlight_theme.style.editor_background, Some(gpui::red()));
        assert!(theirs.cursor_blink.is_steady(), "no blinking: idle stays idle");
    });
}

#[gpui::test]
fn the_inspector_key_is_gone(cx: &mut TestAppContext) {
    let _ = host(cx, |_, _| div().into_any_element());
    cx.update(|cx| {
        let keymap = cx.key_bindings();
        let keymap = keymap.borrow();
        assert!(keymap.bindings().all(|b| !b.action().name().ends_with("ToggleInspector")));
        assert!(keymap.bindings().any(|b| b.predicate().is_some_and(|p| p.to_string().contains("Input"))), "the editor's keys stay");
    });
}

/// Every key the editor binds with ctrl or alt is one devdeck leaves to it in a note.
#[gpui::test]
fn every_editor_key_is_known(cx: &mut TestAppContext) {
    let _ = host(cx, |_, _| div().into_any_element());
    let known: Vec<Keystroke> = EDITOR_KEYS.iter().map(|k| Keystroke::parse(k).unwrap()).collect();
    let mut missing = Vec::new();
    cx.update(|cx| {
        let keymap = cx.key_bindings();
        for b in keymap.borrow().bindings() {
            let in_input = b.predicate().is_some_and(|p| p.to_string().contains("Input"));
            let [k] = b.keystrokes() else { continue };
            let k = k.inner();
            if !in_input || !(k.modifiers.control || k.modifiers.alt) {
                continue;
            }
            if !known.iter().any(|n| n.modifiers == k.modifiers && n.key == k.key) {
                missing.push(k.unparse(gpui::OperatingSystem::Linux));
            }
        }
    });
    missing.sort();
    missing.dedup();
    assert!(missing.is_empty(), "not in EDITOR_KEYS: {missing:?}");
    // And the other way: every key listed is one the editor binds here.
    let bound: Vec<Keystroke> = cx.update(|cx| {
        let keymap = cx.key_bindings();
        let keymap = keymap.borrow();
        keymap
            .bindings()
            .filter(|b| b.predicate().is_some_and(|p| p.to_string().contains("Input")))
            .filter_map(|b| match b.keystrokes() {
                [k] => Some(k.inner().clone()),
                _ => None,
            })
            .collect()
    });
    let unbound: Vec<&str> = EDITOR_KEYS.iter().copied().filter(|k| {
        let k = Keystroke::parse(k).unwrap();
        !bound.iter().any(|b| b.modifiers == k.modifiers && b.key == k.key)
    }).collect();
    assert!(unbound.is_empty(), "listed but not bound: {unbound:?}");
    // The editor's alt-shift-arrows are devdeck's (moving tabs).
    for k in ["alt-shift-left", "alt-shift-right", "alt-shift-up", "alt-shift-down"] {
        let k = Keystroke::parse(k).unwrap();
        assert!(!bound.iter().any(|b| b.modifiers == k.modifiers && b.key == k.key), "{k:?} given back");
    }
    assert!(editor_takes("a") && editor_takes("enter") && editor_takes("shift-up") && editor_takes("ctrl-c"));
    assert!(!editor_takes("ctrl-w") && !editor_takes("ctrl-tab") && !editor_takes("ctrl-shift-m"));
    assert!(!editor_takes("alt-left") && !editor_takes("alt-shift-left"), "pane keys stay devdeck's");
}

#[test]
fn languages_by_file_name() {
    use std::path::Path;
    assert_eq!(code::language_for(Path::new("/a/main.rs")), "rust");
    assert_eq!(code::language_for(Path::new("/a/App.TSX")), "tsx");
    assert_eq!(code::language_for(Path::new("/a/Makefile")), "make");
    assert_eq!(code::language_for(Path::new("/a/notes.md")), "markdown");
    assert_eq!(code::language_for(Path::new("/a/.env")), "text");
    assert_eq!(code::language_for(Path::new("/a/README")), "text");
}

#[test]
fn big_or_minified_text_isnt_highlighted() {
    assert!(code::highlightable("fn main() {}\n"));
    assert!(!code::highlightable(&"x".repeat(code::HIGHLIGHT_MAX_LINE)));
    assert!(!code::highlightable(&"short line\n".repeat(code::HIGHLIGHT_MAX_BYTES / 10)));
}
