# Apps list: quieter rows

Asked 2026-11-08 (user). One step, one commit. Touches R4.3, R4.6.

## What

- An app row shows no text on its right: no toolchain (`go`, `pnpm`…), no warning text
  ("docker not found").
- A missing tool is only the ⚠ icon, at the row's very right, like a folded folder's.
  The row's tooltip reads `name · reason`, so the reason and a truncated name (R4.4)
  stay readable.
- The left slot is always the run dot or ring.
- Only the Apps list changes: the Cluster, Ports and Terminals lists keep their right text.

## How

`sidebar.rs`:
- `items` for Apps: no `detail` (the toolchain); the hint is `name · reason` when the
  app has a warning.
- `Row::Item` drawing: the left slot is always `dot` (running or idle); with a warning,
  ⚠ is the row's last child, after `holds`; the warning text is no longer drawn. The
  right text (`detail`) stays for the other lists.

## Tests (`devdeck-app/tests/sidebar.rs`)

- An app with a missing tool: ⚠ is the row's rightmost element, no warning or toolchain
  text is drawn, the tooltip is `name · reason`, the left slot is its dot.
- A plain app shows no right text.
- The Cluster list keeps its detail text.

## Visual check (snapshots)

50 (apps list), 58 at 150 %, 71 (missing tool).

## Decisions (user, 2026-11-08)

Running commands and terminals shown under their app were planned, reviewed, then
dropped: too many open issues (pins, counts, folding, monorepo roots, selection).

## Plan review (subagent, 2026-11-08) and assessment

Done on the larger plan. What still applies: the tooltip `name · reason` (13) and a
test that the Cluster list keeps its detail. The rest concerned the dropped children.

## Diff review (self, 2026-11-08: the review subagent was refused by the permission check; the user chose a self-review) and assessment

- Fixed: the right text took the warning colour whenever the item had a warning, so in
  the quiet list a detail would have been amber; the colour now comes with the text.
- Fixed: `flex_none` on ⚠ was redundant (`icon` sets it).
- Noted: a folded monorepo root with its own missing tool and a missing tool below it
  shows ⚠ twice (the summary's and its own, last); the demo has no such case.
- Noted: `row-warning` is one selector for every warning row (test-only; the demo has one).
- Checked: other lists draw as before (`quiet_warning` is the Apps list only); the label
  still dims for a warning; rows without a warning keep their label as tooltip; R4.4/R4.5
  (label truncates, ⚠ `flex_none` through `icon`); DESIGN and R4.3/R4.6 still hold.

## Out of scope

Running commands and terminals under their app (dropped, see Decisions).
