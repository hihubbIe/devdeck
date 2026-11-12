# Settings with categories and search

Asked 2026-11-12 (user): "we just dump everything, I want a proper settings navigation".
One step, one commit. Requirements: R9.4, R4.1, R4.4, R5.1, R9.3, R9.10, R10.1, R6.14,
R13.8. No OS code (R11.6).

Decided with the user (2026-11-12): categories Agents (read only), API, Appearance,
Deploys, Files, Machine (read only), Notifications; alphabetical (R4.1); the search looks
through the settings only; Settings opens on the last category shown while the app runs.

## What it looks like

The modal (`modal_frame`, as now): 900 wide, 70 % of the window high, capped at the
window's width less 80 and never resized by what shows (R9.10).

```
┌ Settings ─────────────────────────────────────────────────────── close ┐
│ [ search settings                         ]                            │
│ Agents          │ APPEARANCE                                           │
│ API             │ UI scale     −  100 %  +   90 100 110 125 150       │
│▸Appearance      │              on top of the desktop's own scaling     │
│ Deploys         │ Theme        devdeck-dark  light-x                   │
│ Files           │              themes are TOML files in ~/.config/…    │
│ Machine         │                                                      │
│ Notifications   │                                                      │
└─────────────────┴──────────────────────────────────────────────────────┘
```

- Left: the categories (`devdeck_ui::nav_list`, new: 200 wide, rows of `row_h`, a label
  and an optional count, the selected one's background brighter when the list has the
  keys; truncates, R4.4).
- Right: the selected category's page, scrolling with its scrollbar: rows of a caption
  (160 wide, dim) and a value or control; a dim line under it only where it says
  something the control doesn't.
- The search field on top filters: the list keeps only the categories with a match, each
  with its count; the page shows every matching row under its category's heading. A
  click on a heading, or Enter, shows that category (the search cleared) with the first
  match's row tinted (`t.selection`) until something else is selected. "nothing matches"
  when nothing does. Esc clears a search, then closes.

## The pages

- **Agents** (read only, `machine.toml`): one block per agent, alphabetical by name: its
  name and id, the line a new session runs (`AgentKind::line` with `{id}`), the resumed
  one, idle stop, the facts it reports (the `fields` keys), its usage limits' count. None:
  "no agent: add one to machine.toml ([agents.<id>])".
- **API** (read only): what the API block shows today, unchanged.
- **Appearance**: UI scale (−, value, +, the steps), theme (one toggle each), the themes
  folder and theme problems.
- **Deploys**: short name (field, save, its check), as today.
- **Files**: settings file, config folder, `machine.toml`, state folder, data folder
  (R10.1), read and save errors.
- **Machine** (read only, `machine.toml`): Node folders, PATH additions, the login
  shell's state (asked, or the problem and "devdeck's own environment"), the config
  problems.
- **Notifications**: every service subscription of every workspace (what the palette's
  "service notifications" menu lists today), alphabetical by service then env: its
  name and env (workspace when several), its options as toggle buttons, each label
  colored by its toast's severity (the standing rule), a "go there" icon to its service
  page; then "Turn all off". None: "no service notifications: tick an option in a
  service's notify menu". The palette's action opens Settings on this page; the
  subscriptions menu goes (`subscriptions_menu`, `open_subscriptions`).

## How it's built

- `settings_view.rs` becomes the page: `Category` (the seven, `ALL` alphabetical, `label`),
  `SettingsView { category, search: Entity<TextInput>, highlight, … }`. The category is
  kept in the view, which the main window keeps: it opens where it was.
- A registry of entries (`settings_view::entries()`): `Entry { id, category, label, words,
  draw: fn(&mut SettingsView, &Theme, &mut Context<SettingsView>) -> AnyElement }`. The
  list, the search (`devdeck_core::fuzzy` over label, words and category) and the pages
  all read it; a new setting is one entry.
- `Environment` keeps the `MachineConfig` it loaded (`machine`), for Agents and Machine.
- `SettingsView::show(category)` opens on a category; `ServiceNotifications` calls it.

## Keys (R5.1)

Opening focuses the search: typing searches at once. ↑↓ (in the search or the list) move
through the categories, or the matches' categories while searching. Tab and shift+Tab go
through the search, the list and every control of the page (`ModalFocus`): each control
is a focusable button (`focus` border color only, R9.10); Enter or space uses the
focused one (`settings::Activate`, in the `Settings` context, run on the control whose
handle has the focus). ctrl+, and Esc close (Esc clears a search first).

## Tests

- `devdeck-ui/tests/ui/nav.rs`: a click selects, the count and the selected row's
  background, rows keep their height and width (R9.10), a long label truncates.
