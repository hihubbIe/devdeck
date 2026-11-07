# M1 · Native prototype with fake data — plan

Goal: the whole interface of the approved HTML prototype, plus the decisions taken
since (generic panes, "Send to…", detached ⇲/×, workspaces, alphabetical Pinned+All
lists, full-window settings with UI scale, themes, quit guard), running natively on
the M0 engine with fake data. M2 and M3 then replace the fake data with real
processes and a real cluster without reshaping the UI.

Requirements: R2.1–R2.3, R3.10, R3.11, R4.1–R4.5, R5.1–R5.5, R6.2–R6.6 (UI side),
R7.1–R7.7 (UI side), R8.1, R9.1–R9.4, R10.1 (settings file), R11.1–R11.5.

## Architecture changes

```
crates/
  devdeck-core   + domain models (App, Command, Env, Deployment, Pod, Run…),
                   + lists (Pinned/All ordering, filtering), + fuzzy search (nucleo-matcher),
                   + settings and theme files (TOML), + actions registry (pure data),
                   + demo data provider (fake workspaces, apps, envs, deployments)
  devdeck-ui     NEW. Theme (tokens × UI scale) and every shared component:
                   Label (truncating + tooltip), Button, IconButton, Kbd, Chip, Row,
                   SectionHeader, TextInput (single line, adapted from GPUI's input
                   example), Menu (anchored popover), Modal, Toast, Sparkline.
  devdeck-app    Shell (workspaces, main window chrome, overlays), Deck per workspace,
                   views/ (one module per view type), fake runner (timers).
```

Views stay in `devdeck-app/src/views/` for M1 (they need the app state); a separate
crate is reconsidered when M2 adds real data sources.

### Shell and workspaces
- `Shell` entity owns: workspaces (each a `Deck` + its fake data), the active index,
  overlays (palette, menu, send-to, cheat sheet, quit guard, toasts), settings.
- The main OS window belongs to the Shell and renders the active workspace's main
  layout. Detached windows belong to their workspace's Deck and are opened only while
  that workspace is active (closed as OS windows, kept in the model, when switching
  away; reopened when coming back). Views and runs of inactive workspaces stay alive.
- Quit (ctrl q, window close): if runs are active in any workspace, a modal lists them
  with "Stop all and quit" / "Cancel".

### UI scale and theme
- `devdeck-ui::Theme` = color tokens from a theme file + sizes from a base table, all
  multiplied by `ui_scale`. Components and views read sizes only from the theme (no
  raw `px(…)` literals outside devdeck-ui; enforced by review and a grep check).
- Settings file `~/.config/devdeck/settings.toml` (`ui_scale = 1.0`, `theme = "devdeck-dark"`),
  written when changed. Theme files: built-in `devdeck-dark` embedded, plus any
  `~/.config/devdeck/themes/*.toml`.
- Settings view: full window (replaces the body; title bar stays), opened by a title-bar
  button or ctrl+,; Esc closes. UI scale: 80–150 % in 10 % steps with buttons and keys;
  theme picker; shows the config folder path.

### Lists
- One `List` model in core: items with a stable key, a label, a pinned flag; ordering is
  Pinned (alphabetical) then All (alphabetical, excluding pinned). Filter is fuzzy.
  "running only" filter for apps. Pin/unpin by menu, key `p`, or dragging a row onto the
  other section header.

### Actions
- Registry of `ActionSpec { id, label, group, default_keys, context }`; availability is a
  function returning `Ok` or a reason. Buttons, menus, palette, keys and the cheat sheet
  are generated from it. GPUI `actions!` remain the dispatch mechanism; the registry is
  the single source of labels and keys.

### Fake data
- Two demo workspaces: "work" (apps modeled on the real repos, envs dev31–36 + protected
  hades/dls/lab, deployments with branches and versions) and "personal" (a few apps, no
  envs, no cluster) to prove optional cluster/env UI (R2.3).
- Fake runner: commands produce scripted output over time, some long-running, one crashes,
  missing-tool apps refuse with the reason (R6.5).
- Fake cluster: deployments, pods, metrics random walk, deploy/rollout/undo simulation
  including image-pull failure, protected-env unlock.

## Steps

1. **Settings and UI scale** (detailed below).
2. **devdeck-ui and theme files**: extract components, TextInput, Menu, Modal, Toast;
   load theme files; snapshot every component state.
3. **Workspaces**: Shell, switcher in the title bar (and palette), per-workspace Deck and
   detached windows, quit guard.
4. **Activities and lists**: activity bar, sidebar with Pinned/All for Apps, Cluster
   (env selector), Ports, Layouts; filters; pin by key/menu/drag.
5. **Views with fake data**: app, command output, service inspector (tag editor, confirm,
   rollout, undo, protected unlock, metrics, pods, container, history), logs, pod, text,
   branch matrix, ports.
6. **Actions everywhere**: palette (fuzzy, @env, Tab drill), context menus on every
   element, "Send to…" with numbered panes, cheat sheet, hint bar.
7. **Layouts per workspace** (save, switch, keep-in-every-layout) and M1 measurements.

Each step: own short plan section appended here, subagent review, build, snapshots of
every listed state, tests, clippy, commit.

