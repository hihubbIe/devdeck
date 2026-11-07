# M0 · Spike plan

Goal: prove the foundation before building on it. Keep GPUI or rethink.

Requirements touched: R1.1, R1.3 (measured), R3.1, R3.2, R3.4, R3.5, R3.6, R3.7,
R3.8, R3.9, R4.4, R7.8 (read only), R11.2, R11.3.

## Dependencies (pinned)

| crate | version | why |
|---|---|---|
| gpui-pre (renamed `gpui`) | =0.3.11 | Snapshot of Zed's GPUI published 2026-11-02; same one gpui-component 0.9 pins. crates.io `gpui` 0.2.2 is a year old. |
| gpui-pre-platform (renamed `gpui_platform`) | =0.3.11, features `wayland`, `x11` | Platform backend (wgpu renderer on Linux). Without these features the app silently runs headless. |
| serde, serde_json | latest 1.x | layout snapshot serialization |
| kube | 5.0.0 | read-only deployment list in the kube probe |
| k8s-openapi | 0.29.0 | types for kube |
| tokio | 1.x | runtime for kube, in its own thread |

System packages (Ubuntu): `libfontconfig-dev libxkbcommon-x11-dev libx11-xcb-dev libvulkan-dev`
(documented in `docs/BUILDING.md`).

## Crates created in M0

```
crates/devdeck-core   layout model (pure), unit tests
crates/devdeck-app    binary: renders the layout model with GPUI
crates/devdeck-kube   kube probe library + `devdeck-kube-probe` binary
```

## 1. Layout model (`devdeck-core::layout`) — pure data, no GPUI

Types:
- `PaneId`, `WindowId` (newtype u64, allocated by `Layout`).
- `ViewRef` (string-like address, e.g. `demo/logs`, `demo/text/3`).
- `Axis { Horizontal, Vertical }`.
- `Node = Split { axis, children: Vec<Child> } | Pane(PaneId)`, `Child { node, weight: f32 }`.
- `Pane { tabs: Vec<ViewRef>, active: usize, collapsed: bool }`.
- `Window { id, root: Node, maximized: Option<PaneId> }`; window 0 is the main window.
- `Layout { windows, panes, next_id }`.
- `Edge { Left, Right, Top, Bottom }`, `Drop { Into(PaneId), Edge(PaneId, Edge) }`.

Operations (each returns `Result<_, LayoutError>` and keeps invariants):
- `open(view, target)` add a tab to a pane (or focus it if already open anywhere).
- `close_tab(pane, index)` removes; an empty pane is removed and its parent split
  simplified (a split with one child is replaced by that child). The last pane of the
  main window stays (empty).
- `move_tab(from, index, drop)` into a pane or onto an edge (creates a split in the
  right axis and order). Same-pane edge drop with a single tab is a no-op.
- `move_pane(pane, drop)` relocate a whole pane.
- `resize(split_path, index, delta)` adjust neighbouring weights, with a minimum.
- `toggle_collapse(pane)`, `toggle_maximize(window, pane)`.
- `detach(pane, index) -> WindowId` new window holding one pane with that tab.
- `attach(window, drop)` move every tab of a detached window back; window removed.
- `close_window(id)`: closing the main window closes all windows.
- `snapshot()` / `restore()` via serde.

Invariants (checked by `Layout::validate()`, called in every test):
- every `PaneId` in a tree exists in `panes` and appears exactly once across windows;
- no split has fewer than two children; weights are positive and finite;
- a pane's `active` is within bounds (or 0 when empty);
- only the main window may hold an empty pane.

Tests (unit, `cargo test -p devdeck-core`): one per operation plus edge cases
(drop on own edge, close last tab of detached window closes the window, nested
split simplification, collapse of every pane in a split keeps at least one usable,
move into same pane, serialize round trip, 10 000 random operations fuzz with
`validate()` after each).

## 2. App (`devdeck-app`)

- Opens a 1600×1000 window titled "devdeck (spike)". Bundled JetBrains Mono (OFL, with
  its license file) loaded at start; everything renders monospace.
- Renders `Layout` recursively: splits as flex rows/columns sized by weights, 1 px
  dividers draggable to resize (R3.5), panes with a tab strip and a body.
- Tab strip: click activates, middle-click closes, drag a tab and drop it on another
  tab strip (into) or on a pane body's edge zone (split, R3.4). Drop zones highlight
  while dragging.
- Pane header buttons: collapse (R3.6), maximize (R3.7), detach (R3.8).
- Collapsed pane renders only its tab strip; clicking a tab expands it.
- Detached tab opens a second OS window with just a pane. Closing the main window quits
  the app and so closes the detached ones. Closing a detached window drops its tabs.
- Demo views: `logs` (virtualized list using `uniform_list`, appends 2 000 lines/s
  until 50 000, then stops; follow toggle), `text` (static text with a long line to
  check truncation), `empty`.
- Theme: the token values from DESIGN.md, held in one `Theme` struct (no files yet).
- `DEVDECK_TRACE_LATENCY=1` prints key-down → next-frame duration.

## 3. Kube probe (`devdeck-kube`)

- `devdeck-kube-probe --kubeconfig <path> --context <ctx> --namespace <ns>` lists
  deployments (name, image tag, ready/desired) using kube-rs. Read only.
  No path or context is hard-coded (R1.5). Run once against dev35 for the report.

## 4. Measurements (scripted in `scripts/measure.sh`)

