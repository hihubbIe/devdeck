# Layouts become pulses (the rename)

## Goal

The user's name for a saved arrangement of panes and tabs is **pulse** (user, 2026-11-11).
Everything a user or an agent sees says pulse; the code keeps its names. A mechanical step:
no behaviour changes. It is the first of a series: pulses become the first activity, and
each other activity gets a full view of its own (Apps next).

Requirements: R3.10 (the name), R13.2 (the API may change with its consumers), R2.1,
R5.3, R5.6, R6.12, R10.1 (wording).

## What changes

- API (level 6): `layout.list|get|switch|save_as|rename` become `pulse.*` (each `since 6`),
  the `layout` topic `pulse`, every `layout` field `pulse` (`Place`, `Ui`, the pane, tab,
  window, view, file and terminal params), the types `Pulses`, `Pulse`, `PulseSummary`
  (their names show in the schemas), the `layouts` list of `sidebar.pin` and `group.*`
  `pulses`, and the API's messages. `API_LEVEL`'s comment lists it.
- Consumers: `devdeck-ctl` ("shown: pulse …"), `devdeck-mcp` (the Present and Organize
  descriptions).
- App: the activity's name and id, the title bar button's tooltip, the palette (entries,
  placeholder), the action labels and group, the pulse menu and its prompts, the delete
  reason, the `BookError` messages, the startup and save problems, the gallery and snapshot
  texts.
- A sidebar saved on the `layouts` activity opens on Pulses (and saves `pulses`).
- Docs: REQUIREMENTS (R3.10, R13.2 rewritten, wording), DESIGN (current sections, one line
  mapping pulse to `Layout`, the decision), BUILDING (what the state folder holds).

## Out of scope

- Code names: `Layout`, `LayoutBook`, `Deck`, `Activity::Layouts`, action ids
  (`layout.new`, `sidebar.layouts`), element ids, snapshot names, the pin storage key.
- State files `layout.json` and `layouts.json` (a migration for nothing the user sees).
- Moving pulses to the first activity, the title bar's dropdown, the shortcuts order.
- Dated entries of the decisions log, ROADMAP and older plans: history, as written.
- Not OS-specific (R11.6): no `devdeck-sys` change.

## Tests

- `the_catalog_says_pulse_never_layout`: no method name, summary or schema says layout.
- `a_sidebar_saved_on_layouts_opens_on_pulses`.
- The existing API, palette, book and ctl tests, on the new names.
- Snapshots `B0-`, `B1-`, `B2-` (the pulse menu, the list, the save error).

## Review

Inline, not a subagent (user's standing rule for small work: no agents unless asked).
Checked: every string literal with "layout" in `crates/*/src` is either renamed or internal
(file names, element ids, storage keys, test names); the API's doc comments are schema
descriptions, so they were renamed too.
