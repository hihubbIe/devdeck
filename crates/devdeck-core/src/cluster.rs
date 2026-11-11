//! Cluster domain (R7): envs, deployments, pods, metrics, rollouts and devdeck's own
//! deploy history. M1 drives it from a fake cluster; M3 from kube-rs.

pub use crate::catalog::Env;

/// Something fetched from elsewhere.
#[derive(Clone, Debug, PartialEq)]
pub enum Remote<T> {
    Loading,
    Ready(T),
    Error(String),
}

impl<T> Remote<T> {
    pub fn ready(&self) -> Option<&T> {
        match self {
            Remote::Ready(t) => Some(t),
            _ => None,
        }
    }
}

/// A container's requests and limits; each may be unset (millicores, MiB).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Resources {
    pub cpu_request_m: Option<u32>,
    pub cpu_limit_m: Option<u32>,
    pub mem_request_mi: Option<u32>,
    pub mem_limit_mi: Option<u32>,
}

/// A Kubernetes CPU quantity in millicores: `500m`, `1`, `1.5`, `2k`.
pub fn cpu_millis(q: &str) -> Option<u32> {
    let q = q.trim();
    if let Some(m) = q.strip_suffix('m') {
        return m.parse::<f64>().ok().map(|v| v.round() as u32);
    }
    if let Some(u) = q.strip_suffix('u') {
        return u.parse::<f64>().ok().map(|v| (v / 1000.).ceil() as u32);
    }
    if let Some(n) = q.strip_suffix('n') {
        return n.parse::<f64>().ok().map(|v| (v / 1_000_000.).ceil() as u32);
    }
    let (num, mult) = match q.strip_suffix('k') {
        Some(k) => (k, 1000.),
        None => (q, 1.),
    };
    num.parse::<f64>().ok().map(|v| (v * mult * 1000.).round() as u32)
}

/// A Kubernetes memory quantity in MiB: `512Mi`, `1Gi`, `500M`, `1e9`, `134217728`.
pub fn mem_mib(q: &str) -> Option<u32> {
    let q = q.trim();
    const UNITS: [(&str, f64); 12] = [
        ("Ki", 1024.),
        ("Mi", 1024. * 1024.),
        ("Gi", 1024. * 1024. * 1024.),
        ("Ti", 1024. * 1024. * 1024. * 1024.),
        ("Pi", 1024. * 1024. * 1024. * 1024. * 1024.),
        ("Ei", 1024. * 1024. * 1024. * 1024. * 1024. * 1024.),
        ("k", 1e3),
        ("M", 1e6),
        ("G", 1e9),
        ("T", 1e12),
        ("P", 1e15),
        ("E", 1e18),
    ];
    let (num, mult) = UNITS.iter().find_map(|(u, m)| q.strip_suffix(u).map(|n| (n, *m))).unwrap_or((q, 1.));
    num.parse::<f64>().ok().map(|v| (v * mult / (1024. * 1024.)).round() as u32)
}

/// A container image: `repo[:tag][@digest]`.
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    /// Registry and path: `europe-west1-docker.pkg.dev/project/registry06/graphservice`.
    pub repo: String,
    /// `None` only for an image pinned by digest without a tag; no tag otherwise means
    /// `latest`.
    pub tag: Option<String>,
    pub digest: Option<String>,
    /// As the spec writes it (`repo` without a tag stays so): what a deploy pins to.
    pub raw: String,
}

impl Image {
    pub fn parse(image: &str) -> Image {
        let (rest, digest) = match image.split_once('@') {
            Some((r, d)) => (r, Some(d.to_string())),
            None => (image, None),
        };
        // A `:` is a tag's only when no `/` follows it (`host:5000/img` has a port).
        let (repo, tag) = match rest.rsplit_once(':') {
            Some((r, t)) if !t.contains('/') => (r, Some(t.to_string())),
            _ => (rest, None),
        };
        let tag = if tag.is_none() && digest.is_none() { Some("latest".to_string()) } else { tag };
        Image { repo: repo.to_string(), tag, digest, raw: image.to_string() }
    }

    /// The last path segment of the repo (`graphservice`).
    pub fn short_name(&self) -> &str {
        self.repo.rsplit('/').next().unwrap_or(&self.repo)
    }

    /// What lists show: the tag, else the start of the digest.
    pub fn label(&self) -> String {
        match (&self.tag, &self.digest) {
            (Some(t), _) => t.clone(),
            // By characters: the digest comes from the cluster, a byte cut could split one.
            (None, Some(d)) => format!("@{}", d.chars().take(19).collect::<String>()),
            (None, None) => "latest".into(),
        }
    }
}

