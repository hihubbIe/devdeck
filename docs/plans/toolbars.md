# Toolbars — plan (2026-11-10)

Requirements: R9.9 (new), R5.1, R5.5, R4.4, R9.3, R1.6, R7.5 (the logs view).

The user, after the logs toolbar rework: the rule must hold for every toolbar, with
correct wide and narrow forms that resize well and prefer icons when short; set the rule,
build it once, apply it to the logs view first, the others later one at a time. Reviewed
by two subagents: a UI reviewer (quality of the rules, accessibility, scenarios, looks)
and a logic reviewer (the generic model and its rules).

## The component (`devdeck-ui`, `toolbar.rs`)

```rust
pub struct Toolbar { /* id, items, more: Vec<MenuItem>, more_tip: SharedString */ }
pub enum Fold { Never, At(u8) }              // a higher number folds first
pub struct ToolAction {
    pub id: SharedString,                    // the registered action's id
    pub icon: Icon,
    pub label: SharedString,                 // shown wide; the tooltip and menu text otherwise
    pub key: Option<SharedString>,           // shown in the tooltip
    pub on: Option<bool>,                    // Some: a toggle
    pub why_not: Option<SharedString>,       // disabled, and why (R5.5)
    pub fold: Fold,
    pub count: Option<SharedString>,         // a count after it ("N new")
    pub run: Rc<dyn Fn(&mut Window, &mut App)>,
}
pub enum ToolItem {
    Field { input: AnyView /* a TextInput */, min_chars: usize, count: Option<SharedString> },
    Action(ToolAction),
    Menu { action: ToolAction, value: Option<SharedString>, items: Rc<dyn Fn(&mut App) -> Vec<MenuItem>> },
    Status { icon: Icon, color: Hsla, tip: SharedString },
}
impl Toolbar {
    pub fn new(id) -> Self;
    pub fn item(self, ToolItem) -> Self;
    pub fn more(self, items: Vec<MenuItem>, tip: impl Into<SharedString>) -> Self; // always in "more"
}
impl RenderOnce for Toolbar
```

- Width: the toolbar's bounds at its last layout, kept in its element state by id; the
  first frame lays out wide.
- Item widths are computed: an icon item is `icon_button`; with a label, plus the label's
  characters times the cell width and its padding; a field's minimum is `min_chars` cells
  plus padding; gaps from the theme. So the form is chosen before layout, without
  measuring each element.
- The form: wide if everything fits with labels; else compact; else narrow, folding
  `Fold::At` items from the highest number down until it fits; `Fold::Never` items and
  statuses stay. "More" shows when it has items (its own `more` list or folded ones);
  folded items come first in it, then a separator, then the `more` list.
- The "more" menu opens by click and by its registered action's key; menus opened by key
  open under the toolbar's right end.
- Tooltips: an item's label and its key ("wrap long lines · w"), or its reason when
  disabled.

## The logs view on it

Field find (`Never`, count "3/12"); status warning (only when something is wrong); menu
level (value "warn+", `At(3)`); toggles wrap (`At(2)`), pause (count "N new" while paused,
`At(1)`), follow (`At(4)`: folds first); more: pod ▸, container ▸, previous run, show the
time, show the pod; its tooltip the counts. The hand-made toolbar goes.

## Tests (`devdeck-ui/tests/ui/toolbar.rs`)

The three forms at three host widths; the folding order; a folded toggle's checked state
and a folded menu as a submenu; the field keeps its minimum; a disabled item's reason in
its tooltip and in the folded item; "more" hidden when empty; clicks run the same
callbacks whatever the form; one line at every width (no wrap). App: the logs view's
items, its keys unchanged, its snapshot wide and narrow.

## Out of scope

Moving the other toolbars (one step each, later); pane headers.

## Reviews (2026-11-10) and assessment

Two subagents: **UI** (quality, accessibility, scenarios, looks; 12 points) and **logic**
(the generic model, GPUI; 10 points). Accepted unless said; the revised model below
replaces "The component" and "The logs view on it" above. Logic 2 checked in GPUI's source
(`window.refresh()` does nothing while drawing).

