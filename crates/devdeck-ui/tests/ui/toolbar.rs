//! The toolbar on its own, in a host window resized through its forms: which items are
//! drawn at each width, clicks running the same callbacks whatever the form, "more" with
//! the folded items (a toggle checked, a menu as a submenu, a disabled one with its
//! reason) and hidden when empty, no key anywhere, the field keeping its minimum until the
//! floor, right-click listing every item, one line at every width.

use crate::common;

use common::{host, the_theme};
use devdeck_ui::overlay::layer;
use devdeck_ui::toolbar::Shape;
use devdeck_ui::{Count, Fold, Icon, MenuItem, MenuValue, TextInput, ToolAction, ToolItem, Toolbar, ToolbarState, logical, theme};
use gpui::{
    AnyWindowHandle, App, AppContext, Entity, Focusable, InteractiveElement, IntoElement, Modifiers, MouseButton, ParentElement, Styled,
    TestAppContext, VisualTestContext, Window, div, size,
};
use std::cell::RefCell;
use std::rc::Rc;

type Log = Rc<RefCell<Vec<&'static str>>>;

#[derive(Clone)]
struct Probe {
    state: Rc<ToolbarState>,
    log: Log,
    input: Entity<TextInput>,
    /// The view puts items in "more".
    own_more: bool,
}

