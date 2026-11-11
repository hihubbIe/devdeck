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

## Decisions (user, 2026-11-10)

- No deploy undo: a failed deploy is fixed by deploying another branch. Failure
  detection stays, with its reason.
- kube-rs for logs, shell, port-forward and writes (its `ws` feature for shell and
  port-forward): one connection path, errors in devdeck's words, kubectl not needed.
- No k9s: devdeck's own UI does what the user needs (the plan of step 4a, k9s in a
  terminal, is kept below, dropped).
- Every deploy seen on the deployment watch is recorded in the history, whoever made it,
  with its branch.
- Desktop notifications per service and env (the service view's env), the events picked
  in a dropdown on the service view (also an action, R5.1): new branch deployed, same
  branch redeployed, rollout finished, rollout failed, pods crashing outside a deploy.
  Always the desktop's own notification, not a toast, whether devdeck is in front or not.
  Off at every start. Step 7 plans them.
- What's left is a list of features built in order of what matters, one at a time, each
  planned, reviewed and committed on its own: logs, changing the deployed branch,
  redeploying the same branch, notifications ("99% of what I need"), then the close; the
  rest when wanted.

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
Steps 4 to 8 replace the earlier "4. logs, shell, port-forward, k9s; 5. writes; 6. close"
(user, 2026-11-10). Cluster writes are tested against a fake API only: devdeck's own
tests and checks never write to the user's cluster; the user tries writes by hand on an
env of their choice.

4. **Logs** (R7.5): real logs of a service's pods on the logs view (several pods,
   follow, pause, level filter, find).
5. **Change the deployed branch** (R7.3, R7.4, R7.6): the tag editor sets the image
   (patch) with the exact command in the confirmation; the rollout followed from the
   deployment's status and its pods (image pull and crash detection with the reason, no
   undo); protected envs; the history in the state folder, of every deploy seen.
6. **Redeploy the same branch** (R7.3): its newest build (rollout restart).
7. **Notifications**: desktop notifications per service and env, events picked on the
   service view.
8. **Close**: idle CPU and memory with every env watched, the whole suite, all
   snapshots, leak run with cluster views, manual test guide.

Later, when wanted, one at a time: shell in a pod (kube `ws`), port-forward (shown in
Ports), replicas, restart, delete a pod, jobs and cron jobs, config maps and secrets,
services and ingresses, a resource's YAML, the control API's cluster methods (its phase
6). Events stay out: the user's account can't read them (checked again 2026-11-10).

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
  Changed (user, 2026-11-10): 2 and 3 run at once after the check, no modal; only 4 asks.
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

## Step 4a · k9s (plan, 2026-11-10) — dropped

Dropped by the user on 2026-11-10, after its review: no k9s, devdeck's own UI does what
matters. Kept as a record.

Requirements: R7.5 (open k9s scoped to the env), R5.6 (k9s read only unless edit mode is
on), R6.12 and R6.13 (terminals, opening somewhere), R3.14 (its tab), R5.1, R5.5, R2.3,
R1.4 (k9s optional, detected).

