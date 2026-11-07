use super::*;
use proptest::prelude::*;

fn v(s: &str) -> ViewRef {
    ViewRef::new(s)
}

fn ok(l: &Layout) {
    l.validate().unwrap_or_else(|e| panic!("invalid layout: {e}\n{l:#?}"));
}

/// Main window with one pane holding `tabs`.
fn with_tabs(tabs: &[&str]) -> (Layout, PaneId) {
    let mut l = Layout::new();
    let p = l.main().focused;
    for t in tabs {
        l.open(v(t), p).unwrap();
    }
    ok(&l);
    (l, p)
}

fn root_split(l: &Layout) -> &Split {
    match &l.main().root {
        Node::Split(s) => s,
        Node::Pane(_) => panic!("root is a pane"),
    }
}

#[test]
fn new_layout_is_valid_and_empty() {
    let l = Layout::new();
    ok(&l);
    assert_eq!(l.panes_in(MAIN_WINDOW).unwrap().len(), 1);
    assert!(l.views().is_empty());
}

#[test]
fn open_adds_and_activates() {
    let (l, p) = with_tabs(&["a", "b"]);
    assert_eq!(l.pane(p).unwrap().tabs, vec![v("a"), v("b")]);
    assert_eq!(l.pane(p).unwrap().active, 1);
}

#[test]
fn open_existing_view_activates_instead_of_duplicating() {
    let (mut l, p) = with_tabs(&["a", "b"]);
    l.move_tab(p, 0, Drop::Edge(p, Edge::Right)).unwrap();
    let shown = l.open(v("a"), p).unwrap();
    assert_ne!(shown, p, "a lives in the new pane");
    assert_eq!(l.views().len(), 2);
    assert_eq!(l.main().focused, shown);
    ok(&l);
}

#[test]
fn close_last_tab_of_lone_main_pane_keeps_empty_pane() {
    let (mut l, p) = with_tabs(&["a"]);
    l.close_tab(p, 0).unwrap();
    ok(&l);
    assert_eq!(l.panes_in(MAIN_WINDOW).unwrap(), vec![p]);
    assert!(l.pane(p).unwrap().tabs.is_empty());
}

#[test]
fn close_last_tab_removes_pane_and_simplifies_split() {
    let (mut l, p) = with_tabs(&["a", "b"]);
    l.move_tab(p, 1, Drop::Edge(p, Edge::Bottom)).unwrap();
    assert!(matches!(l.main().root, Node::Split(_)));
    let (q, _) = l.find_view(&v("b")).unwrap();
    l.close_tab(q, 0).unwrap();
    ok(&l);
    assert_eq!(l.main().root, Node::Pane(p));
}

#[test]
fn close_tab_keeps_active_index_sensible() {
    let (mut l, p) = with_tabs(&["a", "b", "c"]);
    l.activate(p, 2).unwrap();
    l.close_tab(p, 0).unwrap();
    assert_eq!(l.pane(p).unwrap().active_view(), Some(&v("c")));
    l.close_tab(p, 1).unwrap();
    assert_eq!(l.pane(p).unwrap().active_view(), Some(&v("b")));
    ok(&l);
}

#[test]
fn edge_drop_creates_split_in_right_order() {
    for (edge, axis, new_first) in [
        (Edge::Left, Axis::Row, true),
        (Edge::Right, Axis::Row, false),
        (Edge::Top, Axis::Column, true),
        (Edge::Bottom, Axis::Column, false),
    ] {
        let (mut l, p) = with_tabs(&["a", "b"]);
        l.move_tab(p, 1, Drop::Edge(p, edge)).unwrap();
        ok(&l);
        let s = root_split(&l);
        assert_eq!(s.axis, axis);
        let (q, _) = l.find_view(&v("b")).unwrap();
        let first = matches!(s.children[0].node, Node::Pane(x) if x == q);
        assert_eq!(first, new_first, "{edge:?}");
        assert!((s.children[0].weight - 0.5).abs() < 1e-6);
    }
}

