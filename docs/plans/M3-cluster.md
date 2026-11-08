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
  Changed 2026-11-08 (user): a git panel per app and service with switch, fetch, pull and
  create branch; no commit or push (step 3).

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

## Step 2 · Service inspector on real data (plan, 2026-11-08)

Requirements: R7.1 (restarts, running since), R7.2, R7.8, R5.5.

Seen on the real cluster (read only): about 100 pods per dev env, 20 on hades; the
account can list and watch pods and read `metrics.k8s.io` (`kubectl top` works), and has
logs, exec and port-forward (step 4); no ReplicaSets, no events.

### Decisions (user, 2026-11-08)

- A **watch** toggle in the Cluster list's header, off by default, remembered per
  workspace (in its sidebar state). On: every pod of the list's env is watched and rows
  get a warning mark (a pod crash-looping, failing to pull, or not ready) with the reason.
  Off: only the pods of services open in a view are watched, one watch per open service,
  filtered by its deployment's labels; the list shows no pod marks. With the toggle on,
  open services use its env-wide watch instead of their own.
- Deployment lists stay watched from first use (step 1). The toggle is about pods only.

### Model (`devdeck-core::cluster`)

- `Deployment.selector: BTreeMap<String, String>` (its `matchLabels`).
- `Pod`: `labels`, `template` (its `pod-template-hash`), `started: Option<u64>` (epoch,
  replaces the fixed `age_s`), `containers: Vec<ContainerState>` (name, ready, restarts,
  state text: "running", "waiting: CrashLoopBackOff", "terminated: Error (exit 1)", last
  termination), `last_termination: Option<Termination>` (reason, exit code, message,
  when), `usage: Option<Usage>` (none until metrics are read). `tag`, `node`, `phase`,
  `ready`, `restarts` stay.
- `Selector::matches(labels)` (every `matchLabels` pair present).
- The demo builds the same (labels `app=<name>`, one container, a template per tag).

### devdeck-kube

- `watch_pods(target, selector: Option<label selector>) -> (Receiver<PodUpdate>,
  WatchHandle)`: the deployments watch made generic over the resource and its
  conversion (same relist, backoff, settle, timeout and send-only-changes rules), on
  pods of the namespace, optionally filtered. `PodUpdate::{Pods(Vec<Pod>), Problem}`.
- `convert::pod`: phase from the pod's phase, `deletionTimestamp` (Terminating) and the
  main container's waiting reason; ready from the `Ready` condition; restarts summed;
  last termination from the container with the latest one; tag from the main container
  (named like the deployment's app label, else the first); node; start time.
- `metrics(target) -> Receiver<Result<BTreeMap<pod, Usage>, String>>`: one list of
  `metrics.k8s.io/v1beta1` PodMetrics in the namespace (`DynamicObject`, quantities
  parsed, containers summed). 403: "your account can't read metrics in <env>"; 404:
  "metrics aren't available on <cluster>".
- Errors for pods: 403 "your account can't list pods in <env>".

### The cluster entity

- Pods come from sources per env: the env-wide one or one per service. `pods(env, name)`
  returns the deployment's pods (by selector from the env-wide source, else its own),
  owned (`Vec<Pod>`); `find_pod` searches both.
- Interest handles, like `Sampling`: `pods_interest(env, Some(service) | None)` returns a
  handle; dropping the last one of a key sends its key on a channel the entity listens on
  (no timer), which stops that watch. `refresh_pods(env)` keeps the driver's pod watches
  in line with the interests: the env-wide one alone when held, else one per held
  service whose deployment (and selector) is known; called on an interest, a drop, a
  new deployments list (a selector arrives or changes), and env or driver changes.
- Driver: `watch_pods(env, selector, cx)`, `sample(env, cx)` (one metrics read; the fake
  walks its numbers), `metrics_every()` (fake 2 s, kube 15 s), `logs()` (why logs aren't
  shown: kube "logs come in step 4 of M3"; the fake streams its own). `pods()` goes.
- Metrics per env: `Sampling` handles per env; while any is held, the env's metrics are
  read every `metrics_every()` and merged into its pods' `usage` (kept across pod
  updates by name). A metrics problem is kept per env and shown in the Metrics section.
- "Running since": `Cluster::since(env, d)`: when the deployment's pods are known, the
  earliest start among the pods of the newest template (the template of the most
  recently started pod); else the deployment's own `since` (step 1). A scale no longer
  moves it; a restart does.
