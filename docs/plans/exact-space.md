# A pane that goes away gives its space to one neighbour

Asked 2026-11-08 (user): "Merging and splitting should always use the exact space." One
step, one commit. Touches R3.4, R3.5, R3.13.

## Symptom

Merging two side-by-side panes (a pane, or a pane's last tab, dropped into the other)
gives a pane smaller than the two together.

## Cause

A pane leaving the tree (`remove_pane_node`, from `remove_pane_if_empty` and
`detach_node`) is cut out of its split, and `tidy` normalizes the weights left, so its
share is spread over every remaining sibling in proportion. `[a ⅓ | b ⅓ | c ⅓]`, b into
a: `[a ½ | c ½]`, not `[a ⅔ | c ⅓]`. Splitting is already exact (half of a pane, half of
a split).

## Rule (user, 2026-11-08)

The weight of a pane that goes away goes, whole, to one sibling in its split:
- merged into a pane (`Into`, a tab strip): the sibling holding that pane (the pane
  itself, or the column or row it is in), so the two together are exactly the result;
- otherwise (moved beside something else, closed, detached): the sibling just before it
  (left or above), or the one after when it was first;
- never a collapsed sibling (its size is fixed): the nearest expanded one on that side,
  else the other side;
- a split left with one child is lifted by `tidy` with its own weight, as today.

## How (`devdeck-core::layout`)

- `remove_pane_node(node, pane, receiver: Option<PaneId>, collapsed)`: in the split
  holding `pane` at index i, the receiving child is the one containing `receiver` when
  there is one, else i-1 (else i+1), skipping collapsed panes; its weight grows by
  `pane`'s before the child is removed. Weights then still sum to 1, so `tidy` changes
  nothing more.
- `remove_pane_if_empty(pane, receiver)` and `detach_node` pass it on: `move_tab` and
  `move_tab_to` into a pane, `move_pane` into a pane pass that pane; every other caller
  passes `None`.
- A removed collapsed pane gives nothing (its weight never drew anything).

Limit: a removed pane also frees its divider (5 px), shared by the expanded siblings as
today; weights know no pixels. Within a divider of exact, stated in the test.

## Tests (`layout/tests.rs`)

- `[a | b | c]`: b into a (as a pane, and as its last tab, and through a strip): a is
  b's width plus its own (plus the freed divider), c's rectangle unchanged.
- `[a | [b / c]]`: a into b: the column takes a's width; b and c share it as before.
- Closing b's last tab in `[a | b | c]`: a gets b's space, c unchanged; closing a (first):
  b gets it.
- A collapsed neighbour is skipped.
- Proptest: after a merge into a pane (`Into` drops of a pane or of a last tab), every
  pane not inside the receiving sibling keeps its rectangle (within a divider).

## Plan review (subagent, 2026-11-08) and assessment

All taken.
- 1 A receiver that was collapsed (uncollapsed by the merge) takes exactly the source's
  weight instead of adding it: callers pass whether it was collapsed.
- 2 A collapsed source frees its rail (28 px), shared like a divider: a stated limit;
  "weights still sum to 1" reworded (they may not; `normalize` rescales, harmlessly).
- 3 Moving a pane beside a pane of its own split: the receiver is the sibling holding
  the drop's target, so `[a|b|c]`, b to `Edge(c, Left)`, gives the layout back.
- 4 The rule, exactly: a child of the source's split that contains the receiver, else
  the expanded sibling before, else after. A receiver in another window or another split
  gets nothing (no rectangle can hold both).
- 5 No expanded sibling: the one before, else after, anyway.
- 6 Proptest: on `Into` moves that remove the source, in the source's window, not
  maximized, neither source nor receiver collapsed: panes outside the receiving child
  keep their rectangles within a divider.
- 7 Lift and flatten stay exact; a test where the split lifts into a same-axis grandparent.
- 8 No path missed.
- 9 `remove_pane_node` takes the `expanded` closure, like `insert_beside`.
- 10 Tests: a collapsed sibling between, the restoring move, a tab into a collapsed pane.

## Diff review (subagent, 2026-11-08) and assessment

- 1 Fixed: a pane moved beside a pane of its own split, along it, is a reorder with its
  own size, so any layout is given back (tested with uneven sizes).
- 2 Fixed: beside a whole split, the space goes to the one before (no arbitrary first pane).
- 3 Fixed: a collapsed sibling takes space only when the merge opens it.
- 4 Documented as a limit (a collapsed receiver inside a nested split).
- 5 Taken: the proptest also checks the rule on its own (a target next to the source
  grows by it). Edge/Around and collapsed targets stay in unit tests.
- 6 Taken: a merge into a pane further away; the collapsed case asserts replace, exactly.
- 7 Nothing to change.
- 8 Taken: DESIGN states "else any", "beside a split: the one before", the limit.

## Out of scope

Pixel-exact dividers; resizing by keyboard.
