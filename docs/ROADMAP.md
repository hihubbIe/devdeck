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

## Later · From the user's ideas
A file explorer, text files in tabs (basic syntax highlighting, save), tools for Claude
to drive devdeck (open a tab, start a server, show logs filtered), detection of a
devdeck terminal through an environment variable. To be assessed.

## M3 · Real cluster
kube-rs data, service inspector, git read only, deploys with journal and undo, logs,
exec, port-forward, metrics, protected envs, k9s launch. Plan:
[plans/M3-cluster.md](plans/M3-cluster.md). Steps, one at a time:
1. Config and real deployments, read only — done 2026-11-07.
2. Service inspector on real data (pods, metrics) — done 2026-11-08.
3. Git panel — done 2026-11-08.
4. Logs, shell, port-forward, k9s.
5. Writes.
6. Close.

## M4 · Shareable
`SETUP.md` interactive setup for Claude, config reference, view and theme guides,
packaging of the executable.