- The fake fills an env-wide source at load as today (rollouts replace a service's pods
  in it).

### Views

- Sidebar: the **watch** toggle (Cluster activity only; also a registered action, in the
  palette and cheat sheet, without a default key), holding an env-wide interest for the list's env while on; rows get a warning (reason: the worst
  pod state, "CrashLoopBackOff", "ImagePullBackOff", "1/2 ready") with the tag in the
  hover text. Off: no marks.
- Service view: holds its service's interest while open and `Sampling` for its env while
  visible. Pods table from real pods (age from `started`, last termination with its
  age); "not read yet" goes; a pods problem shows in place of the table; metrics bars
  sum pod usage; logs action disabled with the driver's `logs()` reason.
- Pod view: holds its service's interest (the deployment from the pod's name:
  `<deployment>-<hash>-<id>`); the made-up "Events" section becomes "Containers" (each
  container's state, ready, restarts, last termination) plus the pod's usage.
- Logs view: on a driver whose `logs()` refuses, says why and streams nothing (today it
  would invent lines from real pods).

### Tests

- Core: selector matching; demo pods' new fields.
- devdeck-kube: pod conversion from fixtures shaped like real pods (crash loop with last
  termination, image pull back-off, terminating, two containers, pending); the pod watch
  on the mock API (list, changes, selector in the request, 403); metrics parsing (sums,
  quantities), 403 and 404.
- App: interests (a service view opens and closes its watch; the toggle replaces the
  service watches and gives them back when off; nothing watched when nothing holds);
  sampling per env only while visible, at the driver's pace; "since" from pods (scale
  keeps it, newer template moves it); the toggle's state saved; logs refused with a
  reason; the existing fake tests unchanged in behaviour.
- Snapshots: a service with a crash-looping pod (kube-like driver), the pod view's
  containers, metrics refused, the list with the watch toggle on and marks.
- By hand, read only: dev35 pods of one service match `kubectl get pods -l`; metrics
  match `kubectl top`.

### Out of scope

Git (step 3); logs, shell, port-forward, k9s (step 4); writes (step 5).

### Review of the step 2 plan (subagent, 2026-11-08) and assessment

Every point accepted; the plan above reads with these changes.
1. A changed target didn't restart pod watches. **Accepted**: `restart(env)` restarts the
   env's pod watches and clears its metrics problem too.
2. Pods had no loading state. **Accepted**: pod sources hold `Remote`-like state (no list
   yet, a list, a problem next to a kept list); per-service sources stay until the
   env-wide one has listed; a source's pods go with its watch.
3. `matchLabels` alone is wrong. **Accepted**: the request selector is built from the full
   `LabelSelector` (kube's `Selector`), kept as a string on the deployment; pods are
   grouped by owner (the ReplicaSet in `ownerReferences` minus `-<pod-template-hash>`),
   which also gives the main container (named like the deployment).
4. Pod name → deployment. **Accepted**: the longest known deployment `d` with the pod named
   `d-<x>-<y>`; the owner confirms once the pod is there.
5. Per-render cost. **Accepted**: pods grouped by deployment once per update, by
   reference; usage joined at read time from a per-env `pod → Usage` map (`Pod.usage`
   goes); a pods update notifies only when the grouped result changed.
6. Interests and lost envs. **Accepted**: holding an interest wants the env; the interest
   table survives `set_envs`/`clear`/`reload`, which re-want and refresh from it.
7. The sidebar's interest. **Accepted**: computed in `load_list` (already called on every
   Services change), replaced when its key changes; held only while the Cluster activity
   shows with the toggle on. An env-wide pods problem shows under the list header, like
   a deployments problem.
8. R7.1. **Accepted**: with the toggle on, a row's hover text says the tag, running since
   and restarts.
9. Pod phases. **Accepted**: `Failed(reason)` and `Succeeded`; init containers read;
   marks from waiting reasons only (CrashLoopBackOff, ImagePullBackOff, ErrImagePull,
   CreateContainerConfigError…), never from Terminating/Failed/Succeeded pods or plain
   "not ready"; pending pods (no start) left out of "since".
10. "Since" from pods. **Accepted**: `None` unless settled; Running pods only.
11. Metrics. **Accepted**: the pod view samples while visible; the first read is at once;
    no read while one is in flight; after a 403/404 an env isn't read again until a
    reload.
