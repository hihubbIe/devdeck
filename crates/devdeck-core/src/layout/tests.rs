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
    let r: BTreeMap<_, _> = l.pane_rects(MAIN_WINDOW, (1000.0, 600.0), Metrics::uniform(32.0)).unwrap().into_iter().collect();
    assert_eq!(r[&p], Rect { x: 0.0, y: 0.0, w: 500.0, h: 600.0 });
    assert_eq!(r[&b], Rect { x: 500.0, y: 0.0, w: 500.0, h: 300.0 });
    l.toggle_collapse(c).unwrap();
    let r: BTreeMap<_, _> = l.pane_rects(MAIN_WINDOW, (1000.0, 600.0), Metrics::uniform(32.0)).unwrap().into_iter().collect();
    assert_eq!(r[&c].h, 32.0);
    assert_eq!(r[&b].h, 568.0);
    l.toggle_maximize(b).unwrap();
    let r = l.pane_rects(MAIN_WINDOW, (1000.0, 600.0), Metrics::uniform(32.0)).unwrap();
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
    assert_eq!(l.neighbor(p, Direction::Right, size, Metrics::uniform(32.0)).unwrap(), Some(b));
    assert_eq!(l.neighbor(b, Direction::Down, size, Metrics::uniform(32.0)).unwrap(), Some(c));
    assert_eq!(l.neighbor(c, Direction::Up, size, Metrics::uniform(32.0)).unwrap(), Some(b));
    assert_eq!(l.neighbor(c, Direction::Left, size, Metrics::uniform(32.0)).unwrap(), Some(p));
    assert_eq!(l.neighbor(p, Direction::Left, size, Metrics::uniform(32.0)).unwrap(), None);
    assert_eq!(l.neighbor(p, Direction::Up, size, Metrics::uniform(32.0)).unwrap(), None);
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
    // 0..4: beside the pane; 4..8: around a split of its window.
    let e = proptest::option::of(0u8..8);
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