## Step 1 · Settings and UI scale

Scope:
- `devdeck-core::settings`: `Settings { ui_scale: f32, theme: String }`, defaults, clamp
  (0.8–1.5, rounded to 0.05), TOML load (missing file → defaults; bad file → defaults +
  error message kept for display) and save (atomic write: temp file + rename).
  Config dir from `$XDG_CONFIG_HOME` or `~/.config`, overridable by `DEVDECK_CONFIG_DIR`
  (tests, snapshots).
- Theme sizes: `Theme::devdeck_dark(scale)`; every size (text, line height, rows, tab
  strip, rail, title, status, dividers' grab area, buttons) scaled. Replace every `px()`
  literal in window_view.rs and views.rs with theme sizes.
- Settings view: full window body; sections "Display" (UI scale: − / value / + buttons,
  preset buttons 90/100/110/125/150 %, keys ctrl+= / ctrl+- / ctrl+0 globally) and
  "Theme" (list of available themes, only devdeck-dark now), footer with config file path
  and last load error if any. Title-bar gear button; ctrl+, toggles; Esc closes.
- Changing scale re-renders all windows immediately and saves.

States to snapshot: settings view at 100 %; main layout at 80 %, 100 %, 125 %, 150 %;
settings view at 150 %; settings with a load error shown.

Tests: settings parse/clamp/round-trip/atomic save/defaults on missing or broken file
(core); theme sizes scale linearly; GPUI test that ctrl+= changes the scale, saves the
file and re-renders; settings view opens and closes with keys.

Out of scope: per-workspace theme choice (step 3), more settings.

## Review (subagent) and assessment

1. Main window ownership moves to a `Shell`; every quit path goes through
   `Shell::request_quit` (window close via `on_window_should_close`). **Accepted, step 3.**
2. Detached windows of an inactive workspace are closed and reopened (size saved, focus
   back to main); workspace switches join the stress/leak test. **Accepted, step 3.**
3. Runs, output buffers and cluster state live in per-workspace model entities fed by
   providers; views only hold UI state. **Accepted, step 5** (log demo moves there).
4. Pane actions move to a `key_context("Panes")` subtree with its own focus handle;
   settings has its own context and restores focus on Esc. **Accepted, step 1.**
5. Also call `window.set_rem_size(16 px × scale)` in every window so rem helpers scale.
   **Accepted, step 1.**
6. Derived sizes rounded once in the theme (whole logical px, matching GPUI's
   line-height rounding); hairlines stay 1 px; minimum grab areas. **Accepted, step 1.**
7. Discrete scale steps only (glyph atlas per size): 80, 90, 100, 110, 125, 150 %;
   ctrl+= / ctrl+- move along them; scroll re-anchoring checked when views move to step 5.
   **Accepted.**
8. `clippy.toml` disallows `gpui::px` and `gpui::rems` outside devdeck-ui. **Accepted, step 1.**
9. Core provider interfaces: `Runner` / `RunEvent` with raw PTY bytes, `Cluster` /
   `ClusterEvent`, `Remote<T>`, `Availability`, `EnvBinding`, stable ids, `SystemTime`,
   injectable `Clock`, fake latency. **Accepted, step 5 (models land at its start).**
10. Adapt GPUI's `examples/input.rs` (add horizontal scroll, word moves, undo,
    placeholder; caret blink that stops so idle renders stay 0). **Accepted, step 2.**
11. One source of truth for actions: registry ids are GPUI action names; bindings built
    from it; displayed keys read back from GPUI; a test checks the sets match.
    **Accepted, step 6** (bindings move to the registry then).
12. Theme global (R9.4) vs per workspace (R2.1) contradiction. **Asked the user.** Step 1
    builds the UI scale only; the theme picker waits for the answer (step 2).
13. Create devdeck-ui with the theme in step 1. **Accepted.**
14. Persistence module (state dir) in step 3; toasts and crash notification in step 5;
    empty/loading/error states listed per view; palette "recent" decided before step 6;
    AccessKit roles where cheap. **Accepted.**
15. Step 1 test and snapshot gaps (NaN/inf, unknown keys, save failure, dir precedence,
    pane keys blocked in settings, focus restore, clicks on − / + / presets, detached
    window and long titles at other scales). **Accepted.**
16. Geometry tests on the real text system with `debug_selector` / `debug_bounds`;
    PNGs stay for human checks. **Accepted.**

## Step 2 · Components and theme files

Scope:
- **Theme files** (`devdeck-core::theme_file`): TOML `name`, `[colors]` with hex tokens
  (`bg`, `bar`, `raised`, `hover`, `selected`, `line`, `line_soft`, `fg`, `hi`, `mute`,
  `dim`, `accent`, `accent_bg`, `drop`, `ok`, `warn`, `err`, `env`, `env_bg`). Built-in
  `devdeck-dark` is embedded from `assets/themes/devdeck-dark.toml`. User themes from
  `<config>/themes/*.toml`; a missing token falls back to devdeck-dark's, a bad value or
  file is reported (shown in Settings) and skipped. Unknown theme name in settings →
  devdeck-dark plus a message. Where the choice lives (global or per workspace) waits for
  the user's answer; the loader is the same either way.