12. Driver API. **Accepted**: `watch_pods` takes the source key; new trait methods have
    defaults; the fake rollout's `refresh_pods` becomes `roll_pods`.
13. Tests and runs. **Accepted**: a scripted driver counting live pod watches (start,
    drop, restart on selector, target and driver changes, `clear` and back); the stress
    run opens a pod view, flips the toggle and reports live pod interests; the test and
    snapshot edits listed.
14. Drop channel. **Accepted**: the strong count is checked on receipt; the pod's tag
    comes from the spec image.

### Step 2 outcome (2026-11-08)

Built as planned with the review's changes; devdeck-kube by a builder (pod conversion,
the watch made generic over deployments and pods, metrics), the app side by the lead.
- Pod state lives in `cluster_pods.rs`: interests, pod watches and their sources, pods
  grouped by owner, usage per env, sampling. `Cluster::pods` stays a slice (empty when
  unknown); `pods_state` gives loading and problems.
- A metrics read is bounded to 10 s (a silently dead connection would block that env's
  metrics forever); the builder raised it.
- Before the first metrics read, the service and pod views say "not read yet" rather than
  0; a pod missing from a read (a crash-looping one) has "no cpu or memory reported".
- Read-only check on dev35: entityservice's pod, its state, restarts, tag and start
  match `kubectl get pods`; its metrics match `kubectl top` (5m, 38Mi); the full
  selector (three labels) came through; 26 of 98 pods have no deployment owner (jobs,
  completed pods) and are left out; no pod in trouble.
- The stress run opens a pod view and flips the watch toggle every cycle, and fails on a
  pod interest left over.
- Snapshots: `95-svc-kube-like` (pods watched for the open service, one crash-looping,
  logs and deploys refused), `96-cluster-watch` (a row marked CrashLoopBackOff); the pod
  view's containers in `91-pod-crashloop`.

### Review of the step 2 diff (subagent, 2026-11-08) and assessment

1. Metrics stopped for good after a cluster clear, an env removed or a new driver: views
   kept their handles but nothing restarted the reading. **Fixed**: envs still sampled get
   their reading back. Test.
2. A finished init container (exit 0) showed as every healthy pod's last termination.
   **Fixed**: only a non-zero current end counts, else the previous one. Test.
3. The list's pods stayed watched while the sidebar was hidden. **Fixed**: held only while
   the list is on screen. A workspace not displayed keeps its own (switching doesn't stop
   anything, R2.2): **accepted**.
4. Turning **watch** off flashed "loading pods…" in open services. **Fixed**: the env-wide
   pods stay shown until the services' own watches have listed. Test.
5. A pod watch failing after listing showed old pods without a word. **Fixed**: the
   problem shows above the kept pods.
6. The pod view never checked its guessed deployment against the pod's owner. **Fixed**:
   it follows the owner once known; a pod no deployment owns says so.
7. A handle taken again while the loop waited read only at the loop's next turn.
   **Fixed**: read at once. Test.
8. `metric_ticks` counted reads that never started; the in-flight guard was untested.
   **Fixed** and tested.
9. A refused env was read again only after a target change, not a reload. **Fixed**: a
   reload retries refused metrics.

## Step 3 · Git panel (plan, 2026-11-08)

Requirements: R6.10 (rewritten), R7.3 (branches as tag suggestions), R5.1, R5.5.

### Decisions (user, 2026-11-08)

- A git panel per app and service, "a proper useful UI": see branches, change branch,
  etc. Not read only. No commit, no push (later if at all); worktrees stay a later idea.

### How git runs

- Through the `git` program, not a library: the user's SSH keys and agent, credential
  helpers, hooks and config behave as in their terminal; no new dependency.
- `devdeck-core::git` (new, GPUI-free): runs `git -C <repo> …` off the UI thread with the
  login shell's environment (the toolbox's, else devdeck's own), stdin closed,
  `GIT_TERMINAL_PROMPT=0` (never waits on a prompt), `GIT_OPTIONAL_LOCKS=0` for reads
  (no index lock taken by a status), a timeout (reads 10 s, fetch and pull 60 s: then
  killed, "… took too long"), output captured; a failure gives git's own message.