fn drop_for(l: &Layout, target: PaneId, e: Option<u8>) -> Drop {
    match e {
        None => Drop::Into(target),
        Some(e) if e < 4 => Drop::Edge(target, edge(e)),
        Some(e) => {
            let mut all = Vec::new();
            if let Ok(w) = l.window_of(target) {
                splits(&l.window(w).unwrap().root, &mut all);
            }
            all.get(target.0 as usize % all.len().max(1)).map_or(Drop::Edge(target, edge(e)), |(s, _)| Drop::Around(*s, edge(e)))
        }
    }
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
                // The preview is where the tab really lands.
                let (size, m) = ((1000.0, 700.0), Metrics { divider: 5.0, rail: 28.0, header: 32.0 });
                let d = drop_for(l, nth(&panes, b), e);
                let view = l.panes[&a].tabs[i].clone();
                let want = l.preview(&Moved::Tab(a, i), d, size, m);
                let before = l.clone();
                if l.move_tab(a, i, d).is_ok() {
                    if let Drop::Into(t) = d {
                        merge_keeps_others(&before, l, a, t);
                    }
                    let (p, _) = l.find_view(&view).unwrap();
                    let w = l.window_of(p).unwrap();
                    let got = l.pane_rects(w, size, m).unwrap().into_iter().find(|(q, _)| *q == p).map(|(_, r)| r);
                    assert_eq!(want, got, "preview of {d:?}");
                } else {
                    assert_eq!(want, None, "a refused drop has no preview: {d:?}");
                }
            }
        }
        Op::MovePane(a, b, e) => {
            let d = drop_for(l, nth(&panes, b), e);
            let before = l.clone();
            if l.move_pane(nth(&panes, a), d).is_ok()
                && let Drop::Into(t) = d
            {
                merge_keeps_others(&before, l, nth(&panes, a), t);
            }
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
            let d = drop_for(l, nth(&panes, p), e);
            let _ = l.attach(nth(&windows, w), d);
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

// ------------------------------------------------------------------ drops beside a split (R3.13)

const M: Metrics = Metrics { divider: 5.0, rail: 28.0, header: 32.0 };
const SIZE: (f32, f32) = (1000.0, 605.0);

/// `[a / b]` (stacked), b holding `b` and `x`; the root split's id.
fn stacked() -> (Layout, PaneId, PaneId, SplitId) {
    let (mut l, a) = with_tabs(&["a", "b", "x"]);
    l.move_tab(a, 1, Drop::Edge(a, Edge::Bottom)).unwrap();
    let (b, _) = l.find_view(&v("b")).unwrap();
    l.move_tab(a, 1, Drop::Into(b)).unwrap();
    let root = root_split(&l).id;
    (l, a, b, root)
}

fn rect_of(l: &Layout, view: &str) -> Rect {
    let (p, _) = l.find_view(&v(view)).unwrap();
    l.pane_rects(MAIN_WINDOW, SIZE, M).unwrap().into_iter().find(|(q, _)| *q == p).unwrap().1
}

#[test]
fn a_tab_dropped_around_two_stacked_panes_gets_a_full_height_column() {
    let (mut l, _, b, root) = stacked();
    let i = l.pane(b).unwrap().tabs.iter().position(|t| *t == v("x")).unwrap();
    let want = l.preview(&Moved::Tab(b, i), Drop::Around(root, Edge::Right), SIZE, M);
    l.move_tab(b, i, Drop::Around(root, Edge::Right)).unwrap();
    ok(&l);
    let x = rect_of(&l, "x");
    assert_eq!(x, Rect { x: 502.5, y: 0.0, w: 497.5, h: 605.0 }, "full height, half the width");
    assert_eq!(want, Some(x));
    assert_eq!(rect_of(&l, "a").w, 497.5, "the stack keeps its two panes on the left");
    assert_eq!(rect_of(&l, "a").x, rect_of(&l, "b").x);
}

#[test]
fn beside_a_split_running_along_the_edge_takes_half_its_expanded_space() {
    // [a(collapsed) | b], a tab dropped right of the whole row.
    let (mut l, a) = with_tabs(&["a", "b", "x"]);
    l.move_tab(a, 1, Drop::Edge(a, Edge::Right)).unwrap();
    let (b, _) = l.find_view(&v("b")).unwrap();
    l.move_tab(a, 1, Drop::Into(b)).unwrap();
    l.toggle_collapse(a).unwrap();
    let root = root_split(&l).id;
    let i = l.pane(b).unwrap().tabs.iter().position(|t| *t == v("x")).unwrap();
    l.move_tab(b, i, Drop::Around(root, Edge::Right)).unwrap();
    ok(&l);
    let expanded = SIZE.0 - 28.0 - 2.0 * 5.0;
    assert!((rect_of(&l, "x").w - expanded / 2.0).abs() < 0.01, "{:?}", rect_of(&l, "x"));
}

#[test]
fn a_pane_moved_beside_its_own_column_and_a_one_tab_pane_beside_itself() {
    // [a | [b / c]]: c beside the whole right column, then a no-op.
    let (mut l, a) = with_tabs(&["a", "b", "c"]);
    l.move_tab(a, 1, Drop::Edge(a, Edge::Right)).unwrap();
    let (b, _) = l.find_view(&v("b")).unwrap();
    l.move_tab(a, 1, Drop::Edge(b, Edge::Bottom)).unwrap();
    let (c, _) = l.find_view(&v("c")).unwrap();
    let Node::Split(root) = &l.main().root else { panic!() };
    let Node::Split(col) = &root.children[1].node else { panic!() };
    let col = col.id;
    let want = l.preview(&Moved::Pane(c), Drop::Around(col, Edge::Right), SIZE, M);
    l.move_pane(c, Drop::Around(col, Edge::Right)).unwrap();
    ok(&l);
    assert_eq!(want, Some(rect_of(&l, "c")));
    assert_eq!(rect_of(&l, "c").h, SIZE.1, "full height");
    let before = l.clone();
    assert_eq!(l.preview(&Moved::Tab(c, 0), Drop::Edge(c, Edge::Left), SIZE, M), Some(rect_of(&l, "c")), "no-op: where it is");
    l.move_tab(c, 0, Drop::Edge(c, Edge::Left)).unwrap();
    assert_eq!(l, before);
}

#[test]
fn a_drop_beside_shows_the_whole_tree_again_and_a_gone_split_is_refused() {
    let (mut l, a, b, root) = stacked();
    l.toggle_maximize(a).unwrap();
    let i = l.pane(b).unwrap().tabs.iter().position(|t| *t == v("x")).unwrap();
    let want = l.preview(&Moved::Tab(b, i), Drop::Edge(a, Edge::Right), SIZE, M);
    l.move_tab(b, i, Drop::Edge(a, Edge::Right)).unwrap();
    assert_eq!(l.main().maximized, None, "the new pane is seen");
    assert_eq!(want, Some(rect_of(&l, "x")));
    let gone = SplitId(root.0 + 1000);
    assert!(matches!(l.move_tab(b, 0, Drop::Around(gone, Edge::Left)), Err(LayoutError::UnknownSplit(_))));
    assert_eq!(l.preview(&Moved::Tab(b, 0), Drop::Around(gone, Edge::Left), SIZE, M), None);
}

#[test]
fn drop_at_reads_centre_bands_strips_headers_and_dividers() {
    let (l, a, b, root) = stacked();
    let at = |x: f32, y: f32| l.drop_at(MAIN_WINDOW, SIZE, M, 16.0, (x, y)).unwrap();
    // a: y 0..300 (body 32..300), divider 300..305, b: 305..605 (body 337..605).
    assert_eq!(at(500.0, 160.0), Some(Drop::Into(a)), "centre");
    assert_eq!(at(500.0, 10.0), None, "a header");
    assert_eq!(at(500.0, 302.0), None, "the divider");
    assert_eq!(at(960.0, 470.0), Some(Drop::Edge(b, Edge::Right)), "the band, inside the strips");
    assert_eq!(at(995.0, 470.0), Some(Drop::Around(root, Edge::Right)), "the strip at the edge: the whole window");
    assert_eq!(at(500.0, 600.0), Some(Drop::Around(root, Edge::Bottom)), "b ends the column: its bottom is the window's");
    assert_eq!(at(500.0, 580.0), Some(Drop::Edge(b, Edge::Bottom)));
    assert_eq!(at(500.0, 290.0), Some(Drop::Edge(a, Edge::Bottom)), "a's bottom is inside the column: no strip");
}

#[test]
fn every_level_sharing_the_edge_gets_a_strip_the_outermost_at_the_edge() {
    // [[a | b] / c]: b's right side is the row's and the window's.
    let (mut l, a) = with_tabs(&["a", "b", "c"]);
    l.move_tab(a, 2, Drop::Edge(a, Edge::Bottom)).unwrap();
    l.move_tab(a, 1, Drop::Edge(a, Edge::Right)).unwrap();
    let (b, _) = l.find_view(&v("b")).unwrap();
    let col = root_split(&l).id;
    let Node::Split(row) = &root_split(&l).children[0].node else { panic!() };
    let row = row.id;
    let at = |x: f32| l.drop_at(MAIN_WINDOW, SIZE, M, 16.0, (x, 150.0)).unwrap();
    assert_eq!(at(998.0), Some(Drop::Around(col, Edge::Right)), "outermost");
    assert_eq!(at(984.0), Some(Drop::Around(row, Edge::Right)));
    assert_eq!(at(960.0), Some(Drop::Edge(b, Edge::Right)));
    // a's right side is inside the row: its own edge only.
    assert_eq!(l.drop_at(MAIN_WINDOW, SIZE, M, 16.0, (495.0, 150.0)).unwrap(), Some(Drop::Edge(a, Edge::Right)));
    // A narrow pane: the strips share 2/3 of the band, the pane's own edge keeps the rest.
    let narrow = (120.0, 605.0);
    let band = (120.0 - 5.0) / 2.0 * 0.25;
    let near = |d: f32| l.drop_at(MAIN_WINDOW, narrow, M, 16.0, (120.0 - d, 150.0)).unwrap();
    assert_eq!(near(band * 2.0 / 3.0 + 0.5), Some(Drop::Edge(b, Edge::Right)));
    assert_eq!(near(0.5), Some(Drop::Around(col, Edge::Right)));
}

#[test]
fn a_lone_tab_moved_beside_its_own_split_takes_half_like_its_pane_would() {
    // [a | b], a's only tab to the right of the whole row: the pane moves, half and half.
    let (mut l, a) = with_tabs(&["a", "b"]);
    l.move_tab(a, 1, Drop::Edge(a, Edge::Right)).unwrap();
    let root = root_split(&l).id;
    let want = l.preview(&Moved::Tab(a, 0), Drop::Around(root, Edge::Right), SIZE, M);
    l.move_tab(a, 0, Drop::Around(root, Edge::Right)).unwrap();
    ok(&l);
    assert_eq!(rect_of(&l, "a").w, (SIZE.0 - 5.0) / 2.0);
    assert!(rect_of(&l, "a").x > rect_of(&l, "b").x, "now on the right");
    assert_eq!(want, Some(rect_of(&l, "a")));
}

#[test]
fn opening_beside_a_split_leaves_every_other_pane_as_it_was() {
    // [a(collapsed, tabs a and c, a active) / b]: a row opened right of the whole column.
    let (mut l, a) = with_tabs(&["a", "b", "c"]);
    l.move_tab(a, 1, Drop::Edge(a, Edge::Bottom)).unwrap();
    l.activate(a, 0).unwrap();
    l.toggle_collapse(a).unwrap();
    let root = root_split(&l).id;
    let before = l.pane(a).unwrap().clone();
    l.open_at(v("new"), Drop::Around(root, Edge::Right)).unwrap();
    ok(&l);
    assert_eq!(l.pane(a).unwrap(), &before, "the first pane untouched");
    let (p, _) = l.find_view(&v("new")).unwrap();
    assert_eq!(l.pane(p).unwrap().tabs, [v("new")]);
    assert_eq!(l.main().focused, p);
}

#[test]
fn drop_at_offers_nothing_on_collapsed_panes_no_strips_while_maximized_and_only_into_when_detached() {
    // [a / b / c], c collapsed at the bottom: b ends the column for the bottom strip.
    let (mut l, a) = with_tabs(&["a", "b", "c"]);
    l.move_tab(a, 1, Drop::Edge(a, Edge::Bottom)).unwrap();
    let (b, _) = l.find_view(&v("b")).unwrap();
    l.move_tab(a, 1, Drop::Edge(b, Edge::Bottom)).unwrap();
    let (c, _) = l.find_view(&v("c")).unwrap();
    l.toggle_collapse(c).unwrap();
    let root = root_split(&l).id;
    let rects: BTreeMap<_, _> = l.pane_rects(MAIN_WINDOW, SIZE, M).unwrap().into_iter().collect();
    let at = |l: &Layout, x: f32, y: f32| l.drop_at(MAIN_WINDOW, SIZE, M, 16.0, (x, y)).unwrap();
    assert_eq!(at(&l, 500.0, rects[&c].y + 10.0), None, "a collapsed pane: its strip takes drops itself");
    assert_eq!(at(&l, 500.0, rects[&b].bottom() - 2.0), Some(Drop::Around(root, Edge::Bottom)), "past a collapsed pane");
    l.toggle_maximize(b).unwrap();
    assert_eq!(at(&l, 998.0, 300.0), Some(Drop::Edge(b, Edge::Right)), "maximized: its own edge, no strips");
    let w = l.detach(b, 0).unwrap();
    let (d, _) = l.find_view(&v("b")).unwrap();
    assert_eq!(l.drop_at(w, SIZE, M, 16.0, (998.0, 300.0)).unwrap(), Some(Drop::Into(d)), "detached: into only");
    // Its pane goes beside the main window's column, and the detached window with it.
    l.move_pane(d, Drop::Around(root, Edge::Right)).unwrap();
    ok(&l);
    assert_eq!(l.windows.len(), 1);
    assert_eq!(rect_of(&l, "b").h, SIZE.1);
}

// ------------------------------------------------------------------ exact space when a pane goes

const DIVIDER: f32 = 5.0;

fn near(a: f32, b: f32) -> bool {
    near_by(a, b, 0.0)
}

fn near_by(a: f32, b: f32, slack: f32) -> bool {
    (a - b).abs() <= DIVIDER + slack + 0.01
}

/// How many splits hold `pane` in `node`.
fn depth_of(node: &Node, pane: PaneId) -> Option<usize> {
    match node {
        Node::Pane(p) => (*p == pane).then_some(0),
        Node::Split(s) => s.children.iter().find_map(|c| depth_of(&c.node, pane)).map(|d| d + 1),
    }
}

/// A collapsed pane folded up into its parent's split draws its fixed size (rail or
/// header, and a divider) there instead: no weight makes that exact at every size, so
/// each one moved up a level may cost the others that much.
fn moved_rails(before: &Layout, after: &Layout, window: WindowId, m: Metrics) -> f32 {
    let (Ok(b), Ok(a)) = (before.window(window), after.window(window)) else { return 0.0 };
    let moved = before.panes.iter().filter(|(_, p)| p.collapsed).filter(|(id, _)| matches!((depth_of(&b.root, **id), depth_of(&a.root, **id)), (Some(x), Some(y)) if y < x)).count();
    moved as f32 * (m.rail.max(m.header) + m.divider)
}

/// After `source` merged into `target` (and went away): in its window, every pane outside
/// the sibling that took its space keeps its size, within the freed divider (not always
/// its place: one between the two moves over as the receiver grows). Skipped
/// where no rectangle can tell: maximized, a collapsed source or target, other windows.
fn merge_keeps_others(before: &Layout, after: &Layout, source: PaneId, target: PaneId) {
    if after.panes.contains_key(&source) || before.window_of(source).ok() != before.window_of(target).ok() {
        return;
    }
    let Ok(window) = before.window_of(source) else { return };
    let collapsed = |p: PaneId| before.panes.get(&p).is_some_and(|p| p.collapsed);
    if before.window(window).unwrap().maximized.is_some() || collapsed(source) || collapsed(target) {
        return;
    }
    let Some((split, i, _)) = before.parent_of(source) else { return };
    let s = before.split(split).unwrap();
    let expanded = |n: &Node| !matches!(n, Node::Pane(p) if collapsed(*p));
    let others: Vec<&Child> = s.children.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, c)| c).collect();
    let holds = others.iter().position(|c| contains_pane(&c.node, target));
    let before_i = (0..i).rev().find(|&j| expanded(&others[j].node));
    let after_i = (i..others.len()).find(|&j| expanded(&others[j].node));
    let Some(to) = holds.or(before_i).or(after_i) else { return };
    let mut receiving = Vec::new();
    collect_panes(&others[to].node, &mut receiving);
    let (size, m) = ((1000.0, 700.0), Metrics { divider: DIVIDER, rail: 28.0, header: 32.0 });
    let old: BTreeMap<_, _> = before.pane_rects(window, size, m).unwrap().into_iter().collect();
    let new: BTreeMap<_, _> = after.pane_rects(window, size, m).unwrap().into_iter().collect();
    let slack = moved_rails(before, after, window, m);
    // The rule on its own: a target next to it in its split grows by the source.
    if matches!(&others[to].node, Node::Pane(p) if *p == target) && new.contains_key(&target) {
        let along = |r: &Rect| if s.axis == Axis::Row { r.w } else { r.h };
        let want = along(&old[&target]) + along(&old[&source]) + DIVIDER;
        assert!(near_by(along(&new[&target]), want, slack), "{target:?} took {source:?}: {:?} -> {:?}", old[&target], new[&target]);
    }
    for (p, r) in &old {
        if *p == source || receiving.contains(p) {
            continue;
        }
        let Some(n) = new.get(p) else { continue };
        assert!(near_by(r.w, n.w, slack) && near_by(r.h, n.h, slack), "{p:?} resized: {r:?} -> {n:?}, {source:?} into {target:?}");
    }
}

