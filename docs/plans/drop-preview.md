# Drops land beside any part of the layout, and show where they land

Asked 2026-11-08 (user). One step, one commit. Covers R3.4, R3.13 (new).

## What

- A dragged tab, pane or sidebar item can land beside a whole column or row, not only
  beside one pane. Two stacked panes, a tab dropped on the window's right edge: the two
  stay stacked on the left, the tab gets a full-height column on the right, in one drop.
- While dragging, the highlight is the exact rectangle the dropped thing will take
  (half of the pane or of the column it lands beside; the whole pane for "into").

## Decisions (user, 2026-11-08)

Every level offered, as nested strips at the shared edge (about 16 px each, the
outermost being the biggest area). The new part takes half of what it lands beside.
Detached windows stay a single pane: no edges there (R3.8).

## How

Core (`devdeck-core::layout`), pure and unit-tested:
- `Drop::Around(SplitId, Edge)`: beside a whole split. `insert_beside` generalized from a
  pane to a node (`NodeRef::Pane | Split`): when the node's parent has the edge's axis
  the new pane is its sibling and takes half the node's weight; otherwise the node is
  wrapped in a new split, half and half. On the root split, the root is wrapped.
  `Drop::pane()` callers adjusted (`Around` has no pane: `land_in_empty` and the
  detached check use the split's window).
- `Layout::node_rects(window, size, strip)`: rectangles of panes and splits (the pane
  one, `pane_rects`, built on it).
- `Layout::drop_at(window, size, strip, point) -> Option<Drop>`: the drop under a point.
  Inside a pane's body: the centre half is `Into`; an edge band (25 %) is `Edge(pane)`,
  and within the band, at the very edge, one strip per enclosing split whose edge on
  that side is the pane's own (outermost strip at the edge = outermost split, `Around`).
  Strip width is passed in (theme size). Tab strips and headers are not part of it
  (they keep their own drop: into, at a position).
- `Layout::preview(window, size, strip, what, drop) -> Option<Rect>`: applies the drop
  to a clone (`move_tab`, `move_pane`, or opening a view for a sidebar item) and returns
  the rectangle of the pane that then holds it. Never differs from the real result.

App (`window_view.rs`):
- The five per-pane `drop_zone`s go. While something is dragged, one overlay over the
  window's panes area takes `on_drag_move` for tabs, panes and list items, keeps the
  pointer's `drop_at` (a `Cell`), and draws the preview rectangle (`drop_bg`, accent
  border). Its `on_drop` applies the kept drop. Tab strips and headers stay on top of
  it as their own targets (their highlight as today).

## Tests

- Core: `insert_beside` on a node (sibling when the parent's axis matches, wrap
  otherwise, the root), weights; `drop_at` over a 2×1 and a 2-stacked layout (centre,
  pane band, each strip, nothing over a header); `preview` equals the rectangle after
  the real move for every drop of a layout; `Around` refused in a detached window.
- App: dragging a tab of the bottom of two stacked panes onto the window's right
  strip gives a row split [stack, new] with a full-height right pane; the preview
  overlay's bounds equal the pane's bounds after the drop.

## Visual check (snapshots)

A new state with a drag over a pane band and one over an outer strip, the preview shown
(a snapshot can hold a drag through the test platform's drag state, else a debug
setting of the overlay).

## Plan review (subagent, 2026-11-08) and assessment

All taken; they change the How above as follows.
- 1 The drop hitbox is painted first (an absolute child before the tree): strips and
  headers keep their own `on_drop` (they stop propagation), bodies have none, so a drop
  on a body falls through to it. The preview is a plain absolute div after the tree (no
  id, no listeners: no hitbox). App test: header drop and tab reorder still work.
- 2 Geometry takes metrics `{divider, rail, header}` (theme sizes) and subtracts the
  dividers; the overlay's own bounds are the size. Preview equals rendered bounds.
- 3 Edge and Around drops clear `maximized` (today's edge drop on a maximized pane
  makes a pane nobody sees); tests for the drop and its preview while maximized.
- 4 Wrapping a split with the edge's own axis inserts straight into it, weighted the
  sum of its expanded weights, so `tidy` and collapsed panes keep "half"; tested.
- 5 Shared edges come from the tree, not floats (Right: Column ancestors always, Row
  ancestors when the chain is their last child; mirrored), skipping collapsed trailing
  siblings, so a rail or collapsed header at the edge doesn't hide the outer strips.
- 6 Strips take at most 2/3 of the band, shared equally (the pane's own edge keeps 1/3);
  corners go to the nearest edge by normalized distance; `None` over headers and dividers.
- 7 The drag move checks its bounds and clears the kept drop outside, refreshes only
  when the drop changes; a stale `SplitId` returns `UnknownSplit`, never panics.
- 8 R5.1: "Move tab to window edge" left/right/up/down (palette actions, `Around` the
  root, or `Edge` when the root is a pane).
- 9 `open_at` opens an `Around` drop in the split's first pane, then moves it; R3.13
  added, R3.4 reworded, DESIGN (pane edge splits, drop zones) updated; the overlay shows
  only for tab, pane and list item drags, not divider or sidebar-edge drags.
- 10 The layout proptest generates `Around`; added cases: a pane beside its own ancestor,
  a one-tab pane's tab around its own parent and a no-op `Edge(self)`, maximized,
  collapsed neighbours, narrow bands, inner vs window-edge pane with three levels,
  `None` over a divider. The drag snapshot: first check that `HeadlessAppContext`
  can start a drag through `Window::dispatch_event`; else no snapshot, the app test
  covers the overlay's bounds.

## Diff review (subagent, 2026-11-08) and assessment

- 1 Fixed: a tab alone in its pane, moved beside something, now moves its pane (the pane
  leaves its place first), so it takes half, as a dragged pane does; tested.
- 2 Fixed: `open_at` beside something makes its own pane: no other pane is opened,
  expanded or switched to another tab; tested with a collapsed first pane.
- 3 Plan corrected: GPUI keeps the drag's type private, so the layer is there for any
  drag; it only reacts to tabs, panes, list rows and files.
- 4 Fixed: a file drag reads the disk only over the panes and shows nothing without a
  workspace; the preview (the copy) is redone only when the drop changes.
- 5 Fixed: the narrow-pane test uses the real body width.
- 6 Taken: core tests for a collapsed pane (none), past a collapsed pane, maximized (no
  strips), detached (into only), a detached pane moved around a split; an app test for a
  pane dragged by its grip and a centre drop. Files through the layer: `tests/files.rs`.
- 7 Fixed: the edge actions do nothing in a detached window. Placing beside an inner
  column by keyboard stays out (R3.13 asks the window edge).

## Out of scope

Dragging between windows (R3.8); keyboard placement beyond "Send to…" (R3.11).