#[test]
fn edge_drop_on_same_axis_extends_parent_split_instead_of_nesting() {
    let (mut l, p) = with_tabs(&["a", "b", "c"]);
    l.move_tab(p, 2, Drop::Edge(p, Edge::Right)).unwrap();
    l.move_tab(p, 1, Drop::Edge(p, Edge::Right)).unwrap();
    ok(&l);
    let s = root_split(&l);
    assert_eq!(s.children.len(), 3);
    let order: Vec<_> = l.panes_in(MAIN_WINDOW).unwrap().iter().map(|q| l.pane(*q).unwrap().tabs[0].clone()).collect();
    assert_eq!(order, vec![v("a"), v("b"), v("c")]);
}

#[test]
fn edge_drop_of_only_tab_on_own_pane_is_noop() {
    let (mut l, p) = with_tabs(&["a"]);
    let before = l.clone();
    l.move_tab(p, 0, Drop::Edge(p, Edge::Right)).unwrap();
    assert_eq!(l, before);
}

#[test]
fn move_into_same_pane_is_noop() {
    let (mut l, p) = with_tabs(&["a", "b"]);
    let before = l.clone();
    l.move_tab(p, 0, Drop::Into(p)).unwrap();
    assert_eq!(l, before);
}

#[test]
fn move_tab_into_other_pane_removes_emptied_source() {
    let (mut l, p) = with_tabs(&["a", "b"]);
    l.move_tab(p, 1, Drop::Edge(p, Edge::Right)).unwrap();
    let (q, _) = l.find_view(&v("b")).unwrap();
    l.move_tab(q, 0, Drop::Into(p)).unwrap();
    ok(&l);
    assert_eq!(l.panes_in(MAIN_WINDOW).unwrap(), vec![p]);
    assert_eq!(l.pane(p).unwrap().active_view(), Some(&v("b")));
}

#[test]
fn nested_split_collapses_when_children_leave() {
    // [a | [b / c]] then remove c then b → back to a single pane.
    let (mut l, p) = with_tabs(&["a", "b", "c"]);
    l.move_tab(p, 1, Drop::Edge(p, Edge::Right)).unwrap();
    let (q, _) = l.find_view(&v("b")).unwrap();
    l.move_tab(p, 1, Drop::Edge(q, Edge::Bottom)).unwrap();
    ok(&l);
    assert_eq!(l.panes_in(MAIN_WINDOW).unwrap().len(), 3);
    l.close_view(&v("c")).unwrap();
    ok(&l);
    l.close_view(&v("b")).unwrap();
    ok(&l);
    assert_eq!(l.main().root, Node::Pane(p));
}

#[test]
fn same_axis_nesting_is_flattened_after_removal() {
    // [a | [b / c] | d] → remove c → [a | b | d] as one row split.
    let (mut l, p) = with_tabs(&["a", "b", "c", "d"]);
    l.move_tab(p, 3, Drop::Edge(p, Edge::Right)).unwrap();
    l.move_tab(p, 1, Drop::Edge(p, Edge::Right)).unwrap();
    let (b, _) = l.find_view(&v("b")).unwrap();
    l.move_tab(p, 1, Drop::Edge(b, Edge::Bottom)).unwrap();
    ok(&l);
    l.close_view(&v("c")).unwrap();
    ok(&l);
    let s = root_split(&l);
    assert_eq!(s.axis, Axis::Row);
    assert_eq!(s.children.len(), 3);
    assert!(s.children.iter().all(|c| matches!(c.node, Node::Pane(_))));
}