- Reads: `status --porcelain=v2 --branch -z` (branch, upstream, ahead/behind, changes with
  their kind, staged or not, conflicts) and `for-each-ref` over `refs/heads` and
  `refs/remotes` with a fixed format (name, upstream, commit, subject, author, date),
  remote `HEAD` symrefs left out.
- Writes, one at a time per repo: `switch <branch>`; a remote branch without a local one:
  `switch --track <remote>/<branch>` (creates the local branch); `fetch --prune`; `pull
  --ff-only`; `switch -c <name>` (the name checked with `check-ref-format --branch`
  first). Nothing forced: no `-f`, `reset`, `clean`, `--discard-changes`. When git refuses
  (local changes would be overwritten, a pull that isn't a fast-forward), its message is
  shown and nothing else happens.
- The repo of an app: the nearest folder holding `.git` (a folder, or a file for a
  worktree or submodule) from the app's folder up to its workspace folder; apps of one
  monorepo share it.

### The repo entity (devdeck-app)

- `Repo` per repository root, shared by every view showing it (the services keep weak
  ones by root): status and branches as last read, the operation running (and its
  output), the last failure.
- Refreshed when a view holding it opens or comes on screen, after each operation, on
  `r`, and when git's files change: the git dir (`HEAD`, `index`, `FETCH_HEAD`,
  `packed-refs`, non-recursive) and `refs/` (recursive) are watched while a holder lives
  (a worktree's own and common git dirs both). Edits in the working tree alone aren't
  watched (a repo can hold `node_modules`): the changes count updates on the next refresh.
- No reads while nobody holds it: nothing runs while idle.

### Services and apps linked

- A service's app: the app whose folder name is the deployment's name, else the image's
  short name; or an override, per app in the workspace file:
  `[apps.<id>] deployments = ["canvasmodulesservice"]` (an app can deploy as several).
- A branch's tag: the branch name with every character a Docker tag can't hold
  (`[A-Za-z0-9_.-]` only) made `-` (`feature/x` → `feature-x`). A branch runs on an env
  when its service there runs that tag (heuristic: the CI's naming isn't known; exact
  names match first).

### Views

- **Git panel** (`git/<app id>`, tab "<app> · git"), opened from the app view, the service
  view (its app), the palette and their menus:
  - header: the repo (`~/…`), the current branch (or "detached at <commit>"), its
    upstream with ahead/behind, clean or "N changes";
  - actions: fetch, pull, new branch…, refresh; busy while one runs ("fetching…"), its
    failure shown under the header with git's message (also a toast);
  - changes: the changed files (kind, staged), up to a screenful, then "and N more";
  - branches: local, then remote ones without a local branch of the same name, each with
    its last commit (subject, author, age) and the envs running it (env chips,
    protected in red); the current one marked; switch with ⏎, a double click or the row
    menu; new branch asks a name (modal, ⏎ creates and switches);
  - keys registered as actions (palette, cheat sheet): `f` fetch, `p` pull, `n` new
    branch, `r` refresh, ⏎ switch.
- **Service view**: a line under the header: its app and repo, the branch checked out
  locally, and "N commits on origin/<branch> since this rollout" when the running tag is
  a branch of the repo (counted from the rollout's time: `rev-list --count --since`);
  a "git" action opening the panel; unlinked: "no app of this workspace deploys as <name>"
  with how to link it (the override).
- **Tag suggestions**: the linked repo's branches as tags ("branch"), after the tags on
  other envs.
- **App view**: a "git" action and the current branch next to the app's folder.

### Tests

- Core, against real temporary repositories made with `git` in a `TempDir` (a bare
  "origin" and a clone): status (clean, changes of each kind, staged, conflict,
  detached, upstream ahead/behind); branches (local, remote, upstream, remote HEAD left
  out); switch, track a remote branch, fetch (a new remote branch appears), pull
  fast-forward, pull refused when diverged (git's message, nothing changed), create (bad
  name refused), switch refused over local changes; timeout kills; tag of a branch; repo
  root (dir, `.git` file, none).
- App: one `Repo` per root shared and released; refreshed on git dir changes (real
  watcher); operations one at a time; the panel's states (loading, clean, changes,
  detached, busy, failure); service ↔ app link (name, image, override, none); branch ↔
  envs; suggestions; keys.
- Snapshots: panel clean on main with branches and env chips, with changes and a failure,
  the new branch modal; service view with its repo line.
- By hand: a panel on one of the user's real repos, reads only (no switch, fetch or pull
  on the user's repos by devdeck's checks).

