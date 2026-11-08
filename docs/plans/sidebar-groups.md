# Sidebar: folded sections, groups, width; layout ids

Asked 2026-11-08 between M3 steps 3b and 4. One step, one commit.

## What

- **R4.7** Pinned and All fold from their header (click or chevron; the header keeps
  its count). Remembered per workspace and list. A filter or "running only" shows the
  matches of a folded section and leaves what's folded as it was. Headers stay out of
  the keyboard selection (Down from the filter goes to the first row, as now).
- **R4.8** Groups: folders the user makes in All, nested, folded like folders,
  remembered. Every list but Terminals (their numbers come and go, as for pins);
  Cluster groups are shared by every env, as its pins are. Groups first at each level,
  alphabetical. Made from the All header's menu ("New group…") or a group's ("New group
  inside…"); renamed; deleted (what it holds moves to its parent, subgroups of the same
  name merge: nothing is lost). An item, or a folder of the Apps tree with all it holds,
  moves by a drop on a group (on the All header: back to the top), or with "Move to
  group ▸" (every group by its full path, and "Top level"; the item itself: "New group
  and move here…"). A pinned item keeps its group and goes back to it when unpinned; a
  pinned item dropped on a group is unpinned into it. A deployment missing from the
  env keeps its group. Empty groups show, except while filtering. Layouts rows move
  by menu only (they have no drag); Ports rows drag like the others (built: nothing in
  the generic drop needed to refuse them).
- **R4.9** The sidebar's right edge drags its width (the pane dividers' look); a
  double click goes back to the theme's. Kept between 200 px and half the window when
  drawn, without overwriting what's saved. One width for the app, in `app.json`.
- **Layout ids.** A layout is known by an id, its name is a label: renaming keeps its
  pin and group. The first layout's id is `default`, new ones a UUID. Books saved by
  name load with ids; layout pins saved by name move to the ids.

## How

### Core
- `state.rs`: `SidebarState.collapsed` (list id → `pinned`/`all`) and `.groups` (list
  id → `list::Groups`); `AppState.sidebar_w`. `serde(default)`: old files load.
- `list.rs`: `Groups { folders, members }` and its operations (create, rename, delete,
  move a group, assign a node), names trimmed, non-empty, without `/`, unique among
  siblings ignoring case. `rows` takes a `Shape` (open rows, groups, folded sections).
  `Row::Header { section, open, count }` (the count is computed there: a folded
  section has no rows to count); `Row::Folder` gains `group: bool`. A group's row path
  is tagged so it can't meet a real path; it opens through the same `open` set. A
  node moved into a group keeps its real path for opening, selection and tooltip;
  only where it shows changes; the deepest assigned node wins. A member whose group is
  gone shows at the top; one whose node is gone is ignored.
- `layout_book.rs`: `LayoutBook { current: id, layouts: id → Named { name, layout } }`;
  switch, rename, delete by id; `save_as` returns the new id; loading the old shape
  converts it.

### App
- `sidebar.rs`: header fold; group rows (icon, drop target, menu); one drop handler per
  row deciding pin, unpin, group, top level (two `on_drop` of one type on an element
  panic in GPUI); folder and group rows draggable, a group refused into itself
  (`can_drop`); menus as above. The root fills the width it's given.
- A generic name prompt (from the layouts' modal), used by layouts and groups.
- `window_view.rs`: the divider between sidebar and panes (replaces the sidebar's
  border), `on_drag_move` on the body row, double click (`on_mouse_up`,
  `click_count == 2`) resets; `body_size` uses the drawn width plus the divider. The
  live width is the shell's, saved with `main_size` (flush, workspace switch).
- Layouts list, palette, layout menu, title: by id, showing names.

### Docs
R4.3 and DESIGN's "no grouping" decision amended (groups the user makes are allowed);
R4.7–R4.9; DESIGN sidebar; ROADMAP.

### Tests
Core: folding, groups first, nesting, moved subtrees keep their paths, two `apps`
folders in one group stay apart, missing group → top, counts with folded sections and
groups, operations and refusals; layout book by id, old book converted. App: fold
remembered across a restart, create/rename/delete through the prompt, drop on a
group, menu move, unpin into a group, filtering, width drag, reset and restore, pane
neighbours with a wide sidebar, a renamed layout stays pinned. Snapshots: groups,
folded sections, wide sidebar.

## Plan review (subagent, 2026-11-08) and assessment

All 11 findings accepted and folded in above: real paths kept for moved nodes (1),
counts carried by headers (2), one drop handler per row (3), drag sources for folders
and groups, Layouts menu-only (4), headers out of the keyboard selection (5), groups
as tagged folder rows using `open` (6), identity rules (7), width saved like
`main_size`, double click on mouse up, sidebar root filling its slot, the divider in
`body_size` (8), menu defaults and flat group paths, the snapshot tool's modal
downcast (9), Layouts/Ports menu-only stated (10), R4.3 and DESIGN amended (11).
Layout names as ids, found on the way, are replaced by ids (user, 2026-11-08).

## Diff review (subagent, 2026-11-08) and assessment

1. A book converted from names got new random ids on every load, and pins moved to
   them were lost on the next start. **Fixed**: `load_book` writes a converted or
   repaired book back at once; a test loads it twice.
2. Drops and the move menu looked at a node's own group, not the one it shows in
   (something inside a folder put in a group jumped out of its folder on a drop beside
   it, and couldn't go back to the top). **Fixed**: `Groups::shown_in` (the deepest
   node put somewhere, the rule the tree uses), and a `TOP` mark for a node taken back
   to the top while what holds it is in a group (ignored once that's no longer so).
3. Deleting a group could leave sibling groups differing only in case. **Fixed**: they
   merge; names compare ignoring case beyond ASCII.
4. The selection was lost on a group inside one renamed, moved or deleted. **Fixed**:
   it follows the group.
5. Tests: the wide-sidebar test didn't depend on the width. **Replaced** by one
   checking the pane area against what's drawn (it fails with the old width); the menu
   move is clicked; the conversion test loads twice.
6. A pinned item dropped on the All header went back to its group. **Decided**: the
   header is the top (`Target::AllHeader`); on a row at the top it still goes back to
   its group. R4.8 and DESIGN say so.
7. Minor: a dead branch removed. Accepted: refusals return no effect instead of
   `can_drop` (no highlight, the drop does nothing); a deleted layout's id stays in
   pins and groups (never shown, ids aren't reused).

Found while building: a single-line input typed a space for an Enter nothing took (so a
prompt's error vanished at once). **Fixed** in `TextInput`, with a test.
