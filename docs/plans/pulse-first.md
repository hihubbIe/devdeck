# Pulse, the first activity

## Goal

Pulse becomes the first icon of the activity bar and what a workspace lands on: its
sidebar lists the user's pulses (it already does, as the fourth activity). Nothing else
moves yet; the other activities keep their sidebar until each gets a full view of its own
(Apps next, then one by one). Second step of the series after the rename
([pulse-rename.md](pulse-rename.md)).

Requirements: R3.10 (pulses), R5.1 (mouse and keyboard), R4.1 (lists unchanged).

## Decisions taken (veto any before the build)

1. **Order**: Pulse, Apps, Cluster (when configured), Ports, Terminals, Notes.
2. **Shortcuts follow the order**: ctrl+1 Pulse, ctrl+2 Apps, ctrl+3 Cluster, ctrl+4
   Ports; Terminals (ctrl+5) and Notes (ctrl+6) keep theirs. Without a cluster, ctrl+3
   does nothing, as ctrl+2 does today.
3. **Landing**: a workspace with no saved sidebar opens on Pulse; so does one saved on an
   activity it can't show (unknown, or Cluster without a cluster), which fall back to Apps
   today. A saved sidebar keeps its activity: devdeck reopens where the user left it.
4. **Icon**: Lucide's heartbeat (`activity`), from the Lucide set of `gpui-kit-assets`
   0.9.0 already in the build (as `eye`), replacing `panels-top-left`.
5. **Title bar `pulse ▾`** stays: it names the shown pulse and switches from any activity.
   Its fate is settled with the first full view (Apps), when the main area may not show a
   pulse at all.
6. **Clicking the active icon** still hides the sidebar, Pulse included.

## How

- `sidebar.rs`: `Activity::ALL` starts with `Layouts`; the two fallbacks (unknown,
  Cluster without a cluster) go to `Activity::ALL[0]` instead of `Apps`.
- `devdeck-core/src/state.rs`: `SidebarState::default()` saves `"pulses"`.
- `registry.rs`: the four bindings above; `window_view.rs:577`'s comment.
- `devdeck-ui`: `Icon::Layouts` => `"activity"`, the SVG copied into `assets/icons/`
  (doc comment naming its source, as for `Rendered`).
- `snapshot.rs`: its states were made on the Apps list (groups of apps, running dots), so
  it starts its workspaces on Apps, saying why (not rendered in this step).
- Docs: DESIGN (activity bar order and shortcuts, the decision), ROADMAP (a section for
  the series: 1 rename — done, 2 Pulse first, 3 Apps full view, then the others; manual
  test after 3), R6.12 and R12.3 keep their keys.

## Tests

- Update: the activities' order (`sidebar.rs:70,73`), every `ctrl-1..4` keystroke in the
  app tests (`sidebar.rs`, `actions.rs`, `notifications.rs`, `reveal.rs`, `runs.rs`,
  `terminals.rs`, `terminal.rs`), and the assertions that a fresh workspace is on Apps
  (`sidebar.rs:94,200`, `sidebar_groups.rs:352`); tests that relied on the Apps list being
  the default show it first.
- New: a fresh workspace lands on Pulse, ctrl+1 shows it; a saved Cluster sidebar in a
  workspace without a cluster opens on Pulse.
- The tests that read the Apps list without opening it open it first (`apps()` in
  `sidebar.rs` and `sidebar_groups.rs`, two in `runs.rs`). The special case for a saved
  `layouts` id (rename step) goes: an unknown id now lands on pulses anyway, its test stays.
- Snapshots: none (user, 2026-11-11: one or two at most, and none for this step).
- Not OS-specific (R11.6): no `devdeck-sys` change, so no Mac clippy needed beyond
  `devdeck-core` (one string), which it covers.

## Out of scope

- Full views for Apps, Cluster, Ports, Terminals, Notes; "open in a pulse" from them.
- The title bar dropdown's future (decision 5), the activity ids and the action ids.
- Agents activity.