- **TextInput** (devdeck-ui), adapted from GPUI's `examples/input.rs`: single line,
  placeholder, selection by mouse and shift+keys, word moves (ctrl+←/→), ctrl+a,
  ctrl+backspace, clipboard, IME, horizontal scroll that keeps the caret visible, undo
  (ctrl+z). The caret does not blink, so idle renders stay 0. Editing logic lives in a
  pure `TextBuffer` (byte/char safe for UTF-8) with unit tests; the element only draws it.
- **Menu**: anchored popover (`deferred(anchored())`) with items
  `{label, key hint, checked, disabled reason, separator}`; ↑/↓ skip separators and
  disabled items, Enter runs, Esc closes, hover selects, click runs; a disabled item
  shows its reason as a tooltip. Selection logic pure and tested.
- **Modal**: centered panel over a dimmed backdrop, Esc and backdrop click close it,
  focus trapped inside, restored on close.
- **Toast**: bottom-right stack, kinds info/ok/warn/err, optional actions, auto-dismiss
  (6 s, errors 10 s) with a single timer that stops when the stack is empty.
- A **component gallery** state in `devdeck-snapshot` showing every component in its
  states (input empty/placeholder/selected/long text scrolled, menu with checked and
  disabled items, modal, each toast kind) at 100 % and 150 %.

Tests: theme parsing (full, partial fallback, bad hex, unknown name), TextBuffer
operations (insert, delete, word moves, selection, UTF-8, undo), menu navigation,
toast expiry; GPUI tests for typing into a TextInput, menu keyboard flow, modal focus
restore; idle-render check with an input focused.

Out of scope: actions registry and real menus on app elements (step 6).

### Step 2 review and assessment

1. Single-letter and app keys fire inside inputs on Linux (bindings beat text input).
   Bindings that could collide use `"Panes && !TextInput"`; GPUI test types `pw` into an
   input inside Panes and checks nothing else happened. **Accepted.**
2. TextInput gaps vs `input.rs` (own `TextInput` context and ctrl keys, horizontal
   scroll with clamped `scroll_x` computed in prepaint, content mask, scroll-aware
   hit testing and IME bounds, paste normalization of \r\n \r \t, word moves,
   ctrl+backspace/delete, shift+home/end, double-click word, Enter/Esc as events,
   grouped undo, redo). **Accepted.**
3. TextBuffer: grapheme-safe (unicode-segmentation), UTF-16 conversion and marked
   range in the buffer, tested. **Accepted.**
4. Menu: occluding backdrop closes it without clicking through; own focus handle and
   `Menu` context, focus restored through a weak handle, closes on blur; max height
   with scroll. **Accepted.**
5. Overlays render from the window root, never inside a pane; priority constants in
   devdeck-ui (modal < menu < toast). **Accepted.**
6. Modal: own `Modal` context, tab/shift-tab cycle its own focus handles, occluding
   backdrop, focus restore with fallback, a modal stack; context-free bindings stay
   active only where harmless (scale keys), quit opens its guard over modals.
   **Accepted.**
7. Toasts: one task sleeping until the earliest deadline, dropped when empty; time from
   the executor so tests can advance it; no animation. **Accepted.**
8. Theme tokens added: backdrop, focus, selection, caret, scrollbar; optional `[ansi]`
   table (16 colors plus fg/bg/cursor/selection) so M2's terminal doesn't change the
   format; unknown keys warn; a user theme reusing the built-in name is an error.
   **Accepted.**
9. Disabled reason shown inline on the selected menu item (keyboard reachable).
   **Accepted.** Keyboard key to open a context menu: **step 6.**
10. Test list (outside click, menu inside viewport after resize, modal tab cycle,
    focus restore fallback, toast task stops, caret stays visible). **Accepted.**
11. Middle-click primary paste. **Skipped for now** (unverified on Wayland).

## Step 3 · Workspaces, persistence, quit guard

Scope:
- **Shell** (app entity, global): `workspaces: Vec<Workspace { id, name, deck }>`, `active`.
  Owns the single main OS window. Its root view (`WindowView` for MAIN) holds the Shell
  and renders the active workspace's main layout; it re-subscribes to the active deck on
  every switch. Detached windows stay owned by their Deck.
- **Deck changes**: `Deck::sync` never opens an OS window for MAIN_WINDOW. A Deck has an
  `active` flag: when it goes inactive it closes its detached OS windows (model kept,
  last size remembered); when it becomes active again `sync` reopens them, then focus
  returns to the main window. Views of inactive workspaces stay alive (their data keeps
  flowing); hidden views don't render.
- **Switching**: title-bar workspace button (`work ▾`) opens a menu of workspaces
  (alphabetical, current checked); keys ctrl+alt+1…9 pick by position; the window title
  shows the workspace. Settings and the quit guard work in any workspace.
- **Quit guard (R8.1)**: every quit path (ctrl+q, title-bar ×, window manager close)
  goes through `Shell::request_quit`. If anything is running in any workspace, a modal
  lists it per workspace with "Stop all and quit" (default) and "Cancel"; otherwise quit
  at once. In M1 "running" comes from a `Deck::running()` list (the demo log stream
  counts while streaming); M2 plugs in real processes.
