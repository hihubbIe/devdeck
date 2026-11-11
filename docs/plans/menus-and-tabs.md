# Tabs, agents by project, one menu shape (2026-11-11)

The user: tabs need a minimum size, then the strip scrolls; selecting some tabs moves the
strip; Claude Code sessions scoped per project and startable from more places; assess
every pivot and context menu, keep what's needed, a few generic ones everywhere,
consistent across every place.

Requirements: R3.12, R3.14, R4.1–R4.3, R4.10, R5.1–R5.6, R6.13, R6.14, R9.3, R9.10, R9.11.
No OS-specific code (R11.6).

## 1. Tabs move when one is selected

Cause: the tab strip's slot for the active tab's view actions (`window_view.rs`,
`strip_tools_w`) is `Toolbar::embedded_width(active view's action count)`. A run has 4
icons, a note or file 1: selecting a run widens the slot, the tabs (flex_shrink 1) get
narrower. (Before the one-header change the same actions sat in the pane buttons, same
effect.) Selecting only changes a tab's background and text color; no border.

Fix: the slot is as wide as the most actions any tab of that pane has (stable while
switching). Test: switching tabs keeps every tab's bounds.

## 2. Tab size and overflow

Today: tabs shrink to `tab_min_w` (60 px scaled: about the icon and ×), the strip clips.

Fix: `tab_min_w` about 120 px scaled (icon, about 10 characters, ×). Tabs keep their
natural width up to `tab_max_w`, shrink to the minimum, then the strip scrolls sideways:
the wheel scrolls it (vertical wheel moves it sideways), a selected tab is scrolled into
view, a thin horizontal scrollbar over its bottom edge (a horizontal variant of
`devdeck_ui::scrollbar`, R9.10: nothing moves, same thickness hovered or not). Tests: at
a narrow width every tab is at least the minimum and the strip scrolls; selecting a tab
off screen brings it into view.

## 3. Agent sessions by project, started from more places

