# M3 · Real cluster — plan

Goal: the Cluster area shows the user's real envs (deployments, pods, metrics, logs), a
service links to its git repository and branches, and the deploys the Slack flow can't do
(mostly the `hades` stack) are made from devdeck. Viewing comes first, writes last. The
fake cluster stays for `--demo`, tests, snapshots and the stress run.

Requirements: R1.5, R2.3, R7.1–R7.8, R6.10 (rewritten, below), R10.1, R11.1–R11.5.

## Decisions (user, 2026-11-07)

- Steps one at a time, each planned, reviewed, built, tested and committed.
- Viewing matters more than deploying: Slack gives the team visibility, devdeck deploys
  only where Slack can't (mostly `hades`). Writes are the last step before the close.
- Kubeconfigs come from a folder (a git repository of them): the workspace names the
  folder, devdeck reads every kubeconfig in it.
- Each env is one kubeconfig context; the namespace comes from the context. As found on
  2026-11-07 (`dev-environments/kubeconfig` and `f k9s`):

  | env | file | context | namespace | cluster |
  |---|---|---|---|---|
  | dev31, dev32, dev33, dev35, dev36 | `dev-km011gcp` | same as the env | same as the env | km011gcp |
  | hades | `hades00-hecate` | `hades00-hecate` | hecate | km009gcp |
  | dls | `hades00-mnemosyne` | `hades00-mnemosyne` | mnemosyne | km009gcp |
  | lab | `hades00-empusa` | `hades00-empusa` | empusa | km009gcp |

  No context for dev34 exists: it stays out of `envs` until one does.
- dls and lab are protected (R7.6): the user has nothing to do there.
- Git comes back, read only, as far as deploys need it: image tags are branch names
  (`main`, `master`, `devel`, `ET-1502-oidc-session-cookie-scope`). R6.10 becomes "git
  read only: a service's repository, its branches and where each is deployed; no
  checkout, commit or editor features". Worktrees and a git page stay with the user's
  later ideas.

Seen on the real cluster (read only, 2026-11-07), and what it changes:
- Some deployments run several containers (envoy-edge 3, frontendapp 2): a deployment
  holds a list of containers, one of them its main one.
- The image name isn't always the deployment's (canvasmodulesservice runs
  `nestjscanvasmodules`, foreignapiservice `foreignapiservicev2`).
- A tag is a branch, so the newest build of a branch has the same tag: deploying it
  again is a rollout restart, not an image change (step 5).

## Steps

1. **Config and real deployments, read only** (detailed below): the kubeconfig folder
   and the context map; a cluster driver behind the `Cluster` entity, fake or kube-rs;
   live deployment lists per env (R7.1); the matrix from real data (R7.7); errors and
   permissions explained (R7.8); a workspace without `[cluster]` has no Cluster area
   (R2.3).
2. **Service inspector on real data** (R7.2): pods watched per open service, phase,
   readiness, restarts, last termination reason from container states, node, age;
   containers (image, ports, resources, config maps); CPU and memory from
   `metrics.k8s.io`, sampled only while a service view is visible; a missing permission
   says so.
3. **Git, read only**: a service links to its repository under the workspace folders
   (matched by image or deployment name, overridable per app in config); its branches
   and current branch, read off the UI thread and refreshed when `.git` changes; which
   env runs which branch in the app view and the service view; the matrix highlights
   the current branch of a repository; branches are suggestions in the tag editor.
4. **Logs, shell, port-forward, k9s** (R7.5 minus writes): logs of the pods of a service
   (several pods, follow, pause, level filter, find), on the logs view; a shell in a
   container on M2's terminal (kube `ws`); port-forward with the port shown in Ports;
   k9s in a terminal tab with the env's kubeconfig, context and namespace.
5. **Writes** (R7.3, R7.4, R7.6): set a branch (image patch) with the exact command in
   the confirmation; redeploy the newest build of the same branch (rollout restart);
   follow the rollout from the deployment's status and its pods (image pull and crash
   detection), one-action undo; restart and replicas; protected envs; the deploy
   journal in the state folder. Tested against a fake API only: devdeck's own tests and
   checks never write to the user's cluster; the user tries writes by hand on an env of
   their choice.
6. **Close**: idle CPU and memory with every env watched, the whole suite, all
   snapshots, leak run with cluster views, manual test guide.