/// `[a | b | c]`, a third each.
fn thirds() -> (Layout, PaneId, PaneId, PaneId) {
    let (mut l, a) = with_tabs(&["a", "b", "c"]);
    l.move_tab(a, 1, Drop::Edge(a, Edge::Right)).unwrap();
    let (b, _) = l.find_view(&v("b")).unwrap();
    l.move_tab(a, 1, Drop::Edge(b, Edge::Right)).unwrap();
    let (c, _) = l.find_view(&v("c")).unwrap();
    if let Node::Split(s) = &mut l.windows[0].root {
        s.children.iter_mut().for_each(|c| c.weight = 1.0 / 3.0);
    }
    ok(&l);
    (l, a, b, c)
}

fn rects(l: &Layout) -> BTreeMap<PaneId, Rect> {
    l.pane_rects(MAIN_WINDOW, SIZE, M).unwrap().into_iter().collect()
}

#[test]
fn a_folded_split_gives_its_weight_to_its_expanded_panes() {
    // [a | [b / [c | d (collapsed)]]], b into a: [c | d] folds into the top row. Its
    // weight goes to c alone, so a keeps its width but for d's rail now in the row (it
    // went to 641 px of 1000 when d's weight counted).
    let (mut l, a) = with_tabs(&["a", "b", "c", "d"]);
    l.move_tab(a, 1, Drop::Edge(a, Edge::Right)).unwrap();
    let (b, _) = l.find_view(&v("b")).unwrap();
    l.move_tab(a, 1, Drop::Edge(b, Edge::Bottom)).unwrap();
    let (c, _) = l.find_view(&v("c")).unwrap();
    l.move_tab(a, 1, Drop::Edge(c, Edge::Right)).unwrap();
    let (d, _) = l.find_view(&v("d")).unwrap();
    l.toggle_collapse(d).unwrap();
    let r = rects(&l);
    let before = l.clone();
    l.move_pane(b, Drop::Into(a)).unwrap();
    ok(&l);
    let n = rects(&l);
    assert_eq!((n[&a].h, n[&c].h, n[&d].w), (SIZE.1, SIZE.1, M.rail), "one row");
    assert!(near_by(n[&a].w, r[&a].w, M.rail + M.divider), "a: {:?} -> {:?}", r[&a], n[&a]);
    assert!(near_by(n[&c].w, r[&c].w, M.rail + M.divider), "c: {:?} -> {:?}", r[&c], n[&c]);
    merge_keeps_others(&before, &l, b, a);
}