1. Keys in tooltips break R5.4 (UI 1, logic 1). **Accepted**: no key shown anywhere (no
   tooltip text, no `MenuItem.hint`); a key binds the item's registered action only.
   Test: no tooltip or folded item shows a key.
2. One frame late, and a redraw from a bounds callback is dropped while drawing (logic 2,
   UI 4). **Accepted**: `Toolbar` is a custom `Element`: one full-width leaf in
   `request_layout`; in `prepaint`, with the real width, it picks the form, builds the
   row, lays it out (`layout_as_root`) and prepaints it, as `uniform_list` does for its
   rows; `paint` paints it. No lag, no first wrong frame, no element state, no redraw.
3. Widths from characters times a cell width drift (logic 3). **Accepted**: labels are
   shaped with the text system (cached); every padding and gap comes from constants the
   component's own drawing uses.
4. Counts change width and flap the form; a folded count vanishes (UI 4, logic 4).
   **Accepted**: a count reserves a declared width (capped "999+"); the form then depends
   on the width alone (no hysteresis needed: said in the rule). A folded item's count goes
   into its menu text, and "more" shows a dot in the count's color.
5. Folding hides state that changes what's shown (a level other than "all", paused) (UI
   2). **Accepted**: an item whose state isn't its default doesn't fold
   (`ToolAction::state_shown`); a menu keeps its value in compact form (logic 6, UI 9).
6. "More" isn't a cog (UI 3). **Accepted**: Lucide `ellipsis`, the cog stays for app
   settings; its tooltip "more" and the figures; the figures also as a first disabled row
   in its menu (keyboard users get no tooltips).
7. A floor for very narrow widths (UI 5). **Accepted**: "more" and statuses always show;
   below the floor the field gives up its minimum and truncates; at the extreme it becomes
   an icon button that opens it. Tested at 150 px.