#[test]
fn move_pane_beside_another() {
    let (mut l, p) = with_tabs(&["a", "b", "c"]);
    l.move_tab(p, 2, Drop::Edge(p, Edge::Right)).unwrap();
    l.move_tab(p, 1, Drop::Edge(p, Edge::Right)).unwrap();
    let (c, _) = l.find_view(&v("c")).unwrap();
    l.move_pane(c, Drop::Edge(p, Edge::Top)).unwrap();
    ok(&l);
    let s = root_split(&l);
    assert_eq!(s.axis, Axis::Row);
    assert_eq!(s.children.len(), 2);
    assert!(l.pane(c).is_ok(), "pane id survives the move");
}

#[test]
fn move_pane_into_merges_tabs() {
    let (mut l, p) = with_tabs(&["a", "b", "c"]);
    l.move_tab(p, 2, Drop::Edge(p, Edge::Right)).unwrap();
    let (c, _) = l.find_view(&v("c")).unwrap();
    l.move_pane(p, Drop::Into(c)).unwrap();
    ok(&l);
    assert_eq!(l.pane(c).unwrap().tabs, vec![v("c"), v("a"), v("b")]);
    assert_eq!(l.pane(c).unwrap().active_view(), Some(&v("a")));
}

#[test]
fn edge_drop_on_empty_placeholder_fills_it_instead_of_splitting() {
    // Regression (found by the property test): main's placeholder pane ended up
    // empty inside a split.
    let (mut l, p) = with_tabs(&["a", "b"]);
    let w1 = l.detach(p, 0).unwrap();
    l.detach(p, 0).unwrap();
    assert!(l.pane(p).unwrap().tabs.is_empty());
    let Node::Pane(d) = l.window(w1).unwrap().root else { panic!() };
    l.move_pane(d, Drop::Edge(p, Edge::Left)).unwrap();
    ok(&l);
    assert_eq!(l.main().root, Node::Pane(p));
    assert_eq!(l.pane(p).unwrap().tabs, vec![v("a")]);
}

#[test]
fn main_window_last_pane_cannot_move_out() {
    let (mut l, p) = with_tabs(&["a", "b"]);
    let w = l.detach(p, 1).unwrap();
    let Node::Pane(d) = l.window(w).unwrap().root else { panic!() };
    assert_eq!(l.move_pane(p, Drop::Into(d)), Err(LayoutError::MainWindowLastPane));
    ok(&l);
}

#[test]
fn resize_moves_boundary_and_respects_minimum() {
    let (mut l, p) = with_tabs(&["a", "b"]);
    l.move_tab(p, 1, Drop::Edge(p, Edge::Right)).unwrap();
    let id = root_split(&l).id;
    l.resize(id, 0, 0.2).unwrap();
    assert!((root_split(&l).children[0].weight - 0.7).abs() < 1e-5);
    l.resize(id, 0, 10.0).unwrap();
    assert!((root_split(&l).children[1].weight - MIN_WEIGHT).abs() < 1e-5);
    l.set_boundary(id, 0, 0.25).unwrap();
    assert!((root_split(&l).children[0].weight - 0.25).abs() < 1e-5);
    assert!(l.resize(id, 1, 0.1).is_err(), "no boundary after the last child");
    assert!(l.resize(id, 0, f32::NAN).is_err());
    ok(&l);
}

#[test]
fn parent_of_and_grow() {
    let (mut l, p) = with_tabs(&["a", "b"]);
    assert_eq!(l.parent_of(p), None);
    l.grow(p, 0.1).unwrap(); // no parent: no-op
    l.move_tab(p, 1, Drop::Edge(p, Edge::Right)).unwrap();
    let (q, _) = l.find_view(&v("b")).unwrap();
    let (sid, idx, axis) = l.parent_of(q).unwrap();
    assert_eq!((idx, axis), (1, Axis::Row));
    l.grow(q, 0.1).unwrap();
    assert!((l.split(sid).unwrap().children[1].weight - 0.6).abs() < 1e-5, "last child grows leftwards");
    l.grow(p, 0.2).unwrap();
    assert!((l.split(sid).unwrap().children[0].weight - 0.6).abs() < 1e-5);
    ok(&l);
}

