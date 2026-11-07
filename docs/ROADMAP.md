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

## M1 · Native prototype with fake data
Everything from the approved HTML prototype plus the later changes, backed by a fake
data provider. Steps (each planned, reviewed, built, checked, tested, committed):
1. Settings view (full window) with UI scale and theme; all sizes derive from the scale.
2. Shared components crate and theme files.
3. Workspaces: separate state, switcher in the title bar, quit guard.
4. Activities and lists: Apps, Cluster, Ports, Layouts, Pinned + All, alphabetical.
5. Views with fake data: app, command output, service inspector, logs, pod, text, matrix, ports.
6. Actions everywhere: palette, context menus, "Send to…", cheat sheet, hint bar.
7. Layouts per workspace, keep-in-every-layout; M1 measurements.

## M2 · Real local runner
Config loading, discovery, tool detection, pseudo-terminal runs, env bindings, ports,
quit guard on real processes.

## M3 · Real cluster
kube-rs data, service inspector, deploys with journal and undo, logs, exec,
port-forward, metrics, protected envs, k9s launch.

## M4 · Shareable
`SETUP.md` interactive setup for Claude, config reference, view and theme guides,
packaging of the executable.