fn logs(log: &Log, name: &'static str) -> impl Fn(&mut Window, &mut App) + 'static {
    let log = log.clone();
    move |_, _| log.borrow_mut().push(name)
}

/// The logs view's toolbar, roughly: find with its count, a status, the level menu,
/// toggles folding by order (pause and follow together), a disabled action, "more".
fn toolbar(p: &Probe, cx: &mut App) -> Toolbar {
    let t = theme(cx).clone();
    let focus = p.input.read(cx).focus_handle(cx);
    let log = p.log.clone();
    let l = log.clone();
    let tb = Toolbar::new("tb", p.state.clone())
        .item(ToolItem::Field {
            id: "find".into(),
            label: "find".into(),
            input: p.input.clone().into(),
            focus,
            min_chars: 12,
            count: Some(Count::new("3/12", 6, t.mute)),
            accessories: Vec::new(),
        })
        .item(ToolItem::Status {
            id: "wrong".into(),
            icon: Icon::Warning,
            color: t.warn,
            word: Some("2 waiting".into()),
            tip: "2 pods waiting".into(),
            details: Rc::new(|_| vec![MenuItem::new("pod a: waiting", |_, _| {})]),
        })
        .item(ToolItem::Menu {
            action: ToolAction::new("level", "the lowest level shown", logs(&log, "level")).icon(Icon::Filter).fold(Fold::Order(3)),
            value: Some(MenuValue::new("warn+")),
            max_chars: 8,
            items: Rc::new(move |_| vec![MenuItem::new("errors", logs(&l, "errors")), MenuItem::new("all", logs(&l, "all"))]),
        })
        .item(ToolItem::Action(ToolAction::new("wrap", "wrap long lines", logs(&log, "wrap")).icon(Icon::Wrap).toggle(true).fold(Fold::Order(2))))
        .item(ToolItem::Action(
            ToolAction::new("pause", "pause", logs(&log, "pause"))
                .icon(Icon::Pause)
                .toggle(false)
                .fold(Fold::Order(1))
                .count(Some(Count::new("40 new", 7, t.warn))),
        ))
        .item(ToolItem::Action(ToolAction::new("follow", "follow the newest line", logs(&log, "follow")).icon(Icon::Follow).toggle(true).fold(Fold::Order(1))))
        .item(ToolItem::Action(ToolAction::new("deploy", "deploy", logs(&log, "deploy")).icon(Icon::Run).disabled("no cluster").fold(Fold::Order(1))));
    if p.own_more {
        let l = log.clone();
        tb.more(Rc::new(move |_| vec![MenuItem::new("show the time", logs(&l, "time"))]), Some("3 pods · 120 lines".into()))
    } else {
        tb
    }
}

fn setup(cx: &mut TestAppContext, own_more: bool) -> (VisualTestContext, AnyWindowHandle, Probe) {
    let input = cx.update(|cx| cx.new(|cx| TextInput::new("find", cx)));
    let p = Probe { state: ToolbarState::new(), log: Log::default(), input, own_more };
    let probe = p.clone();
    let window = host(cx, move |_, cx| div().w_full().child(div().debug_selector(|| "holder".into()).child(toolbar(&probe, cx))).into_any_element());
    let mut vcx = VisualTestContext::from_window(window, cx);
    vcx.update(|window, _| window.activate_window());
    (vcx, window, p)
}

fn at_width(vcx: &mut VisualTestContext, window: AnyWindowHandle, w: f32) {
    vcx.simulate_window_resize(window, size(logical(w), logical(300.)));
    vcx.run_until_parked();
}

/// The shape at each width from 1600 px down to 150, every 10 px.
fn sweep(vcx: &mut VisualTestContext, window: AnyWindowHandle, p: &Probe) -> Vec<(f32, Shape)> {
    (0..=145)
        .map(|i| {
            let w = 1600. - 10. * i as f32;
            at_width(vcx, window, w);
            (w, p.state.shape().expect("laid out"))
        })
        .collect()
}

/// The widest width showing `shape`.
fn widest(shapes: &[(f32, Shape)], shape: Shape) -> f32 {
    shapes.iter().find(|(_, s)| *s == shape).unwrap_or_else(|| panic!("{shape:?} never shows")).0
}

fn menu(vcx: &mut VisualTestContext) -> Option<Vec<MenuItem>> {
    vcx.update(|window, cx| layer(window.window_handle(), cx).read(cx).menu.clone().map(|m| m.read(cx).items().to_vec()))
}

fn close_menu(vcx: &mut VisualTestContext) {
    vcx.update(|window, cx| {
        layer(window.window_handle(), cx).update(cx, |l, cx| {
            l.menu = None;
            cx.notify();
        })
    });
    vcx.run_until_parked();
}

fn click(vcx: &mut VisualTestContext, selector: &'static str) {
    let b = vcx.debug_bounds(selector).unwrap_or_else(|| panic!("{selector} drawn"));
    vcx.simulate_click(b.center(), Modifiers::none());
    vcx.run_until_parked();
}

const ITEMS: [&str; 7] = ["tool-find", "tool-wrong", "tool-level", "tool-wrap", "tool-pause", "tool-follow", "tool-deploy"];

#[gpui::test]
fn the_forms_follow_the_width_on_one_line(cx: &mut TestAppContext) {
    let (mut vcx, window, p) = setup(cx, true);
    let t = the_theme(&mut vcx);
    let shapes = sweep(&mut vcx, window, &p);
    let rank = |s: Shape| [Shape::Wide, Shape::Compact, Shape::Narrow, Shape::Floor].iter().position(|x| *x == s).unwrap();
    assert!(shapes.windows(2).all(|w| rank(w[0].1) <= rank(w[1].1)), "narrower never gets a wider form: {shapes:?}");
    for s in [Shape::Wide, Shape::Compact, Shape::Narrow, Shape::Floor] {
        widest(&shapes, s);
    }
    assert_eq!(shapes.last().unwrap().1, Shape::Floor, "150 px");
    for w in [1600., widest(&shapes, Shape::Compact), widest(&shapes, Shape::Narrow), 150.] {
        at_width(&mut vcx, window, w);
        let holder = vcx.debug_bounds("holder").expect("drawn");
        assert_eq!(holder.size.height, t.toolbar_h, "{w}: one line, the toolbar's height");
        for s in ITEMS.iter().copied().chain(["tool-more"]) {
            if let Some(b) = vcx.debug_bounds(s) {
                assert_eq!(b.size.height, t.button_h, "{w}: {s} is button_h tall");
                assert!(b.top() >= holder.top() && b.bottom() <= holder.bottom(), "{w}: {s} within the line");
            }
        }
    }
}

#[gpui::test]
fn which_items_show_in_each_form(cx: &mut TestAppContext) {
    let (mut vcx, window, p) = setup(cx, true);
    let t = the_theme(&mut vcx);
    let shapes = sweep(&mut vcx, window, &p);

    at_width(&mut vcx, window, 1600.);
    for s in ITEMS {
        assert!(vcx.debug_bounds(s).is_some(), "wide: {s} drawn");
    }
    assert!(vcx.debug_bounds("tool-more").is_some(), "the view's own items: \"more\" shows");
    assert!(vcx.debug_bounds("tool-wrap").unwrap().size.width > t.button_h, "wide: with its label");
    assert!(vcx.debug_bounds("tool-wrong").unwrap().size.width > t.button_h, "wide: the status's word");
    assert!(vcx.debug_bounds("tool-more-dot").is_none(), "nothing folded, no dot");

    at_width(&mut vcx, window, widest(&shapes, Shape::Compact));
    assert!(p.state.folded().is_empty());
    assert_eq!(vcx.debug_bounds("tool-wrap").unwrap().size.width, t.button_h, "compact: icon only");
    assert_eq!(vcx.debug_bounds("tool-wrong").unwrap().size.width, t.button_h, "compact: no word");
    assert!(vcx.debug_bounds("tool-level").unwrap().size.width > t.button_h, "compact: the menu keeps its value");

    at_width(&mut vcx, window, widest(&shapes, Shape::Narrow));
    assert_eq!(p.state.folded(), ["pause", "follow", "deploy"], "order 1 first, together, in the toolbar's order");
    for s in ["tool-pause", "tool-follow", "tool-deploy"] {
        assert!(vcx.debug_bounds(s).is_none(), "{s} folded");
    }
    for s in ["tool-find", "tool-wrong", "tool-level", "tool-wrap", "tool-more", "tool-more-dot"] {
        assert!(vcx.debug_bounds(s).is_some(), "narrow: {s} drawn");
    }

    // Narrower still: then 2, then 3.
    let last_narrow = shapes.iter().rev().find(|(_, s)| *s == Shape::Narrow).unwrap().0;
    at_width(&mut vcx, window, last_narrow);
    assert_eq!(p.state.folded(), ["level", "wrap", "pause", "follow", "deploy"]);
    for s in ["tool-find", "tool-wrong", "tool-more"] {
        assert!(vcx.debug_bounds(s).is_some(), "{s}: never folds");
    }
}

#[gpui::test]
fn the_field_keeps_its_minimum_until_the_floor_then_becomes_a_button(cx: &mut TestAppContext) {
    let (mut vcx, window, p) = setup(cx, true);
    let t = the_theme(&mut vcx);
    let cell = vcx.update(|window, cx| devdeck_ui::terminal::cell_size(window, theme(cx)).width);
    for (w, shape) in sweep(&mut vcx, window, &p) {
        at_width(&mut vcx, window, w);
        let find = vcx.debug_bounds("tool-find").expect("the field always shows");
        if shape != Shape::Floor {
            assert!(find.size.width >= cell * 12., "{w}: {find:?} keeps 12 characters");
        }
    }
    at_width(&mut vcx, window, 150.);
    assert_eq!(p.state.shape(), Some(Shape::Floor));
    assert_eq!(vcx.debug_bounds("tool-find").unwrap().size.width, t.button_h, "the extreme: a button");
    for s in ["tool-wrong", "tool-more"] {
        assert!(vcx.debug_bounds(s).is_some(), "150 px: {s} still shows");
    }
    click(&mut vcx, "tool-find");
    let focused = vcx.update(|window, cx| p.input.read(cx).focus_handle(cx).is_focused(window));
    assert!(focused, "the button focuses the field");
    assert!(vcx.debug_bounds("tool-find").unwrap().size.width > t.button_h, "with focus, the field shows");
}

#[gpui::test]
fn a_click_runs_the_item_in_every_form(cx: &mut TestAppContext) {
    let (mut vcx, window, p) = setup(cx, true);
    let shapes = sweep(&mut vcx, window, &p);
    for w in [1600., widest(&shapes, Shape::Compact), widest(&shapes, Shape::Narrow)] {
        at_width(&mut vcx, window, w);
        p.log.borrow_mut().clear();
        click(&mut vcx, "tool-wrap");
        assert_eq!(*p.log.borrow(), ["wrap"], "{w}");
        // A disabled item still runs (it explains why).
        if vcx.debug_bounds("tool-deploy").is_some() {
            click(&mut vcx, "tool-deploy");
            assert_eq!(p.log.borrow().last(), Some(&"deploy"), "{w}");
        }
        // A menu button opens its menu under it.
        click(&mut vcx, "tool-level");
        let items = menu(&mut vcx).expect("the level menu");
        assert_eq!(items.iter().map(|i| i.label.as_ref()).collect::<Vec<_>>(), ["errors", "all"], "{w}");
        close_menu(&mut vcx);
    }
    // Folded, the same callback runs from "more".
    at_width(&mut vcx, window, widest(&shapes, Shape::Narrow));
    p.log.borrow_mut().clear();
    click(&mut vcx, "tool-more");
    assert_eq!(menu(&mut vcx).expect("more")[1].label.as_ref(), "pause · 40 new");
    click(&mut vcx, "menu-item-1");
    assert_eq!(*p.log.borrow(), ["pause"]);
}

#[gpui::test]
fn more_holds_the_folded_items_then_the_views(cx: &mut TestAppContext) {
    let (mut vcx, window, p) = setup(cx, true);
    let shapes = sweep(&mut vcx, window, &p);
    let last_narrow = shapes.iter().rev().find(|(_, s)| *s == Shape::Narrow).unwrap().0;
    at_width(&mut vcx, window, last_narrow);
    click(&mut vcx, "tool-more");
    let items = menu(&mut vcx).expect("more opens");
    let labels: Vec<&str> = items.iter().map(|i| i.label.as_ref()).collect();
    assert_eq!(
        labels,
        ["3 pods · 120 lines", "the lowest level shown: warn+", "wrap long lines", "pause · 40 new", "follow the newest line", "deploy", "", "show the time"]
    );
    assert!(items[0].disabled.is_some() && items[0].run.is_none(), "the figures: a row that does nothing");
    assert_eq!(items[1].submenu.iter().map(|i| i.label.as_ref()).collect::<Vec<_>>(), ["errors", "all"], "a folded menu is a submenu");
    assert!(items[2].checked && !items[3].checked && items[4].checked, "toggles checked when on");
    assert!(items[2].toggle && items[3].toggle && items[4].toggle && !items[1].toggle && !items[5].toggle, "toggles keep the menu open, a submenu and an action don't (R5.8)");
    assert_eq!(items[5].disabled.as_deref(), Some("no cluster"), "a disabled item with its reason");
    assert!(items[6].separator);
    assert!(items.iter().chain(items.iter().flat_map(|i| i.submenu.iter())).all(|i| i.hint.is_none()), "no key shown (R5.4)");
}

#[gpui::test]
fn more_is_hidden_when_it_holds_nothing(cx: &mut TestAppContext) {
    let (mut vcx, window, p) = setup(cx, false);
    at_width(&mut vcx, window, 1600.);
    assert_eq!(p.state.shape(), Some(Shape::Wide));
    assert!(vcx.debug_bounds("tool-more").is_none() && p.state.bounds("more").is_none());
    at_width(&mut vcx, window, 300.);
    assert!(vcx.debug_bounds("tool-more").is_some(), "something folded: it shows");
    assert!(vcx.debug_bounds("tool-more-dot").is_some(), "a folded count: the dot");
}

#[gpui::test]
fn right_click_lists_every_item_shown_or_folded(cx: &mut TestAppContext) {
    let (mut vcx, window, p) = setup(cx, true);
    for w in [1600., 300.] {
        at_width(&mut vcx, window, w);
        let holder = vcx.debug_bounds("holder").expect("drawn");
        let more = p.state.bounds("more").expect("more");
        vcx.simulate_mouse_down(more.center(), MouseButton::Right, Modifiers::none());
        vcx.run_until_parked();
        let items = menu(&mut vcx).expect("a menu");
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_ref()).collect();
        assert_eq!(labels, ["find", "2 waiting", "the lowest level shown: warn+", "wrap long lines", "pause · 40 new", "follow the newest line", "deploy"], "{w}");
        assert!(items.iter().all(|i| i.hint.is_none()), "no key shown (R5.4)");
        assert_eq!(items[1].submenu.len(), 1, "a status's details");
        assert!(holder.size.height > logical(0.));
        close_menu(&mut vcx);
    }
}

