# devdeck design notes

How devdeck is meant to look, behave and be built. Requirements live in
[REQUIREMENTS.md](REQUIREMENTS.md); this file explains the choices behind them.
The interactive HTML prototype that was approved is the visual reference
(published privately as a claude.ai artifact during the design phase).

## Window anatomy

```
┌ title bar ── workspace ▾ · pulse ▾ ─── search (ctrl k) ── env ▾ · ● 3 running · ? ─────────┐
│ activity │ sidebar (list) │ split tree of panes ……………………………………………………………………………………… │
│   bar    │  Pinned        │ ┌ pane: tabs ───────────┐┌ pane: tabs ─────────────────────────┐ │
│          │  All           │ │ view                  ││ view                                │ │
│          │                │ ├ pane (collapsed) ─────┤│                                     │ │
│          │                │ │ tab · tab · tab       ││                                     │ │
├──────────┴────────────────┴─┴───────────────────────┴┴─────────────────────────────────────┤ │
└────────────────────────────────────────────────────────────────────────────────────────────┘
```

- **Activity bar**: one icon per activity contributed by the workspace, in this order and
  on ctrl+1…6: Pulses (where a workspace lands, and where an unknown or unavailable saved
  activity goes), Apps, Cluster (only when configured), Ports, Terminals, Notes. Clicking
  the active one hides the sidebar.
- **Sidebar**: the activity's list. Always Pinned + All, alphabetical, filter field.
  Both sections fold from their header. All holds the groups the user makes (folders,
  nested, first at each level, each list but Terminals and Notes; Cluster groups shared by every
  env, as pins are): an item, or a folder of the Apps tree with all it holds, goes in
  by a drop on the group or "Move to group ▸", and keeps its real path for opening and
  selection. Dropping on a row lands in its group, on the All header at the top; a
  pinned item keeps its group (unpinned on a row at the top, it goes back to it). The sidebar's right edge drags its width (200 px to half
  the window, one width for the app; a double click resets it).
- **Pane tree**: everything else. No fixed docks: "bottom output" and "pinned logs on
  the right" are just panes placed there.
- **No hint bar** and no key hints anywhere else in the UI: shortcuts are listed on one
  page (the cheat sheet), redesigned later.

## Visual language

- Monospace everywhere (JetBrains Mono, bundled so the look is the same on every machine).
- 13 px text, 24 px rows, 32 px tab strips, 1 px dividers, no shadows except floating
  menus, no rounded cards except a few grouped blocks.
- Color carries state only and is always paired with a mark: a filled dot running or
  ready (green), the error icon and the exit code when failed (red), the warning
  triangle for a missing tool or a warning (amber), the turning loader while a run
  stops (a static one for longer things in progress: a rollout, loading), a ring when
  idle (muted), a lock for a protected env (red), a check for a success.
- Icons are Lucide's outline set (R9.5), drawn in one theme color at `t.icon` (14 px
  before the UI scale); buttons brighten their icon on hover. Status dots are shapes,
  not icons.
- Envs use a distinct violet chip so the target env is always visible in labels:
  `start:dev @dev35`.
- Text truncates, never wraps (R4.4). Middle truncation for paths and image names.
- Nothing changes size or moves with a state (R9.10): hover, selection, focus, a press, a
  drag over, a toggle. Their marks are colors, or lines drawn over the element (an
  absolute child, like the pane header's focus line; a `drag_over` style, which GPUI
  draws without laying it out), never a border, a child or empty room added for them;
  but a toggle's check keeps its room on or off (a menu row's, the settings button's).
  A menu's disabled item is only dimmed (R5.5).
- A menu's toggle (`MenuItem::toggle`: on or off) keeps the menu open when used, by
  click or Enter, its check flipped, so several can be changed in a row (R5.8); a pick
  (`checked`: one of several) and an action close it.