## Step 1 · Config and real deployments, read only (plan, 2026-11-07)

Requirements: R1.5, R2.3, R7.1, R7.7, R7.8, R10.1.

### Config

In the workspace file (shareable: the folder is a `~`-path like `folders`):

```toml
envs = ["dev31", "dev32", "dev33", "dev35", "dev36", "hades", "dls", "lab"]
protected = ["dls", "lab"]
default_env = "dev33"            # f k9s's default

[cluster]
kubeconfigs = "~/Documents/repos/dev-environments/kubeconfig"

[cluster.contexts]               # only where an env isn't named like its context
hades = "hades00-hecate"
dls = "hades00-mnemosyne"
lab = "hades00-empusa"
```

- `WorkspaceConfig.cluster: Option<ClusterConfig { kubeconfigs: PathBuf, contexts:
  BTreeMap<String, String> }>`, unknown keys are errors as everywhere.
- Problems (path and line, like `protected`): a `contexts` key that isn't one of `envs`;
  `[cluster]` without envs.
- The context of an env: its entry in `contexts`, else its own name.
- The user's `tadaweb.toml` gets these lines (outside the repository, as in M2 step 5).

### Kubeconfigs

A new crate `devdeck-kube` (planned in DESIGN.md; GPUI-free, so its tests build fast),
owning everything kube-rs:
- `Kubeconfigs::load(folder) -> Kubeconfigs`: every regular file at the top of the
  folder (sub-folders, like `qa/`, are not read) parsed with `Kubeconfig::read_from`;
  files that aren't kubeconfigs (the README) are skipped without a problem. Each context
  is kept with its file. A context name in two files: the first file by name wins, a
  problem names both.
- `Kubeconfigs::target(env, context) -> Result<Target, String>`: the file, the context,
  the namespace from the context (`default` when the context has none, as kubectl), the
  cluster name for messages. Missing: "no context hades00-hecate in
  ~/…/kubeconfig". Missing folder: "~/…/kubeconfig doesn't exist".
- Loaded off the UI thread when the cluster starts, and again when a file in the folder
  changes (the folder is watched like project folders, debounced): envs whose target
  changed reconnect, the others keep their watch. A `git pull` of the folder applies
  without a restart.

### The driver

As M2's `Runner` with a `RunDriver`: the `Cluster` entity keeps the state the views read,
a driver feeds it.
- `trait ClusterDriver { fn watch(&self, env: &str, tx: Sender<EnvUpdate>) -> WatchHandle;
  fn writes(&self) -> Result<(), String>; … }`. `EnvUpdate::{Deployments(Vec<Deployment>),
  Problem(Option<String>)}`; dropping the handle stops the watch.
- `FakeCluster` (`fake_cluster.rs`): today's fake behaviour moved out of the entity
  unchanged (load delay, unreachable dev36, rollouts on a timer, history, metrics walk),
  so every existing cluster test, snapshot and the stress run keep passing as they are.
- `KubeDriver` (`devdeck-kube`): one tokio runtime for the whole app, on its own thread,
  started on the first watch (not at app start: M0 measured +19 MB for TLS, tokio and
  kube, which a workspace without a cluster shouldn't pay). One `kube::Client` per
  target. Per env, `kube::runtime::watcher` on `Deployment` in the target's namespace
  with the default backoff; events applied to a map in the driver; a snapshot sent
  after changes settle (100 ms), so a rollout touching many fields is one update and one
  render.
- An env is watched from the first time something asks for it (the sidebar's selected
  env, a service view, the matrix asks for all) until the workspace's cluster is
  dropped or its config changes. Idle watches cost no CPU and no renders.
- Writes: the kube driver refuses them with "deploys come in a later step" (R5.5: the
  save button is disabled and says why); the fake driver keeps today's rollouts.
- Pods and metrics: the kube driver has none yet; the service view's Pods and Metrics
  sections say "not read yet" instead of a list (step 2 fills them).

### Data model (`devdeck-core::cluster`)

- `Deployment { name, containers: Vec<Container>, main: usize, replicas, ready,
  updated, available, since: Option<u64> /* epoch s */ }`, `Container { name, image:
  Image, ports, resources, config_maps }`, `Image { repo, tag, digest }`.
- `Image::parse`: the tag is after the last `:` only if no `/` follows it
  (`host:5000/img` has none); `@sha256:…` is a digest; no tag means `latest`. Its short
  name is the last path segment (step 3 matches repositories with it).
- The main container: the one named like the deployment, else the first. Lists and the
  matrix show its tag; a deployment with more shows "+2" after it with the others in
  the tooltip; the service view lists every container.
- "Running since": the `Progressing` condition's `lastUpdateTime` when its reason is
  `NewReplicaSetAvailable` (when the current version finished rolling out), else the
  deployment's creation. Shown as an age computed at render: no timer, ages update
  with the next render.
- Conversion from `k8s_openapi` types lives in `devdeck-kube`, pure functions, unit
  tested from JSON fixtures copied from the real cluster's shapes (names and registry
  replaced).