Release build. Each number recorded in `docs/perf/M0.md`:
- RSS and CPU% at idle (10 s, 1 window, 3 panes).
- CPU% while the log view streams; RSS after 50 000 lines.
- Leak check: script drives 500 open/close/detach/attach cycles through a debug
  action triggered by `DEVDECK_STRESS=1`, sampling RSS every 50 cycles; RSS must
  plateau (no steady growth).
- Keystroke-to-frame latency samples (p50/p95) from the trace.
- Binary size (stripped).

## 5. Visual checks (states)

single pane · horizontal split · vertical split · nested 3-level split · collapsed pane in
each axis · maximized pane · detached window with 2 tabs · long tab titles truncated
(no wrap) · empty pane · drop zone highlights during drag · log view at 50 000 lines,
scrolled up (follow off) and following.

## Out of scope

Workspaces, config files, real views, palette, menus, theme files, processes.

## Review notes (subagent review, assessed)

1. Cross-window tab drag on Wayland (no global coordinates, OS drag only carries
   files). **Accepted.** Explicit experiment on Wayland and X11, recorded go/no-go.
   Detach and re-attach always also exist as actions (key, menu, pane button), so
   R3.8 holds even if dragging between windows is impossible. If it is impossible,
   ask the user to amend R3.8.
2. Exit criteria. **Accepted.** See "Go/no-go thresholds" below.
3. Keyboard parity and focus. **Accepted.** `focused: PaneId` per window with pure
   rules after close/move/detach; actions + keys for every layout operation;
   re-attach action; pane drag by its strip handle.
4. Vertical text impossible (no rotation). **Accepted.** A pane collapsed inside a
   row split becomes a 28 px rail of tab monograms (2 letters + status glyph), full
   title as tooltip. In a column split it is its normal 32 px tab strip.
5. View instance ownership. **Accepted.** App-level `ViewRegistry` keyed by
   `ViewRef` owns each view's `Entity`; closing a tab drops it; the leak test asserts
   release with GPUI's leak detection.
6. More invariants + proptest. **Accepted** (all listed invariants; proptest with
   shrinking for the fuzz test).
7. Stable split ids, weights. **Accepted.** `SplitId`; resize works on weights
   (fractions summing to 1, renormalized after each op); the app converts pixel
   deltas and enforces a minimum pane size in pixels; a collapsed child keeps its
   weight for when it expands; the last expanded pane of a window cannot collapse;
   snapshot carries `version`.
8. M1 compatibility. **Accepted.** A view is open at most once across all windows;
   model `WindowId`s are separate from OS windows. Closing a detached window
   **re-attaches** its tabs to the main window (safer; flagged to the user).
9. Pane overflow. **Accepted.** `min_w_0/min_h_0` + `overflow_hidden` on pane bodies;
   6 px invisible hit area on 1 px dividers; test that a long line doesn't widen a pane.
10. Truncation vs scrolling. **Accepted.** Text views scroll horizontally (R3.9);
    labels truncate with a tooltip showing the full text (R4.4).
11. Latency via GPUI `profiler` feature (`input_latency_snapshot`). **Accepted.**
    Separate profiler build; memory measured on the plain release build; numbers
    exclude compositor/scanout.
12. Memory method. **Accepted.** PSS/USS from `smaps_rollup`; warm-up, then 500 vs
    5 000 cycles, bytes per cycle; 3 runs, median; record GPU/driver/scale/session;
    verify a real window (Vulkan mapped, `ZED_HEADLESS` unset); count renders at idle
    (must be 0). heaptrack only if installed.
13. GPUI test support. **Accepted** for actions-level UI tests (collapse, maximize,
    detach, re-attach, close main quits, view release). Truncation pixel tests need
    the real text system: deferred to M1.
14. Kube probe. **Accepted, refocused.** No separate binary: a "deployments" demo view
    fed by a tokio thread over a channel, to prove idle CPU stays 0 with a background
    runtime; a 403 shows as a message, not a crash. kube 5.0.0 and k8s-openapi 0.29.0
    exist on crates.io (checked).
15. X11 smoke run. **Accepted** (XWayland via `DISPLAY`, measurements + drag test).
16. DESIGN.md out of date; fractional scaling; log demo is not the terminal grid.
    **Accepted.**

## Outcome

**Go: keep GPUI.** Every threshold passed (docs/perf/M0.md). Deviations from the plan:

- Visual checks use `devdeck-snapshot`, which renders states offscreen through GPUI's
  headless wgpu renderer with the real text system. GNOME refuses silent screenshots and
  computer use was not available. Caught two layout bugs (fixed).
- The kube probe became a demo view, as the review suggested; 67 deployments listed on
  dev35, and a 403 shows as a message.
- Cross-window tab drag: impossible on Wayland, unverified on X11 (see perf notes).
  Proposed R3.8 amendment pending the user's decision.
- Not done in M0, moved to M1: real-text-system truncation tests, the "keep in every
  layout" flag, layouts per workspace, tooltips on hover checked visually (they exist
  but snapshots can't show hover).

## Go/no-go thresholds

| metric | threshold |
|---|---|
| idle CPU, 3 panes, 10 s | ≤ 0.3 % average, 0 renders |
| PSS after start, 3 panes | ≤ 120 MB (GPU driver mappings included) |
| USS growth, cycles 500→5000 | ≤ 200 bytes per cycle |
| input-to-present p95 | ≤ 16 ms |
| CPU while streaming 2 000 lines/s | ≤ 15 % of one core |
| detach / re-attach by action | works on Wayland and X11 |
