# devdeck design notes

How devdeck is meant to look, behave and be built. Requirements live in
[REQUIREMENTS.md](REQUIREMENTS.md); this file explains the choices behind them.
The interactive HTML prototype that was approved is the visual reference
(published privately as a claude.ai artifact during the design phase).

## Window anatomy

```
┌ title bar ── workspace ▾ · layout ▾ ───── search (ctrl k) ───── env ▾ · ● 3 running · ✖ 1 · ? ┐
│ activity │ sidebar (list) │ split tree of panes ……………………………………………………………………………………… │
│   bar    │  Pinned        │ ┌ pane: tabs ───────────┐┌ pane: tabs ─────────────────────────┐ │
│          │  All           │ │ view                  ││ view                                │ │
│          │                │ ├ pane (collapsed) ─────┤│                                     │ │
│          │                │ │ tab · tab · tab       ││                                     │ │
├──────────┴────────────────┴─┴───────────────────────┴┴─────────────────────────────────────┤ │
└────────────────────────────────────────────────────────────────────────────────────────────┘
```

- **Activity bar**: one icon per activity contributed by the workspace (Apps; Cluster
  only when configured; Ports; Layouts). Clicking the active one hides the sidebar.
- **Sidebar**: the activity's list. Always Pinned + All, alphabetical, filter field.
- **Pane tree**: everything else. No fixed docks: "bottom output" and "pinned logs on
  the right" are just panes placed there.
- **No hint bar** and no key hints anywhere else in the UI: shortcuts are listed on one
  page (the cheat sheet), redesigned later.

## Visual language

- Monospace everywhere (JetBrains Mono, bundled so the look is the same on every machine).
- 13 px text, 24 px rows, 32 px tab strips, 1 px dividers, no shadows except floating
  menus, no rounded cards except a few grouped blocks.
- Color carries state only and is always paired with a glyph:
  `●` running/ready (green), `✖ n` failed with exit code (red), `⚠` missing tool or
  warning (amber), `◐` in progress (blue), `○` idle (muted), `⊘` protected (red).
- Envs use a distinct violet chip so the target env is always visible in labels:
  `start:dev @dev35`.
- Text truncates, never wraps (R4.4). Middle truncation for paths and image names.

### Theme tokens (dark, contrast raised from the prototype)

| token | prototype | shipped | use |
|---|---|---|---|
| bg | #111214 | #0f1012 | pane background |
| bar | #16171a | #15161a | tab strips, title |
| raised | #1b1c20 | #1c1d22 | menus, palette |
| line | #2a2c31 | #30333a | dividers |
| fg | #d4d5d8 | #e1e2e5 | text |
| mute | #80838b | #8d9099 | secondary text |
| dim | #5d6068 | #666a73 | hints |
| accent | #5b9bd5 | #64a6e3 | focus, links |
| ok / warn / err | #5fb36b / #d9a441 / #e06c6c | #67c174 / #e3ad48 / #eb7474 | state |
| env | #c3b4f2 on #2b2540 | #cbbdf7 on #2d2645 | env chip |

Themes are TOML files with these tokens; the app ships `devdeck-dark`.

## Architecture

Cargo workspace:

```
crates/
  devdeck-core      no UI. Config, workspaces, discovery, process runner, tool detection,
                    actions registry, layout model (pure data), fuzzy search. Fully unit tested.
  devdeck-kube      cluster access (kube-rs), deploy journal. Optional at runtime.
  devdeck-ui        shared components + theme. The only place that styles anything.
  devdeck-views     one module per view type (app, run output, logs, service, pod,
                    text, matrix, ports…), each registered in a view registry.
  devdeck-app       the binary: windows, pane tree rendering, activities, palette, menus.
```

Rules:
- Views never draw raw styles; they compose `devdeck-ui` components.
- The layout model (`core::layout`) is plain data with pure operations
  (split, move, collapse, detach, close, serialize). The app renders it. This keeps the
  hardest logic unit-testable without a GPU.
- Every user-facing operation is an `Action` in the registry (R5.1). Buttons, menus,
  palette entries and key bindings all invoke actions by id.

### Layout model