- The fake builds the same model; its deployments get one container (some two, so the
  demo shows "+1").

### Errors and permissions (R7.8)

Shown in the sidebar's status line and the matrix header (warning icon, reason in the
tooltip), mapped from kube errors:
- connection refused, timeout, DNS: "can't reach km011gcp (dev35): <cause>";
- 401: "km011gcp refused your credentials"; 403: "your account can't list
  deployments in dev35";
- missing context or folder: the kubeconfig messages above.
A watch that loses its connection keeps the last list and shows the problem next to it
until it's back (the watcher retries with backoff, no busy loop).

### Views

- Sidebar Cluster list: real deployments of the selected env, tag of the main container,
  ready dot; loading and problem states as today.
- Matrix: rows are every deployment name found on any loaded env (alphabetical), not the
  demo's service list; a cell with several containers shows the main tag and "+n".
- Service view: header, containers section with every container; Pods, Metrics and
  history sections say "not read yet" on the kube driver; the tag editor is disabled with
  the driver's reason.
- The palette's deployments come from the same lists.
- Removed: M0's spike feed (`kube_feed.rs`, the `DEVDECK_KUBECONFIG` variables) and its
  demo view (`DeploymentsView`); the settings test that looked for its message follows.

### Files

- `crates/devdeck-kube/` (new): `lib.rs`, `kubeconfigs.rs`, `convert.rs`, `driver.rs`
  (runtime, clients, watches), `errors.rs`, fixtures under `tests/fixtures/`.
