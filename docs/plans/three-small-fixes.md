# Three small fixes: one-axis scrolling, no name in the title bar, the window's state

Asked 2026-11-08 (user), an accelerated pass: plan, build, no plan review. One commit.

## 1. A scroll gesture moves one axis (R9.6, new)

Symptom: in a pane that scrolls both ways (the logs list with a long line), a sideways
swipe also moves it up or down.

Cause: GPUI keeps the larger axis of each scroll event, but a trackpad gesture's events
wobble, so some of a sideways swipe's events are mostly vertical and move the list.
GPUI has the fix, `restrict_scroll_to_axis`: a gesture (precise deltas) locks to the axis
it starts on, unlocking only for a clear change. No scroll container sets it. It also
stops a vertical wheel from scrolling a sideways-only area; shift+wheel does that (GPUI
turns it horizontal on Wayland and X11).

Fix at the component level: `devdeck_ui::Scrolls` (`scroll_x`, `scroll_y`, `scroll_xy`)
sets the overflow and the axis lock, for stateful divs and uniform lists. Every scroll
container uses it, uniform lists included; `clippy.toml` disallows GPUI's
`overflow_*_scroll` outside devdeck-ui so the rule holds.

Test (`devdeck-ui/tests/scroll.rs`): a container scrolling both ways, a sideways gesture
whose second event is mostly vertical moves only sideways; a plain vertical wheel still
scrolls down; shift is the platform's, not tested here.

## 2. The title bar shows the app's icon, not its name (R9.7, new)

The name next to the icon read as a menu. The icon stays, with the name as its tooltip;
window titles (the OS taskbar) and the empty pane's welcome keep the name. DESIGN's
Kugelblitz decision updated.

## 3. The main window reopens as it closed (R9.8, new)

The size is already kept (`main_size`, the last size while not held by the compositor).
New: `AppState::main_maximized`, set from `window.is_maximized()` on every bounds change,
saved with the app state. A main window that was maximized opens `WindowBounds::Maximized`
over its remembered windowed size (restoring gives that size back).

Test: `core::state` round-trips the flag; an old `app.json` without it reads as windowed.

## Visual check

Snapshots 01 (title bar) and 08 (detached title bar).

## Diff review (subagent, 2026-11-08) and assessment

- 1 Taken: GPUI does maximize a window opened `WindowBounds::Maximized` (`window.rs`
  calls `zoom` before mapping); `window_options` takes `maximized` and the manual
  `zoom_window` (a toggle) is gone. The user is on Wayland.
- 2 Left: on X11 the maximized flag may lag a window manager's ConfigureNotify; Wayland
  sets it before the resize callback.
- 3 Taken: R9.6 says trackpads on Wayland (X11 sends notches, never locked), `[~]`.
- 4 Taken: the tests send `Moved` (as Linux does) in one burst; a wide `scroll_list`
  test; the plain wheel leaves a sideways-only area alone. Three fail without the lock.
- 5 Taken: `devdeck_ui::scroll_list` wraps `uniform_list` with the lock; clippy
  disallows `gpui::uniform_list`.
- 6 Taken: the terminal filters its wheel events with `gpui::OngoingScroll`.
- 7 Noted: nothing becomes unreachable by mouse.
- 8 Taken: R9.7 `[~]` (no test; checked on snapshots 01 and 08).

The maximize on open isn't unit tested: GPUI's test platform doesn't implement `zoom`.

## Out of scope

Detached windows' maximized state.