impl std::fmt::Display for Image {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.repo)?;
        if let Some(t) = &self.tag {
            write!(f, ":{t}")?;
        }
        if let Some(d) = &self.digest {
            write!(f, "@{d}")?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Container {
    pub name: String,
    pub image: Image,
    pub ports: Vec<u16>,
    pub resources: Resources,
    /// Config maps it reads its environment from.
    pub config_maps: Vec<String>,
    /// `imagePullPolicy` as the spec says it; `None` when unset (see
    /// [`Container::pull_policy`]).
    pub pull_policy: Option<String>,
}

impl Container {
    /// The pull policy that applies: the spec's, else the API server's default (`Always`
    /// for a `latest` tag or none, else `IfNotPresent`).
    pub fn pull_policy(&self) -> &str {
        match self.pull_policy.as_deref() {
            Some(p) if !p.is_empty() => p,
            _ if self.image.tag.as_deref() == Some("latest") => "Always",
            _ => "IfNotPresent",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Usage {
    pub cpu_m: u32,
    pub mem_mi: u32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum PodPhase {
    #[default]
    Running,
    Pending,
    /// A container waiting with a reason (`ImagePullBackOff`, `CrashLoopBackOff`,
    /// `Init:CrashLoopBackOff` for an init container).
    Waiting(String),
    Terminating,
    /// Ended badly, with a reason (`Evicted`).
    Failed(String),
    Succeeded,
}

impl PodPhase {
    pub fn text(&self) -> String {
        match self {
            PodPhase::Running => "Running".into(),
            PodPhase::Pending => "Pending".into(),
            PodPhase::Waiting(r) | PodPhase::Failed(r) => r.clone(),
            PodPhase::Terminating => "Terminating".into(),
            PodPhase::Succeeded => "Succeeded".into(),
        }
    }

    pub fn ok(&self) -> bool {
        matches!(self, PodPhase::Running | PodPhase::Succeeded)
    }
}

/// How a container last ended.
#[derive(Clone, Debug, PartialEq)]
pub struct Termination {
    /// `Error`, `OOMKilled`, `Completed`.
    pub reason: String,
    pub exit_code: Option<i32>,
    /// What it said, when anything (first line).
    pub message: Option<String>,
    /// When (epoch seconds).
    pub at: Option<u64>,
}

impl Termination {
    /// `Error (exit 2): panic: missing TADA_DB_URL`.
    pub fn text(&self) -> String {
        let code = self.exit_code.map(|c| format!(" (exit {c})")).unwrap_or_default();
        let message = self.message.as_deref().map(|m| format!(": {m}")).unwrap_or_default();
        format!("{}{code}{message}", self.reason)
    }
}

/// One container of a pod, as the cluster reports it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ContainerStatus {
    pub name: String,
    /// An init container (runs before the others).
    pub init: bool,
    pub ready: bool,
    pub restarts: u32,
    /// `running`, `waiting: CrashLoopBackOff`, `terminated: Error (exit 1)`.
    pub state: String,
    /// When it ended, for one that did (an init container done: epoch seconds).
    pub ended: Option<u64>,
    /// What a waiting container's reason says (`… manifest unknown`, a missing secret).
    pub message: Option<String>,
    pub last_termination: Option<Termination>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pod {
    pub name: String,
    /// The deployment it belongs to (its ReplicaSet's owner), when it has one.
    pub owner: Option<String>,
    /// Its `pod-template-hash`: pods of one version of a deployment share it.
    pub template: String,
    pub phase: PodPhase,
    pub ready: bool,
    /// Every container's restarts.
    pub restarts: u32,
    /// The latest termination of any of its containers.
    pub last_termination: Option<Termination>,
    /// When it started (epoch seconds); `None` while pending.
    pub started: Option<u64>,
    pub node: String,
    /// The main container's tag, from its spec.
    pub tag: String,
    pub containers: Vec<ContainerStatus>,
    /// Why it can't be placed on a node (`PodScheduled` false: "0/5 nodes are available…").
    pub unschedulable: Option<String>,
}

/// Waiting reasons that mean a pod is in trouble, not on its way (`ContainerCreating`).
const TROUBLE: [&str; 6] = ["CrashLoopBackOff", "ImagePullBackOff", "ErrImagePull", "CreateContainerConfigError", "CreateContainerError", "InvalidImageName"];

impl Pod {
    /// What's wrong with it, when something is: a waiting reason that won't fix itself.
    /// Ending pods, ended pods and pods merely not ready yet aren't trouble.
    pub fn trouble(&self) -> Option<&str> {
        match &self.phase {
            PodPhase::Waiting(r) if TROUBLE.iter().any(|t| r.trim_start_matches("Init:") == *t) => Some(r),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Deployment {
    pub name: String,
    /// At least one.
    pub containers: Vec<Container>,
    /// Run before the others, each to its end. Some carry an app of their own (front-end
    /// apps copied into the volume `frontendapp`'s nginx serves): a part, deployed on its
    /// own ([`Deployment::part`]).
    pub init_containers: Vec<Container>,
    pub replicas: u32,
    pub ready: u32,
    /// Replicas on the current version.
    pub updated: u32,
    pub progress: Progress,
    /// Which pods are its, as a label selector (`app=api,tier in (web)`); empty: every pod.
    pub selector: String,
    /// When the current version finished rolling out (epoch seconds), else when the
    /// deployment was created; `None` unless [`Progress::Settled`].
    pub since: Option<u64>,
    /// Its spec's generation, and the one the controller has seen: until they meet, its
    /// status says nothing of the newest spec.
    pub generation: u64,
    pub observed: u64,
    /// `deployment.kubernetes.io/revision`: grows with each new pod template (a deploy),
    /// not with a scale.
    pub revision: u64,
    /// The newest pod template's hash, from the `Progressing` condition (`ReplicaSet
    /// "<name>-<hash>"`), when it says.
    pub template: Option<String>,
    /// Why pods can't be created (the `ReplicaFailure` condition: a quota).
    pub failure: Option<String>,
    /// Who changed its spec last (`managedFields`: `devdeck`, `kubectl-set`, `helm`…).
    pub manager: Option<String>,
    /// Whether its pod template has annotations (a redeploy adds one to them, or adds
    /// them whole).
    pub template_annotated: bool,
    /// The pod template's `kubectl.kubernetes.io/restartedAt`: when it was last
    /// redeployed (`kubectl rollout restart`, devdeck's redeploy).
    pub restarted_at: Option<String>,
}

/// Where a deployment's rollout stands, as the cluster says.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Progress {
    /// The current version runs.
    #[default]
    Settled,
    Rolling,
    /// Rollouts are paused (`kubectl rollout pause`).
    Paused,
    /// The rollout went past its deadline (`ProgressDeadlineExceeded`).
    Stuck,
}

impl Progress {
    /// What lists say next to the tag, when it isn't settled.
    pub fn label(self) -> Option<&'static str> {
        match self {
            Progress::Settled => None,
            Progress::Rolling => Some("rolling out"),
            Progress::Paused => Some("paused"),
            Progress::Stuck => Some("rollout stuck"),
        }
    }
}

impl Deployment {
    /// The container a deploy changes: the main one, refused when several containers run
    /// and none is named like the deployment (a sidecar could be first).
    pub fn deploy_target(&self) -> Result<(usize, &Container), String> {
        match self.containers.iter().position(|c| c.name == self.name) {
            Some(i) => Ok((i, &self.containers[i])),
            None if self.containers.len() == 1 => Ok((0, &self.containers[0])),
            None => Err(format!("{} has several containers and none is named {}", self.name, self.name)),
        }
    }

    /// The container that names the deployment's version: the one named like it, else
    /// the first.
    pub fn main(&self) -> &Container {
        self.containers.iter().find(|c| c.name == self.name).unwrap_or(&self.containers[0])
    }

    pub fn main_mut(&mut self) -> &mut Container {
        let i = self.containers.iter().position(|c| c.name == self.name).unwrap_or(0);
        &mut self.containers[i]
    }

    /// The main container's tag as lists show it.
    pub fn tag(&self) -> String {
        self.main().image.label()
    }

    /// The init container running `image` (its short name, `monitorapp`), with its index:
    /// that part of the deployment, deployed on its own.
    pub fn part(&self, image: &str) -> Option<(usize, &Container)> {
        self.init_containers.iter().enumerate().find(|(_, c)| c.image.short_name() == image)
    }

    /// Every image it runs, the main one first: a change of any one is a deploy.
    pub fn version(&self) -> String {
        std::iter::once(self.main()).chain(&self.init_containers).map(|c| c.image.raw.as_str()).collect::<Vec<_>>().join(" ")
    }

    /// Whether it runs `label`: its tag, or a part's ([`part_label`]).
    pub fn runs(&self, label: &str) -> bool {
        self.tag() == label || self.init_containers.iter().any(|c| part_label(c) == label)
    }

    /// The other containers, for a "+n" next to the tag.
    pub fn others(&self) -> usize {
        self.containers.len().saturating_sub(1)
    }
}

/// One deploy in devdeck's history (R7.2): what it saw, whoever made it. `from` equal to
/// `to` is a redeploy.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DeployRecord {
    /// When it started (epoch seconds).
    pub at_s: u64,
    pub from: String,
    pub to: String,
    /// `devdeck`, the tool that changed the spec (`kubectl-set`, `helm`…), or `seen`.
    #[serde(default)]
    pub by: String,
    /// How it ended (`done`, `failed: …`, `stuck: …`, `replaced by main`); empty while it
    /// runs.
    pub outcome: String,
    /// The deployment's revision and pod template `restartedAt` it came with: a redeploy
    /// left open when devdeck closed is followed again only if they still match.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restarted_at: Option<String>,
}

impl DeployRecord {
    /// The same branch again (`kubectl rollout restart`, devdeck's redeploy).
    pub fn redeploy(&self) -> bool {
        self.from == self.to
    }
}

/// Deploys kept per service (the immediate history is what matters; user, 2026-11-10).
pub const HISTORY_KEPT: usize = 10;

/// Adds `r` to a service's history, newest last, keeping [`HISTORY_KEPT`].
pub fn remember(history: &mut Vec<DeployRecord>, r: DeployRecord) {
    history.push(r);
    let over = history.len().saturating_sub(HISTORY_KEPT);
    history.drain(..over);
}

/// What a field manager of devdeck's ends with: "<short name> via Kugelblitz".
pub const VIA: &str = " via Kugelblitz";

/// The field manager devdeck's deploys and redeploys carry for `short_name`: who made them,
/// as the deployment's `managedFields` (and so its history) will say.
pub fn field_manager(short_name: &str) -> String {
    format!("{short_name}{VIA}")
}

/// Whether a deploy made by `by` is the user's own: their field manager (from any machine
/// with their short name), or devdeck's old one (`devdeck`, in history kept from before).
pub fn by_me(by: &str, short_name: Option<&str>) -> bool {
    by == "devdeck" || short_name.is_some_and(|n| by == field_manager(n))
}

/// The exact command a tag change runs (R7.3 states it before confirming).
pub fn set_image_command(namespace: &str, d: &Deployment, tag: &str) -> String {
    let c = d.main();
    format!("kubectl -n {namespace} set image deployment/{} {}={}:{tag}", d.name, c.name, c.image.repo)
}

/// A part's tag change: the same command on its init container (`kubectl set image` sets
/// init containers by name too).
pub fn set_part_command(namespace: &str, d: &Deployment, part: &Container, tag: &str) -> String {
    format!("kubectl -n {namespace} set image deployment/{} {}={}:{tag}", d.name, part.name, part.image.repo)
}

/// A part's version as history and rollouts say it: `monitorapp:v1.49.0`.
pub fn part_label(c: &Container) -> String {
    format!("{}:{}", c.image.short_name(), c.image.label())
}

/// The command a redeploy is the equivalent of (R7.3 states it before confirming).
pub fn restart_command(namespace: &str, d: &Deployment) -> String {
    format!("kubectl -n {namespace} rollout restart deployment/{}", d.name)
}

/// A redeploy's `restartedAt` at `nanos` since the epoch: RFC 3339 in UTC with nanoseconds
/// (`2026-11-10T09:12:44.512345678Z`), finer than kubectl's whole seconds so devdeck's own
/// redeploy is told from another one made the same second.
pub fn restart_stamp(nanos: u128) -> String {
    let secs = (nanos / 1_000_000_000) as i64;
    let frac = (nanos % 1_000_000_000) as u32;
    let (days, rest) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Days to a civil date (Howard Hinnant's `civil_from_days`).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{frac:09}Z", rest / 3600, rest % 3600 / 60, rest % 60)
}

/// What a redeploy of `c` (the main container) may not do, said before confirming: an
/// image pinned to a digest restarts the same build; a pull policy other than `Always`
/// keeps the build nodes already have.
pub fn redeploy_warning(c: &Container) -> Option<String> {
    if c.image.digest.is_some() {
        return Some("pinned to a digest: the same build restarts".into());
    }
    match c.pull_policy() {
        "Always" => None,
        p => Some(format!("{}'s imagePullPolicy is {p}: pods on nodes that already have this tag keep the build they have", c.name)),
    }
}

/// Whether a write may go ahead on `env`.
pub fn check_write(env: &Env, unlocked: bool) -> Result<(), String> {
    if env.protected && !unlocked {
        Err(format!("{} is protected: unlock it first", env.name))
    } else {
        Ok(())
    }
}

/// Unlocking a protected env takes typing its exact name.
pub fn unlock_matches(env: &Env, typed: &str) -> bool {
    typed.trim() == env.name
}

/// Where a followed deploy stands (R7.4).
#[derive(Clone, Debug, PartialEq)]
pub enum RolloutState {
    /// `ready` of `of` pods of the new version ready.
    Rolling { ready: u32, of: u32 },
    /// Still rolling, but something's wrong that may fix itself (the kubelet keeps
    /// pulling an image CI hasn't pushed yet; a crash loop).
    Failing { ready: u32, of: u32, reason: String },
    Done,
    /// Past its progress deadline: the cluster gave up.
    Stuck { reason: String },
    /// Another deploy came after it.
    Replaced { by: String },
}

/// A deploy followed: devdeck's own, or one seen on the watch.
#[derive(Clone, Debug, PartialEq)]
pub struct Rollout {
    /// The tags as lists show them.
    pub from: String,
    pub to: String,
    /// The new image in full (`repo:tag`, a digest kept): "still runs it" compares this.
    pub image: String,
    /// `devdeck`, the tool that changed the spec, or `seen`.
    pub by: String,
    /// The generation that carries it: the deployment's status counts once the controller
    /// has seen it.
    pub generation: u64,
    pub at_s: u64,
    pub state: RolloutState,
}

impl Rollout {
    /// The same branch again (`kubectl rollout restart`, devdeck's redeploy).
    pub fn redeploy(&self) -> bool {
        self.from == self.to
    }

    pub fn finished(&self) -> bool {
        matches!(self.state, RolloutState::Done | RolloutState::Stuck { .. } | RolloutState::Replaced { .. })
    }

    /// How it ended, for the history; empty while it runs.
    pub fn outcome(&self) -> String {
        match &self.state {
            RolloutState::Rolling { .. } => String::new(),
            RolloutState::Failing { reason, .. } => format!("failing: {reason}"),
            RolloutState::Done => "done".into(),
            RolloutState::Stuck { reason } => format!("stuck: {reason}"),
            RolloutState::Replaced { by } => format!("replaced by {by}"),
        }
    }

    /// What lists say about it: `rolling out 1/2`, `failing: image … not found`.
    pub fn label(&self) -> Option<String> {
        match &self.state {
            RolloutState::Rolling { ready, of } if self.redeploy() => Some(format!("redeploying {} {ready}/{of}", self.to)),
            RolloutState::Rolling { ready, of } => Some(format!("rolling out {ready}/{of}")),
            RolloutState::Failing { reason, .. } => Some(format!("failing: {reason}")),
            RolloutState::Stuck { reason } => Some(format!("stuck: {reason}")),
            RolloutState::Done | RolloutState::Replaced { .. } => None,
        }
    }
}

/// A crash loop counts as failing from this many restarts of a pod of the new version.
pub const CRASHES_TO_FAIL: u32 = 2;

/// Why a pod of a new version is in trouble, if it is: an image that can't be pulled, a
/// crash loop (from [`CRASHES_TO_FAIL`] restarts), a container that can't be made, a pod no
/// node takes.
pub fn pod_failure(pod: &Pod, image: &str) -> Option<String> {
    if let Some(why) = &pod.unschedulable {
        return Some(format!("can't be scheduled: {why}"));
    }
    for c in &pod.containers {
        let Some(reason) = c.state.strip_prefix("waiting: ") else { continue };
        let reason = reason.trim_start_matches("Init:");
        if !TROUBLE.contains(&reason) {
            continue;
        }
        let message = c.message.as_deref().unwrap_or("");
        let init = if c.init { format!("init container {}: ", c.name) } else { String::new() };
        let text = match reason {
            "ErrImagePull" | "ImagePullBackOff" => {
                let lower = message.to_ascii_lowercase();
                if lower.contains("not found") || lower.contains("manifest unknown") { format!("image {image} not found") } else if message.is_empty() { format!("can't pull {image}") } else { message.to_string() }
            }
            "CrashLoopBackOff" => {
                if c.restarts < CRASHES_TO_FAIL {
                    continue;
                }
                let last = c.last_termination.as_ref().map(Termination::text).unwrap_or_else(|| "it keeps exiting".into());
                format!("crash loop: {last}")
            }
            _ if message.is_empty() => reason.to_string(),
            _ => format!("{reason}: {message}"),
        };
        return Some(format!("{init}{text}"));
    }
    None
}

/// Where `r` stands on `d` and its pods: nothing counts until the controller has seen its
/// generation; stuck past the deadline; done once settled with every new pod ready;
/// failing while a new pod is in trouble or pods can't be created; else rolling. Without
/// pods, or for a redeploy without the newest template (its old pods run the same tag),
/// the deployment's own counts decide.
pub fn assess(r: &Rollout, d: &Deployment, pods: &[Pod]) -> RolloutState {
    let of = d.replicas;
    if d.observed < r.generation {
        return RolloutState::Rolling { ready: 0, of };
    }
    if d.progress == Progress::Stuck {
        let reason = d.failure.clone().unwrap_or_else(|| "it went past its progress deadline".into());
        return RolloutState::Stuck { reason };
    }
    if pods.is_empty() || (d.template.is_none() && r.redeploy()) {
        // Nobody watches its pods, or they can't tell old from new: the deployment's own
        // counts, as `kubectl rollout status`.
        if d.progress == Progress::Settled && d.updated >= of && d.ready >= of {
            return RolloutState::Done;
        }
        return match &d.failure {
            Some(reason) => RolloutState::Failing { ready: d.updated.min(d.ready), of, reason: reason.clone() },
            None => RolloutState::Rolling { ready: d.updated.min(d.ready), of },
        };
    }
    let new: Vec<&Pod> = match &d.template {
        Some(t) => pods.iter().filter(|p| &p.template == t).collect(),
        None => pods.iter().filter(|p| p.tag == r.to).collect(),
    };
    let ready = new.iter().filter(|p| p.ready).count() as u32;
    if d.progress == Progress::Settled && ready >= of {
        return RolloutState::Done;
    }
    let reason = d.failure.clone().or_else(|| new.iter().find_map(|p| pod_failure(p, &r.image)));
    match reason {
        Some(reason) => RolloutState::Failing { ready, of, reason },
        None => RolloutState::Rolling { ready, of },
    }
}

/// One step of a bounded random walk (fake metrics): stays within `[lo, hi]`.
pub fn walk(value: u32, seed: &mut u64, lo: u32, hi: u32) -> u32 {
    *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    let span = (hi.saturating_sub(lo) / 10).max(1) as i64;
    let delta = ((*seed >> 33) as i64 % (2 * span + 1)) - span;
    (value as i64 + delta).clamp(lo as i64, hi as i64) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_part_is_an_init_container_by_its_apps_image_and_its_change_is_a_version() {
        let image = |name: &str, i: &str| Container { name: name.into(), image: Image::parse(i), ports: Vec::new(), resources: Resources::default(), config_maps: Vec::new(), pull_policy: None };
        let nginx = image("frontendapp", "registry/frontendapp:v1.0.0");
        let board = image("init-controlboardapp", "registry/controlboardapp:v4.48.0");
        let shell = image("init-shellapp", "registry/shellapp:v8.27.0");
        let d = Deployment { name: "frontendapp".into(), containers: vec![nginx], init_containers: vec![board, shell], ..Default::default() };
        assert_eq!(d.part("shellapp").map(|(i, c)| (i, c.name.as_str())), Some((1, "init-shellapp")));
        assert!(d.part("frontendapp").is_none(), "its main container isn't a part");
        assert_eq!(d.version(), "registry/frontendapp:v1.0.0 registry/controlboardapp:v4.48.0 registry/shellapp:v8.27.0");
        assert_eq!(part_label(d.part("shellapp").unwrap().1), "shellapp:v8.27.0");
        assert!(d.runs("v1.0.0") && d.runs("shellapp:v8.27.0") && !d.runs("shellapp:v8.26.0"));
        let ns = set_part_command("dev33", &d, d.part("shellapp").unwrap().1, "v8.28.0");
        assert_eq!(ns, "kubectl -n dev33 set image deployment/frontendapp init-shellapp=registry/shellapp:v8.28.0");
    }

    fn container(name: &str, image: &str) -> Container {
        Container { name: name.into(), image: Image::parse(image), ports: vec![5800], resources: Resources::default(), config_maps: Vec::new(), pull_policy: None }
    }

    fn dep() -> Deployment {
        Deployment {
            name: "entityservice".into(),
            containers: vec![container("app", "registry/entityservice:1.84.2")],
            replicas: 2,
            ready: 2,
            updated: 2,
            progress: Progress::Settled,
            selector: "app=entityservice".into(),
            since: Some(60),
            ..Default::default()
        }
    }

    #[test]
    fn a_deploy_is_mine_by_my_field_manager_or_devdecks_old_one() {
        assert_eq!(field_manager("i.alglave"), "i.alglave via Kugelblitz");
        assert!(by_me("i.alglave via Kugelblitz", Some("i.alglave")));
        assert!(by_me("devdeck", Some("i.alglave")), "history kept from before");
        assert!(by_me("devdeck", None));
        assert!(!by_me("j.doe via Kugelblitz", Some("i.alglave")), "a colleague's");
        assert!(!by_me("i.alglave via Kugelblitz", None), "nobody named here");
        assert!(!by_me("kubectl-set", Some("i.alglave")));
    }

    #[test]
    fn images_split_into_repo_tag_and_digest() {
        let i = Image::parse("europe-west1-docker.pkg.dev/p/registry06/graphservice:lan-000-slim");
        assert_eq!((i.repo.as_str(), i.tag.as_deref(), i.short_name()), ("europe-west1-docker.pkg.dev/p/registry06/graphservice", Some("lan-000-slim"), "graphservice"));
        let i = Image::parse("localhost:5000/team/app");
        assert_eq!((i.repo.as_str(), i.tag.as_deref()), ("localhost:5000/team/app", Some("latest")), "a port isn't a tag; no tag is latest");
        let i = Image::parse("localhost:5000/app:v2@sha256:abc");
        assert_eq!((i.repo.as_str(), i.tag.as_deref(), i.digest.as_deref()), ("localhost:5000/app", Some("v2"), Some("sha256:abc")));
        let i = Image::parse("registry/app@sha256:0123456789abcdef0123");
        assert_eq!((i.tag.as_deref(), i.label().as_str()), (None, "@sha256:0123456789ab"), "pinned by digest only: no tag");
        assert_eq!(i.to_string(), "registry/app@sha256:0123456789abcdef0123");
        assert_eq!(Image::parse("nginx").to_string(), "nginx:latest");
        let odd = Image::parse("registry/app@sha256:0123456789aé-and-more");
        assert_eq!(odd.label(), "@sha256:0123456789aé", "cut by characters, never inside one");
    }

    #[test]
    fn the_main_container_is_named_like_the_deployment_else_the_first() {
        let mut d = dep();
        d.containers = vec![container("envoy", "envoy:v1"), container("entityservice", "r/entityservice:main"), container("gw", "gw:v1")];
        assert_eq!((d.main().name.as_str(), d.tag(), d.others()), ("entityservice", "main".into(), 2));
        d.containers.remove(1);
        assert_eq!(d.tag(), "v1");
        d.main_mut().image.tag = Some("v2".into());
        assert_eq!(d.containers[0].image.tag.as_deref(), Some("v2"));
    }

    #[test]
    fn quantities() {
        for (q, m) in [("500m", 500), ("1", 1000), ("1.5", 1500), ("250000u", 250), ("2k", 2_000_000)] {
            assert_eq!(cpu_millis(q), Some(m), "{q}");
        }
        for (q, mi) in [("512Mi", 512), ("1Gi", 1024), ("500M", 477), ("134217728", 128), ("1e9", 954)] {
            assert_eq!(mem_mib(q), Some(mi), "{q}");
        }
        assert_eq!((cpu_millis("lots"), mem_mib("")), (None, None));
    }

    fn rollout(to: &str, generation: u64) -> Rollout {
        Rollout { from: "1.84.2".into(), to: to.into(), image: format!("registry/entityservice:{to}"), by: "devdeck".into(), generation, at_s: 0, state: RolloutState::Rolling { ready: 0, of: 2 } }
    }

    fn new_pod(template: &str, ready: bool, state: &str, restarts: u32, message: Option<&str>) -> Pod {
        let c = ContainerStatus { name: "app".into(), ready, restarts, state: state.into(), message: message.map(String::from), ..Default::default() };
        Pod { name: format!("entityservice-{template}-x"), template: template.into(), ready, tag: "lan-160".into(), containers: vec![c], ..Default::default() }
    }

    #[test]
    fn a_rollout_counts_once_the_controller_has_seen_it() {
        let mut d = dep();
        d.generation = 5;
        d.observed = 4;
        d.progress = Progress::Stuck;
        assert_eq!(assess(&rollout("lan-160", 5), &d, &[]), RolloutState::Rolling { ready: 0, of: 2 }, "an old stuck state isn't this deploy's");
        d.observed = 5;
        d.progress = Progress::Rolling;
        d.template = Some("abc".into());
        let pods = [new_pod("abc", true, "running", 0, None), new_pod("old", true, "running", 0, None)];
        assert_eq!(assess(&rollout("lan-160", 5), &d, &pods), RolloutState::Rolling { ready: 1, of: 2 }, "only the new template's ready pods");
        d.progress = Progress::Settled;
        let pods = [new_pod("abc", true, "running", 0, None), new_pod("abc", true, "running", 0, None)];
        assert_eq!(assess(&rollout("lan-160", 5), &d, &pods), RolloutState::Done);
        d.progress = Progress::Stuck;
        d.failure = None;
        assert_eq!(assess(&rollout("lan-160", 5), &d, &pods), RolloutState::Stuck { reason: "it went past its progress deadline".into() });
    }

    #[test]
    fn without_pods_the_deployments_counts_decide() {
        let mut d = dep();
        d.progress = Progress::Rolling;
        d.updated = 1;
        assert_eq!(assess(&rollout("lan-160", 0), &d, &[]), RolloutState::Rolling { ready: 1, of: 2 });
        d.progress = Progress::Settled;
        d.updated = 2;
        assert_eq!(assess(&rollout("lan-160", 0), &d, &[]), RolloutState::Done, "a deploy nobody watches the pods of still ends");
    }

    #[test]
    fn an_untagged_image_keeps_its_spec_text() {
        let i = Image::parse("registry/app");
        assert_eq!((i.raw.as_str(), i.to_string().as_str()), ("registry/app", "registry/app:latest"), "a deploy pins to what the spec says");
    }

    #[test]
    fn failing_says_why_and_keeps_following() {
        let mut d = dep();
        d.progress = Progress::Rolling;
        d.template = Some("abc".into());
        let pull = new_pod("abc", false, "waiting: ImagePullBackOff", 0, Some("rpc error: manifest for registry/entityservice:lan-160 not found: manifest unknown"));
        assert_eq!(assess(&rollout("lan-160", 0), &d, &[pull]), RolloutState::Failing { ready: 0, of: 2, reason: "image registry/entityservice:lan-160 not found".into() });
        let once = new_pod("abc", false, "waiting: CrashLoopBackOff", 1, None);
        assert_eq!(assess(&rollout("lan-160", 0), &d, std::slice::from_ref(&once)), RolloutState::Rolling { ready: 0, of: 2 }, "one crash isn't a loop yet");
        let mut twice = once.clone();
        twice.containers[0].restarts = 2;
        twice.containers[0].last_termination = Some(Termination { reason: "Error".into(), exit_code: Some(2), message: Some("panic: missing TADA_DB_URL".into()), at: None });
        assert_eq!(pod_failure(&twice, "img"), Some("crash loop: Error (exit 2): panic: missing TADA_DB_URL".into()));
        let config = new_pod("abc", false, "waiting: CreateContainerConfigError", 0, Some("secret \"db\" not found"));
        assert_eq!(pod_failure(&config, "img"), Some("CreateContainerConfigError: secret \"db\" not found".into()));
        let mut nowhere = new_pod("abc", false, "waiting: ContainerCreating", 0, None);
        nowhere.unschedulable = Some("0/5 nodes are available: insufficient cpu".into());
        assert_eq!(pod_failure(&nowhere, "img"), Some("can't be scheduled: 0/5 nodes are available: insufficient cpu".into()));
        assert_eq!(pod_failure(&new_pod("abc", false, "waiting: ContainerCreating", 0, None), "img"), None, "on its way");
        d.failure = Some("exceeded quota: pods".into());
        d.updated = 0;
        assert_eq!(assess(&rollout("lan-160", 0), &d, &[]), RolloutState::Failing { ready: 0, of: 2, reason: "exceeded quota: pods".into() });
    }

    #[test]
    fn a_deploy_targets_the_main_container_or_refuses() {
        let mut d = dep();
        assert_eq!(d.deploy_target().map(|(i, _)| i), Ok(0), "one container");
        d.containers.push(container("envoy", "registry/envoy:v1"));
        assert!(d.deploy_target().is_err(), "several, none named like it: a sidecar could be first");
        d.containers[0].name = "entityservice".into();
        assert_eq!(d.deploy_target().map(|(i, _)| i), Ok(0));
    }

    #[test]
    fn the_history_keeps_the_last_ten() {
        let mut h = Vec::new();
        for i in 0..12 {
            remember(&mut h, DeployRecord { at_s: i, from: "a".into(), to: "b".into(), by: "seen".into(), outcome: "done".into(), ..Default::default() });
        }
        assert_eq!(h.len(), HISTORY_KEPT);
        assert_eq!((h[0].at_s, h[9].at_s), (2, 11), "the newest kept");
    }

    #[test]
    fn protected_envs_need_the_exact_name() {
        let hades = Env { name: "hades".into(), protected: true };
        assert!(check_write(&hades, false).is_err());
        assert!(check_write(&hades, true).is_ok());
        assert!(check_write(&Env { name: "dev35".into(), protected: false }, false).is_ok());
        assert!(unlock_matches(&hades, " hades "));
        assert!(!unlock_matches(&hades, "Hades"));
    }

    #[test]
    fn restart_stamps_are_rfc3339_with_nanoseconds() {
        assert_eq!(restart_stamp(0), "1970-01-01T00:00:00.000000000Z");
        assert_eq!(restart_stamp(1_762_766_764_512_345_678), "2025-11-10T09:26:04.512345678Z");
        assert_eq!(restart_stamp(951_782_400_000_000_001), "2000-02-29T00:00:00.000000001Z", "a leap day");
    }

    #[test]
    fn the_restart_command() {
        assert_eq!(restart_command("dev35", &dep()), "kubectl -n dev35 rollout restart deployment/entityservice");
    }

    #[test]
    fn the_pull_policy_that_applies_and_the_redeploy_warning() {
        let mut c = container("app", "registry/app:main");
        assert_eq!(c.pull_policy(), "IfNotPresent", "unset, a tag other than latest");
        assert_eq!(redeploy_warning(&c), Some("app's imagePullPolicy is IfNotPresent: pods on nodes that already have this tag keep the build they have".into()));
        for image in ["registry/app:latest", "registry/app"] {
            assert_eq!(container("app", image).pull_policy(), "Always", "{image}: the API server's default");
            assert_eq!(redeploy_warning(&container("app", image)), None);
        }
        c.pull_policy = Some("Always".into());
        assert_eq!((c.pull_policy(), redeploy_warning(&c)), ("Always", None));
        c.pull_policy = Some("Never".into());
        assert!(redeploy_warning(&c).is_some_and(|w| w.contains("imagePullPolicy is Never")), "Never warns too");
        let mut pinned = container("app", "registry/app:main@sha256:abc");
        pinned.pull_policy = Some("Always".into());
        assert_eq!(redeploy_warning(&pinned), Some("pinned to a digest: the same build restarts".into()));
    }

    #[test]
    fn a_redeploy_without_the_newest_template_goes_by_the_deployments_counts() {
        let mut d = dep();
        d.progress = Progress::Rolling;
        d.updated = 1;
        d.ready = 2;
        let mut redeploy = rollout("1.84.2", 0);
        redeploy.image = "registry/entityservice:1.84.2".into();
        let old = [new_pod("old", true, "running", 0, None), new_pod("old", true, "running", 0, None)];
        let mut old = old.to_vec();
        for p in &mut old {
            p.tag = "1.84.2".into();
        }
        assert_eq!(assess(&redeploy, &d, &old), RolloutState::Rolling { ready: 1, of: 2 }, "old pods on the same tag aren't new");
        assert_eq!(redeploy.label().as_deref(), Some("redeploying 1.84.2 0/2"));
        d.progress = Progress::Settled;
        d.updated = 2;
        assert_eq!(assess(&redeploy, &d, &old), RolloutState::Done);
    }

    #[test]
    fn command_and_metrics() {
        assert_eq!(set_image_command("dev35", &dep(), "lan-160"), "kubectl -n dev35 set image deployment/entityservice app=registry/entityservice:lan-160");
        let mut seed = 7;
        let mut v = 100;
        for _ in 0..1000 {
            v = walk(v, &mut seed, 20, 400);
            assert!((20..=400).contains(&v));
        }
    }
}
