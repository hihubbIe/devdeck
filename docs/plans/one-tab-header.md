# A one-tab pane has a header, not a tab strip

Asked 2026-11-08 (user). One step, one commit.

## What

A pane with one tab (or none) shows a header instead of a tab list: it names what the
pane shows and holds the pane's controls. With two tabs or more it shows the tab strip
as today. Covers R3.12 (new), and touches R3.4, R3.6.

## Decisions (user, 2026-11-08)

- Same place, height (`tab_h`, 32 px) and colour (`bar`) as the strip, so side by side
  panes keep their bodies aligned and the body never jumps when a second tab comes or goes.
- Grip, then the view's title (fg, truncated, full title on hover), no tab shape (no
  active background, no divider after it, no × on it).
- Pane buttons as today, then a thin divider and × at the far right: closes the tab
  (the pane goes with it, except the last pane of a window, which shows its welcome).
- Focus: a 2 px accent line across the whole header.
- With one tab the tab and the pane are the same thing: the title drags as the tab,
  double-click maximizes, middle-click closes, right-click opens the tab's menu.
- A tab dragged over the header highlights it; dropped there it joins the pane (after
  the current tab) and the header becomes the strip. The body's centre zone does the same.
- Collapsed: in a column split a one-tab pane is its header; the row-split rail is unchanged.
- Empty pane: the header without a title (grip and pane buttons), still a drop target.
- Detached window: the header too (it can receive tabs), with the buttons a detached
  pane has today.
- No status mark in the header (not asked).

## How

`window_view.rs`, `render_pane_inner`: `let header = p.tabs.len() <= 1;`.
- The bar keeps what the strip has (drop target, grip, pane buttons); it draws either the
  tab list or the title.
- The title: id `("tab", pane, 0)`, debug selectors `tab-{pane}-0` and `tab-title` kept
  (it is that tab, so tests find it), drag `DraggedTab`, click
  activates and focuses, double-click maximizes, middle-click closes, right-click
  `tab_menu`.
- Right-click on the rest of the header: the tab menu with one tab, the pane menu with none.
- × as a pane button (`icon_button`, `Icon::Close`) after a divider, only with one tab.
- Focus line: `border_t_2` in `accent` on the header when focused (border-box: the
  height stays `tab_h`).
- New debug selectors: `pane-header-{pane}` on the header, `pane-tabs-{pane}` on the strip's
  tab list.

## Docs

- REQUIREMENTS: R3.12 (new); R3.4 "tab strip" → "tab strip or header"; R3.6 collapse
  to "its tab strip or header".
- DESIGN layout section: the header rule and its states.

## Tests (`crates/devdeck-app/tests/pane_header.rs`)

- One tab: the header shows, no tab list; a second tab: the tab list, no header; back to
  one: the header. The body's top is the same in all three.
- × in the header closes the tab and removes the pane (not the last); in the last pane
  it leaves the empty pane with its header.
- Double-click on the title maximizes.
- Right-click on the title opens the tab menu.
- A tab dragged from another pane onto a one-tab pane's header joins it, after its tab.
- Empty pane: the header without a title.

## Visual check (snapshots)

01-initial (two one-tab panes), 03 collapsed in column, 04 rail, 05 maximized, 06 narrow
long titles, 07 empty, 08 detached, 14 detached at 150, and a new 15-one-tab-focused.

## Plan review (subagent, 2026-11-08) and assessment

- 1 Taken: one `border_color` for every side and a 2 px border shifting the content; the
  focus line is an absolute 2 px child.
- 2 Taken: × and its divider in their own `flex_none` box; a narrow-pane test.
- 3 Taken: `tab-title` on the inner text, `tab-{pane}-0` on the title.
- 4 Taken: the menu key places its menu from `pane_rects`, not selectors (the sentence
  above is wrong: selectors are kept for tests only).
- 5 Taken: Shift+F10 on an empty pane opens the pane menu.
- 6 Rejected: a detached one-tab window keeps × in its header. The strip there has the
  tab's × today, which removes the window too; the header keeps what the strip had.
- 7 Noted: × moves from the far right into the tab when a second tab comes (user's design).
- 8 Taken: the title stops propagation on right-click.
- 9 Taken: `pane-body-{pane}`; focused against unfocused.
- 10 Taken: collapsed header click expands, middle-click, detached header, drop on an
  empty pane's header, right-click outside the title, Shift+F10 on an empty pane.
- 11 Taken: R3.4's handle clause, DESIGN's collapsed line, `pane_menu`'s comment.
- 12 Taken: 15-one-tab-focused and 16-one-tab-focused-150; × checked in 06.
- 13 Taken: the divider is a 1 px `line` div sized by tokens.

## Diff review (subagent, 2026-11-08) and assessment

- 1 Taken: the narrow test asserts the pane buttons gave way first (`pane-buttons-{pane}`);
  it fails with the buttons' shrink at 1.
- 2 Taken: a strip's tab right-click asserts its own menu, not the strip's pane menu.
- 3 Taken: the focus line has a selector, asserted only when focused; the title's top
  is the same focused or not.
- 4 Rejected: an empty pane is only ever the last pane of a window (a pane goes with
  its last tab), so no tab of that window can be dragged onto it, and tabs don't drag
  between windows (R3.8). The plan's "drop on an empty pane's header" was not reachable.
- 5 Taken: the tests are in `tests/pane_header.rs`.
- 6 Taken: an empty pane's menu disables "Send pane to…" and "Close pane" (no tabs).
- 7 Taken: the title has the pointer cursor, no hover background (no tab shape).
- 8 Left: a tab dragged over its own header highlights it, as over its own strip today.
- 9 Taken: the closure no longer shadows the pane id.

## Out of scope

A status mark in the header; the layout model (nothing changes in `core::layout`).
