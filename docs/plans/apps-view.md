# Apps, a full view

Step 3 of "Pulses and full views" (`docs/ROADMAP.md`), in two commits: 3a then 3b. The
user's manual test comes after 3b. What it shows was agreed on the mockup
(`docs/mockups/views.html`, Apps); the app draws it with `devdeck-ui` components and theme
tokens only (R9.3), as devdeck looks today: the mockup says what goes where, not how it
looks.

Requirements: R3.10, R4.1, R4.4, R5.1, R5.6, R6.3–R6.7, R9.3, R9.10, R13.1 (3b).

## The model

- An activity is a **full view** (`Activity::full_view`, Apps for now) or a sidebar list
  beside the current pulse's panes (Pulses, and the others until their turn).
- On a full view the main window shows, right of the activity bar: the activity's list
  (the sidebar entity as it is: Pinned + All, tree, groups, pins, filter, running only,
  ⚠, the saved state; its width the sidebar's, dragged the same way), then the
  **details** of its selection instead of the pulse's panes. The list is always shown
  there: ctrl+b and a click on the active icon do nothing, its menu has no "Hide sidebar".
- The details are views like any tab, built and kept by the deck outside the pulse:
  `Deck::outside`, a stack (the selected app, then what it was drilled into). Same
  registry, saving, titles, focus handles and leak check as tabs. While it's shown the
  main window's panes count as hidden (visibility events, R1.3); detached windows go on.
- **Drill-down**: `ViewCx::open` from a view in the stack opens the new view on top of it
  (`Deck::drill`) instead of in a pane; a header over the details says where you are
  (`entityservice › go:run`), each part goes back to it, a back icon to the one before.
  Selecting another app starts a new stack.
- Opening something in the pulse while a full view shows (the palette, a notification's
  Open, `Shell::reveal`) shows Pulses first, so it never opens out of sight.
- A row's Enter or double click on a full view shows its details (no tab is opened); rows
  aren't dragged to panes there (none are shown).

## 3a: the frame and the app view

- `sidebar.rs`: `Activity::full_view`; the selection followed into `Deck::show_outside`
  (an observer of itself), `Deck::hide_outside` on any other activity; `open` on a full
  view only selects.
- `deck.rs`: `outside: Option<Vec<ViewRef>>`, `show_outside`, `hide_outside`, `outside`,
  `drill`, `back_to`; `sync` keeps the stack's views; `compute_visible` swaps the main
  window's panes for the stack's top.
- `views/mod.rs`: `ViewCx::open` drills first.
- `window_view.rs`: the body on a full view (list, edge, details header, details or "pick
  an app"); the active icon's click and menu.
- `shell.rs`, `palette.rs`: Pulses shown before opening in the pulse.
- The app view (`views/app_view.rs`), also as a tab, in two columns when wide (one when
  narrow, from its drawn width):
  - Header: name, toolchain, env chip (and how it applies), branch; icons for a terminal
    in its folder and the git panel.
  - Commands by tier as today (state, line, ports, run/stop/restart/output), plus how long
    a running one has run.
  - Git: the branch, a two-sided bar of commits behind and ahead of its upstream; icons
    fetch (runs), pull, push, switch, new branch (each opens the git panel on top and asks
    there, as the panel does today: its confirmations stay the only ones).
  - Notes as today.
  - Tools: every tool its commands need, found or missing (what it blocks); no versions
    (devdeck doesn't read them).
- `devdeck-ui`: `usage_bar` (moved from `service_view.rs`, which uses it from there) and
  `split_bar` (ahead/behind), each tested in `tests/ui/`.
- Runner: `Run::started` readable (`running_for`).

## 3b: what it links to

- Deployments (a cluster service): one row per env of the workspace with its deployed tag,
  a pods bar (`devdeck-ui::pods_bar`: one segment per pod, ready / starting / failing, the
  old ones of a rollout grey) and `ready/wanted`; a rocket per env deploying the current
  branch there through today's confirmation (R5.6, opens on Cancel); a pivot to Cluster.
- Pivots, "go there": the target activity's icon with a small corner arrow; deployments to
  Cluster (that service and env selected), terminals in its folder to Terminals, a running
  command's port to Ports. Until a target has its full view, a pivot shows that activity
  with the item selected and its view opened in the pulse, as clicking its row does today.
- Terminals block: the shells in its folder, what runs in them.
- Open in pulse: the Pulse icon with the corner arrow, in the details header (the app, or
  what's drilled into) and in a row's menu; a menu of the pulses (alphabetical, the
  current one checked); the view joins the first pane of that pulse's main window (the
  live one through `Deck::apply`, a saved one in its book); a toast says where.
- The title bar's pulse menu goes (Pulses is ctrl+1).
- API level 7: `ui.get` says which activity the main window shows (`activity`), so "what's
  on screen" stays true; agents still open views in pulses only.

## Not done

- The whole Apps view as a tab in a pulse.
- Changed files in the app view (the full git scan the app view avoids on purpose).
- CPU and memory per run (needs a `devdeck-sys` capability).
- Tool versions.

## Tests

- 3a: a full view shows the list and the selection's details, follows the selection,
  hides the pulse's panes (visibility events), drills and goes back, a new selection
  starts a new stack, Enter opens no tab, ctrl+b does nothing there, the palette and a
  reveal show Pulses first, views of the stack are released when it changes (leak check);
  the app view's new parts (uptime, behind/ahead, tools); the two bars in `tests/ui/`.
- 3b: deployments rows and the deploy confirmation, each pivot, open in pulse (live and
  saved), the title bar without the pulse menu, `ui.get`'s `activity`, the pods bar.
- Snapshots: one per step, the Apps view with an app selected.
- Not OS-specific (R11.6). 3b touches `devdeck-api`: the Mac clippy check runs.
