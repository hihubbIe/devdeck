# Terminals run on their own; tabs are views on them

Asked 2026-11-08. The user: "sometimes a terminal is put in another pane so I have 2
instead of 3 panes … after restarting or changing layout"; then, settled: "a terminal
'runtime' is independent from its tab? It'll keep running, in the terminals list, and we
can stop it or re-attach it there, but the tabs are just a view on it … we decorrelate
the ownership of the terminal from the tabs"; "I'm still not a huge fan of empty panes,
for me we shouldn't have them, just the one when there is nothing at all." Plan only;
replaces the two earlier versions (sharing undone, then a last-instance rule and
empty panes, both dropped). Reviewed (below); built after the user's GO.

## What goes wrong today

Closing a terminal tab ends its shell and removes it from every layout: `Deck::sync`
calls `end_terminal` for a terminal view leaving the live layout outside a switch
(`deck.rs`), and `end_terminal` closes the tab in every saved layout (each losing the
pane it was alone in, `remove_pane_if_empty`). "Save layout as" shares terminals
between layouts (as wanted), so closing one in a layout costs the other layouts their
terminal and its pane. Nothing moves tabs at load; on a switch, only kept views ("Keep
in every layout") are added to the target's focused pane (`LayoutBook::switch`); the
user's books keep none.

## The model

- **A terminal is its shell, on its own.** It's terminal `n` of the workspace
  (`RunKey::terminal(n)`); its tabs (`term/<n>/<dir>`, in any number of layouts) are
  views on it. Closing a tab, closing a detached window, deleting a layout or switching
  never ends the shell.
- **The Terminals list manages terminals**: every terminal running (shown or not) and
  every terminal a layout refers to (not started since a restart), with what it runs
  and where it shows ("here", the layouts' names, or "not shown"). Its menu:
  - **Open** re-attaches it in the current layout (the same shell; a terminal not
    started starts there), at its default place or any other (R6.13, as today);
  - **Stop** ends its shell; its tabs show "shell ended", and ⏎ in one starts it again;
  - **End** stops it and removes its tabs from every layout (their panes go as any
    pane losing its last tab); it leaves the list.
- **Quitting ends every shell**, as today; a terminal shown by a layout starts a fresh
  shell when shown after a restart. A terminal no layout shows is gone after a quit.
- **Panes are unchanged**: a pane goes when its last tab goes; the welcome shows only
  when the window has nothing (its one pane empty).

## How

- `Deck::sync`: a terminal view leaving the live layout is only dropped (view, focus,
  saver), never ended. The `switching` exception for terminals goes (nothing to
  except).
- `Deck::end_terminal(n)` (the list's End only): out of every saved layout first (the
  book saved), then its tab in the live layout closes, then the runner ends it
  (`Runner::end_terminal`: stop, then forget). Nothing else calls it.
- Terminals list (`sidebar.rs`):
  - rows from the runner's terminals plus the terminals of the live layout and the
    book (`ViewAddr::Terminal` refs, one row per number; the live layout's folder wins,
    the book's copy of the current layout skipped as stale);
  - the detail: what runs ("vite", "idle", "ended", "not started") and where it shows;
  - `view_ref` takes the folder from those refs when the runner doesn't know the
    terminal (after a restart; today such a row has no menu);
  - the menu: Open (places, as today), Stop (while its shell runs), End.
- `places`, Share with, `next_terminal`, "keep in every layout": unchanged (re-attaching
  through Open or Share with puts a view of the same shell in the current layout, which
  is what "a view on it" means).
- Docs: R6.12 (closing a tab or a detached window no longer ends the shell; the list's
  Open / Stop / End), DESIGN decision (terminal runtime apart from its tabs; supersedes
  "closing is closing, in every layout"), ROADMAP entry.

## Tests (first; the user's case fails before the fix)

- The user's case: a layout with three panes (app, run, terminal) saved as a copy;
  closing the terminal's tab in the copy leaves the first layout its three panes and
  the terminal its running shell (the same one: `runs == 1`).
- Closing a terminal's last tab: the shell runs on, listed "not shown"; Open
  re-attaches the same shell in the focused pane.
- Closing a detached window holding a terminal: the shell runs on (rewrites
  `closing_a_detached_window_ends_its_terminals`).
- Deleting a layout holding a terminal: the shell runs on.
- Stop from the list: "shell ended" in its tabs, still listed; ⏎ starts it again.
- End from the list: its tabs leave every layout, the shell ends, the row goes.
- After a restart: a terminal only a hidden layout shows is listed "not started" with
  a menu; Open starts it in the current layout.
- Rewritten: terminals.rs 2–3 (module doc), 114–118 ("closing its tab ends it"),
  124 (detached window), 182–198 ("closing is closing, in every layout").
- Snapshot: the Terminals list with a running terminal not shown and one not started.

## Not in this step

Found by the review: a tab's saved state (follow, selection, filters) is lost when its
layout is hidden, and after a restart for hidden layouts (`view_states` keeps only live
views). Unrelated to terminals; its own step.

## Plan review (subagent, 2026-11-08) and assessment

Diagnosis confirmed. Taken:

1. **Ended terminals with no tab would pile up** (open, `exit`, close: today that
   forgets it; under the plan an "ended · not shown" row would stay, holding up to ~5 MB
   of buffer). Fix: a terminal whose shell isn't running is forgotten when its last view
   goes (no tab in the live layout or a saved one, the current's stale copy excluded),
   and a shell that exits while no tab shows it is forgotten too. **User to confirm.**
2. **Closing the tab of a busy terminal keeps its process, and its port** (`port_scan`
   then refuses an app start: "port held by terminal · x"; "Stop holder" ends the whole
   shell). Said in R6.12 and tested. Closing the last tab of a terminal running a
   program (`terminal_program` is some) shows a toast naming it, with **Stop**.
   **User to confirm.**
3. **One source for a terminal's ref**: a helper giving a terminal's ref by number,
   preferring a layout's exact ref, the runner's folder only as a fallback. The list's
   rows, `view_ref`, `open` (no more falling through to `switch_layout` on `None`),
   Share with and End use it; End closes every live ref with that number.
4. **End while the shell hangs up**: Open and Stop are hidden for a terminal that is
   ending (`Runner::is_ending`, new); End forgets it in the runner at once rather than
   when its exit arrives, so an Open in between can't build a dead tab.
5. **Stop doesn't survive a restart**: a stopped terminal a layout shows starts a fresh
   shell when shown after a restart (terminals always restore running). Stated and
   tested.
6. **Plan facts corrected**: a switch adds kept views (above); there is no DESIGN entry
   "closing is closing, in every layout" (only a test): a new DESIGN decision is added,
   and R6.12's "Closing its tab … ends the shell" and "open or end it there" change.
7. **Tests added**: closing the tab of an ended shell (forgotten); End while Stopping;
   End on a terminal only a saved layout holds, not started (the book still purged);
   Stop hidden when the shell isn't running; Open at another place for a terminal not
   started; Share with a hidden terminal re-attaching it here; the current's stale copy
   not making a row after a restart (closed in the live layout, restart without a
   switch); the restart tests through a real `StateStore` round trip.
8. **End stays**: with (1), Stop plus closing the tabs already makes a terminal go; End
   remains the one action that strips its tabs from hidden layouts too. **User to
   confirm** (or drop End).

## Decisions after the review (user, 2026-11-08)

- (1) Taken: a terminal whose shell isn't running is forgotten once no tab shows it.
- (2) No toast: the user runs dev servers from the app, where a run shows as running
  whatever happens to tabs.
- (8) **No End.** Closing a shell never changes a layout. The Terminals list's menu is
  Open and Stop (Stop while its shell runs). A running terminal no tab shows goes when
  stopped; a stopped one still shown stays listed "ended" until its last tab closes; one
  not started since a restart, only in a hidden layout, stays listed (Open) until its
  tabs close.
- No test reproducing the user's case (it fits the diagnosis); the behaviour tests stay.

So in "How": `Deck::end_terminal` and the `ending` set go; the deck tells the runner
which terminals its layouts show (`Runner::set_shown_terminals`, after every sync and
book change), and the runner forgets a terminal that isn't running and isn't shown
(at once, or when its shell exits).


## Diff review (2026-11-08)

The review subagent couldn't be started (refused by the session's permission check);
reviewed by the lead instead. Checked: views are built before the deck reports shown
terminals (sync), a switch counts the layout left (stored before `current` changes),
save-as and delete report through `save_book`, each workspace's deck reports to its own
runner, Stop is offered only while the shell runs, Open goes through the layouts' own
ref. Fixed: a terminal restarting (■ then ▶ as one restart) with no tab showing it was
forgotten instead of restarted (`exited` now checks the restart first).