#[test]
fn collapse_rules() {
    let (mut l, p) = with_tabs(&["a", "b"]);
    assert_eq!(l.toggle_collapse(p), Err(LayoutError::LastExpandedPane));
    l.move_tab(p, 1, Drop::Edge(p, Edge::Bottom)).unwrap();
    let (q, _) = l.find_view(&v("b")).unwrap();
    l.toggle_collapse(q).unwrap();
    assert!(l.pane(q).unwrap().collapsed);
    assert_eq!(l.toggle_collapse(p), Err(LayoutError::LastExpandedPane));
    l.activate(q, 0).unwrap();
    assert!(!l.pane(q).unwrap().collapsed, "activating a tab expands its pane");
    ok(&l);
}

#[test]
fn moving_the_only_expanded_pane_away_expands_another() {
    let (mut l, p) = with_tabs(&["a", "b", "c"]);
    l.move_tab(p, 2, Drop::Edge(p, Edge::Bottom)).unwrap();
    let (c, _) = l.find_view(&v("c")).unwrap();
    l.toggle_collapse(c).unwrap();
    // Close everything in the expanded pane: the collapsed one must expand.
    l.close_view(&v("a")).unwrap();
    l.close_view(&v("b")).unwrap();
    ok(&l);
    assert!(!l.pane(c).unwrap().collapsed);
}

#[test]
fn maximize_toggles_and_clears_when_pane_leaves() {
    let (mut l, p) = with_tabs(&["a", "b"]);
    l.move_tab(p, 1, Drop::Edge(p, Edge::Right)).unwrap();
    let (q, _) = l.find_view(&v("b")).unwrap();
    l.toggle_maximize(q).unwrap();
    assert_eq!(l.main().maximized, Some(q));
    l.close_view(&v("b")).unwrap();
    ok(&l);
    assert_eq!(l.main().maximized, None);
    l.toggle_maximize(p).unwrap();
    l.toggle_maximize(p).unwrap();
    assert_eq!(l.main().maximized, None);
}

#[test]
fn collapsing_maximized_pane_unmaximizes() {
    let (mut l, p) = with_tabs(&["a", "b"]);
    l.move_tab(p, 1, Drop::Edge(p, Edge::Right)).unwrap();
    let (q, _) = l.find_view(&v("b")).unwrap();
    l.toggle_maximize(q).unwrap();
    l.toggle_collapse(q).unwrap();
    assert_eq!(l.main().maximized, None);
    ok(&l);
}

#[test]
fn detach_and_attach_round_trip() {
    let (mut l, p) = with_tabs(&["a", "b"]);
    let w = l.detach(p, 0).unwrap();
    ok(&l);
    assert_eq!(l.windows.len(), 2);
    let Node::Pane(d) = l.window(w).unwrap().root else { panic!("detached root must be a pane") };
    assert_eq!(l.pane(d).unwrap().tabs, vec![v("a")]);
    // Edge drops into a detached window are refused.
    assert_eq!(l.move_tab(p, 0, Drop::Edge(d, Edge::Left)), Err(LayoutError::DetachedWindowIsSinglePane));
    // More tabs can join it.
    l.open(v("c"), d).unwrap();
    l.attach(w, Drop::Edge(p, Edge::Right)).unwrap();
    ok(&l);
    assert_eq!(l.windows.len(), 1);
    assert_eq!(l.views(), [v("a"), v("b"), v("c")].into_iter().collect());
}

#[test]
fn attach_errors() {
    let (mut l, p) = with_tabs(&["a", "b"]);
    assert_eq!(l.attach(MAIN_WINDOW, Drop::Into(p)), Err(LayoutError::NotDetached));
    let w = l.detach(p, 0).unwrap();
    let Node::Pane(d) = l.window(w).unwrap().root else { panic!() };
    assert_eq!(l.attach(w, Drop::Into(d)), Err(LayoutError::SameWindow));
    assert_eq!(l.detach(d, 0), Err(LayoutError::AlreadyAlone));
}