```
Layout { version, windows, panes, focus }
Window { id, root: Node, maximized: Option<PaneId>, focused: PaneId }
                                   main window, plus one per detached tab
Node = Split { id: SplitId, axis, children: [(Node, weight)] } | Pane(PaneId)
Pane { tabs: [ViewRef], active, collapsed: bool }
ViewRef = a serializable address, e.g. app/<id>, run/<id>, svc/<env>/<name>,
          logs/<env>/<name>, pod/<env>/<pod>, file/<path>, matrix, ports
```

- A view is open at most once across all windows.
- Weights are fractions of their split, renormalized after every operation; the app
  converts pixel drags to weights and enforces a minimum pane size in pixels.
- A detached window's root is always a single pane. Its title bar has ⇲ (send the
  tabs back to the main window's focused pane) next to × (close the window and its
  tabs).
- "Send to…" moves a tab or a whole pane to any pane of any window, or to a new
  window: number badges appear on every pane, pick by key or click.
- Each window has a focused pane; focus moves by pure rules after close, move and
  detach, and directionally using pane rectangles computed from the model.
- View instances (GPUI entities) are owned by an app-level registry keyed by `ViewRef`,
  so they survive moving between panes and windows and are dropped on close.

- Moving a tab onto a pane edge replaces that pane with a split.
- Removing the last tab removes the pane and collapses the split (except the last pane
  of a window).
- Collapsed pane: in a column split it is its 32 px tab strip; in a row split it
  becomes a 28 px rail of tab monograms (GPUI cannot rotate text), full titles as
  tooltips. The last expanded pane of a window cannot collapse.
- Detach: a new `Window` whose root is a single pane holding the tab. Detached windows
  belong to the workspace that created them and hide when you switch workspace.
- Layouts are snapshots of the main window tree plus detached windows.

### Workspaces

- Each workspace has its own config file, state, process table, layout and windows.
- All workspaces are loaded at start; only the active one renders. Processes of
  inactive workspaces keep running; their output keeps buffering.

### Toolchains

- Everything about one language or tool (Node, Cargo, Go, Taskfile, Make, compose,
  VS Code launch configs) lives in its module under `devdeck-core/src/toolchains/`,
  behind one `Toolchain` trait: what it reads and finds in a project folder, which
  folders it keeps to itself, its monorepo packages, which of its commands others
  already run, the programs it provides, what it puts first on a run's PATH, and
  whether the project is installed. Discovery, the watcher and the tool check only go
  through the trait and the list `toolchains::ALL`.
- What a toolchain keeps about an app (`Facts`) or the machine (`Machine`) is one
  field per toolchain, so values stay plain, comparable data. Shared pieces sit next
  to the trait: reading a project folder, shell scripts into programs.