### Out of scope

Commit, push, stash, merge, rebase, delete branch, diff viewer, worktrees (later); logs,
shell, port-forward, k9s (step 4); writes to the cluster (step 5).

### Review of the step 3 plan (subagent, 2026-11-08) and assessment

The plan above reads with these changes; every point accepted unless said.
1. `switch` overwrites ignored files (a local `.env` a branch tracks). **Accepted**: every
   switch and merge gets `--no-overwrite-ignore`; tested.
2. `pull` follows the user's config (`pull.rebase=true` in front-end; autostash leaves a
   conflict and reports success). **Accepted**: pull is `fetch` then `merge --ff-only
   --no-autostash --no-overwrite-ignore @{u}`; tested with those configs.
3. Killing git mid-merge or mid-switch leaves a half tree and lock files. **Accepted**: only
   fetch times out (60 s, SIGTERM then SIGKILL after 5 s); anything touching the working
   tree is never killed ("still running…").
4. Hangs and the runner. **Accepted**: a git runner of its own: `setsid` (no terminal, so
   ssh can't prompt), stdin closed, `GIT_TERMINAL_PROMPT=0`, stderr and exit status kept,
   `GIT_DIR`/`GIT_WORK_TREE`/`GIT_INDEX_FILE` removed from the environment.
5. The service ↔ app link misses on real names. **Accepted**: names compared lowercased
   without non-alphanumerics; a project's `.gitlab-ci.yml` `variables: IMAGE_NAME` is
   read (boardservice, nestjs-canvasmodules → nestjscanvasmodules…); an override
   (`deployments = [...]`) wins and removes implicit matches; two apps matching is no
   link and a workspace problem; hidden apps don't link.
6. The CI tags `${IMAGE_NAME}:${CI_COMMIT_REF_NAME}`: exact names. **Accepted**: exact
   match is the rule (the sanitized form stays a fallback); the running tag is also
   looked up in `refs/tags`; tag suggestions offer remote branches only.
7. Missing states. **Accepted**: rebasing, merging, cherry-picking, bisecting (from the
   git dir), upstream gone, no upstream, unborn branch, a branch checked out in another
   worktree; each action gives its reason when unavailable (R5.5).
8. Idle cost and refresh storms. **Accepted**: refresh only while a holder is visible
   (else marked stale, read when it shows), 300 ms debounce, one read at a time plus a
   dirty flag, events during devdeck's own operation ignored, render only on change; the
   service view's line reads HEAD and the running tag only.
9. Watch mechanics. **Accepted**: watch directories and filter by name; the dirs from
   `rev-parse --path-format=absolute --git-dir --git-common-dir --show-toplevel`;
   `reftable/` when present; one shared debouncer (the app's watcher, a route like the
   kubeconfigs folder).
10. Repo lifecycle. **Accepted**: an operation's task holds the repo until git exits, and
    the services keep one lock per root; watch callbacks hold weak handles.
11. "N commits since this rollout" can mislead. **Accepted**: dropped; the line says the
    remote branch's last commit date and the last fetch's time, next to the rollout's.
12. Branch list. **Accepted**: `uniform_list` with a filter; remote HEAD left out by
    `%(symref)`.
13. Branch name check. **Accepted**: `check-ref-format refs/heads/<name>`, `@` refused,
    `--` before names.
14. Monorepos. **Accepted**: the panel is keyed by repo root (`git/<root>`); each app
    opens its repo's.
15. Env chips. **Accepted**: an open panel wants every env (as the matrix); idle watches
    cost no CPU.
16. Tests. **Accepted**: isolated (`GIT_CONFIG_GLOBAL=/dev/null`, `GIT_CONFIG_NOSYSTEM=1`,
    fixed identity, `LC_ALL=C` where messages are checked); tests for 1–3, 7, renames
    under `-z`, no lock files after a fetch timeout. A switch with local changes that
    come along asks first ("N changes come along to <branch>").

### Interfaces (fixed before building)

`devdeck-core::git` (builder):
```rust
pub struct Git { root: PathBuf, vars: Vec<(String, String)> }   // vars: the toolbox's
impl Git {
    pub fn open(dir: &Path, vars) -> Result<(Git, GitDirs), String>; // rev-parse
    pub fn status(&self) -> Result<Status, String>;
    pub fn branches(&self) -> Result<Vec<Branch>, String>;
    pub fn head(&self) -> Result<Head, String>;           // light: branch or detached
    pub fn has_ref(&self, name: &str) -> bool;            // heads, remotes, tags
    pub fn run(&self, op: &Op) -> Result<String, String>; // git's message on failure
}
pub struct GitDirs { pub root: PathBuf, pub git_dir: PathBuf, pub common_dir: PathBuf }
pub struct Status { head: Head, upstream: Option<Upstream>, changes: Vec<Change>, doing: Option<Doing> }
pub enum Head { Branch(String), Detached(String), Unborn(String) }
pub struct Upstream { name: String, ahead: u32, behind: u32, gone: bool }
pub struct Change { path: String, from: Option<String>, kind: ChangeKind, staged: bool }
pub enum Doing { Rebasing, Merging, CherryPicking, Reverting, Bisecting }
pub struct Branch { name, remote: Option<String>, upstream: Option<String>, commit, subject, author, at: u64, current: bool, worktree: Option<PathBuf> }
pub enum Op { Fetch, Pull, Switch(String), Track(String /* origin/x */), Create(String) }
pub fn check_branch_name(git: &Git, name: &str) -> Result<(), String>;
```
`devdeck-core` apps: `App.images: Vec<String>` (from `.gitlab-ci.yml` `IMAGE_NAME`),
`AppOverride.deployments: Vec<String>`; `links::app_for(deployment, image, apps) ->
Result<Option<String>, String>` (the app id, or a problem when ambiguous).

### More decisions (user, 2026-11-08), "super extra careful"

- **Every operation that changes branches, files or the remote asks first** in a modal
  saying in plain words what will happen (switch, new branch, pull, push, abort). Fetch
  doesn't ask (it changes neither your branches nor your files) but shows what came in.
- **Push comes in, as a smart one** (commit stays out):
  1. it fetches the remote first, so the decision is made on its real state;
  2. the branch isn't on the remote: "Publish <branch> to <remote>; later pushes go
     there" (`push --set-upstream`);
  3. the remote is behind (its tip is in your branch): "Send N commits of <branch> to
     <remote>" (a plain push);
  4. history was rewritten (a rebase or an amend) and every commit on the remote is in
     your branch's history (its reflog): "Replace <remote>'s <branch> with yours: the N
     commits there are already in your history, nothing is lost; refused if someone
     pushed meanwhile" (`push --force-with-lease=<branch>:<the tip just fetched>
     --force-if-includes`);
  5. the remote has commits you never had: refused, "pull them first, or handle it in a
     terminal". **Never a force push that can lose anything**; anything else is done by
     hand in a terminal;
  6. up to date: says so, nothing to do.
  The remote: the branch's upstream remote, else its `pushRemote`, `remote.pushDefault`,
  else the only remote; several and none chosen: refused with the reason.
- **An operation under way (rebase, merge, cherry-pick, revert, bisect) is shown, and
  blocks every write**: the only action is **Abort** (`rebase --abort`, `merge --abort`,
  `cherry-pick --abort`, `revert --abort`, `bisect reset`), behind a modal saying what
  goes back and what's lost (conflict resolutions made so far). No rebase button: the
  user rebases through Claude or by hand.

Interface additions: `Op::Push(PushPlan)`, `Op::Abort(Doing)`, `Git::push_plan() ->
Result<PushPlan, String>` (fetches, then decides), `enum PushPlan { Publish { remote,
branch }, Send { remote, branch, commits }, Replace { remote, branch, expected: String,
commits }, Behind { remote, branch, commits }, UpToDate { remote, branch } }` (`Behind`:
the remote has commits you don't have, refused), and `Git::writes_blocked() -> Option<String>`
(the reason, from `Status::doing`).

### Step 3 outcome (2026-11-08)

Built with the review's changes and the user's later decisions (push, abort only during
an operation, every write confirmed); the core by a builder, the app side by the lead.
- git 2.53 facts: `switch -c -- <name>` takes `--` as the name (names starting with `-`
  are refused instead); `check-ref-format refs/heads/<n>` accepts `@` and `-x` (refused by
  hand); `--force-if-includes` does nothing when the lease names its commit, so devdeck's
  own reflog check decides a replace (the flag stays).
- Fetch is refused too during an operation under way (only abort); switch adds
  `--no-guess` (a remote-only branch is tracked explicitly).
- The git tests live in their own binary (`crates/devdeck-core/tests/git.rs`): in the
  unit binary, their many processes made a port test fail on full runs.
- Read-only check on the user's repos (no fetch, switch, pull or push): devdeck,
  front-end (graph-perf-x, 14 ahead and 6 behind origin, 36 local and 206 remote branches,
  read in 189 ms), nestjs-canvasmodules and dev-environments match `git status`.
- Snapshot `97-git-panel` (a throwaway repository: changes, local and remote branches,
  pull unavailable with its reason).

### Review of the step 3 diff (subagent, 2026-11-08, data safety first) and assessment

1. A replace could drop a colleague's commit only left in the reflog (pulled, then
   `reset --hard`). **Fixed**: a replace also needs every commit on the remote to be the
   user's (their author identity) or already in the branch as the same patch; else
   refused. A backup ref `refs/devdeck/replaced/<branch>/<time>` keeps the old tip; the
   modal lists the replaced commits. Tested (both ways).
2. `fetch --prune` with `fetch.pruneTags` deleted local-only tags. **Fixed**:
   `--no-prune-tags` on every fetch; tested.
3. The abort modal understated what's reset. **Fixed**: it lists the tracked files whose
   changes the abort resets.
4. A cherry-pick sequence resolved with `git commit` (only `sequencer/` left) and `git
   am` weren't seen. **Fixed**: detected (`am` aborts with `am --abort`), watched; tested.
5. Stale operations at confirm time. **Fixed**: push sends the planned commit and is
   refused if HEAD moved; pull carries its branch and re-checks after its fetch; the
   switch modal reads the status fresh.
6. Two `Repo`s for one repository (two workspaces, a symlink). **Fixed** in the core: one
   write at a time per repository, process-wide (canonical common dir); tested.
7. Repository discovery went above the workspace folder. **Fixed**: bounded there (and
   `GIT_CEILING_DIRECTORIES`).
8. Hangs and kills. **Fixed**: working-tree operations write to files, not pipes (quitting
   devdeck can't kill git mid-checkout), never time out and show their elapsed time; push
   allows 600 s (pre-push hooks); no GUI askpass. No cancel for a working-tree operation
   (killing it is what would break the repository): **accepted**.
9. Links: an explicit image now wins over folder names; CI values with `$` skipped.
   **Fixed**; tested.
10. Service and app views read the full state. **Fixed**: they read the branches only
    (no working-tree scan). Looking the running tag up among git tags: **not done** (only
    the line's wording depends on it: "no branch <tag> on the remote").
11. Released repositories stayed watched; remote names with `/`. **Fixed** (the watch goes
    with the repository); remote names with `/`: **accepted** (rare).
12. Test holes. **Fixed** as listed; plus an app test refusing a second operation while
    one runs.

Found while checking snapshots: the demo's apps point at made-up folders that exist on the
user's machine, so the demo opened their real repositories. **Fixed**: the demo opens only
throwaway repositories in the temp folder.

## Step 3b · Edit mode (plan, 2026-11-08)

Requirements: R5.6 (new), R5.5, R7.6, R6.10.

### Decisions (user, 2026-11-08)

- An **edit mode**: a warning button in the title bar. Turning it on asks first in a modal
  saying what it allows; turning it off doesn't ask. Off at every start; no auto-off.
  One switch for the whole app.
- Only in edit mode can a potentially destructive action run: a deploy (set image,
  undo, restart, replicas — step 5; the demo's simulated ones too), a git **replace**
  push (it rewrites the remote branch's history), a git **abort** (it resets changes),
  stopping a process that isn't one of devdeck's runs ("Stop holder"), a shell in a pod
  (step 4); k9s opens `--readonly` unless edit mode is on (step 4). Not gated: fetch,
  switch, pull, new branch, publish and plain pushes, stopping devdeck's own runs, ending
  terminals, logs, port-forward.
- **devdeck never pushes `main` or `master`**, not even in edit mode: refused in the
  core (publish, send and replace alike), the push button says why.
- A protected env still asks for its name on top of edit mode (R7.6).

### Design

- `edit_mode.rs` (app): a global (`EditMode { on: bool }`), `is_on(cx)`, `turn_on(cx)` (after
  the modal), `turn_off(cx)`, observable (views redraw their buttons). Not saved.
- `edit_mode::refusal(cx) -> Option<String>`: "turn on edit mode first (the warning button
  in the title bar)" when off. The gated places check it themselves, below the UI:
  `Cluster::set_image` and `Cluster::undo`, `Repo::run` for `Op::Abort` and
  `Op::Push(PushPlan::Replace)`, `port_scan::stop_holder` for a process that isn't a run.
  Their buttons show the reason disabled (R5.5); the palette and keys go through the same
  checks.
- Title bar: off, a dim warning icon ("edit mode is off: destructive actions are
  refused"); on, a red chip "edit mode" with the icon. Click on → modal ("Turn on edit
  mode?": what it allows, that protected envs still ask, Cancel / Turn on); click off →
  off at once. An action "toggle edit mode" in the palette and cheat sheet, no key.
- Core (`git.rs`): `push_plan` and `run(Push)` refuse a branch named `main` or `master`
  ("devdeck never pushes main or master"); the panel's push button says so.

### Tests

- App: off at start; set image, undo, abort, replace push and stop-holder of a process
  refused with the reason while off and allowed once on; the modal turns it on, a click
  turns it off without one; the title bar's states; existing deploy tests turn it on
  first.
- Core: pushes of `main` and `master` refused (publish, send, replace), nothing pushed.
- Snapshots: the title bar off and on, the modal.

### Review of the edit mode plan (subagent, 2026-11-08) and assessment

1. The stop-process confirmation killed even if edit mode went off while it was open.
   **Fixed**: checked again at the kill (and before asking).
2. A protected env asked for its name before edit mode refused. **Fixed**: edit mode is
   checked first; Save and Undo show the reason.
3. A replace is only known after the fetch. **Fixed**: refused there with a toast, no
   confirmation; a confirmation confirmed after turning off goes through `Repo::run`'s
   check.
4. Where the cluster check sits. **Accepted**: in `Cluster::writes` (step 5's restart and
   replicas can't forget it); clearing a failed rollout (nothing changes in the cluster)
   stays allowed; the watch restart isn't gated.
5. Tests that break. **Fixed** (they turn edit mode on, the snapshot scenes too).
6. Detached windows. **Accepted**: their title bar has the button; its modal opens in the
   window clicked.
7. What stays ungated. **Accepted**: listed in DESIGN (the leftovers dialog; commands and
   terminals can run anything themselves).
8. main/master: any case; the push button says so before any fetch; another default
   branch name is a known limit (DESIGN). **Accepted**.
9. Bisect: aborting it resets nothing: **not gated**.

### Step 3b outcome (2026-11-08)

`edit_mode.rs` (global, off at start), the title bar button in every window, the gates in
`Cluster::writes`, `Repo::refusal` and the stop of a foreign process, `never_pushed` in the
core. Snapshots `98-edit-mode-modal`, `99-edit-mode-on`. The deploy, port and git tests
turn edit mode on where they deploy, stop a foreign process or abort; new `edit_mode.rs`
tests and a core test for main and master.

### Review of the step 3b diff (subagent, 2026-11-08) and assessment

No way found to run a gated action while edit mode is off: every gate sits below the UI
(`Cluster::writes`, `Repo::refusal`, the stop of a foreign process checked at the kill).
The holes were in the UI and the tests:

1. The git abort button looked enabled while off. **Fixed**: dimmed with the reason;
   `abort()` checks the refusal first.
2. Snapshot states could leak edit mode into the next scene. **Fixed**: turned off per
   state.
3. The port refusal toast offered "Stop holder" for a foreign process while off.
   **Fixed**: offered only for a run of devdeck's or once edit mode is on.
4. Enter on the modal turned edit mode on. **Fixed**: focus starts on Cancel, so a stray
   Enter cancels; Tab then Enter turns it on.
5. The modal's git line left out merge, cherry-pick, revert and am. **Fixed**.
6. Missing asserts. **Fixed**: the panel refuses a push of main; the port test checks the
   offer and the direct call while off; `edit_mode.rs` covers the modal's keys, a deploy
   confirmed after turning off, and the git refusals on a real repository.

Not part of this step: the core test `ports::tests::a_real_ipv4_listener_is_found` fails
now and then in full runs only (passes alone).