- `devdeck-core`: `config.rs` (cluster section), `catalog.rs` (`ClusterSpec` becomes
  the fake's services or the real config), `cluster.rs` (model, `Image`).
- `devdeck-app`: `cluster.rs` (entity over a driver), `fake_cluster.rs` (new, moved),
  `services.rs` (`from_config` builds the cluster), `sidebar.rs`, `views/matrix_view.rs`,
  `views/service_view.rs`, `palette.rs`, `watcher.rs` (the kubeconfig folder),
  `views/demo.rs` and `kube_feed.rs` (spike removed), `bin/snapshot.rs`.
- `Cargo.toml`: `kube` with `runtime` (the watcher); `tower` as a dev-dependency for the
  mock API; both pinned.
- Docs: `DESIGN.md` (Cluster section, decisions), `REQUIREMENTS.md` (R6.10 rewritten,
  R7.1 and R7.8 progress), `ROADMAP.md` (M2 done and waiting for the manual test, M3
  steps).

### Tests

- `devdeck-core`: cluster config (contexts, defaults, problems with lines; R2.3 without
  `[cluster]`); `Image::parse` (registry with a port, digest, no tag); main container.
- `devdeck-kube`: kubeconfig folder from a `TempDir` (two files, README skipped, sub-folder
  ignored, duplicate context, namespace from the context and `default`, missing context
  and folder); conversion from fixtures (several containers, ready and updated counts,
  since); error mapping; the driver against a mock API (`kube::Client::new` over a
  `tower` service): list then watch events give snapshots, a 403 gives a problem, a
  dropped handle ends the watch.
- `devdeck-app`: the entity with a scripted driver (loading, list, problem with the last
  list kept, recovery); a config workspace with `[cluster]` has the Cluster activity and
  one without doesn't (replaces "no cluster until M3"); the matrix's rows from data; the
  service view's "not read yet" and disabled save with its reason. Existing cluster
  tests run on `FakeCluster` unchanged.
- Snapshots: the sidebar list with a problem kept over a list; the matrix with a
  multi-container cell and an unreachable env; the service view on the kube driver.
- Check by hand against the real cluster, read only: dev35 and hades lists match
  `kubectl get deploy`; no renders while idle with every env watched (`DEVDECK_STATS`).

### States to check

Loading, ready, unreachable env, 403, missing context, lost connection with the last list,
recovered; a multi-container deployment in the list, matrix and service view; a
workspace without `[cluster]` (no Cluster activity); the demo unchanged.

### Out of scope

Pods, metrics and the inspector's live parts (step 2); git (step 3); logs, exec,
port-forward, k9s (step 4); writes and the deploy journal (step 5); measurements beyond
the idle check (step 6).

### Review of the step 1 plan (subagent, 2026-11-07) and assessment

Every point accepted; the plan above reads with these changes.
1. The driver can't be GPUI-free and drive the fake on GPUI timers. **Accepted**:
   `devdeck-kube` exposes `watch(target) -> (Receiver<EnvUpdate>, WatchHandle)` without
   GPUI; the app's `ClusterDriver` takes the entity's context (as `RunDriver`); the
   entity keeps pods, rollouts and history. The trait is written out below.
2. Existing tests read `tag`, `spec.services`, `LOAD_DELAY`, `Remote::Error`. **Accepted**:
   `Deployment::tag()` (main container), constants stay in `cluster.rs`, the test edits
   are listed below.
3. No state holds a kept list and a problem. **Accepted**: per env
   `EnvState { deployments: Option<BTreeMap<..>>, problem: Option<String> }`, read by
   reference.
4. The watcher resumes after an error without an event, so a problem could stay. **Accepted**
   (to confirm in kube-runtime's source): after an error the watch is dropped and a new
   one starts after a backoff; its initial list clears the problem.
5. Relists must replace the list. **Accepted**: `Init`…`InitDone` builds a new map,
   swapped in at `InitDone`.
6. Status-only changes would render. **Accepted**: a snapshot is sent only when the
   converted deployments differ.
7. Reloads. **Accepted**: one `Cluster` per workspace, `set_config` in place; a target
   whose file content changed (a rotated token) reconnects; adding `[cluster]` creates
   the cluster, removing it empties it and hides the Cluster activity.
8. A README can't be told from a broken kubeconfig. **Accepted**: hidden files, `*.md`,
   `*.txt` skipped; other files that don't parse and duplicate contexts are workspace
   problems (the Problems view).
9. "Running since". **Accepted**: "rolling out" while a rollout runs, the creation time
   only without the condition; an injected clock (snapshots stay fixed). Known limit:
   a scale changes the condition's time; step 2 revisits it with pods.
10. The matrix clones per cell and isn't virtualized. **Accepted**: by reference,
    `uniform_list` rows.
11. The spike view is in the demo layout and generic tests. **Accepted**: the matrix
    replaces it in the demo layout; those tests use another view; snapshots updated.
12. kube details. **Accepted**: clients built on the runtime; one client per file,
    cluster and user; connect timeout 5 s; handles abort their task; the mock keeps the
    watch open; `tokio::time::pause` for the settle delay.
13. Watching from first use contradicts "per visible env". **Accepted, user decision**:
    watches stay from first use until the workspace closes (no CPU while idle), recorded
    in DESIGN.md; can be revisited.
14. Model gaps. **Accepted**: optional requests and limits, quantities parsed; the digest
    split first, a digest-only image has no tag; per-deployment restarts wait for pods
    (step 2).
15. Logs and pod views on the kube driver; `command_for` used the env as namespace.
    **Accepted**: they say "not read yet", the logs action is disabled with that reason;
    each env keeps its target's namespace.
16. The kubeconfig folder needs its own reload route. **Accepted**: its own watch; a folder
    missing at start is looked at again on a config save.

### Interfaces (fixed before building)

`devdeck-kube` (GPUI-free):
```rust
pub struct Kubeconfigs { /* contexts by name, with their file */ }
impl Kubeconfigs {
    pub fn load(folder: &Path) -> (Kubeconfigs, Vec<String> /* problems */);
    pub fn target(&self, env: &str, context: &str) -> Result<Target, String>;
}
#[derive(Clone, PartialEq)] pub struct Target {
    pub env: String, pub file: PathBuf, pub context: String, pub namespace: String,
    pub cluster: String, /* for messages */ pub fingerprint: u64, /* file content */
}
pub enum EnvUpdate { Deployments(Vec<Deployment>), Problem(Option<String>) }
pub struct WatchHandle; // aborts on drop
pub fn watch(target: &Target) -> (async_channel::Receiver<EnvUpdate>, WatchHandle);
```
App (`cluster.rs`):
```rust
pub trait ClusterDriver {
    /// Starts feeding `env`'s deployments into the entity.
    fn watch(&self, env: &Env, cx: &mut Context<Cluster>) -> Option<Task<()>>;
    /// Why writes aren't possible, if they aren't.
    fn writes(&self) -> Result<(), String>;
    /// Whether pods and metrics are read (the fake: yes; kube: step 2).
    fn reads_pods(&self) -> bool;
}
```
Pods, rollouts, history and metrics stay in the entity; the fake fills them as today.

### Step 1 outcome (2026-11-07)

Built as planned, with the review's changes; `devdeck-kube` by a builder, the app side by
the lead. Departures and findings:
- kube-runtime 5.0.0's watcher, after a failed watch, retries from its last resource
  version with no backoff and resumes without an event (confirmed in its source): the
  driver drops it and lists again after a backoff, as planned. A 410 (expired version)
  relists without showing a problem.
- The user's folder has a context in two files (centraladmin01, devstore01) that no env
  uses: a duplicate is a problem only for an env using it.
- A list kept over a lost connection shows its problem above the sidebar's list and in
  the service view. A rollout seen on the cluster says "rolling out" next to the tag,
  without a spinner (a stuck one would animate forever).
- A new driver (config change) drops the previous driver's pods.
- `scripts/measure.py`'s `kube` scenario (M0's environment variables) is removed; step 6
  measures the real cluster.
- Read-only check against the real cluster: dev35 lists 67 deployments and hades 10, as
  `kubectl get deploy`; envoy-edge (3 containers), executablecdn (3) and frontendapp (2)
  show "+n". Idle renders with every env watched are measured at the close (step 6); the
  tests show an unchanged list sends and renders nothing.
- Snapshots added: `94-svc-lost-connection`, `95-svc-deployments-only`; the demo layout's
  spike view is now the branch matrix (`01-initial` and the scale scenes change).

### Review of the step 1 diff (subagent, 2026-11-07) and assessment

1. A replaced driver's kubeconfig load could still land (old problems, restarts on the
   new driver). **Fixed**: the cluster keeps the load task; a newer start, a new driver
   or `clear` drops it. Test: a newer load wins over an older one still running.
