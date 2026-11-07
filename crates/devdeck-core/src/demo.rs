//! Fake catalog for the M1 prototype: what discovery (M2) and the cluster (M3) will
//! provide later. Deterministic, so snapshots and tests are stable.

use crate::apps::{App, Command, EnvApply, Tier, Tools};
pub use crate::cluster::Env;
use crate::cluster::{Deployment, Pod, PodPhase, Resources, Usage};

#[derive(Clone, Debug, PartialEq)]
pub struct ClusterSpec {
    /// Alphabetical.
    pub envs: Vec<Env>,
    pub default_env: String,
    pub services: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PortSpec {
    pub port: u16,
    pub holder: String,
    pub command: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Catalog {
    pub apps: Vec<App>,
    pub tools: Tools,
    /// `None`: the workspace has no cluster, so no Cluster activity (R2.3).
    pub cluster: Option<ClusterSpec>,
    pub ports: Vec<PortSpec>,
    /// Commands that run as soon as the workspace starts (the demo's "already running").
    pub autostart: Vec<(String, String)>,
}

impl Catalog {
    pub fn app(&self, id: &str) -> Option<&App> {
        self.apps.iter().find(|a| a.id == id)
    }
}

/// FNV-1a: stable across runs and platforms.
pub fn hash(s: &str) -> u64 {
    s.bytes().fold(0xcbf29ce484222325, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3))
}

pub fn catalog(workspace: &str) -> Catalog {
    match workspace {
        "work" => work(),
        "personal" => personal(),
        _ => Catalog { apps: Vec::new(), tools: Tools::default(), cluster: None, ports: Vec::new(), autostart: Vec::new() },
    }
}

fn command(id: &str, line: &str, tools: &[&str], ports: &[u16]) -> Command {
    Command { id: id.into(), line: line.into(), tier: Tier::of(id), tools: tools.iter().map(|t| t.to_string()).collect(), ports: ports.to_vec() }
}

fn go_app(name: &str, port: u16) -> App {
    App {
        id: name.into(),
        name: name.into(),
        folder: format!("~/Documents/repos/{name}"),
        kind: "go".into(),
        commands: vec![
            command("run", "go run ./cmd/server", &["go"], &[port]),
            command("build", "go build ./...", &["go"], &[]),
            command("test", "go test ./...", &["go"], &[]),
            command("lint", "golangci-lint run", &["golangci-lint"], &[]),
        ],
        env: EnvApply::Variable("TADA_ENV".into()),
    }
}

fn pnpm_app(name: &str, port: u16) -> App {
    let short = name.rsplit('/').next().unwrap_or(name);
    App {
        id: name.into(),
        name: name.into(),
        folder: format!("~/Documents/repos/front-end/apps/{short}"),
        kind: "pnpm".into(),
        commands: vec![
            command("start:dev", "pnpm run start:dev", &["pnpm", "node"], &[port]),
            command("build", "pnpm run build", &["pnpm", "node"], &[]),
            command("test", "pnpm run test", &["pnpm", "node"], &[]),
            command("lint", "pnpm run lint", &["pnpm", "node"], &[]),
            command("prepare", "pnpm run prepare", &["pnpm", "node"], &[]),
        ],
        env: EnvApply::FileRewrite(".env.local".into()),
    }
}

fn work() -> Catalog {
    let go = [
        "boardservice",
        "contentverticalservice",
        "entityservice",
        "eventservice",
        "geodataservice",
        "graphservice",
        "mastertadaservice",
        "pagerendererservice",
        "proxyapi",
        "replayerservicev2",
        "reportservicev2",
        "schedulerservice",
        "tadawebapi",
        "vesselservice",
        "visualizerservice",
        "a-service-with-a-name-long-enough-to-truncate-in-the-sidebar",
    ];
    let mut apps: Vec<App> = go.iter().enumerate().map(|(i, n)| go_app(n, 5700 + i as u16 * 10)).collect();
    apps.extend(
        ["front-end/controlboardapp", "front-end/mastertadaapp", "front-end/storeadminapp"]
            .iter()
            .enumerate()
            .map(|(i, n)| pnpm_app(n, 3000 + i as u16)),
    );
    apps.push(App {
        id: "remotedesktopapp".into(),
        name: "remotedesktopapp".into(),
        folder: "~/Documents/repos/remotedesktopapp".into(),
        kind: "docker".into(),
        commands: vec![command("up", "docker compose up", &["docker"], &[6080]), command("down", "docker compose down", &["docker"], &[])],
        env: EnvApply::Argument("--profile".into()),
    });
    let services: Vec<String> = go.iter().take(15).map(|s| s.to_string()).collect();
    let mut envs: Vec<Env> = (31..=36).map(|n| Env { name: format!("dev{n}"), protected: false }).collect();
    envs.extend(["dls", "hades", "lab"].map(|n| Env { name: n.into(), protected: true }));
    envs.sort_by(|a, b| a.name.cmp(&b.name));
    let ports = vec![PortSpec { port: 5900, holder: "kubectl".into(), command: "kubectl -n dev35 port-forward svc/graphservice 5900:80".into() }];
    Catalog {
        apps,
        tools: Tools::new(&["go", "pnpm", "node", "kubectl"]),
        cluster: Some(ClusterSpec { envs, default_env: "dev35".into(), services }),
        ports,
        autostart: vec![("entityservice".into(), "run".into()), ("front-end/mastertadaapp".into(), "start:dev".into())],
    }
}

fn personal() -> Catalog {
    let apps = vec![
        App {
            id: "blog".into(),
            name: "blog".into(),
            folder: "~/Documents/private_repos/blog".into(),
            kind: "pnpm".into(),
            commands: vec![command("dev", "pnpm run dev", &["pnpm", "node"], &[4321]), command("build", "pnpm run build", &["pnpm", "node"], &[])],
            env: EnvApply::None,
        },
        App {
            id: "devdeck".into(),
            name: "devdeck".into(),
            folder: "~/Documents/private_repos/devdeck".into(),
            kind: "cargo".into(),
            commands: vec![command("run", "cargo run", &["cargo"], &[]), command("test", "cargo test", &["cargo"], &[]), command("build", "cargo build --release", &["cargo"], &[])],
            env: EnvApply::None,
        },
        App {
            id: "dotfiles-sync".into(),
            name: "dotfiles-sync".into(),
            folder: "~/Documents/private_repos/dotfiles".into(),
            kind: "make".into(),
            commands: vec![command("sync", "make sync", &["make"], &[])],
            env: EnvApply::None,
        },
    ];
    Catalog {
        apps,
        tools: Tools::new(&["pnpm", "node", "cargo", "make"]),
        cluster: None,
        ports: Vec::new(),
        autostart: vec![("blog".into(), "dev".into())],
    }
}

/// The deployment of `name` in `env`, or `None` when it isn't deployed there.
pub fn deployment(env: &str, name: &str) -> Option<Deployment> {
    let h = hash(&format!("{env}/{name}"));
    if env == "dls" && h.is_multiple_of(3) {
        return None;
    }
    let tag = match h % 5 {
        0 => format!("lan-{}", 100 + h % 90),
        1 => "main".to_string(),
        _ => format!("1.{}.{}", 60 + h % 30, h % 9),
    };
    let replicas = 1 + (h % 3) as u32;
    let ready = if h.is_multiple_of(11) { replicas - 1 } else { replicas };
    Some(Deployment {
        name: name.into(),
        container: "app".into(),
        repo: format!("europe-west1-docker.pkg.dev/registry/{name}"),
        tag,
        replicas,
        ready,
        since_s: 3600 * (1 + h % 70),
        resources: Resources { cpu_request_m: 100, cpu_limit_m: 500, mem_request_mi: 128, mem_limit_mi: 512 },
        ports: vec![80],
        configmap: Some(format!("{name}-config")),
    })
}

/// The pods of a deployment: one per replica; the last one waits when not all are ready.
pub fn pods(env: &str, d: &Deployment) -> Vec<Pod> {
    (0..d.replicas)
        .map(|i| {
            let h = hash(&format!("{env}/{}/{i}", d.name));
            let healthy = i < d.ready;
            Pod {
                name: format!("{}-{:x}-{:05x}", d.name, h % 0xffff, (h >> 20) % 0xfffff),
                phase: if healthy { PodPhase::Running } else { PodPhase::Waiting("CrashLoopBackOff".into()) },
                ready: healthy,
                restarts: if healthy { (h % 3) as u32 } else { 7 + (h % 5) as u32 },
                last_termination: (!healthy).then(|| "Error (exit 2): panic: missing TADA_DB_URL".to_string()),
                age_s: d.since_s,
                node: format!("gke-{env}-pool-{}", h % 4),
                tag: d.tag.clone(),
                usage: Usage { cpu_m: 40 + (h % 120) as u32, mem_mi: 90 + (h % 200) as u32 },
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn work_has_a_cluster_and_personal_does_not() {
        let w = catalog("work");
        let c = w.cluster.as_ref().expect("cluster");
        assert!(c.envs.iter().any(|e| e.name == c.default_env));
        assert!(c.envs.windows(2).all(|p| p[0].name < p[1].name), "envs alphabetical");
        assert_eq!(deployment("dev35", "entityservice"), deployment("dev35", "entityservice"), "deterministic");
        assert!(c.services.iter().any(|s| deployment("dls", s).is_none()), "some services aren't on dls");
        assert!(catalog("personal").cluster.is_none());
        assert_eq!(w.tools.missing_for(w.app("remotedesktopapp").unwrap()), ["docker"]);
    }

    #[test]
    fn pods_follow_readiness() {
        let d = Deployment { ready: 1, replicas: 2, ..deployment("dev35", "entityservice").unwrap() };
        let p = pods("dev35", &d);
        assert_eq!(p.len(), 2);
        assert!(p[0].ready && !p[1].ready);
        assert!(p[1].last_termination.is_some());
    }
}
