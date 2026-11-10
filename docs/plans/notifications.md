# Notifications — plan (2026-11-10)

Three steps, one at a time, each reviewed and committed. Decided with the user on
2026-11-10 (mockups shown): every toast is a notification; one notification center; a
shared notify dropdown; OS notifications postponed (M3 step 7's plan, kept). The rule in
`docs/DESIGN.md` "Notifications". The user's TODO (devdeck "features" note): "toasts have
an ellipsis that almost always triggers and i have no way of knowing what it says".

Requirements: R4.4 (a toast opts in to wrapping), R5.1, R5.4 (no key hints shown), R5.5,
R9.1 (the severity colors' named exception), R9.3, R2.2 (background workspaces), R3.8
(detached windows), R7.4.

## Step A · The notification center and toasts

- **Toasts** (`devdeck-ui` overlay): an optional title (one line) and a text that wraps,
  at most 3 lines, then "… see notifications" (a click opens the center on it); at most
  3 shown, "+N more" opens the center; every toast leaves after 8 s (errors 12 s);
  hovering pauses its timer; actions are buttons; the close button stays. Every
  `toasts(cx).push` call site keeps working (the title optional).
- **The center's store** (app, global): every toast as a notification (id, kind, title,
  text, time, workspace, actions, read), newest first, the last 500 in memory; read when
  shown in the center; cleared on request. Toasts and the center share it (the toast
  queue's history becomes it).
- **The bell** in the title bar of every window, next to settings: nothing unread, the
  bell; unread, a small blue dot; unread errors or failures (kind Err, and Warn?), a
  larger red dot with their count (red wins). A registered action opens the center.
  `Icon::Bell` added (Lucide `bell`).
- **The center:** a full-window view like Settings (not a tab, in no layout): a toolbar
  (R9.9, `devdeck_ui::Toolbar`): unread / all, workspace (a menu), mark all read, clear;
  the list (a uniform list of rows of different heights is not uniform: rows wrap, so a
  plain scrolling list bounded to 500): kind icon and color, title, time ("2 min ago"),
  workspace, the full text wrapped, the actions as buttons; unread rows lighter. Esc or
  the bell closes it. Keyboard: arrows, Enter (the first action), Delete (removes it).
- Tests (devdeck-ui and app): wrapping and the 3-line cut with its link; hover pauses;
  "+N more"; the store's cap, read state, kinds; the badge's three states and counts; the
  center's filters, mark read, clear, keys; every existing toast still shows.

## Step B · Open: reveal a view

- `Shell::reveal(workspace, view_ref)`: switch to the workspace if needed; find the view
  in its current layout, else in its other layouts (switch to that layout); activate its
  tab, expand a collapsed pane, bring a detached window to the front (as far as the
  window system allows: on Wayland it may only ask for attention); then a flash: the
  tab and the pane's border in the accent, fading over ~1 s (an animation GPUI draws only
  while it runs). Not found anywhere: open it in the focused pane ("Open here").
- An action helper for toasts and the center: `Open` (or `Open here` when the view isn't
  open) built from a workspace and a view address.
- Tests: found in the current layout, another layout, another workspace, a detached
  window, a collapsed pane; not found (opened here); the flash ends (no animation left
  running); a workspace or view gone (nothing, logged).

## Step C · The notify dropdown, the deploy rocket, the service's notifications

- **The notify dropdown** (`devdeck-ui`, a shared component): a crossed bell (nothing
  ticked) or the bell in the accent (anything ticked); a click or its action opens a menu
  of checkboxes, one per option, each label colored with its toast's severity (the
  standing rule), no icons, no "all"/"none"; the menu stays open while ticking; arrows,
  space, Esc. No tooltip.
- **The service view's tag row:** the tag field, then a **deploy** icon (rocket: dim with
  its reason when it can't deploy, ready when another tag is typed; ctrl+s still opens the
  confirmation), then the notify dropdown; "save" and the image's repository path go (the
  full image stays in the Container section).
- **Options for a service on its env:** branch deployed (info), redeployed (info),
  rollout finished (success), deploy failed or stuck (error), pods crashing outside a
  deploy (error). Subscriptions by (workspace, env, service), off at start, kept until
  devdeck quits (closing the view keeps them), shown ticked again when the view reopens;
  `b` opens the dropdown in the service view.