Checked on 2026-11-10 against the k9s README and `k9s help` (v0.51.0 on this machine):
`--kubeconfig <file>`, `--context <name>`, `-n, --namespace <ns>`, `--readonly` (sets read
only over its config's `readOnly`), `--write` (sets write mode over it).

### Decisions (defaults stated to the user, 2026-11-10)

- k9s is a terminal like the others: listed in Terminals, outlives its tabs, ended with
  Stop, started again with ⏎ (R6.12). It runs k9s itself, not a shell: quitting k9s ends
  the terminal, which then shows it ended.
- Opened from the service view (that view's env) and from the Cluster list's header (the
  list's env), plus a palette action for the list's env.
- Edit mode is read at each start of k9s: off, `--readonly`; on, no flag (the user's own
  k9s config decides; devdeck never passes `--write`). A k9s started read only stays so
  until it's started again.
- Turning edit mode off while a k9s started without `--readonly` runs leaves it running
  (devdeck can't change a running k9s's flags) and says so: a toast naming those k9s,
  "still able to write: quit them to end it". The edit mode modal's line says k9s opened
  from then on can write.

### Design

- **What a terminal runs.** Today every terminal runs the user's shell (`SHELL_LINE`).
  A terminal gets a program: `Shell` (today's) or `K9s { env }`. It lives in the view
  address, so a layout restored after a restart starts the same thing on ⏎:
  `ViewAddr::K9s { n, env }`, `k9s/<n>/<env>` (terminal numbers stay one sequence per
  workspace, `Deck::next_terminal` counts both). Its folder is the workspace's first
  folder (k9s doesn't care; the runner needs one).
- **One k9s per env and workspace.** Opening k9s for an env that has one shows that
  terminal here (as Terminals' **Open** does), running or not; a new one is made only
  when none exists.
- **The line.** `exec k9s --kubeconfig <file> --context <context> --namespace <ns>`,
  each value quoted for `/bin/sh` (`devdeck_core` gets a public quoting function; the
  toolchains' private `quote` moves there), plus ` --readonly` when edit mode is off at
  that start. The target (file, context, namespace) comes from the cluster driver:
  `ClusterDriver::k9s_target(env) -> Result<K9sTarget, String>`; the kube driver answers
  from its kubeconfigs (the same `Target` its watches use), the fake answers "the demo
  has no kubeconfig". The target is read at each start, so a changed kubeconfig applies on
  the next ⏎.
- **Runner.** `Runner::open_terminal` takes the program. For `K9s`, the runner asks a
  provider the workspace's services give it (`set_k9s(Rc<dyn Fn(&str, &App) ->
  Result<String, String>>)`, the line for an env with edit mode applied) at every start;
  an `Err` refuses the start with that reason (R5.5), shown in the terminal's tab like any
  run that can't start. k9s missing from the toolbox refuses the same way ("k9s not
  found"). No `DEVDECK_SOCKET` / `DEVDECK_TERMINAL` for k9s: it isn't a shell (R13.4), so
  nothing it starts is that terminal's caller.
- **Where it opens** (R6.13). The service view's **k9s** button: in the service view's
  pane. The Cluster list header's button and the palette action: the focused pane. The
  service view's and the Cluster list header's right-click menus get "Open k9s ▸" with
  every place, as "Open terminal here".
- **Availability** (R5.5, one reason, in this order): no cluster (the action isn't there
  at all, R2.3); k9s not found; the env's target can't be had (the driver's reason,
  e.g. "no context hades00-hecate in ~/…/kubeconfig").
- **Labels** (R3.14). Tab: a terminal icon, "k9s · dev35" (a protected env in red), "read
  only" in its tooltip when started so. Terminals list: "k9s @dev35", what it shows as
  today. The API lists it among terminals with its program (`terminal.list` gains a
  `program` field: "shell" or "k9s"); reading its output works as for any terminal.
  Opening k9s through the API is out of scope.
- **Edit mode.** `edit_mode::ALLOWS` loses "undoing a rollout" (2026-11-10 decision) and
  says "k9s opened from now on with its write actions". `turn_off` asks every workspace's
  runner for k9s terminals running without `--readonly` and toasts their names.

### Files

- `devdeck-core`: `view_addr.rs` (`K9s`), a public shell quoting function (moved from
  `toolchains/project.rs`).
- `devdeck-app`: `runner.rs` (terminal programs, the k9s provider, refusals),
  `cluster.rs` and `kube_cluster.rs` and `fake_cluster.rs` (`k9s_target`), `services.rs`
  (gives the runner its provider), `deck.rs` (`open_k9s`, terminal numbering),
  `views/mod.rs` (building the view: the terminal view over the run; labels),
  `views/service_view.rs` (the button), `sidebar.rs` (header button, Terminals row),
  `context_menu.rs` ("Open k9s ▸"), `actions.rs` / `palette.rs` (the action),
  `edit_mode.rs` (text, the toast), `api/` (the `program` field), `bin/snapshot.rs`.
- `devdeck-api`: `types.rs` (`program`).
- Docs: `DESIGN.md` (Cluster: k9s), `REQUIREMENTS.md` (R7.5 progress), `ROADMAP.md`.

### OS (R11.6)

No new OS capability: k9s starts through the runner's pseudo-terminal, which goes
through `devdeck-sys` (macOS step 1d). Same on Linux and macOS.

### States to check

k9s running in a tab (read only, and started in edit mode); k9s quit (the terminal
ended, ⏎ starts it again); the same env opened twice (one terminal, shown here); a
protected env's tab in red; k9s not found (button and action disabled with the reason);
a missing context (refused with the driver's reason); the Terminals list with a shell and
a k9s; a layout restored after a restart ("not started", ⏎ starts k9s); a workspace
without a cluster (no k9s anywhere); the toast when turning edit mode off with a
write-capable k9s running.

### Tests

- Core: `ViewAddr::K9s` round trip (an env with odd characters); quoting (spaces, `'`,
  `$`).
- App, on the fake run driver (which records each `Launch`): the line with the target's
  three values and `--readonly` while edit mode is off, without it while on; read again
  at each start (turn edit mode on, quit, ⏎: no `--readonly`); one k9s per env (opening
  again shows the same terminal); refused with the reason when k9s is missing and when the
  target fails; no `DEVDECK_SOCKET` for k9s (pty driver unit test, next to the shell's);
  the k9s terminal outlives its tab and is forgotten like any terminal; the toast on
  turning edit mode off; no k9s action without a cluster; the API's `program` field.
- Snapshots: the service view with its k9s button enabled and disabled (not found); a k9s
  tab and the Terminals list holding it (the fake driver's output).
- By hand: k9s on dev35 opens on the right context and namespace, read only while edit
  mode is off (a delete is refused by k9s), with write actions after turning edit mode on
  and starting it again.

### Out of scope

Logs (4b), port-forward (4c), shell in a pod (4d); k9s starting on a given resource or
filtered on the service (`-c`); opening k9s through the control API.

### Review of the step 4a plan (subagent, 2026-11-10) and assessment

Every point accepted; the plan above reads with these changes. Points 1, 4 and 7 checked
in the code (`api::identify`, `TerminalSummary.program`, the terminal view's build).
1. Leaving out `DEVDECK_SOCKET` doesn't keep k9s's children from being a caller:
   `api::identify` takes any process of any terminal-keyed run as that terminal's shell,
   and the variables grant nothing. **Accepted**: `identify` and `alive` take only
   terminals whose program is a shell; test: a peer chain through a k9s process is
   `Outside`.
2. "Not a shell" is `cmd.source() == "shell"` in the pty driver and `StopKind::of`.
   **Accepted**: k9s's command is `k9s:<n>` with `tools: ["k9s"]`, so "k9s not found"
   comes from the existing missing-tool refusal; it runs in a scope (whatever k9s starts
   goes with it) and stops like an app's run, not with a shell's hang-up.
3. Edit mode on would give a write-capable k9s on a protected env without its name
   (R7.6). **Accepted**: a protected env's k9s is always `--readonly` (the header and the
   palette have no unlock); said in its tooltip.
4. `terminal.list` already has `program` (the foreground program); the API's `View` has
   no variant for a new address. **Accepted**: the new field is `kind` (`shell` or `k9s`);
   `View::K9s` in `devdeck-api`; `handlers::view`, `terminal_places`, `keeps_terminals`
   and `organize.rs` map it; listed in Files.
5. Many sites match `ViewAddr::Terminal`. **Accepted**: `ViewAddr::K9s { n, env }` stays a
   variant of its own (labels and building differ), and `ViewAddr::terminal_number()`
   serves every place that only needs the number (numbering, Terminals, menus, the API).
6. Every start passes the workspace's env as the run env, which an app's binding would
   apply to the k9s line. **Accepted**: k9s runs as a bare app (`EnvApply::None`) with no
   run env; its env lives only in the program.
7. Building a terminal view starts it, so a restored k9s tab starts by itself; refusals
   at ⏎ go to a toast. **Accepted**: a restored k9s starts like a restored shell (read
   only, edit mode being off at start); "not started" applies only to one no layout of
   the current one shows. A provider refusal is a failed start ("couldn't start: …" in
   the tab), at ⏎ also the toast as today; `k9s_target` says "kubeconfigs still loading"
   before the folder is read. States to check updated accordingly.
8. Edit mode has no `turn_off` (one setter, `edit_mode::set`), and nothing records how a
   k9s started. **Accepted**: `Run::read_only: Option<bool>` set at each k9s start; the
   toast comes from `set` on a change from on to off.
9. Title, Terminals list, exit note and quit guard. **Accepted**: `Runner::title` gives
   "k9s · dev35"; the list's doing column says "k9s" (or "k9s · read only"); its end
   says "k9s ended"; the quit guard doesn't ask for a running k9s (nothing is lost by
   quitting it).
10. "One per env" must also see k9s terminals other layouts hold. **Accepted**: looked up
    through `Deck::terminals`, as the Terminals list's **Open**.
11. Share with could paste into k9s. **Accepted**: k9s terminals are left out of Share
    with and `term.paste`; tested.
12. Tests. **Accepted**: launches are checked on the test-local recording driver of
    `tests/app/runs.rs`, not `FakeDriver`; `quote` is made public where it is
    (`toolchains/project.rs`), not moved; the snapshot tool gives the demo a k9s target of
    its own and scripts a k9s-like screen, so the enabled button and a k9s tab are shot as
    well as the disabled states.

Smaller points, both **accepted**: the palette action opens k9s for the focused service
view's env, else the Cluster list's; the check by hand also tries one of the user's k9s
plugins while read only (k9s's `--readonly` may hide only plugins marked dangerous: if
so, a known limit in DESIGN.md).

## Step 4 · Logs (plan, 2026-11-10)

Requirements: R7.5 (logs: several pods, level filter, pause, follow, find), R7.8, R1.3,
R4.4, R5.1, R5.5.

What exists: the logs view (`logs/<env>/<name>`, `views/logs_view.rs`) with its level
filter, find, pause and follow, fed by a made-up feed while visible; on the kube driver it
says "logs come in a later step" and the service view's **logs** button is refused. The
user's account can read pod logs (checked in step 2). kube-rs 5.0.0's `Api::log_stream`
takes `LogParams` (`container`, `follow`, `previous`, `since_time`, `tail_lines`,
`timestamps`), read in its source.

### Decisions (defaults, for the user to change at review)

- **What opens:** every pod of the service, its main container (the one named like the
  deployment, else the first); a deployment with several containers gets a container
  picker in the toolbar.
- **History:** the last 200 lines of each pod when the view opens, then followed. Lines
  carry the server's timestamps (`timestamps: true`): the first tails are merged by time,
  later lines in the order they arrive; the time shows in a column (local `HH:MM:SS`,
  the date in the hover text), the pod's short name in the next one (as today).
- **Pods that come and go** (a rollout): a new pod's logs join from its start; a pod that
  goes keeps its lines, with a line "pod <short> gone" (dimmed).
- **A crashed container:** for a pod whose container restarted, "previous run" in its
  row menu reads the logs of the run before (`previous: true`, last 200 lines, not
  followed), shown between two marker lines.
- **Only while visible** (R1.3): streams run while the view is on screen. When it shows
  again, each pod resumes from the time of its last line (`since_time`), the lines
  already shown at that same second skipped, so nothing is lost or doubled; a pod that
  went meanwhile says so.
- **Pause** freezes what's shown; lines keep coming into the buffer and the toolbar says
  "N new"; resume shows them.
- **Level** of a line: a JSON line's `level`, `lvl` or `severity` (a string, any case; a
  number as pino's: 50+ error, 40 warn, 30 info, below debug), else today's `level_of`
  on its text. The line itself is shown as it came.
- **Buffer:** 10 000 lines for the whole view (5 000 today), the oldest dropped first and
  counted ("N older lines dropped" at the top).

### devdeck-kube

- `logs(target, pod, container, LogFrom, previous) -> (Receiver<LogUpdate>, WatchHandle)`
  where `LogFrom::{Tail(u32), Since(timestamp)}`; follows unless `previous`.
- `LogUpdate::{Lines(Vec<LogLine>), Waiting(String), Ended, Problem(String)}`, `LogLine
  { at: Option<i64 /* epoch ms */>, text }`: the server's timestamp split off the line;
  lines sent in batches (at most every 50 ms) so a busy pod is one message per batch,
  not per line; a line over 16 KB cut there (marked).
- A container that hasn't started (`ContainerCreating`, the 400 "waiting to start"):
  `Waiting(reason)`, tried again every 2 s. A stream that drops (connection lost, the
  server closing it): tried again from the last line's time, backoff as the watches
  (1 s doubling to 30 s), `Problem` while it fails, cleared by the next line. 403: "your
  account can't read logs in <env>" and no retry. The pod gone (404): `Ended`.
- Same runtime and shared clients as the watches; dropping the handle ends the stream.

### The cluster entity

- `ClusterDriver::stream_logs(env, pod, container, from, previous, tx) -> Option<Task>`
  (the fake: its made-up lines, moved out of the view into `fake_cluster.rs`, so the view
  has one path; the kube driver: `devdeck_kube::logs`). `logs()` (the refusal) goes:
  both drivers stream.
- The logs view holds a `PodsInterest` for its service while open (today it reads pods
  only when a service view happens to hold one: on the kube driver it would show "no
  pods").

### The view

- `LogsFeed` (new, `views/logs_feed.rs`): one stream per pod while visible, started and
  stopped as the service's pods change; per pod the last line's time; merging the first
  tails by time; the "gone", "previous run" and "N new" lines. The view keeps the
  toolbar, filter, find and rendering it has.
- Toolbar: "N pods · N lines", the container picker when there are several, a pod's
  problem or waiting reason in its pod column's hover text and in the toolbar ("2 pods
  waiting: ContainerCreating"), a refusal (403) in place of the lines.
- The service view's **logs** button and its action work on the kube driver.
- Rows stay one line each, cut with an ellipsis, wider content scrolling sideways (R4.4).

### Files

- `devdeck-kube`: `logs.rs` (new: the stream, timestamps, batches, retries), `errors.rs`
  (403 for logs), `lib.rs`; mock API tests in `driver/tests.rs` or a new module there.
- `devdeck-core`: `runs.rs` (JSON level, the line's time and pod kept on `Line` or a
  logs line type of its own).
- `devdeck-app`: `cluster.rs` (`stream_logs`, `logs()` removed), `kube_cluster.rs`,
  `fake_cluster.rs` (the made-up feed), `views/logs_feed.rs` (new), `views/logs_view.rs`,
  `views/service_view.rs`, `bin/snapshot.rs` (its driver streams).
- Docs: `DESIGN.md` (Cluster: logs), `REQUIREMENTS.md` (R7.5 progress), `ROADMAP.md`.

### OS (R11.6)

None: network only, through kube-rs.

### States to check

Lines from two pods merged; JSON lines with levels and the filter; a busy pod
(thousands of lines a second: the view stays responsive, the buffer drops the oldest); a
container picker; a pod waiting to start; a rollout (a pod gone, a new one joining); a
crashed pod's previous run; paused with "N new"; hidden then shown (resumed, nothing
doubled); a 403; a lost connection with its problem then back; long lines (ellipsis,
sideways scroll); the demo unchanged.

### Tests

- devdeck-kube, on the mock API: tail and timestamps parsed; follow delivers later lines;
  batches; a 403 refused once; a waiting container then lines; a dropped stream resumed
  from the last line's time; `previous` asked and not followed; a dropped handle ends it.
- Core: levels from JSON (string, any case, pino numbers, not JSON, JSON without a level).
- App, on a scripted driver: two pods' first tails merged by time; a new pod starts a
  stream, a gone pod's line; hidden stops every stream, shown resumes with each pod's last
  time and skips doubled lines; pause and "N new"; the container picker restarts the
  streams; previous run; the pods interest held while the view is open and dropped with
  it; the buffer cap and its count; the existing logs tests on the fake.
- Snapshots: the logs view with two pods and JSON levels; a pod waiting; a 403; paused
  with "N new".
- By hand, read only: a dev35 service's logs match `kubectl logs` (the same lines, the
  same order within a pod); a rollout seen live; the CPU while a busy service streams.

### Out of scope

Logs through the control API (its phase 6); saving logs to a file; following several
services in one view; changing the branch (step 5).

### Review of the step 4 plan (subagent, 2026-11-10) and assessment

Every point accepted; the plan above reads with these changes. Points 1 and 9 checked in
the code (kube-core rounds `since_time` to the nearest second; the service view's pod
menu "Logs" opens the whole service).
1. Resuming could lose or double lines: kube-core rounds `since_time` to the nearest
   second, and milliseconds can't tell a busy pod's lines apart. **Accepted**: the
   server's timestamp kept whole (nanoseconds); the time asked is the last line's,
   truncated to the second; on resume, lines older than the last one seen are dropped,
   and at exactly its timestamp as many as were already seen (a count, not a text
   compare). The same for devdeck-kube's own retries. Tests: a last line at .700 with
   lines at .800 after the resume; two identical lines at one timestamp.
2. On the first list every pod would look new, and gone after a driver change.
   **Accepted**: the feed follows `pods_state`; pods there when the first list arrives
   get the 200-line tail, only later ones are new; a list emptied by the cluster
   forgetting its pods (driver or envs changed, `clear`) isn't "gone". Every `Since`
   also caps the lines read (`tailLines` with `sinceTime`): a long-running new pod or a
   long hidden spell reads at most 1 000 lines a pod.
3. Target changes never reached the streams. **Accepted**: a generation per env, bumped
   by `Cluster::restart` and `set_driver`, that the feed answers by resuming with
   `Since`; a refused env (403) is tried again on a reload, as metrics.
4. Waiting and crashing containers handled by polling and false problems. **Accepted**:
   the app decides from the pod's container states (the held interest gives them): a
   stream starts only for a running or terminated container; a waiting one shows its
   reason from the pod model (ImagePullBackOff…) with no request; a stream that ends
   waits for a change of restarts or state, with no problem shown; devdeck-kube retries
   with backoff on transport errors only. An open stream already catches a crashed run's
   last lines; "previous run" is for a crash before the view opened.
5. A dead connection would hang a stream silently (no read timeout, no server timeout).
   **Accepted**: when the env's pod watch reports a problem and then recovers, every
   stream of that env resumes with `Since`; besides, a stream with no line for 5 minutes
   is ended and resumed.
6. `lines()` dies on invalid UTF-8 and grows an endless line. **Accepted**:
   `read_until(b'\n')` with a byte cap and lossy UTF-8; both tested.
7. Render cost at high rates. **Accepted**: one channel per view, drained at most once
   per frame; the filtered rows, matches and longest line kept up to date as lines come
   and go, not recomputed per render; level and timestamp worked out in devdeck-kube,
   off the UI thread; the view no longer re-renders on every cluster change, only on
   its service's pods. Lines cut at 4 KB (marked), and a byte budget of 16 MB on top of
   the 10 000 lines; the check by hand records the memory and CPU of a busy service.
8. Pause and the cap conflicted. **Accepted**: lines arriving while paused go to a
   capped, counted queue appended on resume; the test asserting an unchanged count while
   paused stays true; `paused` isn't restored from saved state.
9. Controls. **Accepted**: the container picker, "previous run of <pod>" and a pod filter
   are registered actions (palette, cheat sheet, a key each where the view has room),
   each saying why when unavailable (no restart yet; the 400 "previous terminated
   container not found"); the service view's pod row "Logs" opens the logs filtered to
   that pod; the picker lists init containers too (a failing migration is a common failed
   deploy); changing the container clears the lines and starts again with the tails; a
   chosen container missing after a rollout falls back to the main one, said once.
10. A gone pod's last lines. **Accepted**: its stream runs to its end or a 404, 30 s at
    most.
11. Error wording. **Accepted**: `describe` takes the verb ("read logs", "list pods");
    the waiting, previous-not-found and invalid-container 400s get their own messages.

### Step 4 outcome (2026-11-10)

Built as planned with the review's changes; devdeck-kube by a builder (`logs.rs`: the
stream, timestamps, the skip after a resume, batches, retries, errors), the app side by
the lead (`views/logs_feed.rs`, the view, the drivers).
- `LogFrom` derives `Copy` and `PartialEq`; `since_time` is kube-core's `jiff::Timestamp`,
  through `k8s_openapi::jiff` (no new dependency). A resume after a `Tail(n)` start caps
  at `max(n, 1 000)`. 429 and 5xx are problems tried again (a kubelet timeout is
  temporary); a 401 and other codes are refusals. A last line without a newline at EOF is
  still sent; a trailing `\r` goes.
- `describe` takes what was being done ("list pods", "read logs"); deployments' and pods'
  messages are unchanged.
- The fake's made-up feed moved to `fake_cluster.rs` and goes through `stream_logs` like
  the real one; `ClusterDriver::logs()` and `NO_LOGS` are gone, the service view's logs
  button works on every driver.
- Not done: the date in the time column's hover text (devdeck-sys gives only the local
  time of day); the time column shows `HH:MM:SS`.
- Found outside the step: the snapshot tool didn't build on the last commit (control API
  phase 3 made `Connection` private); a `Connection::new` lets it build again.
- Snapshots: `90-logs-json` (two pods' JSON lines with their levels, one waiting on its
  image), `90-logs-refused` (a 403 in place of the lines).
- Not checked by hand on the real cluster: a dev35 service's logs against `kubectl logs`,
  a rollout seen live, the CPU and memory of a busy service; they're the user's manual
  test.

### Review of the step 4 diff (subagent, 2026-11-10) and assessment

1. A busy pod's later lines showed before its first ones while a quiet pod held the merge.
   **Fixed**: every line of a merge's pods is held until it's over. Test.
2. Hiding the view during the merge dropped the held lines, which a resume then skipped.
   **Fixed**: hiding shows them. Test.
3. A stream ended cleanly while the container still ran (a proxy, an API server restart)
   was never asked again. **Fixed**: asked again after 5 s if the pod still says running.
   Test.
4. "Not started yet" and "no such container" stuck until a reload. **Fixed**: only a 403
   or 401 waits for a reload; the others are asked again when the container changes or
   after 5 s while it runs. Test.
5. A stream's problem stayed shown after it connected again without new lines. **Fixed**
   in devdeck-kube: cleared on connecting. Test.
6. A previous run could have live lines between its notes, lose its end note when hidden,
   or be read twice at once. **Fixed**: held and shown whole at its end; a second one is
   refused while one is read. Test.
7. Lines cleared on purpose counted as dropped; the container fallback note hid under a
   pod filter; "previous run" looked at the pod's restarts, not its shown container's.
   **Fixed**.
8. Finding the longest line counted every line's characters again. **Fixed**: counts are
   kept.
Tests added too: the pod watch coming back resumes streams; the pod row's "Logs" opens
on that pod; lines past the cap while paused are counted.

## Step 5 · Change the deployed branch (plan, 2026-11-10)

Requirements: R7.3, R7.4, R7.6, R7.8, R5.5, R5.6, R7.2 (deploy history).

What exists: the service view's tag editor (suggestions, unsaved marker, ctrl+s, a
confirmation with the exact command, a protected env's name typed, edit mode first); on
the kube driver the save is refused ("deploys come in a later step"); on the fake, a
rollout simulated on a timer, an undo, and a history kept in memory only.

### Decisions (user, 2026-11-10)

- No undo: a failed deploy is fixed by deploying another branch; failure detection stays,
  with its reason.
- Every deploy seen is recorded, whoever made it; the last 10 per service are kept (the
  immediate history is what matters).
- The user's account may patch deployments on dev35 and hades (`kubectl auth can-i patch
  deployments`, checked by the user). devdeck's own tests and checks never write to the
  user's cluster; the first real deploy is the user's own, by hand, on dev33.
- Before writing, devdeck asks the cluster whether it may (a `SelfSubjectAccessReview`,
  what `kubectl auth can-i` sends): Save is disabled up front with the reason when not.

### The write

- Only the main container's image changes: same image, the tag set (`registry/x:main` →
  `registry/x:feature-y`; a digest in the old image is dropped). A strategic merge patch
  of `spec.template.spec.containers[name=<main>].image` through kube-rs, sent on the
  driver's runtime and client. The confirmation keeps saying the equivalent
  `kubectl -n <ns> set image deployment/<name> <container>=<repo>:<tag>`.
- Guards, all below the UI and in this order: edit mode (R5.6), the driver may write,
  the account may (the access review's answer, when known), a protected env unlocked
  (R7.6), no rollout of the same service under way, the tag differs from the running one
  (the same tag is step 6, "redeploy").
- The patch's own refusal: 403 "your account can't change deployments in <env>"; 404 "the
  deployment <name> is gone from <env>"; 409/422 and others in the cluster's words. The
  confirmation stays shown until the patch answers, then closes or shows the refusal.

### May I? (access review)

- `devdeck_kube::can(target, verb, group, resource) -> Receiver<Result<bool, String>>`:
  one `SelfSubjectAccessReview` (`authorization.k8s.io/v1`) in the target's namespace.
- The cluster asks `patch deployments.apps` once per env when a service view of it opens,
  keeps the answer per env, and asks again when the env's log generation moves (target
  changed, driver changed, reload). Unknown or failed: Save stays possible, the patch
  decides. No: Save is disabled with "your account can't change deployments in <env>".

### Following a rollout

- A rollout is followed from what the watches already give: the deployment's `progress`
  (the cluster observed the new spec, every replica updated and available), its ready and
  updated counts, and the pods of its newest template (`Pod.template` of pods running the
  new tag). While a rollout is followed, the cluster holds that service's `PodsInterest`
  itself, so nothing needs to be open.
- States: rolling out `updated/replicas`; done (settled on the new tag, its pods ready);
  failed with the reason, from a new-template pod's container waiting on
  `ErrImagePull`/`ImagePullBackOff` ("image <repo>:<tag> not found" when the message says
  so, else the message), `CrashLoopBackOff` (its last termination: "Error (exit 2): panic:
  …"), `CreateContainerConfigError` (its message); stuck when the deployment's
  progress says `Stuck` (its deadline passed). A failed or stuck rollout stays shown until
  the next deploy of that service; devdeck does nothing by itself.
- Shown where rollouts show today: the service view's header and status, the Cluster
  list row ("rolling out 1/2", "failed: image not found"), the matrix cell.
- Deploys seen from elsewhere (the watch sees the tag change on a service devdeck didn't
  deploy) are followed the same way, marked "not from devdeck". Another tag seen while
  devdeck follows its own: "replaced by <tag>", and the newer one is followed.
- The fake keeps its simulation (`--demo`, tests, snapshots) through the same states.

### History

- Per workspace, in the state folder (`deploys.json`): for each (env, service) the last
  10 deploys: when, from, to, who (`devdeck` or `seen`), how it ended (done, failed: reason,
  stuck, replaced by <tag>, still rolling when devdeck quit). Read at start, written when
  a deploy starts and when it ends.
- Shown in the service view's "Deploy history" (newest first) and as "previous" tag
  suggestions, as today.
- A deploy seen while devdeck wasn't running isn't known: the history says what devdeck
  saw.

### Undo goes

The Undo button, the `u` key (`svc.undo`), `Cluster::undo`, `Rollout::undo_target`, the
edit mode modal's "undoing a rollout", and the tests that used them (rewritten to deploy
another tag instead).

### Files

- `devdeck-kube`: `deploy.rs` (new: the patch, the access review, their errors), tests on
  the mock API (the exact patch body and path, refusals).
- `devdeck-core`: `cluster.rs` (`Rollout` with who and its states, `DeployRecord` with
  who, history per service capped at 10, the reason from a pod's state), `state.rs`
  (`deploys.json`).
- `devdeck-app`: `cluster.rs` (`ClusterDriver::set_image`, `can_write`, the follow, the
  history and its saving), `kube_cluster.rs`, `fake_cluster.rs`, `cluster_pods.rs` (the
  cluster's own interest while following), `views/service_view.rs` (Save's reasons, the
  confirmation waiting for the answer, no undo), `sidebar.rs` and `views/matrix_view.rs`
  (labels), `edit_mode.rs`, `registry.rs`, `bin/snapshot.rs`.
- Docs: `DESIGN.md` (Cluster: writes), `REQUIREMENTS.md` (R7.3, R7.4 progress),
  `ROADMAP.md`.

### OS (R11.6)

None: network only, through kube-rs; the history file through the state store.

### States to check

Save disabled by edit mode off, by the access review's no, by a locked protected env, by
a rollout under way, by an unchanged tag; the confirmation waiting, then a refusal (403,
404) shown in it; rolling out 1/2 then done; failed on an image not found; failed on a
crash loop with its last error; stuck; a deploy seen from elsewhere; replaced by another
tag; the history with 10 entries, from devdeck and seen; the Cluster list row and matrix
cell for each rollout state; the demo unchanged.

### Tests

- devdeck-kube, on the mock API: the patch's path, method, content type and body (only
  the main container's image); 403, 404 and 422 in words; the access review's request
  and its yes, no and failure.
- Core: the reason from pod states (each waiting reason, a crash loop's last
  termination); history capped at 10 per service, newest kept; `deploys.json` round trip.
- App, on a scripted driver: guards in order (edit mode, access, protected, under way,
  same tag); a rollout followed to done from deployment and pod updates; failed on
  ImagePullBackOff and on CrashLoopBackOff with the reason; stuck; a deploy seen from
  elsewhere recorded as seen; replaced; the cluster's pods interest held while following
  and dropped after; the history saved and read back after a restart; the access review
  asked once per env and again after a reload; Undo gone (no action, no key).
- Snapshots: the confirmation waiting and with a refusal; the service view with a
  failed rollout (image not found) and with Save disabled by the access review; the
  history with seen and devdeck entries; the Cluster list with a rolling and a failed row.
- By hand: the user's first real deploy, on dev33, by the user.

### Out of scope

Redeploying the same branch (step 6); notifications (step 7); containers other than the
main one; checking the image registry before deploying; cluster writes through the
control API.

### Review of the step 5 plan (subagent, 2026-11-10) and assessment

Every point accepted; the plan above reads with these changes. Points 1 and 4 checked in
the code (`progress()` says `Stuck` before looking at `observedGeneration`; a container's
state keeps its waiting reason, not its message).
1. A deploy over a stuck deployment read as stuck at once, and "pods running the new tag"
   breaks for A → B → A. **Accepted**: the generation the patch answers with is kept, and
   status and conditions don't count until `observedGeneration` reaches it; `progress()`
   checks the generation first. The newest template comes from the `Progressing`
   condition's message (`ReplicaSet "x-<hash>"`), no ReplicaSet read (the account can't).
2. "A rollout under way" blocked the fix for a stuck deploy for up to 10 minutes.
   **Accepted**: only devdeck's own patch in flight blocks; a later deploy marks the
   earlier one "replaced by <tag>".
3. Failure declared final too early: the kubelet keeps pulling (a branch whose image CI
   hasn't pushed yet succeeds minutes later), and a crash loop shows after one crash.
   **Accepted**: "failing: <reason>" is a state that keeps following and ends as done,
   stuck or replaced (the history's outcome follows); a crash loop counts from 2 restarts
   of a newest-template pod.
4. The model lacks the reasons. **Accepted**: `convert.rs` (added to Files) keeps a
   waiting container's message, a pod's `PodScheduled=False` reason (unschedulable) and
   the deployment's `ReplicaFailure` condition (a quota: no pods at all); reasons use
   `Pod::trouble`'s list (`Init:`, `CreateContainerError`, `InvalidImageName` included). A
   paused deployment refuses the deploy ("rollouts of <name> are paused").
5. Races made devdeck's own deploy look like someone else's (the watch before the patch's
   answer; two workspaces on one env). **Accepted**: the intended deploy (env, service, from,
   to) is registered before the patch is sent; the patch carries `field_manager: devdeck`,
   and who made a deploy is read from the deployment's `managedFields` (the newest entry
   touching the spec: devdeck, kubectl, helm, argocd…; else "seen").
6. Spotting deploys by tag misses a redeploy of the same branch and digest-pinned images.
   **Accepted**: the deployment's `deployment.kubernetes.io/revision` is kept; a revision
   that grows is a deploy (another tag: a branch change; the same tag: "redeployed",
   recorded now though devdeck's own redeploy is step 6). The same-tag guard compares the
   whole image.
7. What runs wasn't pinned to what was confirmed. **Accepted**: `ConfirmDeploy` passes
   `from`, `to` and the container; the write is a JSON Patch that `test`s the main
   container's image (by index) is still the one shown, then `replace`s it; a 422 says
   "<name> changed since you confirmed". The `kubectl set image` line stays as the
   equivalent.
8. The wrong container could be patched (several containers, none named like the
   deployment: the first, maybe a sidecar). **Accepted**: such a deploy is refused with the
   reason ("<name> has several containers and none is named <name>").
9. `updatedReplicas` counts pods created, not ready; a pod watch per seen deploy could
   open dozens on a stack-wide deploy. **Accepted**: progress shows ready pods of the
   newest template over replicas; devdeck's own deploys hold the service's pods, deploys
   seen elsewhere are followed from the deployment's status only (pods when a view holds
   them).
10. A restart mid-rollout. **Accepted**: at start, a last record without an end resumes
    following if the deployment still runs that image, else it ends as "unknown: devdeck
    was closed"; `deploys.json` is `{ "version": 1, "envs": { env: { service: [records] } } }`.
11. Every 403 read as a permission problem (admission webhooks answer 403 too).
    **Accepted**: "your account can't …" only for an RBAC refusal ("… is forbidden: User
    …"), else the cluster's words; the access review's yes doesn't skip showing a refusal
    in the confirmation.
12. The fake's simulation bypassed the follow-up. **Accepted**: the fake's `set_image`
    sends deployment and pod updates through `apply` as a watch would; the simulation and
    its rollout timer leave `Cluster`; the `82-svc-failed-undo` snapshot becomes a
    failing rollout.
13. The decisions said notifications were planned in step 5. **Fixed**: step 7.

### Step 5 outcome (2026-11-10)

Built as planned with the review's changes; devdeck-kube by a builder (`deploy.rs`: the
JSON Patch and its errors, the access review; `convert.rs`: generation, revision,
newest template, `ReplicaFailure`, the pod template's last manager, waiting messages,
unschedulable pods), the core and the app by the lead (`cluster::assess`,
`cluster_deploys.rs`, the views, the fake).
- A failed json-patch `test` comes back as the API server's generic 422 ("the server
  rejected our request due to an error in our request"); that, 409, or a cause naming the
  test reads "<name> changed since you confirmed". A container index that no longer exists
  reads the same.
- "Who deployed" is the last manager of `f:spec.f:template` (a scale or an HPA isn't a
  deploy). `kube`'s `jsonpatch` feature is named in `Cargo.toml` (it came only through
  `runtime` before).
- The write's task is kept until the next write of the service (the fake goes on rolling
  after it answered); a write times out after 30 s with "it may have gone through, the
  rollout will say".
- The matrix shows tags only, as before: the rollout states show in the service view and
  the Cluster list.
- Snapshots: `82-svc-failing` (image not found, followed, in the history),
  `84-svc-confirm-refused` (someone deployed meanwhile).
- Not checked by hand: the first real deploy, on dev33, is the user's.

### Review of the step 5 diff (subagent, 2026-11-10) and assessment

1. On a real cluster the image changes before the controller moves the revision, so the
   history would read `feature → feature`. **Fixed**: the image kept is the one at the last
   revision. Test with the intermediate event.
2. Deploys whose pods nobody watches (seen from elsewhere, reopened, a write refused after
   the watch saw it) never ended. **Fixed**: without pods, the deployment's own counts
   decide, as `kubectl rollout status`. Test.
3. An untagged image could never be deployed (pinned to `repo:latest`, which the spec
   doesn't say). **Fixed**: `Image.raw` keeps the spec's text; deploys pin to it. Test.
4. A refusal was lost when the confirmation closed while sending. **Fixed**: a write no
   confirmation waits on says its refusal in a toast and leaves nothing behind. Test.
5. A deploy left failing when devdeck closed was never followed again. **Fixed**: reopened
   too. Test.
6. Baselines survived a change of target. **Fixed**: a restarted env or a new driver starts
   a new baseline. Test.
7. Save asked the protected env's name before other refusals; an access answer could be
   filed under a newer generation; an unasked review was cached as unknown. **Fixed**:
   every other guard first; the answer goes under the generation asked; not cached when
   not asked.
8. A timeout said nothing of a write that may have landed; saves could land out of order;
   a deleted deployment's rollout was kept forever. **Fixed**: the timeout says so; saves
   run one at a time; a deployment gone ends its rollout.

## Step 6 · Redeploy the same branch (plan, 2026-11-10)

Requirements: R7.3 (a deploy of the same branch), R7.5 (restart), R7.4, R7.6, R5.5, R5.6.

What exists after step 5: deploys pinned to the image confirmed, every deploy followed
(a growing revision with the same image already reads as "redeployed"), the history, the
access review; the service view's "restart" button says "comes in a later step of M3".

### Decisions (user, 2026-11-10)

- One action, **redeploy**: what `kubectl rollout restart` does. It is R7.5's restart too;
  the "restart" placeholder becomes "redeploy".
- The services that matter pull with `imagePullPolicy: Always` (checked by the user on
  dev33); some use `IfNotPresent`. When the main container's policy isn't `Always`, the
  confirmation warns that pods on nodes that already have the tag keep the build they
  have; the redeploy isn't refused.

### The write

- `devdeck_kube::restart(target, deployment, &Restart { index, container, image, at })`:
  a JSON Patch that tests the main container's name and image (still the one shown), then
  sets the pod template's `kubectl.kubernetes.io/restartedAt` annotation to `at` (RFC
  3339, now): `add` on `/spec/template/metadata/annotations/kubectl.kubernetes.io~1restartedAt`
  when the template has annotations, else `add` of the whole `annotations` map; field
  manager `devdeck`. Same errors and timeout as `set_image` (`DeployError`).
- `convert.rs` keeps, per container, `pull_policy: Option<String>`, and on the deployment
  whether its pod template has annotations (`template_annotated: bool`), so the patch is
  built right without reading the object again.
- `ClusterDriver::restart(env, name, change, cx)`; the answer goes to
  `Cluster::write_done` as a branch change's does. `Cluster::redeploy(env, name, image,
  unlocked, cx)` with the same guards as `deploy` (edit mode, the account, a protected env
  unlocked, no write of the service in flight, not paused, a main container clearly
  named), but the same image is what it asks for.

### Following and history

- The pending write is devdeck's with `to_image` = the image it runs; the watch sees the
  revision grow with that image: the existing detection records it, `by devdeck`, `main →
  main`, and follows it. The history says "redeployed" when from and to are the same
  (`outcome` unchanged: done, failing…); the service view's line says "redeploying main:
  1/2 pods ready".

### The view

- The service view's **redeploy** button and a registered action (`svc.redeploy`, key `r`
  in the service view, palette, cheat sheet); disabled with the guard's reason (R5.5).
- A confirmation "Redeploy graphservice on dev33?" with the branch, the equivalent
  `kubectl -n <ns> rollout restart deployment/<name>`, the pull policy warning when not
  `Always`, waiting for the answer and showing a refusal as the branch change's does (the
  same modal, its title and command chosen by the kind of write).
- A protected env asks its name first, as for a branch change.

### Fake

The demo's redeploy: a new template on the same tag, one replica per step, through the
same `apply` path as its branch changes.

### Files

- `devdeck-kube`: `deploy.rs` (`restart`, `Restart`), `convert.rs` (pull policy,
  template annotated), tests.
- `devdeck-core`: `cluster.rs` (`Container.pull_policy`, `Deployment.template_annotated`,
  `restart_command`).
- `devdeck-app`: `cluster.rs` (`ClusterDriver::restart`), `cluster_deploys.rs`
  (`redeploy`, its guards), `kube_cluster.rs`, `fake_cluster.rs`,
  `views/service_view.rs` (button, action, the modal's kinds), `registry.rs`,
  `bin/snapshot.rs`.
- Docs: `DESIGN.md` (Cluster), `REQUIREMENTS.md` (R7.5 progress), `ROADMAP.md`.

### OS (R11.6)

None.

### States to check

The redeploy button enabled and disabled (edit mode off, the account, locked protected,
in flight, paused); the confirmation with and without the pull policy warning; waiting;
refused (changed since confirmed); rolling "redeploying main 1/2"; done; failing; the
history's "redeployed" entry.

### Tests

- devdeck-kube, on the mock API: the patch body with and without existing template
  annotations (escaped `~1`), the test operations, field manager, the generation returned,
  the errors; conversion of the pull policy and of `template_annotated`.
- Core: `restart_command`.
- App, on a scripted driver: the guards; the write pinned to the image shown; the
  redeploy recorded as devdeck's with from = to and followed to done; a redeploy seen from
  elsewhere (already covered) unchanged; the button's reasons; the action registered.
- Snapshots: the confirmation with the pull policy warning; the service view rolling a
  redeploy.
- By hand: the user's, on dev33.

### Out of scope

Notifications (step 7); scaling replicas; restarting one pod (deleting it); checking the
registry for a newer build.

### Review of the step 6 plan (subagent, 2026-11-10) and assessment

Every point accepted; the plan above reads with these changes.
1. Adding the whole `annotations` map replaces it (RFC 6902): a stale "not annotated"
   would wipe annotations added meanwhile. **Accepted**: that patch also `test`s
   `/metadata/generation` equals the one seen (a spec change since refuses it, as
   "changed since you confirmed"). Tested on the mock API.
2. For a redeploy, "ours" (`pending.to_image == image`) matches any redeploy while the
   write is pending, and a patch that changes nothing (same `restartedAt` within a second)
   leaves a pending write for good. **Accepted**: `convert.rs` keeps the template's
   `kubectl.kubernetes.io/restartedAt`; a redeploy is devdeck's when it equals the
   pending `at`, which has fractional seconds; an answer whose generation isn't higher
   than the one seen drops the pending write and says "nothing changed". Tests: a foreign
   redeploy while sending, a patch that changes nothing.
3. The fake can't show a redeploy (`demo::pods` hashes the template from name and tag).
   **Accepted**: the template and pod names are seeded with the revision; `demo.rs` added
   to Files; a test that the fake's redeploy rolls then ends.
4. Without a template, `assess` counts old pods as new when from = to. **Accepted**: then
   the deployment's own counts decide, as without pods. Core test.
5. The pull-policy warning's cases. **Accepted**: an absent policy means the API server's
   default (`Always` for `latest` or no tag, else `IfNotPresent`); `Never` warns too; a
   digest-pinned image says "pinned to a digest: the same build restarts"; only the main
   container is checked (said in the warning's wording).
6. How the guards split. **Accepted**: `write_blocked(env, name, unlocked)` holds every
   guard but "already runs", shared by both; the protected env's name is still asked last;
   the modal carries its kind (a branch change with `from`/`to`, or a redeploy with its
   image and `at`) and sends the right write.
7. Smaller ones. **Accepted**: the edit mode modal says "redeploys"; `reopen` compares the
   revision (and `restartedAt`) for a redeploy, not the tag; a redeploy replaced by another
   reads "replaced by a redeploy".
8. kube's `Api::restart` sends a merge patch on `kube.kubernetes.io/restartedAt` (not
   kubectl's key), with no field manager and no test. **Accepted**: not used, said in the
   plan so a builder doesn't swap it in.

## Logs toolbar rework (user, 2026-11-10)

The user, on the logs view in use: too many things in the header, no way to wrap, find
barely fits a character. Built without a review, as asked:
- The toolbar keeps find (all the free width, at least a few words) with its match count,
  a warning icon only when something is wrong (pods waiting, streams' problems, in its
  hover text), the level as one icon button with a menu (errors, warn+, info+, all), and
  icon toggles for wrap, pause ("N new" while paused) and follow; a cog opens the rest:
  the pod and the container (submenus), a previous run, showing the time and pod
  columns, wrapping. "env › service" and the counts left the toolbar (the tab names it;
  the counts are the cog's hover text).
- Wrapping is on by default and remembered per tab with the columns: lines are cut by the
  monospace cell width into rows of the same list (still a uniform list), worked out
  again only when the width or what's shown changes; off, one row a line scrolls
  sideways. Keys: `w` wrap, `.` the cog's menu, the others as before.
- `devdeck-ui`: `icon_toggle` (with its test) and the icons funnel, text-wrap, pause and
  arrow-down-to-line (Lucide, from gpui-kit-assets 0.9.0).

## Logs: filter, and scrolling far back (plan, 2026-11-10)

The user: "I don't need find, I need filter"; and scrolling back far, as k9s does, not a
set number of lines.

### Filter

- The find field becomes **filter**: typing shows only the lines holding the text
  (ignoring case, per character), each match tinted behind its characters; the count says
  how many lines show ("12 lines"). No current match, no `n` / `shift+n` (gone from the
  registry); `/` focuses the filter; Esc clears it. Clearing shows every line again,
  scrolled to the bottom when following.
- In the buffer the filter is one more condition of what's shown (with the level and the
  pod), kept up to date line by line as today.

### Scrolling far back

- A container's logs reach back only as far as its node kept them (from its start unless
  the node rotated the files, by default about 10 MB a container), current run (and the
  previous one through "previous run"); the logs API has no paging: earlier lines are read
  by asking a bigger tail once and keeping what is older than what's shown.
- `devdeck_kube::read_logs(target, pod, container, tail: Option<u32>)`: one read, not
  followed, timestamps, the same line reading, batches, errors and `Ended` at its end
  (`None`: everything the node kept). `ClusterDriver::read_logs` (the fake makes up older
  lines before the first one it sent).
- Scrolling to the top (the list's first row in view, filter or not) reads, for every pod
  still there whose container ran, its last `held + 5 000` lines, keeps those older than
  the oldest it has (the same skip as a resume: older than its oldest line's time, and at
  exactly that time only the ones beyond those it has), merges them by time and puts them
  above; the view keeps the rows in sight where they were. A row at the top says
  "loading earlier lines…" while it reads, and "start of the logs the cluster kept" once
  a read brings nothing older for every pod (no more reads then, until the pods change).
- "From the start" in the `…` menu reads everything the node kept for each pod, the same
  way.
- The buffer grows to 100 000 lines and 64 MB. Reading back never pushes out what's
  shown: when it would pass the cap, the read keeps only what fits (said in the top row).
  While not following (scrolled back), new lines that would push out the oldest wait in
  the paused queue, counted ("N new"), until following again.
- Absolute line indices start high enough that lines can be added above (indices go
  down), so rows, matches and the longest line stay right.

### Tests

- devdeck-kube, on the mock API: `read_logs` asks `tailLines` (or none), not `follow`,
  ends with `Ended`.
- App: the filter shows only matching lines and their count, follows new lines, clears;
  `n` gone; scrolling to the top reads earlier lines with the bigger tail, keeps only the
  older ones (two identical lines at the oldest time), merges two pods by time, keeps the
  rows in sight, stops at the start; "from the start" reads without a tail; the cap holds
  while reading back and new lines wait while scrolled back; the buffer's indices going
  down keep rows and the longest line right.
- Snapshot: a filtered view; the top row loading and at the start.

### Out of scope

Saving logs to a file; reading logs of pods that are gone (their stream ended with them).

### Review of the filter and scroll-back plan (subagent, 2026-11-10) and assessment

The filter was built first (committed on its own); the scroll-back reads as revised here.
1. Scrolling up doesn't stop following, so "scrolled back" can't happen. **Accepted**:
   scrolling up turns follow off, reaching the bottom turns it on (as the run view); the
   feed knows whether the view follows.
2. "First row in view" fires on open and chains reads; nothing stops a second one.
   **Accepted**: a user gesture only (scrolling up at the top of a scrollable list), the
   top row clickable, and a registered action (`logs.earlier`, key `home`, also "from
   the start" in `…`); one read at a time, its result dropped if the container, the
   generation, the pod's restarts changed or the view was hidden.
3. Reading back keeps a prefix (the opposite of the resume skip); the oldest per pod
   isn't tracked. **Accepted**: at the read's start each pod's oldest line is taken from
   what's shown, merging and held (notes and previous runs left out); once the read
   ends, it's cut by position at that line's timestamp group (keep before the first line
   of the group plus the group's lines not shown yet); no boundary found keeps nothing and
   says so.
4. The start condition and growing tails. **Accepted, the simpler way**: one read of
   everything the cluster kept per pod (no tail), cut as in 3, inserted up to the cap; a
   pod read once is at its start until its restarts or the generation change.
5. The cap and the paused queue. **Accepted**: "N new" shows whenever lines wait; a
   read-back that passes the cap keeps its newest part and the top row says "full".
6. Wrapping rebuilt every frame. **Accepted**: wrapped pieces are kept by absolute line,
   appended as lines come, dropped from the front as they go, rebuilt only when the width,
   the level, the pod or the filter changes, or lines are put above.
7. The filter's cost. **Accepted**: an ASCII path without allocation; one match per line;
   the match bookkeeping of find (`hits`, counts) goes, every shown row being a match.
8. Keeping the rows in sight. **Accepted**: the scroll offset grows by the rows put above,
   set directly on the list's scroll handle (no notify loop).
9. `dropped()` once indices go down. **Accepted**: an explicit count of lines dropped for
   room.
10. `read_logs` on a transport failure. **Accepted**: the same stream code with no follow
    and no tail; a failure ends the read with its reason (no resume mid-read). Whether the
    kubelet serves rotated files is unverified: the top row says "start of what the
    cluster serves".
11. Leftovers of find. **Accepted**: tests updated, the figures count every line, Esc
    scrolls to the bottom when following, R7.5 says filter, a filter set never folds.
12. Tests. **Accepted** as listed, but the 100k timing check (a test of time is flaky):
    measured by hand instead.

### Filter and scroll-back outcome (2026-11-10)

The filter committed first; the scroll-back by a builder, checked by the lead (whole suite
and clippy, snapshots `90-logs-earlier-loading`, `90-logs-earlier-start`).
- `devdeck_kube::read_logs` goes through the same stream code (no follow, no tail); a
  failure ends it with its reason. The fake makes up older lines.
- Scrolling up stops following, reaching the bottom follows again; `home` and "from the
  start" also scroll to the top; a wheel up at the top or a click on the top row reads.
  One read at a time, dropped when the container, the generation, a pod's restarts
  changed or the view was hidden.
- Lines read back are cut at each pod's oldest shown line (notes and previous runs left
  out), merged by time, put above (indices going down from 1<<40), the rows in sight kept
  where they were; caps 100 000 lines / 64 MB, the newest part of a read kept when full.
- Wrapped pieces are kept by line (appended, dropped, rebuilt only on width, level, pod,
  filter or lines put above); the filter matches without allocating on ASCII.
- Limits: pods with no line shown aren't read; whether the kubelet serves rotated files is
  unverified ("the logs the cluster serves"); the click on the top row shares the wheel's
  code but has no test of its own.
- Security review after the commit (automated): a read-back held a pod's whole log in
  memory before the cap applied. **Fixed**: the read asks a bounded tail (the pod's lines
  shown, the buffer's room and a margin of 1 000), and the lines are collected within the
  room (the oldest dropped first), the read stopping at the first line past the oldest
  line shown; a read cut short for room says "full". Tests: the collector never holds more
  than the room (lines and bytes), every read asks a bounded tail, the mock API's
  `tailLines`.

### Step 6 outcome (2026-11-10)

Built by a builder from the plan as assessed, checked by the lead (suite, clippy, the
confirmation's snapshot `84-svc-redeploy-confirm`, the rolling `81-svc-redeploying`).
- The stamp is decided when the write is sent (a retry never reuses one); on a fixed
  clock, the second plus a counter. "Nothing changed" applies to redeploys only. The
  warning names the container ("<container>'s imagePullPolicy is IfNotPresent: …").
- `DeployRecord` keeps the revision and the stamp (old files read: they default).
- The demo gives about one service in six `IfNotPresent`; its pods are seeded with the
  revision, so pod names after a demo deploy change in snapshots.
- Found outside the step: the snapshot tool didn't build after `e62ac6d` (its fake
  `read_logs` lacked the tail); fixed.

### Review of the step 6 diff (subagent, 2026-11-10) and assessment

No path writes without edit mode, the protected env's name or the image confirmed.
1. The pin from confirmation to the cluster had no end-to-end test. **Fixed**: a test
   where someone deploys between `r` and Enter; the confirmation stays with the refusal.
2. A second write could race the first once answered but not yet seen. **Fixed**: one
   write per service until the watch has seen it ("still being picked up"). Test.
3. Without a revision annotation a redeploy was never seen and stayed pending. **Fixed**:
   devdeck's own is known by its stamp then. Test.
4. Wording. **Fixed**: an open redeploy reads "redeploying main" in the history.
5. Tests. **Fixed**: two redeploys get two stamps; an old `deploys.json` reads.
6. Docs. **Fixed**: DESIGN, ROADMAP, this outcome.
7. Demo pod names after a deploy change in snapshots: **accepted** (cosmetic).

## Step 7 · Notifications (plan, 2026-11-10) — OS notifications postponed

Postponed by the user on 2026-11-10: notifications are toasts and a notification center
first (`docs/plans/notifications.md`); desktop notifications come back later. Kept as a
record.

Requirements: the 2026-11-10 decisions (per service and env, events picked on the service
view, the desktop's own notification always, off at every start), R11.6 (OS-specific code
in `devdeck-sys`), R11.3 (a pinned dependency), R5.1, R5.4, R5.5.

### Decisions (user, 2026-11-10; the three open points take the defaults proposed)

- A **bell** on the service view opens a dropdown with a checkbox per event, for that
  service on the view's env; also a registered action (`svc.notify`, a key free in
  `ServiceView`). The bell shows in the accent while any event is ticked.
- Events: a new branch deployed (by anyone, with who); the same branch redeployed; the
  rollout finished; the rollout failed or got stuck (with its reason); pods crashing
  outside a deploy (a pod entering a crash loop or failing to pull while no rollout of the
  service is under way).
- Default (1): subscriptions outlive the service view until devdeck quits; the Cluster
  list row of a service with notifications on shows a small bell (its hover lists the
  events; its row menu turns them off).
- Default (2): `notify-rust` 4.18.2 (pinned; 4.19.0 is two days old), D-Bus through `zbus`
  on Linux; on macOS its default backend works from a plain binary without a bundle
  (checked in its README, 2026-11-10).
- Default (3): clicking a notification brings devdeck forward with that service's view
  open, where the desktop supports actions; nothing where it doesn't.

### The OS layer (`devdeck-sys`, `notify` module)

- `notify(&Note { title, body, action: Option<label> }) -> Result<NoteHandle, SysError>`
  and a way to hear the action's click (a channel); Linux backend through `notify-rust`
  (XDG, zbus), macOS backend through `notify-rust`'s default macOS backend; other targets
  `SysError::Unsupported` with its reason. A conformance test per backend that can run
  headless (Linux: skipped when no session bus, said so).
- `docs/PLATFORMS.md` lists the capability.

### The app

- `Notifications` (one per workspace, in its services): the subscriptions by (env,
  service) with their events; empty at start, never saved.
- It listens to the cluster's deploy follow-up (`cluster_deploys.rs`): a deploy spotted
  (new branch, redeploy, by whom), a rollout's end (done, failing, stuck) — each state
  change once per rollout; and to the service's pods for crashes outside a rollout (a pod
  whose container enters CrashLoopBackOff, or ErrImagePull/ImagePullBackOff, with no
  unfinished rollout of the service), once per pod and container run.
- While a subscription lives it holds the service's pods (a `PodsInterest`) and wants its
  env.
- Grouping: events within 3 s for one service are one notification ("graphservice on
  dev33: feature-oidc deployed by devdeck, rollout finished").
- Text: title "<service> on <env>", body the event in words; a protected env's name
  marked in the body ("hades (protected)").
- A click: the main window to the front (GPUI's activation), the service view opened in
  the focused pane.
- When notifications can't be sent (no session bus, unsupported), the bell is disabled
  with the reason (R5.5).

### Files

- `devdeck-sys`: `notify/{mod.rs, linux.rs, macos.rs, unsupported.rs}`, `Cargo.toml`
  (`notify-rust`), tests.
- `devdeck-app`: `notifications.rs` (new), `services.rs`, `cluster_deploys.rs` (events
  out), `views/service_view.rs` (bell, dropdown, action), `sidebar.rs` (the row's bell),
  `registry.rs`, `bin/snapshot.rs`.
- Root `Cargo.toml` (the pinned dependency).
- Docs: `DESIGN.md`, `PLATFORMS.md`, `REQUIREMENTS.md`, `ROADMAP.md`.

### States to check

The bell off, on (some events), disabled (no notifications on this system); the dropdown;
the Cluster row's bell and its hover; a notification for each event (a fake sender
records them in tests); grouping; a click opening the service.

### Tests

- devdeck-sys: the module's API on a fake (the app uses a sender trait so tests record
  notes); the Linux backend's conformance test skipped without a session bus.
- App, on the scripted deploy driver: each event notifies once when ticked and not when
  not; nothing at start; a deploy by Slack says who; a failing rollout says why; a crash
  outside a rollout notifies once per pod run, not during a rollout; grouping within 3 s;
  the subscription outlives the view and is turned off from the row's menu; the bell's
  reason when sending is unsupported; a click opens the service view.
- Snapshots: the bell's dropdown; the Cluster row with a bell.

### Out of scope

Saving subscriptions across restarts; notifications for other things than the cluster;
sounds; Windows.

### Review of the step 7 plan (subagent, 2026-11-10) and assessment

Every point accepted; defaults (2) and (3) change with points 1–3, for the user to see.
1. `notify-rust` on macOS builds Objective-C (`mac-notification-sys`'s `build.rs`), which
   breaks the Mac check from Linux (PLATFORMS §4); without a bundle it notifies as
   Finder; a click needs the main run loop. **Accepted**: no `notify-rust`; macOS is
   `unsupported` ("needs an app bundle: comes with packaging"), the bell disabled with
   that reason there.
2. On Linux, plain `zbus` fits better (`notify-rust` opens a connection per send, a
   blocked thread per click waiter, and unwraps D-Bus calls). **Accepted**: `zbus` 5.19.0
   is already in `Cargo.lock` through GPUI; pinned (`default-features = false, features =
   ["async-io"]`) in `devdeck-sys`'s Linux backend, a row in PLATFORMS' "Build or buy":
   one connection, `Notify` with `replaces_id` per (env, service), one stream of
   `ActionInvoked` / `NotificationClosed` read on the background executor into a channel.
   No new crate in the build.
3. A click won't raise the window on Wayland (GPUI's `activate()` needs a recent press;
   the server's `ActivationToken` can't be handed to GPUI). **Accepted**: a click opens
   the service view; on Wayland the window may only ask for attention. A manual check,
   not a promise.
4. Deduping on "each state change" floods (ready counts and failing reasons change on
   every pod update; a crash loop swings failing ↔ rolling). **Accepted**: one
   notification per (env, service, rollout, kind); the rollout is its record's start and
   revision or stamp; failing and stuck are one kind; replaced isn't an event.
5. "Outside a rollout" notified a failure twice (a failing rollout turning stuck, then its
   pods). **Accepted**: pods of a template whose rollout notified failing are skipped; at
   most one crash notification per (service, template) until a pod of it is ready again;
   `pod_failure`'s threshold (2 restarts) decides.
6. Never block the UI thread. **Accepted**: probing (`GetServerInformation`) and sending
   run on the background executor; the bell's reason comes from the capability's runtime
   answer; a send failing later gives one toast and disables the bell until restart.
7. The conformance test would pop real notifications. **Accepted**: tested against an
   in-process fake `org.freedesktop.Notifications` on a peer-to-peer connection (Notify,
   `replaces_id`, ActionInvoked), headless, no skips.
8. Subscriptions on another env than the list's can't be seen. **Accepted**: an action
   "notifications" listing every subscription (palette), each turned off from it, and
   "turn all off".
9. A click across workspaces. **Accepted**: it switches to the workspace first; a gone
   workspace or cluster does nothing (logged); a minimized window is restored. Tested for
   a background workspace.
10. Keys and the toolbar. **Accepted**: `svc.notify` on `b` (free in `ServiceView`); the
    row menu's "turn off" an action too; `Icon::Bell` added; the bell built as a
    `ToolAction` so the service view's later move onto the toolbar costs nothing; a
    disabled reason in its tooltip.
11. Holding the pods. **Accepted**, with the reason written: deploys made elsewhere are
    followed from deployment counts only, so without the pods a crash-looping one wouldn't
    read as failing before stuck.
12. Smaller ones. **Accepted**: deploys during a reconnect's new baseline are missed (said
    in DESIGN); two workspaces on one env notify twice (accepted, said); the module is
    `desktop_note`; the 3 s grouping on a timer the tests advance.

## Step 8 · Close (2026-11-11)

- Suite 1057/1057, clippy clean. The leak cycle (`stress.rs`) now shows the Cluster list
  with its pod watch on and off, and a notification's center and Open each cycle; the
  stats line counts the envs watched; `scripts/measure.py cluster` is the idle scenario
  with every demo env watched (the matrix open).
- Not run yet: the window measurements (`measure.py idle`, `cluster`, `leak`) open a window
  on the user's screen, and no headless display is installed: run when the user says,
  then `docs/perf/M3.md`.

### Manual test (M3 close), on dev33

1. Settings → Deploys: set your short name (`i.alglave`). Without it the rocket is dim
   with "set your short name in Settings first", and ctrl+s says so.
2. Open a service on dev33: header, pods, metrics, container, history; no k9s button.
3. `l` logs: several pods, level filter, text filter (only matching lines), wrap, pause,
   follow; scroll up: older lines load as far as the cluster kept.
4. Type another tag, click the rocket (or ctrl+s): the confirmation shows the exact
   command; Deploy; the rollout is followed to "now running", a toast says it with Open.
   The history reads "by i.alglave via Kugelblitz".
5. `r` redeploy: confirmation, rollout restart followed, toast "redeployed …".
6. `b` notify dropdown: five options colored by severity, ticking keeps it open. Tick
   "branch deployed" and have someone (or kubectl) deploy: a toast "… deployed by …".
   Your own deploys never say "deployed", only how they ended.
7. Bell next to settings: a blue dot when unread, a red count for errors; the center
   filters (unread/all, workspace), mark read, clear; Open switches workspace and layout
   to the view and flashes it; "Open here" opens it in the focused pane.
8. A protected env asks for its name before a deploy; edit mode off refuses deploys.
9. Palette → "service notifications": lists what's ticked; "turn all off".