- Adding a toolchain: a module, a line in `ALL` (its place decides an app's kind), and
  its `Facts`/`Machine` fields and machine config key when it keeps anything.

### Processes

- Each run is a child in a pseudo-terminal opened with rustix (the mechanism
  `alacritty_terminal`'s `tty` uses, without its drawbacks: it passes devdeck's own
  environment on, ignores a missing folder and blocks when dropped), started as
  `/bin/sh -c <line>` with exactly the run's environment, in its own session and process
  group. One thread per run reads output, reaps the child (pidfd) and takes input.
- A driver reports output bytes and the exit; the runner owns one terminal per run
  (`core::terminal` over alacritty_terminal, 2 000 lines of history), so output is kept
  while no view shows it. The run view draws only visible rows, wrapping at the pane
  width; the terminal is resized to the pane.
- Quit asks when anything runs (R8.1) and kills the groups.
- Env applied through the app's declared binding (variable, argument, file rewrite).

### Cluster

- kube-rs on a background tokio runtime; results reach the UI over channels.
- Watches deployments and pods per visible env; metrics are sampled only for views
  that are visible.
- The user's RBAC cannot read ReplicaSets or events, so deploy history is kept by
  devdeck in its state folder, and failure reasons come from pod container states.
- Writes: patch the container image, rollout restart, scale. Protected envs need an
  unlock (R7.6).

## Chosen libraries (to be pinned in M0)

| need | choice | why | notes |
|---|---|---|---|
| UI | GPUI (Zed) via `gpui-pre =0.3.11` + `gpui-pre-platform` (wayland, x11) | GPU text rendering built for an editor, idle at zero CPU, multi-window | crates.io `gpui` is stale (0.2.2, Oct 2025); gpui-pre is a published Zed snapshot, the one gpui-component pins |
| widgets | own components; may borrow from gpui-component | full control of the look | gpui-component has docks, inputs, menus |
| terminal grid | alacritty_terminal | same as Zed's terminal | own view on top |
| pty | rustix (`pty`, `termios`) | own session per child, exact environment, real exit status with the signal | portable-pty dropped (M2): no signal number, SIGHUP to one pid only; alacritty's `tty`: inherited environment |
| YAML | serde-saphyr 2 | safe Rust, maintained | serde_yaml archived, serde_yml unsound |
| JSONC | jsonc-parser 0.34 | VS Code's leniency, serde | launch.json |
| fuzzy | nucleo | Helix's matcher, very fast | |
| kubernetes | kube + k8s-openapi | native watch, exec, port-forward, logs | needs tokio |
| config | serde + toml | | |
| file watching | notify 8.2 + notify-debouncer-full | discover new folders | non-recursive watches; drop access events |

## Settings and scaling

- Settings open as a full-window view from a title-bar button, not as a tab.
- UI scale multiplies every theme size (text, rows, tab strips, rails, icons). It is
  applied on top of the desktop's scale factor, which GPUI already honours.
- Global, per-machine settings live in `~/.config/devdeck/settings.toml`; per-workspace
  choices live with the workspace.

## Decisions log

- 2026-11-07 Native Rust over a web app: user wants native, fast, low memory.
- 2026-11-07 GPUI over iced/egui/Slint: best text rendering and editor-grade
  performance; risk is API churn (pinned commit mitigates).
- 2026-11-07 Own layout engine instead of a dock widget: detach windows and
  collapse are central and must be testable as pure data.
- 2026-11-07 No git or editor features (user decision).
- 2026-11-07 Lists are Pinned + All, alphabetical; no "mine", no "recent" (user decision).
- 2026-11-07 Separate workspaces, one shown at a time, others keep running (user decision).
- 2026-11-07 No tab drag between windows; "Send to…" with numbered panes instead, plus
  ⇲ re-attach and × close on detached windows (user decision after the M0 finding).
- 2026-11-07 Settings as a full-window view; UI scale is the first setting, early in M1
  (user decision).
- 2026-11-07 The theme is global, like the UI scale, not per workspace (user decision).
- 2026-11-07 A list row dragged onto a pane opens its item there (into the pane or at an
  edge). GPUI shows the pane drop zones for any drag, so rows use them instead of
  hiding them.
- 2026-11-07 Deployment pins are per service name and shared by every env; pins of
  items that no longer exist stay in the state file and are not shown.
- 2026-11-07 Context menus open with `window.prevent_default()`, otherwise GPUI hands
  focus back to the focusable ancestor of the clicked element as the mouse down bubbles.

- 2026-11-07 Shortcuts are not shown in the UI (menus, tooltips, labels, hint bar); one
  page lists them (user decision: it clutters). Layout errors, shown in the hint bar
  before, are toasts, as is a detached window that fails to open.
- 2026-11-07 Window resize edges are computed from the drawn size and offered only on
  sides the compositor doesn't hold (maximized, fullscreen and snapped sides have none):
  GPUI's bounds of a maximized window are its restore size on Wayland, which made most
  of a maximized window behave as a resize border. Each edge has its own hitbox, so the
  cursor follows the pointer without a redraw. The window size is remembered only while
  no side is held, so a maximized or snapped size never becomes the size to reopen with.
- 2026-11-07 In a focused terminal a key goes to the process when it has terminal bytes
  (text, ⏎, ⌫, Tab, Esc, arrows, Home/End, PgUp/PgDn, Ins/Del, F1–F12, ctrl+letter and
  ctrl+`[\]^_/@`); `alt+…`, `ctrl+shift+…` and keys without bytes (ctrl+tab, ctrl+digit,
  ctrl+`=`/`-`/`,`, menu) stay devdeck's. One rule, derived from the encoder
  (`core::terminal::key_bytes`): every binding whose key a terminal takes is off inside
  `Terminal` (`registry::terminal_takes`), with no per-binding list. So in a terminal
  ctrl+k, ctrl+w, ctrl+q, `?` and the run view's ⏎/s/r reach the process.
- 2026-11-07 Devdeck keys from a terminal: ctrl+shift+⏎ run, ctrl+shift+x stop and
  ctrl+shift+r restart (second keys of the run actions, valid in and out of the
  terminal); ctrl+shift+c / ctrl+shift+v copy and paste (Linux terminal habit);
  ctrl+shift+esc leaves the terminal; the palette gets ctrl+shift+p as a second key, so
  every action stays reachable from a terminal. Collapse pane moves from ctrl+shift+c
  to ctrl+shift+h to free copy.
- 2026-11-07 A run view's pane focuses the run view, so its single keys work; a click on
  the terminal or `i` gives the keys to the process. The cursor shows only while the run
  runs and its terminal has the keys (and the program hasn't hidden it).
- 2026-11-07 A terminal is drawn by one canvas that shapes and paints the visible rows
  (no element per row), and streaming output redraws at most every 33 ms except within
  half a second of typing (echo shows at once). Together: 20.4 % → 10.9 % CPU at
  3 × 2 000 lines/s, one shown (`scripts/measure.py terminal`).
- 2026-11-07 The view keeps its scroll position and selection in rows counted from the
  first row the run wrote (`Terminal::dropped`), so both stay on their text while old
  rows fall off the 2 000-line history; a restart or a resize (rows rewrap) clears the
  selection. The wheel scrolls the history, or sends cursor keys on a full-screen
  program's screen when it asks for that (alternate scroll).
- 2026-11-07 Colour and size queries are answered with the theme and the cell size, one
  look for every terminal (`core::terminal::set_look`, set by the view): the theme and
  the UI scale are global, and a restarted run's fresh terminal answers right away.
- 2026-11-07 When several views show one run, the one focused last sizes its terminal.

## Open questions

- 2026-11-07 Views stay in `devdeck-app/src/views/` for M1 (one module per view, the
  registry in `views/mod.rs`); they reach their workspace through `ViewCx`. Runs and
  the cluster sit behind stable entities (`Runner` with a `RunDriver`, `Cluster`) whose
  M1 drivers are fakes, so M2/M3 swap drivers, not views.
- 2026-11-07 View addresses are typed (`core::view_addr::ViewAddr`) and escape `/` in ids;
  unknown or old refs open a message view instead of breaking a saved layout.
- 2026-11-07 Live work follows visibility: metrics sampling and the fake log feed run only
  while their view is on screen (active tab, expanded pane, shown workspace); rollouts
  and runs keep going regardless.
- 2026-11-07 One action registry (`devdeck-app/src/registry.rs`) binds every key, the
  shared components' too; menus, palette and the cheat sheet read labels from it,
  and only the cheat sheet shows keys. App-wide keys don't fire under a modal or a menu.
- 2026-11-07 "Send to…" shows a floating list without a backdrop, so every pane (in
  every window) stays clickable as a target while the pick runs.
- 2026-11-07 The deploy confirmation shows the whole command, wrapping if needed: the one
  place a view opts into wrapping (R4.4), so the deployed tag is never cut off.
- Should the palette's empty state show recent actions (ranking by use), given lists
  are alphabetical? Current answer: yes, the palette is a search tool, not a list.
- 2026-11-07 Envs belong to the workspace (catalog), not the cluster: runs use them
  without a cluster. App ids are the folder relative to the workspace folder; command ids
  are `source:name` (`task:run`), and the UI shows the name.
- 2026-11-07 Runs follow the real exit: a stop asks the driver and the run is Stopped
  when the process exits (no failure toast); late events of an earlier start are
  ignored. Each start gets a fresh terminal.
- 2026-11-07 Machine paths (Node folders, PATH additions) live in `machine.toml`, written
  only by the user; devdeck rewrites `settings.toml` itself, which would drop comments and
  anything it can't parse.
- Running section in the Apps sidebar: replaced by the row indicator and a filter.
  Revisit if it feels slower in use.
