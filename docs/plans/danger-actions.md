# Dangerous actions: a modal each, no edit mode

Asked 2026-11-11 (user): edit mode (a title bar switch turned on before any potentially
destructive action) is redundant, since each such action already asks in its own modal.
Remove it. Mark every dangerous action in the code (not in the UI) so none is missed: it
needs a confirmation modal. Write that as a rule. Edit mode leaves the API too: the API may
change when a feature goes, but that change is limited. One step, one commit.

Requirements: R5.6 (rewritten), R13.2 (an exception), R13.5, R13.7. No OS-specific code.

## The marker: `devdeck-app/src/danger.rs`

- `Confirmed`: a token, `#[must_use]`, not `Clone`, `Copy` or `Default`; its field is
  private, so only `danger` makes one.
- `DangerModal`: a sealed trait (a private supertrait, its impls listed in `danger.rs`), so
  adding a modal that hands out tokens means editing that one file. Its method is the
  modal's Cancel focus handle.
- `danger::confirmed(&modal)`: the token, from a `DangerModal`.
- `danger::show(modal, window, cx)`: pushes a danger modal with Cancel focused. Enter
  confirms only on the action button, so a stray Enter cancels (edit mode used to be that
  second step).
- `danger::for_tests()`: `#[doc(hidden)]`, the one bypass, for tests and the snapshot tool.
- `clippy.toml` bans calling `danger::for_tests`, `devdeck_core::git::Git::run`,
  `devdeck_core::process::terminate` and the cluster driver's `set_image` and `restart`;
  an explicit `#[allow]` only where each belongs.

Dangerous actions and their modals:

| Action | Takes `Confirmed` | Modal |
|---|---|---|
| Deploy (change branch), redeploy | `Cluster::deploy`, `Cluster::redeploy`, `ServiceView::deploy`, `ServiceView::send_redeploy` | `ConfirmDeploy` |
| Git replace push, abort (not a bisect's) | `Repo::run_confirmed`; `Repo::run` refuses them | git `ConfirmModal` |
| Stopping a process devdeck didn't start | `port_scan::stop_process` (new, with the pid-still-holds check) | `StopProcess` |
| Deleting a note | `Notes::delete` | `DeleteNote` |

Which git ops are dangerous: `Op::dangerous()` in devdeck-core, an exhaustive match.
Replicas and a shell in a pod aren't built; they take a token when they are.

Not dangerous: fetch, switch, pull, new branch, publish and plain pushes, stopping
devdeck's own runs, ending terminals, logs, port-forward, the leftovers dialog at start
(devdeck's own processes), deleting a layout.

## Edit mode removed

- App: `edit_mode.rs`, the title bar button, `app.edit_mode` and `edit_mode.confirm`,
  `ToggleEditMode`, the observers in the service, git, port views and window_view; the
  checks in `Cluster::writes`, `Repo::refusal`, the git replace-push toast, port_scan and
  port_view ("Stop holder" is offered for any process holding a port, behind its modal).
- API: `editmode.get`, the `EditMode` type and topic, the hub's field. `API_LEVEL` 5,
  its comment naming what level 5 removed. R13.2 gets the exception: a dropped feature's
  methods, fields and topics go with it, at a new level, listed there; never to rename or
  reshape what stays.
- Protected envs (type the name), the short name and main/master never pushed stay.

## Tests

- core (a unit test in `git.rs`): `Op::dangerous()` for every variant (a bisect abort
  isn't).
- app: `Repo::run` refuses a replace push and an abort without a token, runs a bisect
  abort; `refusal` no longer refuses them.
- Each danger modal: Enter at open cancels.
- Ports: "Stop holder" offered for a foreign process (never confirmed: the holder is the
  test process).
- Edit-mode tests removed or turned into the above; `cluster_config` keeps showing the
  kube driver can write (the short name refuses instead).

## Visual check

None (user, 2026-11-11: no snapshots). The snapshot tool's edit mode calls and its states
98 and 99 (edit mode) are removed only so it still compiles; it wasn't run.

## Out of scope

Protected envs unchanged. Deleting a layout stays without a confirmation.

## Plan review (subagent, 2026-11-11) and assessment

1. Removing `editmode.get` broke R13.2. Taken differently: the user decided the API
   changes when a feature goes, but limited: R13.2's exception and level 5.
2. The modals focus their action first; with edit mode gone a second Enter acts. Taken:
   Cancel first, Enter acts only on the action button.
3. A pub trait is a convention. Taken: sealed trait, one `for_tests`, clippy bans,
   `#[must_use]` and no derives.
4. "Hard to undo" was vague. Taken: an explicit list in R5.6 and a "not dangerous" list;
   deleting a note is dangerous (it had a modal); deleting a layout isn't.
5. Unnamed plumbing. Taken: `ServiceView::deploy`/`send_redeploy` take the token,
   `stop_process` is new, the driver's writes are banned in clippy.
6. `Option<Confirmed>`. Taken: `run` plus `run_confirmed`; `Op::dangerous` exhaustive.
7. Test holes. Taken (cluster_config, ports, api.rs, renamed tests).
8. Snapshots and visual states. Dropped by the user: no snapshots.
9. Missed docs and comments. Taken; `rg -i "edit.?mode"` leaves only history.
10. Uncommitted work in the same files. The git-line removal is committed separately when
    the user says so.
11. Stop holder for any process: fine; the toast for a refused stop (EPERM) checked.
12. Mac check covers devdeck-core too. Taken.

## Diff review (subagent, 2026-11-11) and assessment

Nothing blocking; the four modals' focus, every path to a dangerous action and the
sealing checked.

1. R5.6 said an API approval hands out the token: taken, said as future (R13.6).
2. The removed topic's wire name is `edit_mode`: taken (`API_LEVEL`'s comment, docs).
3. The layer under the driver and the notes store weren't banned: taken
   (`devdeck_kube::set_image`, `restart`, `NotesStore::delete`, each allowed once).
4. `danger::confirmed` could be called anywhere: taken, banned outside the four action
   methods.
5. Handle order differed in the deploy modal: taken, `[action, cancel]` in all four.
6. The git modal minted a token for safe ops: taken (`run` for those). Enter with
   nothing focused now cancels a safe op too: kept.
7. An old client subscribing to `edit_mode` is refused whole: kept, as for any unknown
   topic; said in `API_LEVEL`'s comment.
8. R5.6 named unbuilt actions under `[x]`: taken ("when built"); this plan's test
   location fixed.
9. The snapshot tool's confirm-refused state is unverified: left, no snapshots (user).
10. Untracked files: added at commit time.
