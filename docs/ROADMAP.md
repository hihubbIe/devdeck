# Roadmap

Each milestone ends with a manual test by the user.

## M0 · Spike: prove the foundation — done 2026-11-07, waiting for the user's manual test
- Cargo workspace, pinned GPUI.
- A window with a split tree of panes rendered from the pure layout model.
- Collapse a pane, maximize a pane, resize dividers.
- Detach a tab into a second OS window; closing the main window closes it.
- A virtualized log view streaming 50 000 lines.
- kube-rs: authenticate with a configured kubeconfig and list deployments (read only).
- Measure: RAM at idle, CPU at idle, CPU while streaming, keystroke-to-frame latency.
- Outcome: keep GPUI or rethink, before building on it.

## M1 · Native prototype with fake data — done 2026-11-07, waiting for the user's manual test
Everything from the approved HTML prototype plus the later changes, backed by a fake
data provider. Steps (each planned, reviewed, built, checked, tested, committed):
1. Settings view (full window) with UI scale and theme; all sizes derive from the scale.
2. Shared components crate and theme files.
3. Workspaces: separate state, switcher in the title bar, quit guard.
4. Activities and lists: Apps, Cluster, Ports, Layouts, Pinned + All, alphabetical.
5. Views with fake data: app, command output, service inspector, logs, pod, text, matrix, ports.
6. Actions everywhere: palette, context menus, "Send to…", cheat sheet, hint bar.
7. Layouts per workspace, keep-in-every-layout; M1 measurements.

What to try: [plans/M1-manual-test.md](plans/M1-manual-test.md). Numbers:
[perf/M1.md](perf/M1.md).

## M2 · Real local runner — done 2026-11-07, manual test passed
Config loading, discovery, tool detection, pseudo-terminal runs, env bindings, ports,
quit guard on real processes, a plain shell terminal tab. Plan:
[plans/M2-runner.md](plans/M2-runner.md).
1. Foundations: run engine on driver events, a terminal per run, data model, interfaces.
2. Builders in parallel: config, discovery, environment, processes, ports, terminal element.
3. Wiring: real workspaces from config.
4. Terminal tab.
5. Close: the user's config, states, measurements, manual test.

## Files and notes
View-only file tabs with highlighting (a file dropped on a pane), and Markdown notes per
app, edited and rendered, saved by themselves (R12). Plan:
[plans/files-and-notes.md](plans/files-and-notes.md). Steps: 1. spike and docs; 2. components
in `devdeck-ui`; 3. engine: drops, view actions in the header, live titles; 4. file
view; 5. notes.

## Control API (after Files and notes)
A generic API for everything devdeck shows and does (R13); a CLI and an MCP server for
Claude are its clients. Plan: [plans/control-api.md](plans/control-api.md), decided
2026-11-08. Phases, one at a time:
0. Decisions, requirements, design — done 2026-11-08.
1. API crate, socket, identity, audit, observe methods, CLI, first MCP — done 2026-11-08,
   waiting for the user's manual test ([plans/control-api-p1.md](plans/control-api-p1.md)).
   1b. The MCP server's instructions: generic, saying when the user means devdeck and
   which methods find apps and commands (brought forward from 5) — done 2026-11-08.
   1c. Starting and stopping runs, a slice of 3 brought forward: `run.start` (an app's
   main commands, no arguments), `run.stop`/`run.restart` on runs the caller started,
   who started what, the `devdeck_run` tool — done 2026-11-08.
2. Bounded reads: output, screen, search, notes, files — done 2026-11-08.
3. Operations layer, events, jobs, waits; present, run, organize
   ([plans/control-api-p3.md](plans/control-api-p3.md)) — done 2026-11-10, manual test
   passed 2026-11-10.
4. Approvals and its proofs, the approval card; type, remote and destructive methods;
   and what phase 3 left asking: `window.detach`, `workspace.activate`, `app.open_url`,
   `leftovers.list`, the `git` topic.
   Later: `ui.selection`, `sidebar.show`, the matrix highlight.
5. Chosen MCP tools, instructions, setup docs.
6. Cluster methods, as M3 steps 4–5 land.