- **Events** from the deploy follow-up (`cluster_deploys.rs`), the rules of M3 step 7's
  review kept: one notification per (env, service, rollout, kind) (failing and stuck one
  kind; replaced isn't an event); a crash outside a rollout once per (service, template)
  until a pod of it is ready again, using `pod_failure`'s threshold; a subscribed service
  holds its pods (deploys made elsewhere are followed from counts otherwise).
- **Toasts:** title "<service> on <env>", the event in words, kind by severity; actions
  Open (step B) and, for a failure, Logs. devdeck's own deploys and redeploys always
  toast their outcome (finished, failing/stuck), ticked or not.
- Tests: the dropdown (component tests: toggling, staying open, keys, colors from kinds);
  the rocket's states; each option notifies once when ticked and not otherwise; the
  anti-flood rules; own deploys always toast; subscriptions survive the view closing;
  Open from such a toast reveals the service view.

## Out of scope

OS notifications; saving the center or subscriptions across restarts; notify dropdowns
on other views (later, one at a time).

## Review (subagent, 2026-11-10) and assessment

Accepted unless said; one point (7) needs the user.
1. The toast history is the control API's (`ui.toasts`, the `toasts` event, R13.2).
   **Accepted**: `history` and `Told` stay as they are; the center's store sits beside them
   in `devdeck-ui::Toasts` with an opaque `source`; clear and delete touch only the store;
   a title reaches the API as a new optional field. Test: `ui.toasts` unchanged after a
   clear.
2. Many toasts aren't a workspace's, and the screen's workspace is wrong for background
   sources. **Accepted**: the source is optional (app-level shows under "all"); `push_in`
   for background sources (runs, deploys, these notifications).
3. 500 stored closures keep things alive and capture workspace indices. **Accepted**: stored
   actions are data (`Open { workspace, view }`, `Logs { … }`) resolved on click;
   closures stay on live toasts only.
4. "Like Settings" needs Settings generalised. **Accepted**: one cover state (none,
   settings, notifications) excluding each other; a `Notifications` key context with Esc;
   in a detached window the bell brings the main window forward and opens it there.
5. "Read only when shown in the center" keeps the dot on. **Accepted**: closing a toast or
   using one of its actions marks it read too.
6. Warn is mostly refusals of the user's own action. **Accepted**: the red count counts Err
   only.
7. Reveal: `views::open_in` already finds, activates, expands and focuses in the current
   layout; switching layout closes the other layout's tabs and drops detached windows.
   **Decided by the user**: a layout switch loses nothing (the layout left is saved with
   its tabs and detached windows, runs and terminals go on, switching back restores it),
   so Open switches to the layout holding the view directly, as first asked: workspace,
   layout, tab, then the flash; not open anywhere, "Open here". Plus: switch workspace by
   id, undo a maximized pane hiding the target, raise a detached window (an attention
   request where the window system refuses).
8. The flash. **Accepted**: a flash deadline on the deck cleared by a one-shot timer (state
   tested), a fresh element id per flash.
9. The notify dropdown already exists as a toolbar menu with toggles that keep the menu
   open. **Accepted**: no separate component; `MenuItem.color` added (menus and submenus,
   dimmed when disabled), `Icon::BellOff`.
10. The rocket vs the toolbar plan (a labelled `primary` save). **Accepted as the user
    decided** (an icon): the action keeps the id `svc.save`, its label becomes "deploy";
    the reason shows in its tooltip and, by keyboard, ctrl+s's toast; `toolbars.md` updated
    to say the tag row's deploy is an icon (a named departure); the `tag-save` selector and
    the snapshot follow.
11. The 3-line cut. **Accepted**: `line_clamp(3)`, a click anywhere on a toast's text opens
    the center on it (no conditional link); the R4.4 opt-in written in DESIGN.
12. 500 wrapped rows. **Accepted**: `gpui::list` (rows of different heights, virtualized).
13. Dropped review points and double toasts. **Accepted**: a palette action listing every
    subscription with "turn all off"; pods held only while "pods crashing" is ticked; two
    workspaces on one env notify twice (said); one notification per (env, service,
    rollout, kind) whatever caused it, and no "branch deployed" for devdeck's own deploy.
14. Existing tests. **Accepted**: the queue keeps hidden toasts for "+N more"; the tests of
    its cap and lifetimes are updated on purpose (said in the commit).