#[test]
fn a_merge_gives_the_two_widths_together_and_leaves_the_rest() {
    for how in ["pane", "last tab", "tab strip"] {
        let (mut l, a, b, c) = thirds();
        let r = rects(&l);
        match how {
            "pane" => l.move_pane(b, Drop::Into(a)).unwrap(),
            "last tab" => l.move_tab(b, 0, Drop::Into(a)).unwrap(),
            _ => l.move_tab_to(b, 0, a, 0).unwrap(),
        }
        ok(&l);
        let n = rects(&l);
        assert!(near(n[&a].w, r[&a].w + r[&b].w + DIVIDER), "{how}: {:?} from {:?} and {:?}", n[&a], r[&a], r[&b]);
        assert!(near(n[&c].w, r[&c].w) && near(n[&c].x, r[&c].x), "{how}: c stays");
    }
}

#[test]
fn a_merge_into_a_pane_of_a_column_widens_the_column() {
    // [a | [b / c]], a into b.
    let (mut l, a) = with_tabs(&["a", "b", "c"]);
    l.move_tab(a, 1, Drop::Edge(a, Edge::Right)).unwrap();
    let (b, _) = l.find_view(&v("b")).unwrap();
    l.move_tab(a, 1, Drop::Edge(b, Edge::Bottom)).unwrap();
    let (c, _) = l.find_view(&v("c")).unwrap();
    let r = rects(&l);
    l.move_pane(a, Drop::Into(b)).unwrap();
    ok(&l);
    let n = rects(&l);
    assert_eq!((n[&b].w, n[&c].w), (SIZE.0, SIZE.0), "the column takes it all");
    assert!(near(n[&b].h, r[&b].h) && near(n[&c].h, r[&c].h), "b and c share it as before");
}

