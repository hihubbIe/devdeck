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