#[gpui::test]
fn a_status_opens_its_details_and_menus_open_by_key_under_their_button(cx: &mut TestAppContext) {
    let (mut vcx, window, p) = setup(cx, true);
    at_width(&mut vcx, window, 1600.);
    click(&mut vcx, "tool-wrong");
    assert_eq!(menu(&mut vcx).expect("details")[0].label.as_ref(), "pod a: waiting");
    close_menu(&mut vcx);
    // By key: the view builds its toolbar again and asks it.
    let probe = p.clone();
    vcx.update(|window, cx| toolbar(&probe, cx).open_menu_of("level", window, cx));
    vcx.run_until_parked();
    assert_eq!(menu(&mut vcx).map(|m| m.len()), Some(2));
    close_menu(&mut vcx);
    let probe = p.clone();
    vcx.update(|window, cx| toolbar(&probe, cx).open_more(window, cx));
    vcx.run_until_parked();
    assert_eq!(menu(&mut vcx).expect("more")[0].label.as_ref(), "3 pods · 120 lines");
    assert!(p.state.bounds("level").is_some(), "the view can read where its items are");
}

#[gpui::test]
fn items_show_their_tooltip_on_hover(cx: &mut TestAppContext) {
    let (mut vcx, window, _p) = setup(cx, true);
    at_width(&mut vcx, window, 1600.);
    for s in ["tool-deploy", "tool-wrap", "tool-more"] {
        let b = vcx.debug_bounds(s).expect("drawn");
        vcx.simulate_mouse_move(b.center(), None, Modifiers::none());
        vcx.executor().advance_clock(std::time::Duration::from_millis(600));
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("tooltip").is_some(), "{s}: its tooltip (its name, or why it can't run)");
        vcx.simulate_mouse_move(b.center() + gpui::point(logical(0.), logical(200.)), None, Modifiers::none());
        vcx.executor().advance_clock(std::time::Duration::from_millis(600));
        vcx.run_until_parked();
    }
}