#[test]
fn a_closed_pane_gives_its_space_to_the_one_before_or_after_skipping_collapsed() {
    let (mut l, a, b, c) = thirds();
    let r = rects(&l);
    l.close_tab(b, 0).unwrap();
    let n = rects(&l);
    assert!(near(n[&a].w, r[&a].w + r[&b].w + DIVIDER) && near(n[&c].w, r[&c].w), "b's space to a");
    let (mut l, a, b, c) = thirds();
    let r = rects(&l);
    l.close_tab(a, 0).unwrap();
    let n = rects(&l);
    assert!(near(n[&b].w, r[&a].w + r[&b].w + DIVIDER) && near(n[&c].w, r[&c].w), "the first one's to the next");
    // [a | b(collapsed) | c]: c closed, a gets it past the collapsed b.
    let (mut l, a, b, c) = thirds();
    l.toggle_collapse(b).unwrap();
    let r = rects(&l);
    l.close_tab(c, 0).unwrap();
    let n = rects(&l);
    assert!(near(n[&a].w, r[&a].w + r[&c].w + DIVIDER), "{:?} from {:?}", n[&a], r[&a]);
    assert_eq!(n[&b].w, 28.0);
}

#[test]
fn a_pane_put_back_beside_its_neighbour_gives_the_same_layout() {
    let (mut l, _, b, c) = thirds();
    let r = rects(&l);
    l.move_pane(b, Drop::Edge(c, Edge::Left)).unwrap();
    ok(&l);
    let n = rects(&l);
    for (p, x) in &r {
        assert!(near(n[p].x, x.x) && near(n[p].w, x.w), "{p:?}: {:?} -> {:?}", x, n[p]);
    }
}

