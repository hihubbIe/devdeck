# devdeck requirements

The source of truth for what devdeck must do. Every requirement has an id so plans,
tests and commits can point at it. When a requirement changes, edit it here first.

Status markers: `[ ]` not started · `[~]` partly done · `[x]` done and tested.

## 1. Product

- [~] **R1.1** Native desktop app written in Rust, shipped as a single executable.
- [ ] **R1.2** Linux first (Wayland and X11). Wide desktop screens (1920×1080 and up). No mobile or narrow layouts.
- [~] **R1.3** As fast as possible and as cheap as possible: GPU-rendered text, near-zero CPU when idle, minimal RAM. Only visible content is drawn; long lists and logs are virtualized.
- [ ] **R1.4** Self-contained. No dependency on the author's own scripts or tools (`cs`, `f`, the project daemon, aliases). External programs (pnpm, go, kubectl, k9s, docker…) are optional capabilities, detected at runtime.
- [ ] **R1.5** Shareable. Nothing about one machine is hard-coded: paths, envs, clusters, kubeconfigs, folders and tools all come from config.
- [~] **R1.6** Monospace font everywhere.

## 2. Workspaces

- [~] **R2.1** The app holds several workspaces (for example "work" and "personal"). They share nothing: folders, envs, cluster settings, layouts, pins, processes, history. The theme and the UI scale are global (decided 2026-11-07).
- [x] **R2.2** Exactly one workspace is displayed at a time. Switching does not stop anything: the others keep running in the background and come back in their exact state.
- [~] **R2.3** Envs, cluster access and k9s are optional per workspace. When a workspace has none, the related UI (env selector, Cluster area, deploy actions) does not appear at all.

## 3. Layout engine (panes, tabs, windows)

- [~] **R3.1** The window body is one tree of splits. Every leaf is a pane. There are no special docks: any pane can sit anywhere.
- [~] **R3.2** A pane is a generic container of tabs. A tab shows exactly one view. Any view type can open in any pane.
- [~] **R3.3** Views are implemented once and registered natively (examples: command output, app details, service inspector, pod, logs, text or config file, branch matrix, ports). Adding a view type must not require touching the layout engine.
- [~] **R3.4** Tabs move by drag and drop: into another pane's tab strip, or onto a pane edge to split it. Panes move the same way (drag the pane by its tab strip handle).
- [~] **R3.5** Dividers are resizable by dragging.
- [~] **R3.6** A pane can collapse to only its tab strip, giving its space to its neighbours. Clicking a tab (or a key) expands it again.
- [~] **R3.7** A pane can be maximized and restored.
- [~] **R3.8** A tab can be detached into its own OS window. That window contains a single pane (it can receive more tabs), nothing else of the app. It is owned by the main process: closing the main app closes it. The detached window's title bar has a re-attach button (⇲) next to close (×): ⇲ sends its tabs back to the main window, × closes the window and its tabs. Dragging tabs between windows is not required (decided 2026-11-07: GPUI can't do it on Wayland; "Send to…" covers it).
- [x] **R3.11** "Send to…" on a tab and on a pane (menu, pane button, key): every pane of every window shows a number and is listed by its active tab and position, plus "new window". Pick by number key, by clicking the list, or by clicking the pane. Esc cancels. Sending a pane moves all its tabs.
- [~] **R3.9** Panes never overlap. Each pane clips its own content; content wider than the pane scrolls inside it.
- [ ] **R3.10** Named layouts per workspace: save the current arrangement, switch between layouts. A view can be marked "keep in every layout".

## 4. Lists and text

- [~] **R4.1** Every list is sorted alphabetically.
- [x] **R4.2** Lists have two sections, Pinned and All, both alphabetical. Pin or unpin with a menu action, a key, or by dragging a row between sections.
- [~] **R4.3** No "mine" grouping and no recency-ordered sections. Running state is shown as an indicator on the row, with a "running only" filter.
- [~] **R4.4** Text never wraps unless a view explicitly allows it (for example a log view with wrapping toggled on). Single-line text truncates with an ellipsis (end or middle truncation) and shows the full text on hover.
- [~] **R4.5** Status markers and glyphs never shrink or move to another line; the label truncates instead.

## 5. Interaction

- [~] **R5.1** Every action works with both mouse and keyboard. Actions are registered once (id, label, default key, availability with a reason) and reused by buttons, context menus, the palette and shortcuts.
- [~] **R5.2** Right-click opens a context menu on every interactive element (rows, tabs, panes, commands, pods).
- [~] **R5.3** Command palette (ctrl+k) with fuzzy search over apps, commands, deployments, envs, layouts, workspaces and actions. `@35` picks the env for one run. Tab shows an app's commands.
- [x] **R5.4** k9s-inspired single-key shortcuts, a context-sensitive hint bar, and a searchable cheat sheet (`?`).
- [~] **R5.5** Unavailable actions show why (missing tool, protected env, no permission).

## 6. Local apps

