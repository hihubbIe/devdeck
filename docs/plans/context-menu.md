# Right-click menu, Share with; short terminal names

Asked 2026-11-08 between M3 steps 3c and 4. One step, one commit.

## What

R5.7: a right-click menu built generically: a view gives its own items and what was
right-clicked; actions add themselves where they apply. First: a terminal's selection
(run output or shell) offers Copy and Share with, which pastes it as it is into another
terminal of the workspace and shows that terminal. Terminals are named by their
folder's last part everywhere.

## How

- `context_menu.rs`: `Content::Text`, `Origin` (deck, services, window, source
  terminal), `ACTIONS = [copy, share_with]`, `items`/`open` (no menu when empty).
- Share with: the workspace's running terminals but the source, by number, short names
  (with the number when two share one); refused "doesn't take pastes safely" without
  bracketed paste; control characters stripped but line breaks and tabs; no Enter;
  the target shown (`places::show`: its tab in front, focused, a detached window
  raised). Its own click: the last target, else the first that takes pastes; disabled
  with why when none.
- `run_view`: right-click in the output; the menu key (`pane.menu`) at the selection's
  end, propagating to the tab menu without a selection.
- `places::showing` / `places::show` shared with the sidebar's Terminals "Open".
- `views::terminal_name` for tabs, the runner's titles and the Terminals list (the
  full folder on hover).

## Decisions (user, 2026-11-08)

Raw text, no header. Label "Share with" (not tied to terminals; "Send to…" is the tab
menu's). No Claude Code detection: every terminal by name.

## Plan review (subagent, 2026-11-08) and assessment

All taken: only bracketed-paste targets get text, others refused with why (1);
control characters stripped (2); no "▸" in the label, the overlay draws it (3); the
name clash avoided (4); the menu key handled on the run view root, propagating without
a selection (5); no empty menu (6); `Origin` defined (7); one show helper, the target
found by number, a detached window raised (8); the own click never lands in a terminal
that can't take it (9); tests with a per-run recording driver feeding `ESC[?2004h` (11);
R5.7, DESIGN, a snapshot (12); the fence question gone with raw text (13). Claude Code
detection (10) dropped by the user.

## Diff review (subagent, 2026-11-08) and assessment

All fixed, with tests: a target in the main window, shared to from a detached one, is
raised and focused (1); the menu key with a selection scrolled away opens at the
output's top left, not the tab menu (2); the "where text went last" test now sends from
the list first (3); the tab menu asserted by its own item (4); with every terminal
refusing, Share with stays open so its list says why, and its click sends nothing (5);
a maximized other pane is restored so the target is drawn (6); a doc comment put back
(7); the Terminals list's hover shows `~/…` (8).