#[test]
fn a_tab_into_a_collapsed_pane_gives_it_exactly_the_space_freed() {
    // [a | b(collapsed) | c], a's only tab into b.
    let (mut l, a, b, c) = thirds();
    l.toggle_collapse(b).unwrap();
    let r = rects(&l);
    l.move_tab(a, 0, Drop::Into(b)).unwrap();
    ok(&l);
    let n = rects(&l);
    assert!(!l.pane(b).unwrap().collapsed);
    // b takes a's weight (not a's plus its own): b and c are a third each again, the
    // rail it no longer draws shared like a divider (a limit).
    assert_eq!(n[&b].w, n[&c].w, "b replaced a: {:?} {:?}", n[&b], n[&c]);
    assert!((n[&c].w - r[&c].w).abs() <= 28.0 + DIVIDER, "c about stays: {:?} -> {:?}", r[&c], n[&c]);
}

#[test]
fn a_split_lifted_into_its_grandparent_keeps_every_size() {
    // [x | [y / [b | c]]], y closed: [b | c] joins the row, x unchanged.
    let (mut l, x) = with_tabs(&["x", "y", "b", "c"]);
    l.move_tab(x, 1, Drop::Edge(x, Edge::Right)).unwrap();
    let (y, _) = l.find_view(&v("y")).unwrap();
    l.move_tab(x, 1, Drop::Edge(y, Edge::Bottom)).unwrap();
    let (b, _) = l.find_view(&v("b")).unwrap();
    l.move_tab(x, 1, Drop::Edge(b, Edge::Right)).unwrap();
    let (c, _) = l.find_view(&v("c")).unwrap();
    let r = rects(&l);
    l.close_tab(y, 0).unwrap();
    ok(&l);
    let n = rects(&l);
    assert!(near(n[&x].w, r[&x].w) && near(n[&x].x, r[&x].x), "x stays");
    assert!(near(n[&b].w, r[&b].w) && near(n[&c].w, r[&c].w), "b and c keep their widths");
    assert_eq!(n[&b].h, SIZE.1, "and take the full height");
}

