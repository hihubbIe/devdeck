# Open in, dialogs as modals, folded secondary commands, no "…"

Asked 2026-11-12 (user). One step, one commit. Requirements: R5.1, R5.5, R6.3, R6.13,
R6.14, R9.3, R9.4, R9.10, R4.1. No OS code (R11.6): nothing in `devdeck-sys`.

## 1. Secondary commands fold (R6.3)

The app view's Commands panel folds Secondary as it folds Hidden: its heading carries the
open/closed icon, a click or the new `app_view.secondary` action (key `e`, "secondary
commands"; `s` is stop) toggles it. Closed by default, like Hidden. Primary never folds.
`AppView::show_secondary` sits beside `show_hidden`, saved in the view's state
(`"show_secondary"`, false when missing). `visible_commands` (what ↑↓ go through) leaves
out a folded tier's commands. The snapshot tool opens both.

Tests (`tests/app/apps_view.rs`): Secondary closed on a fresh view, its commands not in
the selectable list; `e` opens it; the state round-trips.

## 2. "Open in": one modal, no submenus (R6.13, R6.14, R5.1)

### The component: `devdeck_ui::chooser`

A modal of two columns, drawn in `modal_frame` (the backdrop, a press outside closes it):
the search, the picker and the help use the same frame. WHAT on the left, WHERE on the
right; each WHAT row has its own WHERE list (`Choice { what: ChoiceRow, wheres:
Vec<ChoiceRow> }`, `ChoiceRow { value, label, detail }`). Review points below amend this
section.

- Opens with the first WHAT row and its first WHERE row that isn't refused selected, the
  keys in it (its own focus handle, key context `Modal Chooser`).
- ↑↓ move in the column that has the keys; ←→ and Tab / shift+Tab (`ModalNext`,
  `ModalPrev`) go to the other column. A WHAT change keeps the WHERE row of the same value
  when the new list has it, else its first allowed one.
- A click selects a row (its column gets the keys); a double click on a WHERE row
  confirms. Enter or the Open button confirms; Cancel, Esc or a press outside close.
- A refused row is dimmed with its reason as detail (R5.5); it can't be confirmed (Open
  disabled while it's selected).
- Confirming closes first, then calls `on_choose(what, where)`; focus goes back where it
  was unless the caller moves it.
- Its columns are fixed widths (220 / 380 before scale), rows truncate (R4.4), the
  selection is a background color only (R9.10). Lists scroll with a scrollbar past
  12 rows.

Actions `ChooserUp/Down/Left/Right/Confirm` in `devdeck_ui::chooser`, bound in the
registry (`Chooser` context: up, down, left, right, enter).

Tests (`crates/devdeck-ui/tests/ui/chooser.rs`, a module of `tests/ui/main.rs`): opens
with the keys and the first rows; ↑↓ ←→ Tab; WHERE follows WHAT and keeps the same
value; click, double click; Enter gives (what, where); a refused row can't be confirmed
(Enter and Open do nothing); Esc closes and focus comes back; the backdrop is drawn.

### The app's use: `crate::open_in`

`open_in::open(deck, subject, from, window, cx)`. A subject is what the menu was opened on:
the view itself (`Option<ViewRef>`, with its title) and its own folder
(`Option<PathBuf>`: an app's, a terminal's, an agent session's; `Deck::own_folder`, new,
`None` for notes, services, ports). `from` is `Outside` (a full view: a list row, the
details header, a project row) or `Pane(pane, window)` (a tab's or a pane's menu).

WHAT, in this order: the view itself (outside only; inside, Send to moves it), then, with
a folder, Terminal and each agent of the machine (`environment::agent_kinds`, its order).

WHERE:
- Terminal: "Terminals" first (its own place, the user's "brings to the app"); an agent:
  "Agents". The view itself has none (its full view shows it already).
- Outside: every pulse, alphabetical (`book.entries`), the current one's detail "current".
- Inside a pulse: its panes as Send to names them (`places::panes`, "1 · main window ·
  run web") then "new window"; the pane the menu came from first selected.

What a choice does:
- view + pulse: `Deck::open_in_pulse`, a toast says where (as today).
- Terminal + Terminals: `Runner::open_terminal(next, dir)` makes it known, then the
  sidebar shows it in Terminals (its full view starts it).
- Agent + Agents: `sidebar::new_agent_session` (as today).
- Terminal or agent + pulse: the new terminal or session is made, then
  `Deck::open_in_pulse` with its view; a toast says where. A saved pulse keeps it until
  shown (the Terminals list says "in <pulse>").
- Terminal or agent + pane or new window: `Deck::open_terminal` / `open_agent` at that
  place, keys to it when it's in the menu's window (as `places::item_or` does now).

Every caller shows one item, "Open in", that opens the modal:
- Row menus of Apps (an app, and a folder row: the view itself only), Terminals, Agents,
  Notes, Cluster, Ports, and the Agents and Apps project rows (folder only).
- The details header's go icon (tooltip "open in").
- The tab menu and the pane menu (inside a pulse).

Removed: `places::item`, `item_or`, `agent_items`, `refusal`; `Sidebar::start_here`,
`start_here_in`, `agents_project_menu`'s list; `window_view::open_terminal_items`;
`layouts::open_in_pulse` (its toast moves to `open_in`). `places::open`, `show`, `panes`
stay.

Tests (`tests/app/open_in.rs`, new module): the WHAT and WHERE of an app row, a note row,
a tab menu; Terminal + Terminals selects the new terminal there; Terminal + a saved pulse
puts it in that pulse's first pane; view + pulse (the existing `open_in_pulse` test
stays); Terminal + a pane opens it there; agent + Agents. Menu assertions updated
(`terminals.rs`, `notes_sidebar.rs`, `agents.rs`, `sidebar*.rs`).

## 3. Settings and notifications are modals (R9.4)

Both open with `push_modal` in `modal_frame`, like the help: the main window's panes stay
drawn, dimmed by the backdrop. Settings: 720 wide; the center: 760 wide, 70 % of the
window's height (its list needs a fixed height). Esc, the close button, a press outside,
or their key again close them; focus goes back where it was.

- `app_settings::Cover` becomes `Dialog` (`None`, `Settings`, `Notifications`): which one
  is open, main window only, one at a time (opening one closes the other).
- Gone: `Deck::covered` / `set_covered` (the views behind stay on screen), the body
  swap in `WindowView::render`, `is_covered` guards that only existed because the panes
  were replaced (each one checked: under a modal, app keys don't fire anyway).
- A notification's action used in the center closes the center first, then runs
  (deferred, outside the window's update): `Shell::reveal` refuses while a modal shows.
- The bell shows in the main window only (user: not in a detached one); toasts there
  still open the center in the main window.
- The settings button loses its check (it can't be seen under the backdrop) and the room
  kept for it.

Tests: `settings.rs`, `notifications.rs`, `reveal.rs`, `deck.rs` follow (`dialog` instead
of `cover`; a modal is open; the views behind stay visible; an Open from the center
reveals); a detached window draws no bell. `stress.rs` and the snapshot tool use the same
calls.

## 4. No "…" on action labels

Menu items, buttons, tooltips and the registry's labels lose the trailing "…" ("Rename",
"Delete", "New note", "Send to", "Deploy", "Unlock", "Stop holder", "new pulse", "save
pulse as", "new arguments"); the Send to title reads "Send web to". Status text ("reading…",
"stopping…") and truncation keep it. DESIGN.md's visual language gets the rule.

## Docs

REQUIREMENTS R9.4 (a modal, not a full-window view), R6.3 (secondary and hidden fold,
closed). DESIGN: a 2026-11-12 decision (Open in, the dialogs, the bell, the folding),
the "…" rule, the anatomy line's bell. `places.rs`'s header.

## Visual check

One snapshot: the Open in modal over the Apps view, replacing the tab menu's submenu
snapshot (`snapshot.rs`, "Open terminal here" with its places).

## Plan review (subagent, 2026-11-12) and assessment

- 1 Taken: inside a pulse the folder falls back to `Deck::terminal_folder` (as today),
  so a tab or pane menu always offers Terminal and the agents; "Open in" is left out of a
  menu whose WHAT would be empty.
- 2 Taken (R9.10): both columns are 12 rows high whatever the lists; a test checks the
  panel's bounds across WHAT changes.
- 3 Taken: `Shell::reveal` closes an open dialog (Settings, the center) before its modal
  check, without a `Window` (`overlay::remove_modal` by entity on the main window's
  layer); so the center's actions, toasts' and the API's `view.open` reveal as before.
  The center's actions need nothing else.
- 4 Taken: contexts `Modal Settings` / `Modal Notifications`, both handle `ModalClose`
  (Esc, a press outside); `app.close_settings` takes ctrl-, and `app.close_notifications`
  ctrl-shift-n in their contexts; Settings cycles Tab through its controls with
  `ModalFocus`.
- 5 Taken: `follow_shell`'s guards stay, on "a dialog is open".
- 6 Taken: Settings has a maximum height and scrolls (`with_scrollbar`); its own
  background and padding go (the frame has them).
- 7 Taken: folding a tier clears a selection inside it; a restored selection in a folded
  tier opens that tier. Tested.
- 8 Taken: R6.13, R6.14 and R9.10 reworded; DESIGN's lines on "…", the bell and the
  full-window Settings updated.
- 9 Taken: inside a pulse WHERE is Terminals/Agents first, then the panes and new window.
  Terminals or Agents from a detached window shows them in the main window, raised
  (deferred, as `places::show`); a main-window pane picked while it shows a full view
  shows Pulses (as the old "Open terminal here" did).
- 10 Taken: every label listed; the "args…" placeholder stays (not an action's label).
- 11 Taken: Terminal + Terminals makes the terminal known and selects it in the same
  update; tested across a sync.
- 12 Taken: a dialog is closed by its own entity (`remove_modal`), never by popping the
  top.
- 13 Taken: nothing is refused today, so `ChoiceRow` has no `refused` (no dead state).
- 14 Taken: the column with the keys shows the selection color, the other a dimmer one;
  the WHERE column shrinks (`min_w_0`) in a narrow window.
- 15 Partly: tests for keyboard-only use (menu then arrows and Enter), no agents, and
  ctrl-shift-n in a detached window opening the center in the main window. The chooser in
  a detached window is the layer's existing per-window behaviour, not retested.
- 16 Taken (see Visual check).

## Out of scope

Send to keeps its pick without a backdrop (its panes are click targets, 2026-11-07).
"Move to group ▸" and the notify dropdown keep their menus. The app view's "+" (new agent
session in its folder) stays.

## Diff review (subagent, 2026-11-12) and assessment

- 1 Taken: the Agents view's empty hint pointed to "<agent> here"; it says "Open in".
- 2 Taken: Settings (720, as planned) and the center are capped at the window's width less
  80, as the chooser is, so a narrow window keeps them on screen and truncating (R4.4).
- 3–7 Left for the user (optional): a failed reveal still closes the dialog; the keys
  after choosing Terminals or Agents from a tab stay on an undrawn pane; an Agents project
  row's chooser starts on the current pulse, not Terminals; test holes (Cancel, closing
  the dialogs by a press outside or their key, the toast, a tab with agents); stale
  wording in four comments and the window diagram's bell.