#[test]
fn moving_last_tab_out_of_detached_window_closes_it() {
    let (mut l, p) = with_tabs(&["a", "b"]);
    let w = l.detach(p, 0).unwrap();
    let Node::Pane(d) = l.window(w).unwrap().root else { panic!() };
    l.move_tab(d, 0, Drop::Into(p)).unwrap();
    ok(&l);
    assert!(l.window(w).is_err());
}

#[test]
fn closing_detached_window_closes_its_tabs_and_closing_main_quits() {
    let (mut l, p) = with_tabs(&["a", "b"]);
    let w = l.detach(p, 1).unwrap();
    let Node::Pane(d) = l.window(w).unwrap().root else { panic!() };
    l.open(v("c"), d).unwrap();
    assert_eq!(l.close_window(w).unwrap(), CloseOutcome::Closed(vec![v("b"), v("c")]));
    ok(&l);
    assert_eq!(l.views(), [v("a")].into_iter().collect());
    assert!(l.window(w).is_err());
    assert_eq!(l.close_window(MAIN_WINDOW).unwrap(), CloseOutcome::Quit);
}

#[test]
fn reattach_sends_tabs_to_main_focused_pane() {
    let (mut l, p) = with_tabs(&["a", "b"]);
    let w = l.detach(p, 1).unwrap();
    assert_eq!(l.reattach(w).unwrap(), p);
    assert_eq!(l.pane(p).unwrap().tabs, vec![v("a"), v("b")]);
    assert_eq!(l.windows.len(), 1);
    assert_eq!(l.reattach(MAIN_WINDOW), Err(LayoutError::NotDetached));
    ok(&l);
}

#[test]
fn rects_honour_weights_and_collapse() {
    let (mut l, p) = with_tabs(&["a", "b", "c"]);
    l.move_tab(p, 1, Drop::Edge(p, Edge::Right)).unwrap();
    let (b, _) = l.find_view(&v("b")).unwrap();
    l.move_tab(p, 1, Drop::Edge(b, Edge::Bottom)).unwrap();
    let (c, _) = l.find_view(&v("c")).unwrap();
    let r: BTreeMap<_, _> = l.pane_rects(MAIN_WINDOW, (1000.0, 600.0), 32.0).unwrap().into_iter().collect();
    assert_eq!(r[&p], Rect { x: 0.0, y: 0.0, w: 500.0, h: 600.0 });
    assert_eq!(r[&b], Rect { x: 500.0, y: 0.0, w: 500.0, h: 300.0 });
    l.toggle_collapse(c).unwrap();
    let r: BTreeMap<_, _> = l.pane_rects(MAIN_WINDOW, (1000.0, 600.0), 32.0).unwrap().into_iter().collect();
    assert_eq!(r[&c].h, 32.0);
    assert_eq!(r[&b].h, 568.0);
    l.toggle_maximize(b).unwrap();
    let r = l.pane_rects(MAIN_WINDOW, (1000.0, 600.0), 32.0).unwrap();
    assert_eq!(r, vec![(b, Rect { x: 0.0, y: 0.0, w: 1000.0, h: 600.0 })]);
}

#[test]
fn directional_neighbor() {
    // [a | [b / c]]
    let (mut l, p) = with_tabs(&["a", "b", "c"]);
    l.move_tab(p, 1, Drop::Edge(p, Edge::Right)).unwrap();
    let (b, _) = l.find_view(&v("b")).unwrap();
    l.move_tab(p, 1, Drop::Edge(b, Edge::Bottom)).unwrap();
    let (c, _) = l.find_view(&v("c")).unwrap();
    let size = (1000.0, 600.0);
    assert_eq!(l.neighbor(p, Direction::Right, size, 32.0).unwrap(), Some(b));
    assert_eq!(l.neighbor(b, Direction::Down, size, 32.0).unwrap(), Some(c));
    assert_eq!(l.neighbor(c, Direction::Up, size, 32.0).unwrap(), Some(b));
    assert_eq!(l.neighbor(c, Direction::Left, size, 32.0).unwrap(), Some(p));
    assert_eq!(l.neighbor(p, Direction::Left, size, 32.0).unwrap(), None);
    assert_eq!(l.neighbor(p, Direction::Up, size, 32.0).unwrap(), None);
}