- [ ] **R6.1** Apps are discovered automatically from the workspace's folders: `package.json` scripts, pnpm/npm workspaces (monorepo packages), Taskfile tasks, Makefile targets, docker compose files, `.vscode/launch.json`. New folders appear without configuration.
- [ ] **R6.2** Runnable apps and libraries are told apart; libraries stay listed but de-emphasized.
- [~] **R6.3** Commands have tiers: primary (start, dev, serve…), secondary (build, test, codegen…), hidden (lint, format, prepare, pre/post hooks…). Hidden ones stay reachable.
- [ ] **R6.4** Overrides (names, commands, env binding, ports, URLs, hidden apps) live only in the central config, never in the project folders.
- [~] **R6.5** Each command declares the tools it needs. When one is missing, the app and its commands show as disabled with a warning sign and the missing thing named ("go not found", ".nvmrc wants node 20, which isn't installed").
- [~] **R6.6** Env for a run: a workspace-wide default plus a per-app override. How the env is applied is generic and declared: environment variable, command argument, rewrite of a gitignored file, or none. The UI says how it is applied.
- [~] **R6.7** Commands run in a pseudo-terminal: colors, progress output and exit codes are real. Several run at once. Stop, restart, run again, exit status, crash notification.
- [ ] **R6.8** Node version per project from `.nvmrc`, resolved against configured node installations.
- [~] **R6.9** Listening ports are detected per run. Port conflicts are reported before starting, with an option to stop the holder.
- [ ] **R6.10** No git and no editor features (no branch, no ahead/behind, no "open in editor").

## 7. Cluster (optional per workspace)

- [~] **R7.1** Per env: list of deployments (alphabetical, Pinned and All) with status, image tag, ready count, restarts, running since.
- [~] **R7.2** Service inspector: header and actions; live CPU and memory against request and limit; pods (status, ready, restarts, last termination reason, age, node, cpu, memory); container (image, ports, resources, configmap); deploy history recorded by devdeck.
- [~] **R7.3** Image tag editor with suggestions (tags deployed on other envs, devdeck history, free text), an unsaved marker, save with ctrl+s or a button, and a confirmation that states the exact change and the exact command.
- [~] **R7.4** Rollout follow-up: progress, success, and failure detection (image pull errors) with one-action undo.
- [~] **R7.5** Restart, replicas, port-forward, shell (exec in a terminal view), logs (multiple pods, level filter, pause, follow, find), open k9s scoped to the env.
- [~] **R7.6** Protected envs (declared in config) refuse writes until unlocked by typing the env name.
- [~] **R7.7** Branch matrix: services × envs with the image tag in each cell; highlight a tag.
- [~] **R7.8** Works with what the user's access allows; missing permissions are explained, not hidden.

## 8. Lifecycle

- [~] **R8.1** Quitting with running processes (in any workspace) asks: "Stop all and quit" or "Cancel". Cancel keeps the app open.
- [ ] **R8.2** Child processes never outlive the app unless the user chooses so (process groups are killed on quit).

## 9. Look and themes

- [~] **R9.1** Dark, dense, flat look of the approved prototype: 24 px rows, hairline dividers, color only for state and always paired with a glyph.
- [ ] **R9.2** Themes are data files of color tokens. One theme ships for now: the prototype's dark theme with slightly more contrast.
- [~] **R9.3** A shared component library (rows, tabs, buttons, inputs, menus, badges, key hints, text view, terminal view…) is the only way views draw UI, for visual parity.

- [~] **R9.4** A Settings button in the title bar opens settings as a full-window view (not a tab). It holds global, per-machine choices: UI scale (for example 90–150 %, on top of the desktop's own scaling) and theme, and later more. Every size in the UI derives from the UI scale. Settings are saved to the config folder and apply immediately.

## 10. Configuration and setup

- [~] **R10.1** All configuration in `~/.config/devdeck/` (TOML): workspaces, folders, envs, clusters, tools, overrides, themes. State (pins, layouts, history) in `~/.local/state/devdeck/`.
- [ ] **R10.2** `SETUP.md` written for Claude: it walks a new user through an interactive setup on their machine (find project folders, detect tools and kubeconfigs, propose and write the config).
- [ ] **R10.3** Documentation where it is needed: architecture, adding a view, adding an action, components, config reference, themes.

## 11. Engineering

- [~] **R11.1** Work proceeds in steps: plan, review the plan with a subagent, assess, build, check visually across states, run tests, commit.
- [~] **R11.2** Unit tests for every feature; the whole suite runs before every commit so nothing regresses.
- [~] **R11.3** Dependencies are chosen deliberately (modern, fast, maintained) and pinned.
- [~] **R11.4** Performance is measured at every milestone, not assumed: memory at idle and under load, CPU at idle and under load, input latency, binary size, and a leak test (repeated open/close cycles must plateau). Results go in `docs/perf/<milestone>.md` and regressions are fixed before the milestone closes.
- [~] **R11.5** Development stops at the end of every milestone for the user's manual test.