- `devdeck-app/tests/app/settings.rs`: the categories in order; opens on the last
  category; the search narrows the list with counts and shows the matching rows, Enter
  shows the category with the row tinted, Esc clears then closes; ↑↓ move categories; Tab
  reaches a theme button and Enter applies it; the modal's bounds stay the same across
  categories; the existing scale, short name and focus tests adapted.
- Agents and Machine pages list the machine's agents and folders; Files names the
  folders.
- `service_notes.rs`: the palette's action opens Settings on Notifications; a toggle
  ticks and unticks an option; "Turn all off" clears them; the go icon reveals the
  service. The menu tests go with the menu.

## Docs

R9.4 (categories, search, the pages), DESIGN's "Settings and scaling" and a 2026-11-12
decision, the notify standing rule's mention of the palette's list.

## Visual check

One snapshot: Settings on Appearance (`settings_open` states exist); a second only if
the search shows something the first doesn't.

## Out of scope

Editing `machine.toml` from devdeck (it stays the user's file). Workspaces and their config
in Settings. Remembering the category across restarts.

## Plan review (subagent, 2026-11-12) and assessment

Amends "Keys" and "How it's built" above.

- 1 Taken: Esc reaches `ModalClose` even from the search (its Cancel propagates), so the
  `ModalClose` handler clears a search, else closes; a `ModalDismiss` handler (a press
  outside) always closes. Tested.
- 2 Taken, simpler: no `Activate` action. GPUI makes Enter or space on a focused clickable
  element a click (`div.rs`, key-up after a key-down with focus unmoved), so each control
  is a focusable element with its `on_click`; nothing is bound on space, so the fields
  type it. Tested with a key-down then key-up.
- 3 Taken: `SettingsView::control(id)` keeps one focus handle per control id and records
  the order the page draws them in; Tab and shift+Tab cycle over the search, the list and
  that order (`ModalFocus`, built when used).
- 4 Taken: ↑↓ are `SettingsUp`/`SettingsDown` in the `SettingsNav` and `SettingsSearch`
  contexts only; a click in the list gives it the keys, so no control of a page that's
  gone keeps them.
- 5 Taken: the content is 900 wide (capped) and exactly 70 % of the window high; the page
  scrolls inside (`flex_1().min_h_0()`). Tested across categories and a search.
- 6 Taken: the list is at most 30 % of the modal's width; rows clip (never wrap). A test
  at 480 wide and 150 %.
- 7 Taken: the view observes the shell, every workspace's cluster (as they come) and the
  environment; `Environment` keeps the loaded `MachineConfig` (`machine`).
- 8 Taken: "go there" reveals through `Shell::reveal`, which closes Settings first
  (`close_dialog_now`) and gives the service view the keys after. Tested.
- 9 Taken: `ServiceNotifications` from a detached window opens Settings in the main
  window, brought forward; in the main window, `show(Notifications)` then open (open
  already: the category changes).
- 10 Taken: `devdeck_ui::toggle_button_colored` keeps its label's color on and hovered;
  the page's focusable buttons carry a border always (transparent, `t.focus` focused).
- 11 Taken: `Focusable` gives the search's handle; opening clears the search and the tint.
- 12 Taken: matching is word prefixes (every word typed starts a word of the entry's
  label, words or category), not `fuzzy`'s subsequences. Searching: the list shows the
  categories with matches, the current one selected if it has any, else the first; ↑↓
  move among them; Enter shows the selected one; with nothing matching ↑↓ and Enter do
  nothing. Entries' words come from a function, so agents and services are found by
  name. The tint ends when the category or the search changes, or Settings closes.
- 13 Taken: `nav.rs` is a module of `tests/ui/main.rs`; the mockup's steps aside, the
  categories are one alphabetical list (R4.1; R4.2's Pinned + All is for the sidebar's
  lists, this is a view's own).

## Diff review (subagent, 2026-11-12) and assessment

- 1 Taken: a control the keys were on that's no longer drawn (its row gone, the page
  changed) gives the keys back to the search, so Esc, Tab and a press outside keep
  working. Tested: "Turn all off" by the keys, then Esc closes.
- 2 Taken: the Notifications controls' focus handles are keyed by workspace, env and
  service, not by row index.
- 3 Taken: Enter in the list shows the selected category (`settings.pick`).
- 4 Taken: the list scrolls when the window is too short for it.
- 5 Taken: the selected row keeps its color under the pointer.
- 6 Taken: the focus ring is `devdeck_ui::focus_ring`; the search no longer finds Agents
  by a hard-coded agent name.
- 7 Partly: tests for an agent found by its name only when configured, ↓ among matches
  then Enter, the tint ending, unticking by a click; the go icon's keys, a heading's
  click and the nav count's color stay untested.