#[test]
fn focus_follows_operations() {
    let (mut l, p) = with_tabs(&["a", "b"]);
    l.move_tab(p, 1, Drop::Edge(p, Edge::Right)).unwrap();
    let (q, _) = l.find_view(&v("b")).unwrap();
    assert_eq!(l.main().focused, q, "the moved tab's pane takes focus");
    l.close_view(&v("b")).unwrap();
    assert_eq!(l.main().focused, p, "focus falls back when its pane disappears");
}

#[test]
fn json_round_trip_and_version_check() {
    let (mut l, p) = with_tabs(&["a", "b", "c"]);
    l.move_tab(p, 1, Drop::Edge(p, Edge::Right)).unwrap();
    l.detach(p, 1).unwrap();
    let json = l.to_json();
    assert_eq!(Layout::from_json(&json).unwrap(), l);
    let bumped = json.replacen(&format!("\"version\":{LAYOUT_VERSION}"), "\"version\":999", 1);
    assert_eq!(Layout::from_json(&bumped), Err(LayoutError::UnsupportedVersion(999)));
    assert!(Layout::from_json("{}").is_err());
}

#[test]
fn validate_rejects_corruption() {
    let (mut l, p) = with_tabs(&["a", "b"]);
    l.panes.get_mut(&p).unwrap().tabs.push(v("a"));
    assert!(l.validate().is_err(), "duplicate view");
    let (mut l, p) = with_tabs(&["a"]);
    l.panes.get_mut(&p).unwrap().active = 5;
    assert!(l.validate().is_err(), "active out of range");
    let (mut l, _) = with_tabs(&["a"]);
    l.panes.insert(PaneId(999), Pane::default());
    assert!(l.validate().is_err(), "orphan pane");
}

// ------------------------------------------------------------------ property test

#[derive(Clone, Debug)]
enum Op {
    Open(u8, u8),
    Close(u8, u8),
    MoveTab(u8, u8, u8, Option<u8>),
    MovePane(u8, u8, Option<u8>),
    Collapse(u8),
    Maximize(u8),
    Resize(u8, f32),
    Detach(u8, u8),
    Attach(u8, u8, Option<u8>),
    CloseWindow(u8),
    Cycle(u8, i8),
}

fn op() -> impl Strategy<Value = Op> {
    let e = proptest::option::of(0u8..4);
    prop_oneof![
        3 => (any::<u8>(), 0u8..24).prop_map(|(p, x)| Op::Open(p, x)),
        2 => (any::<u8>(), any::<u8>()).prop_map(|(p, i)| Op::Close(p, i)),
        4 => (any::<u8>(), any::<u8>(), any::<u8>(), e.clone()).prop_map(|(a, i, b, e)| Op::MoveTab(a, i, b, e)),
        2 => (any::<u8>(), any::<u8>(), e.clone()).prop_map(|(a, b, e)| Op::MovePane(a, b, e)),
        2 => any::<u8>().prop_map(Op::Collapse),
        1 => any::<u8>().prop_map(Op::Maximize),
        2 => (any::<u8>(), -1.0f32..1.0).prop_map(|(s, d)| Op::Resize(s, d)),
        2 => (any::<u8>(), any::<u8>()).prop_map(|(p, i)| Op::Detach(p, i)),
        1 => (any::<u8>(), any::<u8>(), e).prop_map(|(w, p, e)| Op::Attach(w, p, e)),
        1 => any::<u8>().prop_map(Op::CloseWindow),
        1 => (any::<u8>(), -3i8..3).prop_map(|(p, d)| Op::Cycle(p, d)),
    ]
}

