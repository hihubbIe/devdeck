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