8. The model must fit the other toolbars (UI 6, logic 5). **Accepted**, checked on paper
   against the six (below): text-only actions (`label_required`: never icon-only, they
   fold instead), a primary action (never folds, keeps its label: "save"), a `Text` item
   (flexible, truncates, full text on hover: the run view's command line), field
   accessories (the tag editor's suggestions button), a spacer (what follows is
   right-aligned), statuses with a short word ("unsaved", dropped in compact), menu values
   with a color and an on state (the env button: protected in red with its lock, framed
   when not synced) and a maximum width. The sidebar's list header keeps its activity
   name: it has no tab to carry it (an exception written in the rule).
9. Color without a mark (UI 7). **Accepted**: a safety value (an env) never folds, always
   shows its value, its color with its icon; the sidebar's env button gets the lock when
   later moved.
10. Statuses only on hover, no keyboard path (UI 8). **Accepted**: a status is clickable,
    its details open as a menu, also in "more"; every item's id resolves to a registered
    action (palette, cheat sheet), tested; right-click on the toolbar opens every item,
    shown and folded, in order (R5.2).
11. Icons (UI 9). **Accepted**: follow gets its own glyph (not `Pull`'s
    `arrow-down-to-line`); pause shows `play` with "resume" while paused; a rule: no icon
    means two things in the app.
12. Toggles and sizes (UI 10). **Accepted**: one toggle look (fill and frame on, nothing
    off) and one height (`button_h`) for every item; gaps at least 4 px scaled.
13. Fold order wording (UI 11). **Accepted**: `Fold::Order(n)`, 1 folds first, the same
    words in R9.9 and DESIGN; equal numbers fold together (groups) (logic 6); folded items
    keep the toolbar's order in "more".
14. Folded menus and `MenuItem` (logic 7). **Accepted**: a folded menu button becomes a
    submenu; its items can't hold submenus (asserted; menus are one level deep).
15. One source of truth with the registry (logic 8). **Accepted**: `ToolAction` stays
    generic in devdeck-ui; the app builds items with `tool(id, icon)`, which takes the
    label from the registry and runs the registered action (mouse, folded item and key:
    one path). Logs adds `logs.level`, `logs.time`, `logs.pod_column`; its "more" is
    `logs.options` (`.`).
16. Menus opened by key need an anchor (logic 9). **Accepted**: the element publishes each
    item's bounds to a handle the view owns; a menu opened by key opens under its button.
17. Tests (logic 10). **Accepted**: a pure `fit(widths, available) -> Form` unit-tested
    (order, ties, reserved counts, state shown, floor); host tests in `tests/ui/toolbar.rs`
    at three widths and 150 px.
18. Polish (UI 12). **Accepted**: the count sits at the field's right end; the toolbar's
    right end has its own inset from the pane header's icons; a disabled item is dim, still
    clickable to explain, its reason shown in "more".

## The model, revised

```rust
pub enum Fold { Never, Order(u8) }        // 1 folds first; equal numbers fold together
pub struct ToolAction {
    pub id, icon: Option<Icon>, label,     // label from the registry (app helper)
    pub on: Option<bool>,                  // a toggle
    pub state_shown: bool,                 // not in its default state: doesn't fold
    pub label_required: bool,              // never icon-only: folds instead
    pub primary: bool,                     // never folds, keeps its label
    pub why_not: Option<SharedString>,     // disabled, and why
    pub fold: Fold,
    pub count: Option<(SharedString, usize /* reserved cells */, Hsla)>,
    pub run: Rc<dyn Fn(&mut Window, &mut App)>,
}
pub enum ToolItem {
    Field { input: AnyView, min_chars, count: Option<..>, accessories: Vec<ToolAction> },
    Action(ToolAction),
    Menu { action: ToolAction, value: Option<(SharedString, Option<Hsla>, Option<Icon>)>, max_chars, items: Rc<dyn Fn(&mut App) -> Vec<MenuItem>> },
    Status { icon, color, word: Option<SharedString>, details: Rc<dyn Fn(&mut App) -> Vec<MenuItem>> },
    Text { text, color },                  // flexible, truncates, full text on hover
    Spacer,                                // what follows is right-aligned
}
pub struct Toolbar { id, items, more: Vec<MenuItem>, more_id, figures: Option<SharedString>, anchors: Option<Rc<Anchors>> }
```

Forms from the real width in prepaint: **wide** (icons and labels, values, words) when it
fits; **compact** (labels dropped except `label_required` and `primary`; values kept;
status words dropped); **narrow** (fold `Order` items, 1 first, groups together, never
`Never`, `primary`, `state_shown`, safety values or statuses, until it fits); **floor**
(the field truncates, then becomes an icon button).

On paper: logs (field + count, status, level menu, wrap, pause/resume + count, follow,
more); run view (state status with its word, `Text` command line, arguments menu with its
value, actions); service view (verbs `label_required`, spacer, lock; tag editor: field
with the suggestions accessory, "unsaved" status, `primary` save, `Text` repo); git panel
(icon actions, status words); matrix (title kept as `Text` until its own pass decides);
sidebar header (its activity name, an exception; filter field; running toggle; env menu
with color, lock and on).

## Outcome (2026-11-10)

Built as revised: `devdeck-ui/src/toolbar.rs` by a builder (the custom element, `fit`, 10
unit and 9 host tests, icons `ellipsis` for more, `search`, and `chevrons-down` for follow,
which no longer shares pull's glyph), the logs view moved onto it by the lead.
- Departures: a field also takes an id, a label and its input's focus handle (its bounds,
  the floor button, the right-click row); a menu button shows its icon and value (its
  label in the tooltip as "label: value"); a toolbar shows short words ("wrap",
  "follow"), the palette keeps the registry's full names; folded "more" items and the
  right-click list run the same callbacks as the buttons.
- Logs: find (field, count "3/12"), a status when something is wrong (its word wide, its
  details on click), the level (menu, folds third, never while filtered), wrap (second),
  pause/resume (first, never while paused, "N new"), follow (folds first of all, never
  while off), more (pod, container, previous run, the columns; its figures). New actions
  `logs.level` (`l`), `logs.time`, `logs.pod_column`; `.` opens more.
- Snapshots: `90-logs-json` (wide), `90-logs-medium` (compact, filtered to warn+),
  `90-logs-narrow` (the floor: find as a button).
- R9.9 is partly done: the other toolbars move onto the component one at a time.
