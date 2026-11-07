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
        Image { repo: repo.to_string(), tag, digest }
    }

    /// The last path segment of the repo (`graphservice`).
    pub fn short_name(&self) -> &str {
        self.repo.rsplit('/').next().unwrap_or(&self.repo)
    }

    /// What lists show: the tag, else the start of the digest.
    pub fn label(&self) -> String {
        match (&self.tag, &self.digest) {
            (Some(t), _) => t.clone(),
            (None, Some(d)) => format!("@{}", &d[..d.len().min(19)]),
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
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Usage {
    pub cpu_m: u32,
    pub mem_mi: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PodPhase {
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
#[derive(Clone, Debug, PartialEq)]
pub struct ContainerStatus {
    pub name: String,
    /// An init container (runs before the others).
    pub init: bool,
    pub ready: bool,
    pub restarts: u32,
    /// `running`, `waiting: CrashLoopBackOff`, `terminated: Error (exit 1)`.
    pub state: String,
    pub last_termination: Option<Termination>,
}

#[derive(Clone, Debug, PartialEq)]
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

#[derive(Clone, Debug, PartialEq)]
pub struct Deployment {
    pub name: String,
    /// At least one.
    pub containers: Vec<Container>,
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

    /// The other containers, for a "+n" next to the tag.
    pub fn others(&self) -> usize {
        self.containers.len().saturating_sub(1)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DeployRecord {
    /// Seconds since the epoch.
    pub at_s: u64,
    pub from: String,
    pub to: String,
    pub outcome: String,
}

/// The exact command a tag change runs (R7.3 states it before confirming).
pub fn set_image_command(namespace: &str, d: &Deployment, tag: &str) -> String {
    let c = d.main();
    format!("kubectl -n {namespace} set image deployment/{} {}={}:{tag}", d.name, c.name, c.image.repo)
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

#[derive(Clone, Debug, PartialEq)]
pub enum Rollout {
    Progressing { from: String, to: String, updated: u32, of: u32 },
    Failed { from: String, to: String, reason: String },
    Done { from: String, to: String },
}

impl Rollout {
    /// The tag Undo goes back to.
    pub fn undo_target(&self) -> Option<&str> {
        match self {
            Rollout::Failed { from, .. } | Rollout::Done { from, .. } => Some(from),
            Rollout::Progressing { .. } => None,
        }
    }
}

/// A rollout over time: one replica per step; a tag containing `broken` can't be
/// pulled and fails at the first step.
#[derive(Clone, Debug, PartialEq)]
pub struct RolloutSim {
    pub state: Rollout,
}

impl RolloutSim {
    pub fn start(from: &str, to: &str, replicas: u32) -> Self {
        Self { state: Rollout::Progressing { from: from.into(), to: to.into(), updated: 0, of: replicas.max(1) } }
    }

    pub fn step(&mut self, repo: &str) {
        if let Rollout::Progressing { from, to, updated, of } = &self.state {
            self.state = if to.contains("broken") {
                Rollout::Failed {
                    from: from.clone(),
                    to: to.clone(),
                    reason: format!("ImagePullBackOff: manifest for {repo}:{to} not found"),
                }
            } else if updated + 1 >= *of {
                Rollout::Done { from: from.clone(), to: to.clone() }
            } else {
                Rollout::Progressing { from: from.clone(), to: to.clone(), updated: updated + 1, of: *of }
            };
        }
    }

    pub fn finished(&self) -> bool {
        !matches!(self.state, Rollout::Progressing { .. })
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

    fn container(name: &str, image: &str) -> Container {
        Container { name: name.into(), image: Image::parse(image), ports: vec![5800], resources: Resources::default(), config_maps: Vec::new() }
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
        }
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

    #[test]
    fn rollout_succeeds_one_replica_per_step() {
        let mut r = RolloutSim::start("1.84.2", "lan-160", 2);
        r.step("repo");
        assert_eq!(r.state, Rollout::Progressing { from: "1.84.2".into(), to: "lan-160".into(), updated: 1, of: 2 });
        r.step("repo");
        assert_eq!(r.state, Rollout::Done { from: "1.84.2".into(), to: "lan-160".into() });
        assert_eq!(r.state.undo_target(), Some("1.84.2"));
        r.step("repo");
        assert!(r.finished(), "stays done");
    }

    #[test]
    fn broken_tags_fail_with_a_pull_error_and_undo_to_the_previous() {
        let mut r = RolloutSim::start("1.84.2", "lan-broken", 3);
        assert_eq!(r.state.undo_target(), None);
        r.step("registry/entityservice");
        let Rollout::Failed { reason, .. } = &r.state else { panic!("failed") };
        assert!(reason.starts_with("ImagePullBackOff"));
        assert_eq!(r.state.undo_target(), Some("1.84.2"));
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