- **Persistence (R10.1)**: `devdeck-core::state` with the state folder
  (`DEVDECK_STATE_DIR` > `$XDG_STATE_HOME/devdeck` > `~/.local/state/devdeck`).
  Per workspace `workspaces/<id>/layout.json`; `app.json` with the active workspace.
  Saved after changes, debounced by one timer per deck (≈1 s, no timer when idle);
  restored at start; a broken file falls back to the default layout with a toast.
- **Demo workspaces**: `work` (current demo layout, cluster demo) and `personal` (notes
  and gallery only, no cluster) defined in code until M2's config.

States to snapshot: work active; personal active; quit guard modal; work with a detached
window then switched to personal (detached window gone) and back (window back).

Tests: state dir precedence; layout save/restore round trip and broken-file fallback;
switching keeps each workspace's layout and views alive; detached windows close/reopen
on switch; every quit path shows the guard when something runs and quits directly
otherwise; "Cancel" keeps the app; ctrl+alt+N switches; debounced save writes once after
a burst of changes; stress cycles include workspace switches (leak check).

### Step 3 review and assessment

1. Five direct quit paths. All go through `Shell::request_quit`; Deck never handles the
   main window; ctrl+q bound at app level so it works from detached windows; the
   last-window observer stays as a fallback that flushes saves; stress mode keeps
   bypassing the guard (documented). **Accepted.**
2. Main window `on_window_should_close` returns false and defers `request_quit`;
   `on_app_quit` only flushes saves. **Accepted.**
3. Deferred window open must check the deck is still active. **Accepted.**
4. `forget_layer` on window close; overlay layer count joins the leak check. **Accepted.**
5. Per-workspace theme: **resolved by the user, the theme is global** (already built).
6. One `WindowView` type with a role; main swaps its deck and subscription when the
   active workspace changes; pane tree wrapped in a per-workspace element id.
   **Accepted.**
7. Focus remembered per workspace with fallback to the panes; drags carry their deck
   id and drops from another deck are ignored. **Accepted.**
8. Detached window sizes stored in the model (serde default, survives restart), updated
   from bounds changes; main window size in `app.json`; never positions. **Accepted.**
9. Persistence details: shared `write_atomic` with unique temp name; unreadable or
   unsupported files renamed to `.bad` before falling back; a failing save reported
   once; detached windows restored only for the active workspace; folder layout ready
   for pins and saved layouts. Writing on the background executor and fsync: **accepted
   for the write; fsync of file and folder added.** **Accepted.**