fn nth<T: Copy>(items: &[T], i: u8) -> T {
    items[i as usize % items.len()]
}

fn edge(e: u8) -> Edge {
    [Edge::Left, Edge::Right, Edge::Top, Edge::Bottom][e as usize % 4]
}

fn drop_for(target: PaneId, e: Option<u8>) -> Drop {
    e.map_or(Drop::Into(target), |e| Drop::Edge(target, edge(e)))
}

fn splits(node: &Node, out: &mut Vec<(SplitId, usize)>) {
    if let Node::Split(s) = node {
        out.push((s.id, s.children.len()));
        s.children.iter().for_each(|c| splits(&c.node, out));
    }
}

fn apply(l: &mut Layout, op: &Op) {
    let panes: Vec<PaneId> = l.panes.keys().copied().collect();
    let windows: Vec<WindowId> = l.windows.iter().map(|w| w.id).collect();
    let tab = |l: &Layout, p: PaneId, i: u8| {
        let n = l.panes[&p].tabs.len();
        (n > 0).then(|| i as usize % n)
    };
    let before = l.views();
    let mut closed = None;
    match *op {
        Op::Open(p, x) => {
            let _ = l.open(v(&format!("view-{x}")), nth(&panes, p));
            return;
        }
        Op::Close(p, i) => {
            let p = nth(&panes, p);
            if let Some(i) = tab(l, p, i) {
                closed = l.close_tab(p, i).ok();
            }
        }
        Op::MoveTab(a, i, b, e) => {
            let a = nth(&panes, a);
            if let Some(i) = tab(l, a, i) {
                let _ = l.move_tab(a, i, drop_for(nth(&panes, b), e));
            }
        }
        Op::MovePane(a, b, e) => {
            let _ = l.move_pane(nth(&panes, a), drop_for(nth(&panes, b), e));
        }
        Op::Collapse(p) => {
            let _ = l.toggle_collapse(nth(&panes, p));
        }
        Op::Maximize(p) => {
            let _ = l.toggle_maximize(nth(&panes, p));
        }
        Op::Resize(s, d) => {
            let mut all = Vec::new();
            l.windows.iter().for_each(|w| splits(&w.root, &mut all));
            if !all.is_empty() {
                let (id, n) = all[s as usize % all.len()];
                let _ = l.resize(id, s as usize % (n - 1), d);
            }
        }
        Op::Detach(p, i) => {
            let p = nth(&panes, p);
            if let Some(i) = tab(l, p, i) {
                let _ = l.detach(p, i);
            }
        }
        Op::Attach(w, p, e) => {
            let _ = l.attach(nth(&windows, w), drop_for(nth(&panes, p), e));
        }
        Op::CloseWindow(w) => {
            if let Ok(CloseOutcome::Closed(views)) = l.close_window(nth(&windows, w)) {
                let mut expected = before.clone();
                views.iter().for_each(|v| {
                    expected.remove(v);
                });
                assert_eq!(l.views(), expected, "closing a window drops exactly its views");
                return;
            }
        }
        Op::Cycle(p, d) => {
            let _ = l.cycle_tab(nth(&panes, p), d as isize);
        }
    }
    let mut expected = before;
    if let Some(c) = closed {
        expected.remove(&c);
    }
    assert_eq!(l.views(), expected, "views must be conserved by {op:?}");
}

proptest! {
    // 256 cases by default; set PROPTEST_CASES for a deeper run.
    #[test]
    fn random_operations_keep_invariants(ops in proptest::collection::vec(op(), 1..120)) {
        let mut l = Layout::new();
        for o in &ops {
            apply(&mut l, o);
            if let Err(e) = l.validate() {
                prop_assert!(false, "after {o:?}: {e}\n{l:#?}");
            }
        }
        let back = Layout::from_json(&l.to_json()).unwrap();
        prop_assert_eq!(back, l);
    }
}

