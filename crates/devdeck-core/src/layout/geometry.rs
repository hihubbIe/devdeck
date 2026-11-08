//! Pane and split rectangles computed from the model as the app draws it, used for
//! directional focus, drops (where they land and what they would make) and converting
//! pixel drags into weights.

use super::{Axis, Node, NodeRef, Pane, PaneId};
use std::collections::BTreeMap;

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn right(&self) -> f32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
    pub fn contains(&self, (x, y): (f32, f32)) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }
}

/// The sizes the app draws a layout with: the grab area between split children, a pane
/// collapsed in a row (its rail) and in a column (its header or tab strip).
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Metrics {
    pub divider: f32,
    pub rail: f32,
    pub header: f32,
}

impl Metrics {
    /// No dividers, collapsed panes `strip` long either way.
    pub fn uniform(strip: f32) -> Self {
        Self { divider: 0.0, rail: strip, header: strip }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

/// Rectangles of every node under `node` (itself included), drawn in `r`: dividers
/// between children, collapsed panes at their fixed size, the rest shared by weight.
pub(super) fn rects(node: &Node, r: Rect, m: Metrics, panes: &BTreeMap<PaneId, Pane>, out: &mut Vec<(NodeRef, Rect)>) {
    let s = match node {
        Node::Pane(p) => {
            out.push((NodeRef::Pane(*p), r));
            return;
        }
        Node::Split(s) => s,
    };
    out.push((NodeRef::Split(s.id), r));
    let collapsed = |n: &Node| matches!(n, Node::Pane(p) if panes.get(p).is_some_and(|p| p.collapsed));
    let (total, fixed) = match s.axis {
        Axis::Row => (r.w, m.rail),
        Axis::Column => (r.h, m.header),
    };
    let n_collapsed = s.children.iter().filter(|c| collapsed(&c.node)).count() as f32;
    let dividers = m.divider * (s.children.len() as f32 - 1.0);
    let remaining = (total - dividers - fixed * n_collapsed).max(0.0);
    let weight_sum: f32 = s.children.iter().filter(|c| !collapsed(&c.node)).map(|c| c.weight).sum();
    let mut offset = 0.0;
    for c in &s.children {
        let len = if collapsed(&c.node) {
            fixed.min(total)
        } else if weight_sum > 0.0 {
            remaining * c.weight / weight_sum
        } else {
            0.0
        };
        let child = match s.axis {
            Axis::Row => Rect { x: r.x + offset, y: r.y, w: len, h: r.h },
            Axis::Column => Rect { x: r.x, y: r.y + offset, w: r.w, h: len },
        };
        rects(&c.node, child, m, panes, out);
        offset += len + m.divider;
    }
}

/// The closest pane in `dir` whose span overlaps the source pane's span.
pub(super) fn neighbor(rects: &[(PaneId, Rect)], from: PaneId, dir: Direction) -> Option<PaneId> {
    const EPS: f32 = 0.5;
    let (_, src) = rects.iter().find(|(p, _)| *p == from)?;
    let overlap = |a0: f32, a1: f32, b0: f32, b1: f32| (a1.min(b1) - a0.max(b0)).max(0.0);
    rects
        .iter()
        .filter(|(p, _)| *p != from)
        .filter_map(|(p, r)| {
            let (gap, ov, center) = match dir {
                Direction::Right if r.x >= src.right() - EPS => {
                    (r.x - src.right(), overlap(src.y, src.bottom(), r.y, r.bottom()), (r.y - src.y).abs())
                }
                Direction::Left if r.right() <= src.x + EPS => {
                    (src.x - r.right(), overlap(src.y, src.bottom(), r.y, r.bottom()), (r.y - src.y).abs())
                }
                Direction::Down if r.y >= src.bottom() - EPS => {
                    (r.y - src.bottom(), overlap(src.x, src.right(), r.x, r.right()), (r.x - src.x).abs())
                }
                Direction::Up if r.bottom() <= src.y + EPS => {
                    (src.y - r.bottom(), overlap(src.x, src.right(), r.x, r.right()), (r.x - src.x).abs())
                }
                _ => return None,
            };
            (ov > 0.0).then_some((*p, gap, ov, center))
        })
        .min_by(|a, b| {
            a.1.total_cmp(&b.1)
                .then(b.2.total_cmp(&a.2))
                .then(a.3.total_cmp(&b.3))
        })
        .map(|(p, ..)| p)
}