## macOS from source (started 2026-11-10) — paused 2026-11-10, waiting for round 0 on the Mac
devdeck works on an Apple silicon Mac, macOS 15 or later, for someone who builds it from
source; it feels like a Mac app (shortcuts, title bar, menu bar, Dock). The user has no
Mac: a colleague builds and tests it with `docs/BUILDING.md`: round 0 (the first build)
after step 1, round 1 after step 3, round 2 after step 6. Plan, and where it stands:
[plans/macos.md](plans/macos.md). Steps, one at a time:
0. Inventory and requirements — done 2026-11-10.
1. `devdeck-sys`, Linux only (1a–1d) — done 2026-11-10; the build guide pushed for
   round 0.
2. The macOS backends (2a–2f).
3. The build guide finished; push; round 1 on the Mac.
4. Mac behaviour in the app (L9, L10, L13).
5. What Linux shaped (`docs/PLATFORMS.md` §3).
6. Push; round 2 on the Mac.

Then packaging the `.app` (L11), then CI.

## M3 · Real cluster
kube-rs data, service inspector, git, logs, deploys with a history, notifications,
metrics, protected envs. Plan:
[plans/M3-cluster.md](plans/M3-cluster.md). Steps, one at a time:
1. Config and real deployments, read only — done 2026-11-07.
2. Service inspector on real data (pods, metrics) — done 2026-11-08.
3. Git panel — done 2026-11-08. 3b. Edit mode — done 2026-11-08, removed
   2026-11-11 (each dangerous action asks in its modal:
   [plans/danger-actions.md](plans/danger-actions.md)). 3c. Sidebar
   groups, folded sections, width; layout ids
   ([plans/sidebar-groups.md](plans/sidebar-groups.md)) — done 2026-11-08. 3d. Right-click
   menu with Share with; short terminal names
   ([plans/context-menu.md](plans/context-menu.md)) — done 2026-11-08. 3e. Terminals
   outlive their tabs ([plans/layouts-own-their-tabs.md](plans/layouts-own-their-tabs.md))
   — done 2026-11-08.
4. Logs — done 2026-11-10.
5. Change the deployed branch — done 2026-11-10.
6. Redeploy the same branch — done 2026-11-10.
7. Notifications — done 2026-11-11 ([plans/notifications.md](plans/notifications.md); OS notifications
   postponed).
8. Close — the suite, the leak cycle and the manual test written 2026-11-11; the
   window measurements wait for the user (no headless display).

Later, when wanted: shell in a pod, port-forward, replicas, restart, delete a pod, jobs,
config maps and secrets, services and ingresses, YAML, cluster methods in the control API.

## Pulses and full views (started 2026-11-11)

Pulses (the user's arranged panes, once "layouts") are the first activity, where a
workspace lands; every other activity gets a full view of its own instead of a sidebar
list, which can also open as a tab in a pulse. One activity at a time.

1. Rename layouts to pulses wherever the user or an agent sees them — done 2026-11-11
   ([plans/pulse-rename.md](plans/pulse-rename.md)).
2. Pulses first in the activity bar — done 2026-11-11
   ([plans/pulse-first.md](plans/pulse-first.md)).
3. Apps: a full view ([plans/apps-view.md](plans/apps-view.md)); 3a the frame and the
   app view — done 2026-11-11; 3b deployments, pivots, open in pulse, the title bar's
   pulse menu — done 2026-11-11. Manual test now.
4. Cluster ([plans/cluster-view.md](plans/cluster-view.md)): 4a the picker and the table,
   deploying through the picker — done 2026-11-11; 4b Cluster as a full view, the service
   as one page — done 2026-11-11; 4c front-end apps inside frontendapp. Manual test after 4c.
5. The others, one by one: Ports, Terminals, Notes; then Agents.

## M4 · Shareable
`SETUP.md` interactive setup for Claude, config reference, view and theme guides,
packaging of the executable. Started: a per-user install for this laptop,
`scripts/install.sh` (binary, icon, launcher to pin in the dock; 2026-11-08).