- Scroll containers are `devdeck_ui::Scrolls` (`scroll_x`, `scroll_y`, `scroll_xy`) and
  lists `devdeck_ui::scroll_list`: a trackpad gesture keeps the axis it starts on (GPUI's
  `restrict_scroll_to_axis`, precise deltas only, so Wayland), nothing drifts diagonally
  (R9.6); sideways-only areas take shift+wheel. A terminal filters its own wheel events
  the same way (`gpui::OngoingScroll`). `clippy.toml` keeps GPUI's own scroll methods and
  `uniform_list` out of the app.

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
  devdeck-kube      cluster access (kube-rs): kubeconfigs folder, targets, deployment
                    watches on one tokio runtime started on first use. No UI.
  devdeck-ui        shared components + theme. The only place that styles anything.
  devdeck-views     one module per view type (app, run output, logs, service, pod,
                    text, matrix, ports…), each registered in a view registry.
  devdeck-app       the app: windows, pane tree rendering, activities, palette, menus.
  devdeck-sys       the OS layer (R11.6): the only code that calls the OS, one backend per
                    OS chosen by the target (docs/PLATFORMS.md §4).
  devdeck           the binary: `main.rs` only, starting devdeck-app (a package of its
                    own so test builds don't link it).
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

- Moving a tab onto a pane edge splits that pane (`Drop::Edge`); onto a column's or
  row's edge it lands beside the whole split (`Drop::Around`, R3.13). Either takes half
  of what it lands beside: a sibling when the parent runs along the edge, a new child of a
  split that does (weighted as its expanded children), else the target wrapped in a new
  split. A drop beside clears maximize.
- Drops are one layer under the panes (painted first, so tab strips and headers above
  it keep their own drops) while a tab, pane, list row or file is dragged. Where it
  lands is `Layout::drop_at` (pure, from the tree and `Metrics`): a body's centre is
  into the pane, its quarter bands its edges, and at the very edge one 16 px strip per
  enclosing split sharing that side (2/3 of the band at most), the outermost at the edge.
  The highlight is `Layout::preview`, the drop done on a copy, so it is the real result.
- Removing the last tab removes the pane and collapses the split (except the last pane
  of a window).
- A pane that goes away gives its space whole to one sibling, so merging and splitting
  use the exact space (user, 2026-11-08): the sibling holding the pane it merged into or
  lands beside (when expanded), else the expanded one before it, else after, else any.
  Beside a whole split, or closed: the one before. A collapsed pane a merge opens takes
  the space instead of adding to it. A pane moved beside a pane of its own split, along
  it, only changes place, with its own size. Limits: the freed divider (and a collapsed
  pane's rail) is shared by the siblings, as weights know no pixels; a collapsed pane
  opened by a merge inside a nested split keeps its old share there.
- A pane with one tab or none has a header instead of a tab strip (R3.12): same 32 px,
  same `bar`, so bodies stay aligned. Grip, the view's title (fg, truncated, no tab
  shape), the pane buttons, then a 1 px `line` and × (its own box: a narrow pane clips
  the buttons first, never ×). Focus is a 2 px accent line drawn over its top (not a
  border, so nothing moves). An empty pane's header has no title.
- Collapsed pane: in a column split it is its 32 px tab strip or header; in a row split it
  becomes a 28 px rail of tab monograms (GPUI cannot rotate text), full titles as
  tooltips. The last expanded pane of a window cannot collapse.
- Detach: a new `Window` whose root is a single pane holding the tab. Detached windows
  belong to the workspace that created them and hide when you switch workspace.
- Pulses are snapshots of the main window tree plus detached windows. "Pulse" is the
  user's word, the only one the app, the API and MCP show (R3.10); in the code a pulse is a
  `Layout`, kept with its name in the `LayoutBook`.

### Workspaces

- Each workspace has its own config file, state, process table, pulses and windows.
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

- Each run is a child in a pseudo-terminal opened by the OS layer (`devdeck_sys::child`;
  the mechanism `alacritty_terminal`'s `tty` uses, without its drawbacks: it passes
  devdeck's own environment on, ignores a missing folder and blocks when dropped),
  started as `/bin/sh -c <line>` with exactly the run's environment, in its own session
  and process group. One thread per run reads output, reaps the child (on Linux through
  a pidfd) and takes input.
- A driver reports output bytes and the exit; the runner owns one terminal per run
  (`core::terminal` over alacritty_terminal, 2 000 lines of history), so output is kept
  while no view shows it. The run view draws only visible rows, wrapping at the pane
  width; the terminal is resized to the pane.
- Each run (not a terminal's shell) starts in a systemd user scope of its own
  (`devdeck_sys::scope`: `systemd-run --user --scope`, keeping its pid, session, terminal and
  exact environment), so the kernel keeps every descendant whatever it does (`setsid`,
  a daemon's double fork, `env -i`). A stop empties the scope (^C to the group, SIGTERM
  to the scope through pidfds, then `cgroup.kill`). A run that ends on its own ends its
  group; what it left in its scope (a build's daemon) runs until devdeck quits.
  Without a systemd user manager and cgroup v2, a run follows its process tree instead
  (what leaves it with a double fork is then out of reach).
- A terminal's shell ends with its session only: what was detached from it on purpose
  (tmux, `setsid`) stays, as with any terminal.
- Quit asks when anything runs (R8.1) and kills the groups and this devdeck's scopes.
  The state folder's group log records groups and scopes, so after a crash or `kill -9`
  the next start offers what is left, one row per run (R8.2).
- Env applied through the app's declared binding (variable, argument, file rewrite).

### Cluster

- A workspace's `[cluster]` names a folder of kubeconfigs (a git repository of them);
  every kubeconfig at its top is read. Each env is one context (its own name, or the one
  `[cluster.contexts]` gives); the namespace comes from the context. Files that don't
  parse are workspace problems; a context in two files is one only for an env using it.
- The `Cluster` entity holds what views read; a `ClusterDriver` feeds it, as `Runner`
  and `RunDriver`: the fake (demo, tests, snapshots) or kube-rs (`kube_cluster.rs` over
  `devdeck-kube`).
- kube-rs on one tokio runtime (started on the first watch, so a workspace without a
  cluster pays nothing); results reach the UI over channels. One client per kubeconfig
  file, cluster and user; connect timeout 5 s.
- An env is watched (kube-runtime's watcher on deployments) from the first time a view
  asks for it until the workspace's cluster changes or goes. A relist replaces the list;
  an update is sent only when the converted deployments changed (status noise renders
  nothing). After an error the watcher is dropped and a new one lists again after a
  backoff (1 s doubling to 30 s): kube-runtime's own retry resumes silently, so a
  problem would never clear. A lost connection keeps the last list with the problem.
- The kubeconfigs folder is watched on its own route: a change reloads it, and only envs
  whose target changed (another context, a changed file such as a rotated token)
  reconnect. A config change goes to the same cluster and driver: envs change in place,
  a removed `[cluster]` empties the cluster (views keep it) until it comes back.
- Each watch lasts 60 s on the server, so a connection that died silently shows as a
  problem within about a minute. A paused or stuck rollout is said as such.
- Pods are watched only for whoever holds an interest handle: a service or pod view for
  its deployment (filtered by the deployment's label selector), the Cluster list's
  **watch** toggle for its whole env (off by default, saved with the sidebar). The
  env-wide watch replaces the services' own once it has listed. Pods belong to the
  deployment that owns their ReplicaSet (`ownerReferences`, no ReplicaSet permission
  needed), grouped once per update. A dropped handle tells the cluster over a channel,
  no timer.
- "Running since" comes from the pods when known: the earliest start among running pods
  of the newest template; a scale keeps it, a new version moves it.
- Metrics (`metrics.k8s.io`) are read per env while a service or pod view of it is
  visible: at once, then every 15 s, one read at a time, 10 s at most; a 403 or 404 stops
  the reading until the env's target changes. Usage is joined at read time.
- Logs (`devdeck_kube::logs`, `views/logs_feed.rs`): one stream per pod of the service
  (its main container, or one picked, init containers too) while the logs view is on
  screen; the view holds its service's pods, and a container's state decides: running or
  ended streams, waiting says its reason and asks nothing, an ended stream starts again
  when the container restarts. Lines carry the server's timestamp (nanoseconds): a resume
  (shown again, target changed, pod watch back, a dropped connection) asks from the last
  line's second and skips the lines at exactly its time already shown, so nothing is lost
  or doubled (kube-core rounds `sinceTime` to the second). The first lines of streams
  started together are merged by time; updates are drained once a frame; lines are cut at
  4 KB, 10 000 lines and 16 MB kept; level from JSON (`level`, `lvl`, `severity`, pino's
  numbers) or the text. A stream silent for 5 minutes resumes (no read timeout otherwise).
- Deploys (`cluster_deploys.rs`, `devdeck_kube::set_image`): the main container's image
  only (refused when several containers run and none is named like the deployment), as a
  JSON Patch that tests the image confirmed is still there, then replaces it, with field
  manager "<short name> via Kugelblitz"; confirmed in its modal (R5.6); refused if no short name is set
  (Settings › Deploys: one printable word, at most 32 characters; no default), the account may not (a
  `SelfSubjectAccessReview` asked when a service view opens, per env and log
  generation), a protected env isn't unlocked, a write of the service is in flight, or
  its rollouts are paused. A failing rollout never blocks the next deploy.
- Redeploy (`devdeck_kube::restart`, what `kubectl rollout restart` does, not kube's
  `Api::restart`): stamps the pod template's `kubectl.kubernetes.io/restartedAt` with a
  fractional-second time, pinned to the image shown (and to the generation when the whole
  annotations map is added); devdeck's own is known by that stamp; the confirmation
  warns when the main container's effective pull policy won't fetch a newer build (or the
  image is pinned to a digest). One write per service until the watch has seen it.
- Every deploy is spotted on the deployment watch (a growing revision, or a changed
  image), whoever made it, recorded as written (the field manager this window sent, else
  the last manager of the pod template in `managedFields`, else "seen"), and followed (`cluster::assess`): nothing
  counts before `observedGeneration` reaches it; done once settled with the new
  template's pods ready; failing (keeps following) on an image that can't be pulled, a
  crash loop from 2 restarts, a container that can't be made, an unschedulable pod, a
  `ReplicaFailure`; stuck past the deadline; replaced by a later deploy. devdeck's own
  deploys hold their service's pods while followed.
- Who deployed (user, 2026-11-11): Kugelblitz is devdeck's name on the cluster; every write
  carries "<short name> via Kugelblitz", so the history names a colleague's deploys the
  same way ("j.doe via Kugelblitz"). The user's own are theirs by that name (from any
  machine) or by the old `devdeck` of history kept from before: they never notify that
  they began and always say how they ended; a colleague's follow the notify ticks. The
  demo deploys as "demo" when Settings names nobody.
- The user's RBAC cannot read ReplicaSets or events, so the deploy history is kept by
  devdeck (`deploys.json` in the workspace's state folder, the last 10 per service), and
  failure reasons come from pod container states. A deploy left open when devdeck closed
  is followed again if its image still runs, else ends "unknown".

### Dangerous actions

- What is hard to undo (R5.6) runs only once confirmed in its own modal, saying in plain
  words what will happen: a deploy or a redeploy (and replicas when built), a git replace
  push or abort (not a bisect's), stopping a process devdeck didn't start, deleting a
  note (and a shell in a pod when built). No global switch (edit mode was removed,
  2026-11-11); nothing marks them in the UI.
- In the code (`devdeck_app::danger`): each takes a `Confirmed`, which only a
  `DangerModal` hands out (a sealed trait, its modals listed in `danger.rs`), so calling
  one without its modal doesn't compile: `Cluster::deploy`/`redeploy` and the service
  view's, `Repo::run_confirmed` (`Repo::run` refuses a dangerous op, `Op::dangerous`
  decides: checked when it runs), `port_scan::stop_process`, `Notes::delete`.
  `clippy.toml` bans calling what lies under them (`Git::run`, `process::terminate`, the
  cluster driver's `set_image` and `restart`) and the tests' `danger::for_tests`
  anywhere else; each allowed call says why.
- A danger modal opens on Cancel (`danger::show`) and acts on Enter only from its action
  button: a stray Enter cancels. The git modal does so only for a dangerous op.
- Not dangerous: fetch, switch, pull, new branch, publish and plain pushes, stopping
  devdeck's own runs, ending terminals, logs, port-forward, the leftovers dialog at start
  (devdeck's own processes), deleting a pulse. Commands and terminals can still run
  anything themselves (kubectl, deploy scripts): this guards devdeck's own actions.
- devdeck never pushes `main` or `master` (any case), confirmed or not. A remote whose
  default branch has another name (`develop`, `trunk`) isn't protected by that rule: a
  known limit (plain pushes to it stay allowed; a replace still asks).

### Git

- Through the `git` program (`devdeck-core::git`), not a library: the user's keys, agent,
  credential helpers, hooks and config behave as in their terminal. Each call in its own
  session (no terminal, so nothing can prompt), stdin closed, `GIT_TERMINAL_PROMPT=0`,
  stderr and exit status kept; reads take no optional lock and time out at 10 s, fetch at
  60 s and push at 120 s (SIGTERM, then SIGKILL); what touches the working tree (switch,
  merge, abort) is never killed.
- Writes are few and never forced: switch (`--no-overwrite-ignore`, `--no-guess`), track
  a remote branch, create a branch (`check-ref-format` plus refusals of `-…`, `@`,
  `HEAD`), pull as `fetch` then `merge --ff-only --no-autostash --no-overwrite-ignore`
  (never `git pull`, which follows the user's rebase and autostash config), push, abort.
  An operation under way (rebase, merge, cherry-pick, revert, bisect) refuses every write
  but its abort, in the core, not only in the UI.
- Push decides after fetching: publish, a plain push, or replacing a rewritten branch only
  when the remote's commits are in the branch's reflog (`--force-with-lease` pinned to the
  tip just fetched, plus `--force-if-includes`); a remote with commits never had is
  refused. Never a force push that can lose anything.
- Every write is confirmed in plain words in the panel (fetch isn't: it changes no branch
  or file). One `Repo` entity per root, shared by its views, read only while one is
  visible, when its git dir changes (the app's watcher) or after an operation.
- A service's app: by its deployment's or image's name (normalized), a project's
  `.gitlab-ci.yml` `IMAGE_NAME`, or the app's `deployments` override.

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
| editor, Markdown | gpui-component `=0.9.0` (`Editor` on `EditorState`, `TextView::markdown`), tree-sitter grammars: rust, go, typescript, tsx, javascript, json, yaml, toml, markdown, bash, css, html, python, make, diff, sql | pins `gpui-pre =0.3.11` like us; what Zed-style highlighting needs without Zed's editor | only inside `devdeck-ui` (R9.3); +21 MB binary (12 the library, 9 the grammars), see the spike in plans/files-and-notes.md |
| icons | Lucide (lucide-static 1.61.0, ISC), only the icons used, in `devdeck-ui/assets/icons` | consistent outline set; what Zed and gpui-component start from | built in (`Icon` enum, `include_bytes!`), served by `devdeck_ui::Assets`; adding one = its SVG from the same release + a variant |

## Files and notes

- gpui-component is used only inside `devdeck-ui` (`code.rs`): a read-only code view, the
  note editor and the Markdown view. `gpui_component::init` runs at start (its input's
  own init is private); our theme is copied into its theme with `Theme::update` after a
  single `Theme::change(Dark)`, at start and on every theme or scale change; its highlight
  theme is its dark one with our editor colors. No `Root`.
- Highlighting is most of the memory (a 10 MB file: 567 MB highlighted, 163 MB plain), so
  only files up to 1 MB with lines under 5 000 characters are highlighted.
- The editor adds both axes of every wheel event; the wrapper takes the wheel first and
  locks the axis (R9.6), as the terminal does. Right-click (and the menu key, at the
  caret) opens the view's menu: the editor gives its own items (the note's Cut, Paste;
  Select all) and the selection, and the file and note views open the shared
  `context_menu` with them, so Copy and Share with… work as from a terminal (R5.7). The
  rendered Markdown (`Rendered`, over gpui-component's `TextViewState`) does the same with
  its selection.
- Keys: the note editor's key context is `NoteEditor`. A devdeck binding whose key it uses
  (`code::editor_takes`: no modifier but shift, or one of its own ctrl/alt keys, checked
  against its keymap by a test) is off inside it, as for terminals; ctrl+w, ctrl+tab,
  alt+arrows still work while typing. gpui-component's inspector key is dropped; its
  other keys live in its components' contexts. Its find bar's icons come from its own
  Lucide set (`gpui-kit-assets`), served by our `Assets` after ours.
- Views can say more than their address (`views::Live`): a tab's title (a note's name)
  and the actions they offer now (`ViewAction`: a registry id, an icon, on or off). The
  deck keeps them per view and redraws when such a view changes; every title goes through
  `Deck::title` (tabs, headers, "Send to…", places). The actions are drawn in the header
  (or strip, for the active tab) before the pane's buttons and listed first in the tab's
  menu; a click brings that pane and tab forward and focused, then dispatches the action.
- A file view (`file/<path>`) shares an `OpenFile` per path with its workspace
  (`Services::file`); the app's watcher watches the closest existing folder of each, on its
  own route like git dirs (no rescan). It reads in the background, only while shown, at
  most once a second; the editor is made on the first draw (gpui-component's editor needs
  a window, views are built without one).
- Files dropped from outside (`ExternalPaths`) land through the same drop layer and tab
  strips as tabs and list rows (`drops.rs`).
- Notes live in `~/.local/share/devdeck/workspaces/<id>/notes/` (`index.json` and one
  `<id>.md` each; `DEVDECK_DATA_DIR` moves it), owned by a `Notes` entity per workspace
  (`Services::notes`) that holds their text: a note view only edits it there, so closing
  a tab or quitting never loses typing. It writes 500 ms after the last change (small
  atomic writes, on the UI thread, so never two at once) and everything from
  `Shell::flush`. The demo's notes stay in memory. The note view hands focus on to its
  editor when it's drawn focused (the editor is made on the first draw); Esc in the
  editor gives the keys back to the view until the view loses focus.
- A note without a project has an empty app in the same index (older indexes read the
  same); its tab is named by its name alone. The sidebar's Notes list is the generic list
  (`core::list`) fed by `core::notes::tree`: a project is a folder row one segment deep
  (`notes::project_node`, its app id with `/` replaced), named by the list's `names` and
  put before the notes without a project by `folders_first`; no groups (notes don't
  move), and a drop only pins or unpins.

## Control API

R13, `plans/control-api.md`; phase 1 built (`plans/control-api-p1.md`): the observe
methods, `devdeck-ctl`, `devdeck-mcp`, the API section in Settings.

- Socket names follow the state folder: `main.sock` for the default one, `demo.sock`
  for the demo's, else `s-<hash>.sock`; each guarded by its own lock, so a second devdeck
  never takes over a socket a first one serves.
- The socket's threads hold the channel to the UI thread weakly; `ApiState` holds it,
  so it closes on the UI thread (GPUI's test scheduler refuses wake-ups from other
  threads). For the same reason the tests drive the UI side (`api::open`, `api::call`)
  directly, and the transport's tests use a real socket with a stub sink.
- A caller is fixed when it connects and checked on every call: a terminal caller's
  shell must still be the same run (pid and start count), else `expired` and the
  connection closes.
- Outside callers reach `api.describe`, `devdeck.info`, `session.hello` and
  `session.whoami` only, until the user can raise one (phase 4).
- `devdeck-mcp`: rmcp 3.9.0 with a hand-written handler (no macros); one tool per class
  the caller may use, built from `api.describe`, or from the catalog compiled in when
  devdeck is down.

- Pieces: `devdeck-api` holds the protocol only (types with serde and schemars schemas,
  the catalog, errors, ids, a small client) and depends on nothing of devdeck but
  `devdeck-sys`, for the sockets' folder.
  `devdeck-app/src/api/` is the one part that knows the app: socket, identity, policy,
  approvals, a dispatcher per domain, events, audit. `devdeck-ctl` and `devdeck-mcp` (rmcp,
  stdio) depend only on `devdeck-api`.
- The operations layer: each operation is a model function taking ids; the UI resolves
  focus and selection into ids and calls it, the API calls it directly. What lives in
  views today moves out (a shell's start, the logs text, the protected-env unlock,
  confirmations, Share with's need for a window).
- Threading: the socket's thread hands each request to the UI thread through a channel
  (as the watcher does); the UI thread takes a snapshot, the connection's thread encodes
  and writes; the audit log has a writer thread. Observers exist only while a client
  subscribes, timers only while something waits.
- Identity is the process tree (the peer's user checked and its process pinned by the
  OS layer, `devdeck_sys::peer`: on Linux `SO_PEERCRED` and a pidfd), walked up to a shell
  devdeck started. No tokens: a string in the environment can be read by any process of
  the user.
- Approvals: one service; destructive functions take a proof (`Approval<Op>`, holding its
  arguments) that only the user's answer in the UI makes, used once and bound to the
  state it was decided on.
- Not a sandbox: an agent in a devdeck terminal has the user's power through its own
  shell. The permissions guard what is done through devdeck, give clients least
  privilege, and record everything.

Phase 3 built (`plans/control-api-p3.md`): acting without a window, waits, events.

- The user's keys are theirs: a change made through the API goes through
  `Deck::apply_quietly` (each window keeps its focused pane and each pane its tab unless
  asked; the panes holding the keys, now or when Settings closes or the workspace shows
  again, keep their tab and stay shown). What would move them asks the user
  (`approval_required`, the card in phase 4): a tab shown where the user types, a pulse
  switch, a new window.
- Answers can come later: a handler gives a task the UI thread finishes (a wait, a git
  read, a file read in the background); the connection watches for its client hanging
  up and its caller's shell ending meanwhile.
- Waits look every 100 ms while someone waits, nothing otherwise; a wait whose time is up
  leaves a job to resume, not work in the background.
- Events say what changed, never its content (the client reads it with the methods);
  one bounded queue per connection, written by a thread of its own; the models are
  watched only while someone follows a topic.
- One-off reads hold what views hold (a repository looked at, a port scan with its ports
  watched) until they answer, and count among the caller's waits.
- Caps everywhere a caller could grow something: waits, connections, shells, notes,
  pulses, pins, groups, toasts, a rate for changes.

## Toolbars (R9.9)

- One component, `devdeck_ui::Toolbar`, a custom element: it picks its form in prepaint
  from its real width (no lag, no first wrong frame), lays its row out and paints it. A
  view never lays a toolbar out by hand. Theme toolbar height, a hairline under it, one
  line, never wraps; every item `button_h` tall, one toggle look (fill and frame on),
  gaps at least 4 px scaled; its right end inset from the pane header's icons.
- Items, in the view's order: a **field** (grows, at least its minimum, a count at its
  right end, accessories attached), **actions** and **toggles** (an icon, a label when
  wide; `label_required` ones never go icon-only, `primary` ones keep their label and
  never fold), **menu buttons** (an icon and their value, the value kept in compact; a
  value may carry a color and its icon), **statuses** (an icon, a short word when wide,
  details in a menu on click), **text** (flexible, truncates, full text on hover), a
  **spacer** (what follows is right-aligned), then **more** (`ellipsis`, never the
  settings cog) when it holds anything.
- Each item runs a registered action (the app's `tool(id, icon)` takes its label from the
  registry): mouse, folded item and key run one path. Tooltips name the item or say why it
  can't run (R5.5); no key is ever shown (R5.4). Right-click on the toolbar lists every
  item, shown or folded (R5.2).
- Forms: **wide** when everything fits; **compact** (labels dropped but required ones,
  status words dropped, values kept); **narrow**: items fold into "more" by `Fold::Order`
  (1 first; equal numbers together; folded items keep the toolbar's order there), never
  `Never`, primary, safety values, statuses, or an item in a state other than its default;
  **floor**: the field truncates, then becomes a button. Counts reserve a width (capped
  "999+"), so the form depends on the width alone; a folded item's count goes into its
  menu text and "more" shows a dot in its color. Labels are shaped by the text system;
  paddings come from the component's own constants.
- No icon means two things in the app.
- Toolbars, moved onto it one at a time: logs (first), the run view's, the service view's
  actions row and tag editor, the git panel's header, the branch matrix's header, the
  sidebar's list header (which keeps its activity name: it has no tab). Pane headers keep
  their own view-action icons (part of the layout engine).

## Notifications (user, 2026-11-10)

- Every toast is a notification: it lands in the notification center (the one bell, in
  the title bar next to settings); the center keeps them, so toasts stay short-lived.
- The bell's badge: nothing unread, the bell alone; something unread, a small blue dot;
  unread errors or failures, a larger red dot with their count (red wins over blue).
- What a view can notify about is chosen in one shared **notify** dropdown (a crossed bell
  when nothing is ticked, the bell in the accent when anything is): a checkbox per
  option, no "all" or "none".
- **Standing rule:** a notification option's label is colored with the severity of the
  toast it produces (info blue, success green, warning amber, error red). Its words carry
  the meaning, so no icon is needed: a named exception to R9.1's color-with-an-icon.
- OS (desktop) notifications are postponed (the plan of M3 step 7, kept, marked so).
- Wrapping (R4.4's opt-in): a toast's text wraps to 3 lines, then is cut with an ellipsis
  (a click on it opens the center on it); a center row shows its whole text wrapped.

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
- 2026-11-09 Control API phase 3 (user): the approvals service moves to phase 4 with its
  card; `user.ask` is dropped (an agent asks in its own session); D1–D9 of
  `plans/control-api-p3.md` decided.
- 2026-11-10 Anything an API call would do to the user's keys or screen beyond the
  background asks the user, rather than branching on what the OS allows (GPUI can't open
  a window without focus on Linux).
- 2026-11-07 No git or editor features (user decision). 2026-11-08: git has its panel
  (R6.10); files are view only and notes the one thing devdeck edits (R12, user).
- 2026-11-07 Lists are Pinned + All, alphabetical; no "mine", no "recent" (user decision).
  2026-11-08: groups the user makes are allowed in All (R4.8), still no automatic one.
- 2026-11-07 Separate workspaces, one shown at a time, others keep running (user decision).
- 2026-11-07 No tab drag between windows; "Send to…" with numbered panes instead, plus
  ⇲ re-attach and × close on detached windows (user decision after the M0 finding).
- 2026-11-07 Settings as a full-window view; UI scale is the first setting, early in M1
  (user decision).
- 2026-11-07 The theme is global, like the UI scale, not per workspace (user decision).
- 2026-11-07 A list row dragged onto a pane opens its item there (into the pane or at an
  edge). GPUI shows the pane drop zones for any drag, so rows use them instead of
  hiding them. (2026-11-08: one drop layer under the panes replaced the zones, R3.13.)
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
  Whether the main window is maximized is remembered too (2026-11-08, R9.8): it opens
  `WindowBounds::Maximized` over its windowed size, which restoring gives back.
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
  program's screen when it asks for that (alternate scroll); superseded on 2026-11-09
  for programs that ask for mouse reports.
- 2026-11-07 Colour and size queries are answered with the theme and the cell size, one
  look for every terminal (`core::terminal::set_look`, set by the view): the theme and
  the UI scale are global, and a restarted run's fresh terminal answers right away.
- 2026-11-07 When several views show one run, the one focused last sizes its terminal.
- 2026-11-07 Leftover processes (R8.2): a systemd scope per run, chosen over an
  environment tag (programs overwrite or hide it, and it spreads: a tmux server,
  `dbus-update-activation-environment`), a timer scan (misses a fast double fork, costs
  CPU) and devdeck as a subreaper (it would collect unrelated orphans and still lose
  everything on `kill -9`); reviewed by a subagent with experiments on this machine.
  A build's daemon outlives its run until quit, and a terminal's detached processes
  stay (decided with the user).
- 2026-11-07 Icons replace font glyphs (R9.5): Lucide over Tabler (same style, far more
  than needed), Phosphor (rounder) and Codicons (CC BY, few icons); vendored rather
  than `gpui-component-assets` (tied to another GPUI and component library). Lucide's
  default 2 px stroke kept: at 14 px it matches JetBrains Mono's weight in the
  before/after snapshots. A spinner turns only for a run stopping (it redraws while
  shown); a rollout or a load shows the loader still.
- 2026-11-08 Ids for what has no identity of its own (user decision): a layout is known
  by an id (the first `default`, new ones a UUID), its name is a label, so a rename
  keeps its pin and group. Folder paths, deployment names and port numbers stay the
  keys: they are what they name.
- 2026-11-08 A terminal's runtime is apart from its tabs (user): tabs are views on a
  shell (`term/<n>/…` is shell `n`), shared across layouts; closing a tab, a detached
  window or a layout never ends it (it used to end the shell and close its tab in every
  layout, each losing that pane: the "2 panes instead of 3" bug). The Terminals list is
  built from the layouts and the runner (`Deck::terminals`), with Open and Stop, no End
  (closing a shell never changes a layout, user). The deck tells the runner which
  terminals are shown; one not running and not shown is forgotten. Empty panes stay out
  (user): a pane still goes with its last tab.
- 2026-11-08 The app calls itself **Kugelblitz** (user): the title bar's icon
  (`assets/app/devdeck.svg`, the name only as its tooltip since it read as a menu, R9.7), window titles, an empty pane's welcome and every message
  naming the app. The project, binary, crates, files, refs and app id stay `devdeck`.
- 2026-11-08 Right-click menu (R5.7): a view passes its own items and the content
  (`context_menu::Content`, text for now); a list of action functions adds what applies
  (Copy, Share with). Share with pastes into terminals whose program turned bracketed
  paste on, control characters stripped (a plain shell would run each line); the text
  goes as it is, no header (user). Named "Share with", not "Send to terminal": the
  destination needn't stay a terminal (user), and the tab menu's "Send to…" moves panes.
  No Claude Code detection: every terminal by its short name (user).
- 2026-11-08 Groups as tagged folder rows of the list engine (their row path can't meet
  a real one), so they open, close and take the keyboard like folders; one drop handler
  per row (GPUI panics on two `on_drop` of one type); section headers stay out of the
  keyboard selection, so Down from the filter still reaches the first row.

- 2026-11-08 Claude control becomes a generic control API (user): MCP is one client of
  it, a CLI another; MCP can go without touching the app. Callers are known from their
  process tree (no tokens to steal); permissions by class of operation, by who started
  what, and by devdeck's own rules; risky calls wait for a card only the user answers.
  The six proposals of `plans/control-api.md` taken as they are.

- 2026-11-09 Git panel branches (user): newest first by their last commit (locals,
  then remotes), an exception to R4.1; remote branches hidden until the "remote · N"
  button or shift-r shows them (filtering with them hidden says how many would match);
  a row's message, then its name, give way so the author and age always show.

- 2026-11-09 Tabs name their view with an icon and a short name (user, R3.14):
  `views::tab_label` (from the address and the workspace's services), drawn by
  `views::tab_label_element`; the full title stays the tooltip. A window redraws for run
  tabs only when one's state colour changes (`views::run_tints` against the runner), not
  for every line a run writes. The git panel's buttons are icons (`icon_action`: its
  tooltip names it or says why it's refused), and its first line keeps the branch and
  the changes, the upstream then the path giving way. Icons added from gpui-kit-assets
  0.9.0 (Lucide): git-branch, git-branch-plus, scroll-text, box, table-2, file-code,
  download, arrow-down-to-line, arrow-up-from-line, refresh-cw.

- 2026-11-09 The Cluster list's env is its own (user): `SidebarState.cluster_env`
  (`None`: synced, following the workspace's), picked from a dropdown in its header
  ("sync with workspace (@env)", then every env) or with `e`; the button is highlighted
  while it isn't synced, its tooltip naming the workspace's env. The title bar's env
  stays the workspace's, for runs.

- 2026-11-09 A branch switch asks only when tracked files have uncommitted changes
  (user): none, and it switches at once (a remote branch: tracked, then switched to);
  else the modal keeps its question and lists those changes (`git::uncommitted`), one
  line each cut with an ellipsis, eight then "and N more". Untracked files don't count.

- 2026-11-09 The wheel goes to a terminal's program when it asks for mouse reports
  (modes 1000/1002/1003, written as SGR, UTF-8 or X10), as xterm does: Claude Code's
  full-screen mode scrolls with it instead of getting arrow keys. Otherwise cursor keys
  on a full-screen program's screen (alternate scroll), else the history; scrolled back
  on the main screen, the wheel keeps scrolling the history. Clicks and drags stay
  devdeck's (selection, shift+click on links): reporting them is its own decision.
  `core::terminal::wheel_input` holds the rules.
- 2026-11-09 Ctrl+Backspace in a terminal sends ^W (user): xterm's ^H deletes one
  character in bash; ^W deletes the previous word in shells, vim's insert mode and the
  like (back to a space: `foo/bar` goes at once), as VS Code's terminal does.
- 2026-11-09 "Keep in every layout" removed (user: no use case). A layout shows only
  its own tabs; a saved `layouts.json` with `keep` still loads, the field dropped.
- 2026-11-10 The rest of M3 (user): no deploy undo (a failed deploy is fixed by
  deploying another branch); kube-rs for logs, shell, port-forward and writes (its `ws`
  feature for shell and port-forward); no k9s, devdeck's own UI does what matters; every
  deploy seen on the deployment watch is recorded, whoever made it; desktop
  notifications per service and env, events picked in a dropdown on the service view
  (also an action), always the desktop's own (not a toast), off at every start. What's
  left is built in order of what matters: logs, changing the deployed branch,
  redeploying it, notifications, then the close; the rest when wanted.
- 2026-11-10 (user, M3) A push that can't lose anything on the remote (a publish or a
  plain push) runs at once after its check, no modal; only a replace (force-with-lease)
  asks. Amends "every write confirmed" (2026-11-08).
- 2026-11-11 (user) Edit mode removed: each dangerous action already asks in its own
  modal, so a global switch was redundant. Every dangerous action takes the
  `danger::Confirmed` of its modal, which opens on Cancel. The API dropped `editmode.get`
  and its `edit_mode` topic at level 5: a removed feature takes its methods with it (R13.2's
  exception). Replaces "Edit mode gates destructive actions" (2026-11-08).
- 2026-11-11 (user) Layouts are called pulses everywhere the user or an agent sees them:
  the app, the API (`pulse.*`, the `pulse` topic, `pulse` fields, the `pulses` list,
  level 6), MCP and `devdeck-ctl`. The code keeps `Layout`, `LayoutBook` and `Deck`
  (internal names, not renamed), and the state files keep theirs; a sidebar saved on the
  old `layouts` activity opens on Pulses. The API may change, renames included, when every
  consumer devdeck builds changes with it (R13.2, replacing "it only grows"). First step of
  moving pulses to the first activity, the other activities getting full views of their own.
- 2026-11-11 (user) Pulses are the first activity, what a workspace lands on (and where an
  unknown or unavailable saved activity goes), with a heartbeat icon; ctrl+1…6 follow the
  bar (Pulses, Apps, Cluster, Ports, Terminals, Notes). A saved sidebar reopens where it
  was. The title bar's pulse menu stays until the Apps full view settles what the main
  area shows. Clicking the active icon still hides the sidebar.

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
- 2026-11-07 (user, M3) Cluster envs are watched from first use until the workspace's
  cluster changes, not only while visible: an idle watch costs no CPU and lists show at
  once on return. Can be revisited.
- 2026-11-08 (user) Edit mode gates destructive actions; devdeck never pushes main or
  master. No auto-off.
- 2026-11-08 (user, M3) Git isn't read only: a panel per app and service with switch,
  new branch, fetch, pull, a push that never loses anything, and abort; every write
  confirmed; an operation under way blocks everything but its abort. No commit, merge or
  rebase button.
- 2026-11-08 (user, M3) Pods are watched on demand: open services' own, and every pod of
  the list's env only with the list's **watch** toggle on (off by default).
- 2026-11-07 (user, M3) Git comes back read only (R6.10): image tags are branch names, so
  a service's repository and branches matter to deploys. Worktrees and a git page stay
  with the later ideas.
- 2026-11-07 A deployment has a list of containers; its main one (named like it, else the
  first) gives the tag lists and the matrix show, "+n" for the others. "Running since"
  is when the current version finished rolling out (the Progressing condition at
  NewReplicaSetAvailable); a scale also moves it, a known limit until pods (step 2).
- Running section in the Apps sidebar: replaced by the row indicator and a filter.
  Revisit if it feels slower in use.