/// `[a .2 | b .5 | c .3]`.
fn uneven() -> (Layout, PaneId, PaneId, PaneId) {
    let (mut l, a, b, c) = thirds();
    if let Node::Split(s) = &mut l.windows[0].root {
        for (c, w) in s.children.iter_mut().zip([0.2, 0.5, 0.3]) {
            c.weight = w;
        }
    }
    ok(&l);
    (l, a, b, c)
}

#[test]
fn a_pane_moved_along_its_own_split_keeps_its_size_whatever_the_sizes() {
    let (mut l, a, b, c) = uneven();
    let r = rects(&l);
    l.move_pane(b, Drop::Edge(c, Edge::Left)).unwrap();
    assert_eq!(rects(&l), r, "put back where it is: the same layout");
    l.move_pane(b, Drop::Edge(a, Edge::Left)).unwrap();
    ok(&l);
    let n = rects(&l);
    for p in [a, b, c] {
        assert!(near(n[&p].w, r[&p].w), "{p:?} keeps its width: {:?} -> {:?}", r[&p], n[&p]);
    }
    assert_eq!(n[&b].x, 0.0, "b first now");
}

#[test]
fn a_merge_into_a_pane_further_away_gives_it_the_space_not_the_neighbour() {
    // [a | b | c] uneven, c into a: a grows by c, b keeps its width (it moves over).
    let (mut l, a, b, c) = uneven();
    let r = rects(&l);
    l.move_pane(c, Drop::Into(a)).unwrap();
    ok(&l);
    let n = rects(&l);
    assert!(near(n[&a].w, r[&a].w + r[&c].w + DIVIDER), "{:?}", n[&a]);
    assert!(near(n[&b].w, r[&b].w), "b keeps its width");
}
