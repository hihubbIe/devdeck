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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Resources {
    pub cpu_request_m: u32,
    pub cpu_limit_m: u32,
    pub mem_request_mi: u32,
    pub mem_limit_mi: u32,
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
    /// Waiting with a reason (`ImagePullBackOff`, `CrashLoopBackOff`).
    Waiting(String),
    Terminating,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Pod {
    pub name: String,
    pub phase: PodPhase,
    pub ready: bool,
    pub restarts: u32,
    pub last_termination: Option<String>,
    pub age_s: u64,
    pub node: String,
    pub tag: String,
    pub usage: Usage,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Deployment {
    pub name: String,
    pub container: String,
    pub repo: String,
    pub tag: String,
    pub replicas: u32,
    pub ready: u32,
    pub since_s: u64,
    pub resources: Resources,
    pub ports: Vec<u16>,
    pub configmap: Option<String>,
}

impl Deployment {
    pub fn image(&self) -> String {
        format!("{}:{}", self.repo, self.tag)
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
    format!("kubectl -n {namespace} set image deployment/{} {}={}:{tag}", d.name, d.container, d.repo)
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

    fn dep() -> Deployment {
        Deployment {
            name: "entityservice".into(),
            container: "app".into(),
            repo: "registry/entityservice".into(),
            tag: "1.84.2".into(),
            replicas: 2,
            ready: 2,
            since_s: 60,
            resources: Resources { cpu_request_m: 100, cpu_limit_m: 500, mem_request_mi: 128, mem_limit_mi: 512 },
            ports: vec![5800],
            configmap: None,
        }
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