10. Quit seam on Shell for tests (test platform can't quit); listed tests added.
    **Accepted.**
11. "Stop all" is asynchronous (stopping state, then quit) so M2's process-group kill
    fits; ctrl+alt+N follows the alphabetical order of workspaces. **Accepted.**
12. Detached-gone/back checked with GPUI asserts rather than snapshots. **Accepted.**

### Step 3 outcome

Built as assessed. `Shell` (`crates/devdeck-app/src/shell.rs`) owns the workspaces and
the main window; each `Deck` owns one workspace's layout, views and detached windows.
The two demo workspaces (`personal`, `work`) are defined in code until M2's config
(R2.1 partial). The quit guard lists the demo log stream as running; real processes
come with M2 (R8.1 partial). Layouts save one second after the first change of a
burst, on the background executor, and on quit.

Checks: 11 new tests in `tests/workspaces.rs` (switching, detached windows across
switches including a quick A→B→A, ctrl+alt+N, every quit path, Cancel, Stop all,
debounced save, restore, broken-file fallback, overlay layers freed). Snapshots
40–44 (work, personal, workspace menu, quit guard at 100 % and 150 %) and 08
(detached) checked. The stress run now switches workspaces every 25 cycles; it runs
with the M1 measurements at the end of the milestone, not per step.

## Step 4 · Activities and lists

Covers R4.1–R4.3, R2.3 (Cluster only when configured), R4.4/R4.5 (rows truncate),
R5.1/R5.2 for list rows. Layouts (R3.10) gets its activity in step 7 with the feature.

Scope:
- **Core `list` module** (pure): `ListItem { key, label, detail, running }`, a pin set,
  and `rows(items, pins, filter, running_only) -> Vec<Row>` giving `Header(Pinned)`,
  pinned items, `Header(All)`, the rest. Both sections alphabetical (case-insensitive,
  key as tie-break). Filter: case-insensitive subsequence match on the label; it only
  hides rows, never reorders. "Running only" hides idle rows. Empty sections keep their
  header with an empty line ("nothing pinned", "no match").
- **Core `demo` module**: the fake catalog until M2/M3. `work`: apps named after the
  real repos (boardservice, entityservice, front-end packages, …), envs dev31–dev36 plus
  protected hades/dls/lab, deployments per env, a few ports. `personal`: three apps, no
  envs, no cluster. A couple of apps marked running so the indicator and the filter show.
- **Pins persisted** per workspace in `workspaces/<id>/pins.json` (`{ list id: [keys] }`),
  written atomically on change (rare, so no debounce); broken file → `.bad` + toast.
- **Activity bar** (main window, left, `rail_w` wide): Apps, Cluster (only when the
  workspace has a cluster), Ports. Glyph + tooltip with its key. Clicking the active one
  hides the sidebar; clicking another shows it.
- **Sidebar** (main window, right of the activity bar, fixed `sidebar_w`, theme token):
  header (activity name; Cluster adds the env button `dev35 ▾` with a menu of envs,
  alphabetical, protected ones marked ⊘), filter input, "running only" toggle (Apps),
  then the rows in a `uniform_list` (virtualized). Row: state glyph (● running, ○ idle),
  label truncating, dim detail on the right (never pushes the glyph).
- **Per workspace**: each workspace owns its sidebar state (activity, visible, env,
  filter, selection, pins); switching workspace shows the other one's.
- **Keyboard** (`Sidebar` key context): ctrl+1/2/3 picks the activity and focuses its
  list, ctrl+b shows/hides the sidebar, up/down/home/end move the selection, enter opens
  the selected item (a placeholder tab until step 5), `p` pins/unpins, `/` focuses the
  filter, esc clears the filter, then returns focus to the panes. Typing in the filter
  never triggers row keys.
- **Mouse**: click selects, double-click opens, right-click menu (Open, Pin/Unpin),
  drag a row onto the other section's header to pin/unpin.

States to snapshot: work Apps with pins and running rows; filter with no match; running
only; Cluster with the env menu open; personal (no Cluster icon); sidebar hidden; Apps at
150 %; a long label truncating next to its detail.

Tests: core ordering, filter, running-only, empty sections, pin/unpin (unit); app: no
Cluster activity without a cluster, per-workspace sidebar state, keys (move, `p`, enter,
`/`, esc chain), typing in the filter doesn't pin, right-click menu, drop on header pins,
pins saved and restored, broken pins file.

Out of scope: real discovery and views (step 5, M2), palette (step 6), resizable sidebar.

### Step 4 review and assessment

1. Activity keys (ctrl+1/2/3, ctrl+b) live on the root `Deck` context, main window
   only; ignored in detached windows and while Settings is open. **Accepted.**
2. Pane neighbour geometry must use the pane area, not the viewport, once the activity
   bar and sidebar take width; test alt+arrows with the sidebar shown and hidden.
   **Accepted.**
3. The filter's Enter/Esc arrive as `Submit`/`Cancel` events (they propagate): Enter
   opens the selected row, Esc clears the filter, then a second Esc returns to the
   panes. `p`, `/`, home/end use `Sidebar && !TextInput`; up/down move the selection
   from the filter too. **Accepted.**
4. Focus: remembered per workspace wherever it is (not only in the panes), unless
   Settings is open; closing Settings returns to where focus was; hiding a focused
   sidebar sends focus to the panes. **Accepted.**
5. Env selector: DESIGN puts `env ▾` in the title bar; it is the workspace's env for
   runs and for the Cluster list. **Accepted: title bar**, absent when the workspace has
   no envs (R2.3).
6. Deployment pins are keyed by service name, shared across envs. Pins of keys missing
   from the catalog stay in the file and are not shown; unit test. **Accepted.**
7. Persist the whole sidebar state per workspace (`sidebar.json`: activity, visible,
   env, pins), written on the background executor, failure reported once; every
   workspace's file loads at start. **Accepted.**
8. Selection keyed by item, skipping headers and empty lines; an item hidden by the
   filter, running-only or a pin move clears the selection (the next move picks the
   first); the selected row scrolls into view. **Accepted.**
9. Any drag shows the pane drop zones (GPUI doesn't expose the drag type). Instead of
   hiding them, list rows dropped on a pane open the item there (into or at an edge),
   which R3.2 wants anyway. Dropping on a row of the other section pins/unpins like
   the header. **Accepted with that change.**
10. States added: running-only with nothing running ("nothing running"), protected env
    selected (⊘ in the env button), selected row focused and unfocused, filter with
    matches, right-click menu. A cluster with no envs counts as no cluster (the catalog
    guarantees envs). Drag-over highlight: covered by a test, not a snapshot (headless
    snapshots can't hold a drag). **Partly accepted.**
11. Keyboard paths: `r` toggles running only, ctrl+e opens the env menu. Context menus
    on activity icons and section headers come with step 6 ("context menus
    everywhere"); tooltip key texts are hand-written until the step 6 registry.
    **Partly accepted.**
12. Open: into the main window's focused pane; an open item is activated, not
    duplicated; focus stays in the sidebar. Activity bar gets its own `activity_w`
    token. 150 % snapshot added. Sidebars live as long as their workspace, so no churn
    to leak-test. **Accepted.**

### Step 4 outcome

Built as assessed. `crates/devdeck-core/src/list.rs` (rows, filter, selection steps,
pins), `demo.rs` (fake catalog), `state.rs` (`sidebar.json`), and
`crates/devdeck-app/src/sidebar.rs` (activity bar choice, list, keys, menus, drag).
Lists are alphabetical with Pinned and All everywhere a sidebar list exists (R4.1
partial until every list in later steps follows it); running state is fake until M2.

Found while checking: a context menu opened from a mouse down lost focus to the list
container (GPUI focus-on-mouse-down), so its keys did nothing; fixed once in
`open_menu`. Section counts now follow the filter.

Checks: 10 tests in `tests/sidebar.rs` plus 3 core unit tests; snapshots 50–60 (pins
and selection, filter with and without matches, running only, env menu, protected env,
personal without cluster, sidebar hidden, 150 %, row menu, ports) and 01, 20 again.

## Step 5 · Views with fake data

Covers, on fake data: R3.3 (view registry), R6.3, R6.5, R6.6 (shown, not applied),
R6.7 (runs, stop, restart, exit status, crash toast; plain lines until M2's terminal
grid), R6.9 (ports shown), R7.1–R7.7, R5.5 (reasons on disabled actions). Three commits:
5a apps and runs, 5b service inspector, 5c logs, pod, matrix, ports.

Architecture:
- **New crate `devdeck-views`** (DESIGN's crate map): one module per view type and a
  `ViewRegistry` mapping a `ViewRef` prefix (`app/`, `run/`, `svc/`, `logs/`, `pod/`,
  `matrix`, `port/`) to a builder and a title function. The app registers the demo views
  too; `Deck` asks the registry, so adding a view type never touches the layout engine.
  Sidebar rows open `app/<id>`, `svc/<env>/<name>`, `port/<n>` (renamed from `deploy/`).
- **Core domain models** (pure, tested): `apps` (App, Command with tier
  primary/secondary/hidden, required tools, `EnvBinding` with how it applies), `tools`
  (availability with a reason), `runs` (RunId, RunState idle/running/exited(code)/
  crashed, output lines with a level), `cluster` (Deployment, Pod, container state,
  Metrics, DeployRecord, RolloutState), `Remote<T>` (Loading/Ready/Error) for anything
  fetched.
- **Fake services in the app** (`fake/`): a `FakeRunner` entity per workspace (scripted
  output over time via timers, one long-running server, one that crashes after a few
  seconds, apps whose tool is missing refuse to start with the reason) and a
  `FakeCluster` entity per workspace with a cluster (deployments per env, pods, metrics
  random walk sampled only while a service view is visible, rollout simulation:
  progressing → ready, or image pull failure for tags containing `broken`, undo to the
  previous tag, deploy history). Both are entities that views observe; M2/M3 replace
  them behind the same models.
- Sidebar running indicators and the quit guard read the runner (the demo log stream
  stops counting as a process).

Views:
- **App** (`app/<id>`): header (name, kind, folder, env chip with how the env applies,
  ⚠ missing tools named), commands grouped Primary / Secondary / Hidden (collapsed),
  each with state glyph, ▶ run / ■ stop / ↻ restart, last exit code, ports. Keys:
  up/down, enter runs, s stops, r restarts, o opens output.
- **Run output** (`run/<app>/<cmd>`): toolbar (command, `@env`, state, exit code,
  stop/restart, follow), virtualized lines colored by level, never wrapping. Crash →
  error toast with "View output".
- **Service inspector** (`svc/<env>/<name>`): header (name, env chip, ⊘ protected,
  status, tag, ready n/m, running since), actions row (restart, scale, port-forward,
  logs, shell, k9s; unavailable ones disabled with the reason), tag editor (input with
  suggestions menu from other envs and history, unsaved marker, ctrl+s / Save), confirm
  modal stating the exact change and the exact `kubectl set image …` command, rollout
  progress line, failure with one-action Undo, protected envs require typing the env
  name first. Sections: metrics (CPU and memory against request and limit, numbers and
  bars), pods table (status, ready, restarts, last termination reason, age, node, cpu,
  mem), container (image, ports, resources, configmap), history (devdeck's deploys).
- **Logs** (`logs/<env>/<name>`): merged pods with a pod column, level filter, pause /
  follow, find (highlights, n / shift+n).
- **Pod** (`pod/<env>/<pod>`): pod details, container states, events-like history.
- **Branch matrix** (`matrix`): services × envs, tag per cell, click a cell to highlight
  that tag everywhere; protected env columns marked.
- **Port** (`port/<n>`): holder, command line, "Stop holder" (fake).

States to snapshot: app with all tiers and a missing tool; run output streaming,
exited 0, crashed; service ready, rolling out, failed with undo, protected locked,
confirm modal, tag suggestions menu, loading and error (fake cluster unreachable);
logs filtered and searching; pod; matrix with a highlighted tag; port.

Tests: core models (tiers ordering, tool availability reasons, rollout state machine,
undo target, protected rules); app: run/stop/restart and exit codes on the fake clock,
crash toast, missing tool refuses with reason, sidebar running indicator follows runs,
quit guard lists runs per workspace, tag edit → confirm → rollout → ready, broken tag →
failure → undo, protected unlock flow (wrong name refused), metrics stop sampling when
the view is hidden, logs filter/find, registry builds every prefix and unknown refs fall
back to a "not available" view.

Out of scope: real processes, PTY and terminal grid (M2), real cluster (M3), palette
(step 6), shell/k9s actually opening (disabled with "comes in M3").

### Step 5 review and assessment

1. Crate cycle (views would depend on the app's fakes) and contradiction with the
   Architecture section. **Accepted: views stay in `devdeck-app` (`views/` module, one
   file per view, registry there) for M1**; the crate split comes with real services.
2. Views hold one stable `Runner` / `Cluster` entity (state plus start/stop/set-image
   methods) whose driver is the fake in M1 and real in M2/M3. **Accepted.**
3. Builders get a `ViewCx`: workspace id, catalog, runner, cluster (optional), the
   workspace's env (moved from the sidebar into a per-workspace `Services` entity the
   sidebar also uses), and a weak deck handle to open other views. **Accepted.**
4. View addresses parsed in core (`ViewAddr`) with escaping for `/` in ids and
   round-trip tests. `deploy/` → `svc/` breaks only refs saved during development;
   unknown refs show a "not available" view. **Accepted, no migration.**
5. Restored refs: a `run/` with no run shows the idle view with Run; `svc/` without a
   cluster, a service missing in that env, and a gone pod each get their own message.
   **Accepted.**
6. Metrics sampling: the Deck knows which views are visible (active tab, pane not
   collapsed, workspace active); the service view holds a sampling handle only while
   visible, the cluster ticks metrics only while handles exist. Rollouts tick on their
   own timer while one is in progress, in any workspace. **Accepted.**
7. Sidebar rows and the matrix read the runner and cluster (observed), not frozen
   catalog flags. `Deck::running`/`stop_all` move to the runner; follow is handled by a
   `views::toggle_follow` helper covering every following view. **Accepted.**
8. View keys live in each view's own key context with `&& !TextInput`; views that take
   keys are focusable, and the window focuses the active view of the focused pane
   (pane keys still work: views sit inside `Panes`). Esc in the tag editor reverts the
   unsaved tag. Keys are listed per view for the step 6 registry. **Accepted.**
9. States added: run stopping (and restart while stopping waits for the stop); service
   not deployed in this env and empty matrix cells; metrics forbidden (fake `lab`);
   logs with no pods; Save disabled during a rollout with the reason; unlock lasts
   until the view closes; closing a tab with an unsaved tag discards it (M1, noted).
   **Accepted.**
10. Crash toasts name the workspace; "View output" switches to it, then opens the run.
    **Accepted.**
11. Durations as constants, seeded metrics, output capped (5 000 lines), the server
    writes a line every 2 s, tests advance short spans only. **Accepted.**
12. Tests added: sampling stops when hidden/closed/switched away (counter), new views
    in the leak check, `ViewAddr` round trip, sidebar and matrix follow runs and
    rollouts, crash in an inactive workspace, `r`/`s` typed in the tag editor or find
    input do nothing. **Accepted.**

### Step 5 outcome

Built as assessed, in three commits.
- 5a: runner and the App and Run views.
- 5b: the fake cluster and the service inspector.
- 5c: logs, pod, branch matrix and port views; the matrix opens from the Cluster sidebar header.

Found while checking:
- A view built during the deck's update read the deck to check its visibility and panicked; the check is now deferred.
- The deploy confirmation cut off the command, so it now wraps.
- The pods table was clipped in narrow panes, so it now scrolls sideways.
- `docker compose up` counted as a secondary command, so `up` now counts as primary. An app is flagged unusable when none of its visible commands can run.

Checks:
- New tests: 7 in `tests/runs.rs`, 6 in `tests/service.rs`, 5 in `tests/cluster_views.rs`, plus core tests for apps, runs, the cluster and view addresses. All use short fake-clock spans.
- Snapshots 70–75 (app and run views), 80–89 (inspector states) and 90–93 (logs, pod, matrix, port) checked.

## Step 6 · Actions everywhere

Covers R5.1 (one registry), R5.2 (context menus), R5.3 (palette), R5.4 (hint bar,
cheat sheet), R3.11 (Send to…), R5.5 (reasons shown in menus and the palette).

Scope:
- **Registry** (`app/src/registry.rs`): `ActionSpec { id, label, group, keys, context,
  build }` for every action (layout, scale, settings, quit, workspaces, sidebar, views).
  Key bindings are generated from it; the per-module `bind_keys` functions go away.
  Labels and keys shown anywhere (menus, palette, cheat sheet, hint bar, tooltips that
  name a key) come from it.
- **Hint bar** (replaces the status bar): the actions available where focus is
  (GPUI `available_actions` on the focused path), most specific context first, as
  `key label` pairs, truncating; right side: workspace · env. Debug counters move to
  `DEVDECK_STATS` output only.
- **Cheat sheet** (`?` outside text inputs, `ctrl+/` anywhere): modal listing every
  registry action by group with its keys, a filter input, Esc closes.
- **Palette** (`ctrl+k`): modal at the top with an input and a virtualized result list.
  Sources: apps (open), app commands (run, with `@env` for one run), deployments of the
  current env (open service), envs (switch), workspaces (switch), views (matrix,
  settings), actions (available ones, with key). Fuzzy subsequence scoring in core
  (`fuzzy`: consecutive and word-start bonuses), ties alphabetical. Empty query lists
  actions then workspaces, alphabetical. Tab on an app drills into its commands;
  ↑↓, ⏎ runs, Esc closes (from a drill-down: back first). Disabled results show why.
- **Context menus** on every interactive element: tabs (close, close others, detach,
  send to…, maximize, collapse), pane strip (split right/down, collapse, maximize, send
  pane to…, close pane), sidebar rows (open, pin, send to…), app command rows (run,
  stop, restart, output), pod rows (open pod, logs), matrix cells (open, highlight),
  activity icons (show, hide sidebar), section headers (collapse/expand where it
  applies), workspace and env buttons (their menus). shift+F10 and the Menu key open
  the menu of the focused element (selected row, active tab).
- **Send to…** (tab or pane; menu, `ctrl+shift+s`): every pane of every window of the
  workspace gets a number badge (reading order, main window first) and a modal lists
  them as "n · window · active tab", plus "new window". Pick by number key, by clicking
  the list, or by clicking a pane's badge; Esc cancels. Sending a pane moves all its tabs.

States to snapshot: hint bar in panes / sidebar / app view / service view; cheat sheet
filtered; palette empty, with matches, drilled into an app, with `@35`; tab menu; pane
menu; row menu; Send to with badges in main and detached windows.

Tests: registry has unique ids and keys per context, every bound key maps to a
registry entry; fuzzy scoring order; palette sources and drill-down/back; `@env` run
uses that env once; Send to by number, by click, to a new window, pane with all tabs,
Esc cancels; shift+F10 opens the focused element's menu; hint bar lists the focused
view's keys; cheat sheet filter.

Out of scope: user-configurable keymaps (later), palette recents (open question).

### Step 6 review and assessment

1. Availability: handlers that don't apply aren't on the focus path at all. Sidebar,
   env, settings and Send to handlers attach only in the main window, and not while
   Settings is open. Palette items that can't run say why (missing tool…). **Accepted.**
2. Hint bar: built by walking the registry (not `available_actions`); an entry shows
   when its action is available and a binding matches the current context; the key text
   comes from the highest-precedence binding (skips shadowed keys). When the result
   changes after a frame (a new modal), the window re-renders once. **Accepted.**
3. A text input's Enter and Esc are the `text_input` actions (Submit, Cancel) in the
   registry, so they do show. **Rejected as stated** (they are actions).
4. The app registry also binds the `devdeck-ui` actions; `devdeck_ui::bind_keys` goes
   away so there is one source. Ids are unique strings (`sidebar.next`, `app_view.next`),
   several keys per entry are supported, workspace keys keep generic labels (names
   show in the palette). **Accepted.**
5. Key conflicts are tested against real context stacks (panes › app view, sidebar ›
   text input, modal), plus unique ids. **Accepted.**
6. Global keys don't fire under a modal or menu: app-wide entries use
   `!Modal && !Menu`, `?` also `!TextInput`. **Accepted.**
7. Send to: the pick lives in the Deck (badges in every window of the workspace, a list
   modal in the window it started from); clicking a badge in another window completes
   it; the pick cancels when its source goes away, a window closes, or the workspace
   switches. Numbering in core from the layout (main window reading order, then
   detached windows); keys 1–9, the list covers the rest. Not offered while Settings is
   open. **Accepted.**
8. The source pane is listed disabled ("already here"); "new window" from a single-pane
   detached window is disabled ("already alone"); layout errors (last pane of the main
   window) show their reason. Badges: on the rail of a collapsed pane, only visible
   panes while one is maximized (the list has all). Keys: ctrl+shift+s tab,
   ctrl+alt+shift+s pane, plus a pane-strip button. **Accepted.**
9. The palette lives in the main window (ctrl+k from a detached window brings the main
   window forward); it closes and restores focus before running its item, then
   dispatches. **Accepted.**
10. shift+F10 opens the menu of the focused thing at a computed spot (selected row,
    active tab of the focused pane); nothing inside text inputs; the Menu key is bound
    too but only shift+F10 is tested (headless can't prove the Menu key on Wayland).
    **Accepted.**
11. Palette empty state: this session's recent picks first, then actions, as DESIGN's
    open question answers; layouts join in step 7; title bar gets `search ctrl k` and
    `?`; detached windows drop the bottom bar (R3.8, nothing else). **Accepted.**
12. Step 6 tests go in one binary (`tests/actions.rs`); fuzzy, pane numbering and
    conflict checks are pure. **Accepted.**

### Step 6 outcome

Built as assessed. New: `registry.rs`, `palette.rs`, `cheat_sheet.rs`, `send_to.rs`,
core `fuzzy` and layout `numbered_panes` / `send_tab` / `send_pane`. The bottom bar of
the main window is now the hint bar; detached windows have none.

Found while checking:
- GPUI's `bindings_for_action` matches against the frame's own context stack, not the focus path, so the hint bar evaluates each entry's predicate with `depth_of` on the focused stack.
- Asking for the focus path before a window's first frame panics in GPUI; the hint bar skips the first render.
- The Send to list's backdrop blocked clicks on its own window's panes; it is now a floating panel, and the whole pane is the click target during a pick.
- The snapshot tool listed windows before a state's setup ran, so a window opened during setup leaked into the next state.

Checks: 7 tests in `tests/actions.rs`, plus 5 core tests. Snapshots A0–A8 checked: cheat sheet, palette (empty, matches, drill-down, `@33`, 150 %), tab menu, Send to in the main window and a detached one. Hint bars checked in the panes, the palette and the app view.