#[test]
fn send_to_numbers_panes_in_reading_order_main_first() {
    let (mut l, a) = with_tabs(&["a", "b", "c", "d"]);
    l.move_tab(a, 1, Drop::Edge(a, Edge::Right)).unwrap(); // b right of a
    let b = l.find_view(&v("b")).unwrap().0;
    l.move_tab(a, 1, Drop::Edge(a, Edge::Bottom)).unwrap(); // c under a
    let c = l.find_view(&v("c")).unwrap().0;
    let d_win = l.detach(a, 1).unwrap(); // d alone
    let d = l.window(d_win).unwrap().focused;
    l.toggle_maximize(b).unwrap();
    let numbered: Vec<PaneId> = l.numbered_panes().into_iter().map(|(p, _)| p).collect();
    assert_eq!(numbered, [a, b, c, d], "top row left to right, then below, then detached; maximize ignored");
}

#[test]
fn send_tab_and_pane_into_panes_and_new_windows() {
    let (mut l, a) = with_tabs(&["a", "b", "c"]);
    l.move_tab(a, 2, Drop::Edge(a, Edge::Right)).unwrap();
    let c = l.find_view(&v("c")).unwrap().0;
    l.send_tab(&v("b"), Some(c)).unwrap();
    assert_eq!(l.pane(c).unwrap().tabs, [v("c"), v("b")]);
    ok(&l);
    l.send_pane(c, None).unwrap();
    assert_eq!(l.windows.len(), 2, "the pane's tabs moved to one new window");
    let w = l.windows[1].id;
    let np = l.window(w).unwrap().focused;
    assert_eq!(l.pane(np).unwrap().tabs, [v("c"), v("b")]);
    assert!(matches!(l.send_pane(np, None), Err(LayoutError::AlreadyAlone)), "already alone");
    l.send_pane(np, Some(a)).unwrap();
    assert_eq!(l.windows.len(), 1, "sending a detached window's pane back closes it");
    assert_eq!(l.pane(a).unwrap().tabs.len(), 3);
    ok(&l);
}

#[test]
fn tabs_reorder_within_a_pane_and_land_at_a_place_in_another() {
    let (mut l, p) = with_tabs(&["a", "b", "c", "d"]);
    let names = |l: &Layout, p: PaneId| l.pane(p).unwrap().tabs.iter().map(|r| r.as_str().to_string()).collect::<Vec<_>>();
    l.move_tab_to(p, 0, p, 3).unwrap();
    assert_eq!(names(&l, p), ["b", "c", "a", "d"], "before the tab at 3 (d)");
    assert_eq!(l.pane(p).unwrap().active, 2, "the moved one is active");
    l.move_tab_to(p, 3, p, 0).unwrap();
    assert_eq!(names(&l, p), ["d", "b", "c", "a"]);
    l.move_tab_to(p, 1, p, 4).unwrap();
    assert_eq!(names(&l, p), ["d", "c", "a", "b"], "the count puts it last");
    l.move_tab_to(p, 1, p, 2).unwrap();
    assert_eq!(names(&l, p), ["d", "c", "a", "b"], "onto itself: no move");
    assert_eq!(l.pane(p).unwrap().active, 1);
    ok(&l);
    // "b" goes to a pane of its own on the right; then "d" lands before it there.
    l.move_tab(p, 3, Drop::Edge(p, Edge::Right)).unwrap();
    let q = l.main().focused;
    l.move_tab_to(p, 0, q, 0).unwrap();
    assert_eq!(names(&l, q), ["d", "b"], "first in the other pane");
    assert_eq!(l.main().focused, q);
    assert!(l.move_tab_to(p, 9, q, 0).is_err() && l.move_tab_to(p, 0, q, 9).is_err());
    ok(&l);
}