- A session's project: the workspace app whose folder contains the session's folder
  (the deepest), else its folder (shown like a terminal's).
- The Agents list is a tree like Notes: project rows (named by their app), then their
  sessions, alphabetical (R4.1); Pinned + All kept (R4.2).
- The app view gets an Agents panel: its sessions with their state, "<agent> here"
  (each configured agent), each row going to it in Agents.
- "new session" in the Agents view opens the picker (`devdeck_ui::picker`) of the
  workspace's apps (and its folders), then the agent if several.
- The overview's sessions table is grouped by project.

## 4. Context menus and pivots

### Today

| Where | Menu |
|---|---|
| Tab | view actions · Close · Close others · Send to… · Detach to a window · Maximize · Collapse · Open terminal here · agents |
| Pane | Split right/down · Collapse · Maximize · Send pane to… · Open terminal here · agents · Close pane |
| Apps row | Pin · Move to group · Open in pulse · Open terminal here · agents |
| Cluster / Ports row | Pin · Move to group · Open in pulse |
| Terminals row | Stop · Open in pulse (DESIGN says Stop, Rename, Delete, Pin) |
| Notes row | Pin · Rename… · Delete… · Open in pulse |
| Agents row | Pin · Rename… · Stop · Remove · Open in pulse |
| Pulses row | Switch to it · Rename… · Delete · Move to group |
| Group row | Move to group · New group inside… · Rename… · Delete |
| Activity icon | Show X · Hide sidebar |
| Terminal / run body | Copy · Paste · Share with › |
| Matrix cell, service pods | Open service · Highlight · Open pod · Logs |

Pivots ("go there"): app → Cluster, Terminals, Ports; port → Apps, Terminals, Cluster;
service → Apps, Cluster; nothing to or from Agents.

### The shape, everywhere

The same groups in the same order, separated; an item only when it applies (no greyed
filler):

1. **Its own verbs**: the view's actions (run or stop, restart…), Rename…, Delete…
   (always "…"; a confirmation when hard to undo, R5.6).
2. **Where to see it**: Open in pulse ›, then "Go to" its related things (its app, its
   terminals, its sessions, its deployments, its ports), each the activity's icon with a
   corner arrow (the existing `go_action`).
3. **Start here**: Open terminal here ›, <agent> here › (with places, R6.13), on
   anything with a folder: tab, pane, and app, terminal, port, service, note, agent
   session and file rows.
4. **Copy**: Copy name, Copy path (or URL).
5. **List only**: Pin / Unpin, Move to group.

Removed: "Send to…", "Detach to a window", "Send pane to…" in menus, for one "Move to ›"
with the places (numbered panes, new window); the Send-to pick stays on its key
(R3.11). The activity icon's menu in full views (it repeated a click). "Remove" on an
agent session becomes "Delete…" (the agent keeps the conversation).

Added: Terminals rows Rename…, Delete…, Pin (as DESIGN says); app rows run/stop of their
main command; pivots app ↔ Agents, session → its app, terminal → its app.

One builder: menus are built from what an item is (its view, folder, app, run, list),
so every place gets the same items for the same thing (`menus.rs`, new).

## Tests

Tabs stable across switching; minimum width and scrolling; agents tree by project; app
view's Agents panel; the picker for new sessions; each surface's menu has the groups in
order and the generic items where they apply (a table-driven test over surfaces).

Snapshot: none (the user checks by hand).

## Review (fresh subagent, 2026-11-11) and assessment

1. Accepted: a slot sized from current counts still moves (a run's count changes with
   its state; no slot when the active view has none). The slot is
   `embedded_width(MAX_HEADER_ACTIONS)` (4, the most a view offers, R9.11) whenever any
   tab of the pane is a view with actions, empty otherwise: it changes only when tabs
   come or go. Test: a background run starting keeps every tab's bounds.
2. Accepted: scroll a tab into view only when the active tab changes from elsewhere (a
   key, a reveal, an open), never on its own click (R9.10); a handle per pane on
   `WindowView`, scrolled only on a change, so a wheel scroll isn't undone.
3. Accepted: GPUI already turns the vertical wheel sideways; `tabs_min_w` rises to the
   tab minimum. The horizontal scrollbar: a variant of the shared one, thin, its press
   stopped from reaching the tab under it. Auto-scroll while dragging a tab: later.
   The existing bar's hover growth vs DESIGN: out of scope, noted.
4. Accepted: "Send to…", "Detach to a window", "Send pane to…" stay (R3.11, R6.13,
   tests). No "Move to ›".
5. Accepted: tabs and panes keep their layout group as today.
6. Accepted: "…" only where a confirmation follows. An agent session keeps "Remove"
   (R6.14). Terminal Rename/Delete: not built (no model or API for it); DESIGN.md's line
   saying so is corrected instead.
7. Accepted: the app view's command rows offer only what applies (run, or stop and
   restart; output). The full view's details header gets its item's right-click menu.
   The other menus listed are fine as they are.
8. Accepted: the activity icon keeps its menu (R5.2).
9. Accepted: no Copy group; no "Go to" in menus (pivots stay in views and headers);
   "start here" only where there's a real folder: tab, pane, app, terminal, agent
   session and project rows.
10. Rejected for now: no new menu builder module this step (the menus change little
    once 4 and 9 are taken); the items added use the registry where they're actions.
11. Accepted: a session stores its project (an app id) when made, found by path
    components (`Path::starts_with`, the deepest app); older sessions get it on load.
    An app gone from the catalog: its project row stays, dim, "project not found" (as
    notes). Folder nodes (no app) are keyed by their full path. Inside a project a
    session without a name is "<agent> #<n>". Project rows' menu: "<agent> here".
    R4.8 names Agents among the lists without groups.
12. Accepted: a wheel scroll survives the next render.

## Built (2026-11-11)

As assessed above. Tests: `tab_strip::` (no tab moves on selection, a click, a run
starting; minimum width, scrolling, a tab made active from elsewhere in view, a wheel
scroll kept), `agents::sessions_are_listed_by_project…`, `agents::an_apps_page_lists…`
(and the details header's menu), `agents::the_new_session_button_picks_a_project…`,
`runs::a_command_rows_menu_offers_what_applies`, core `project_of`.