2. A silently dead connection showed no problem for ~5 minutes. **Fixed**: watches last
   60 s, so the next one fails within about a minute.
3. Views asked for envs only when built: an env added while the matrix is open, or a
   `[cluster]` removed then added back, stayed "loading…". **Fixed**: one cluster per
   workspace, kept while `[cluster]` is gone and refilled when it returns; the matrix
   wants every env (those added later too); the sidebar asks again when the workspace
   changes. Tests for both.
4. Any config change rebuilt the driver and relisted every env. **Fixed**: envs change in
   place (a protected flag touches no watch), a changed `[cluster]` goes to the same
   driver and reconnects only envs whose target changed. Test.
5. A missing folder read "no context …" for every env, and new kubeconfig problems were
   never toasted. **Fixed**: every target says the folder doesn't exist; new problems
   are a toast once. A folder cloned later is picked up when the watched folders are
   next looked at (any rescan, a config save): **accepted** as is.
6. Steady churn could hold updates back forever. **Fixed**: sent 100 ms after the first
   unsent change. Test.
7. Paused and stuck rollouts read "rolling out" forever. **Fixed**: `Progress` (settled,
   rolling, paused, stuck), each with its label. Test.
8. Clients were never dropped. **Fixed**: a client goes with its last watch. Test.
9. Duplicated helpers. **Fixed**: `config::shown` and `fnv1a` in devdeck-core.
10. "+n" without the other containers on hover in the sidebar; matrix cell ids could
    collide. **Fixed**: list rows take an optional hover text; ids from the env count.
11. Test holes. **Fixed** as listed above; restarting only changed targets is covered end
    to end by the watcher test (a context added reconnects that env).
12. `lifecycle.rs` `one_devdeck_per_state_folder` fails about 1 run in 12, on the last
    commit too (not this step): **out of scope**, reported to the user.
