//! Fake catalog for the M1 prototype: what discovery (M2) and the cluster (M3) will
//! provide later. Deterministic, so snapshots and tests are stable.

use crate::apps::{App, Command, EnvApply, Tier, Tools};
pub use crate::catalog::{Catalog, ClusterSpec, Env, PortSpec};
use crate::cluster::{Container, Deployment, Image, Pod, PodPhase, Progress, Resources, Usage};

/// FNV-1a of `s` (see [`crate::fnv1a`]).
pub fn hash(s: &str) -> u64 {
    crate::fnv1a(s.as_bytes())
}

pub fn catalog(workspace: &str) -> Catalog {
    match workspace {
        "work" => work(),
        "personal" => personal(),
        _ => Catalog::default(),
    }
}

/// `id` is `source:name` (`go:run`), as discovery makes them.
fn command(id: &str, line: &str, tools: &[&str], ports: &[u16]) -> Command {
    let name = id.split_once(':').map_or(id, |(_, n)| n);
    Command { id: id.into(), line: line.into(), tier: Tier::of(name, &[]), tools: tools.iter().map(|t| t.to_string()).collect(), from_install: Vec::new(), setup: false, ports: ports.to_vec() }
}

fn app(id: &str, folder: String, kind: &str, commands: Vec<Command>, env: EnvApply) -> App {
    App {
        id: id.into(),
        name: id.into(),
        dir: folder.clone().into(),
        folder,
        kind: kind.into(),
        library: false,
        facts: Default::default(),
        commands,
        env,
        urls: Vec::new(),
    }
}

fn go_app(name: &str, port: u16) -> App {
    let commands = vec![
        command("go:run", "go run ./cmd/server", &["go"], &[port]),
        command("go:build", "go build ./...", &["go"], &[]),
        command("go:test", "go test ./...", &["go"], &[]),
        command("go:lint", "golangci-lint run", &["golangci-lint"], &[]),
    ];
    app(name, format!("~/Documents/repos/{name}"), "go", commands, EnvApply::Variable("TADA_ENV".into()))
}

fn pnpm_app(name: &str, port: u16) -> App {
    let short = name.rsplit('/').next().unwrap_or(name);
    let commands = vec![
        command("pnpm:start:dev", "pnpm run start:dev", &["pnpm", "node"], &[port]),
        command("pnpm:build", "pnpm run build", &["pnpm", "node"], &[]),
        command("pnpm:test", "pnpm run test", &["pnpm", "node"], &[]),
        command("pnpm:lint", "pnpm run lint", &["pnpm", "node"], &[]),
        command("pnpm:prepare", "pnpm run prepare", &["pnpm", "node"], &[]),
    ];
    let mut a = app(name, format!("~/Documents/repos/front-end/apps/{short}"), "pnpm", commands, EnvApply::FileRewrite(".env.local".into()));
    a.facts.node = Some(crate::toolchains::node::Facts { nvmrc: Some("22".into()), ..Default::default() });
    a
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
    apps.push(app(
        "remotedesktopapp",
        "~/Documents/repos/remotedesktopapp".into(),
        "docker",
        vec![command("compose:up", "docker compose up", &["docker"], &[6080]), command("compose:down", "docker compose down", &["docker"], &[])],
        EnvApply::Argument("--profile {env}".into()),
    ));
    let services: Vec<String> = go.iter().take(15).map(|s| s.to_string()).collect();
    let mut envs: Vec<Env> = (31..=36).map(|n| Env { name: format!("dev{n}"), protected: false }).collect();
    envs.extend(["dls", "hades", "lab"].map(|n| Env { name: n.into(), protected: true }));
    envs.sort_by(|a, b| a.name.cmp(&b.name));
    let ports = vec![PortSpec { port: 5900, holder: "kubectl".into(), command: "kubectl -n dev35 port-forward svc/graphservice 5900:80".into() }];
    Catalog {
        apps,
        tools: Tools::new(&["go", "pnpm", "node", "kubectl"]),
        envs,
        default_env: Some("dev35".into()),
        cluster: Some(ClusterSpec { services }),
        ports,
        autostart: vec![("entityservice".into(), "go:run".into()), ("front-end/mastertadaapp".into(), "pnpm:start:dev".into())],
    }
}

fn personal() -> Catalog {
    let apps = vec![
        app(
            "blog",
            "~/Documents/private_repos/blog".into(),
            "pnpm",
            vec![command("pnpm:dev", "pnpm run dev", &["pnpm", "node"], &[4321]), command("pnpm:build", "pnpm run build", &["pnpm", "node"], &[])],
            EnvApply::None,
        ),
        app(
            "devdeck",
            "~/Documents/private_repos/devdeck".into(),
            "cargo",
            vec![
                command("cargo:run", "cargo run", &["cargo"], &[]),
                command("cargo:test", "cargo test", &["cargo"], &[]),
                command("cargo:build", "cargo build --release", &["cargo"], &[]),
            ],
            EnvApply::None,
        ),
        app("dotfiles-sync", "~/Documents/private_repos/dotfiles".into(), "make", vec![command("make:sync", "make sync", &["make"], &[])], EnvApply::None),
    ];
    Catalog {
        apps,
        tools: Tools::new(&["pnpm", "node", "cargo", "make"]),
        autostart: vec![("blog".into(), "pnpm:dev".into())],
        ..Catalog::default()
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
    let resources = Resources { cpu_request_m: Some(100), cpu_limit_m: Some(500), mem_request_mi: Some(128), mem_limit_mi: Some(512) };
    let mut containers = vec![Container {
        name: name.into(),
        image: Image::parse(&format!("europe-west1-docker.pkg.dev/registry/{name}:{tag}")),
        ports: vec![80],
        resources,
        config_maps: vec![format!("{name}-config")],
    }];
    // Some run a proxy next to them, as on the real cluster.
    if h.is_multiple_of(4) {
        let image = Image::parse("europe-west1-docker.pkg.dev/registry/envoy:v1.39.1");
        containers.push(Container { name: "envoy".into(), image, ports: vec![9901], resources: Resources::default(), config_maps: Vec::new() });
    }
    Some(Deployment { name: name.into(), containers, replicas, ready, updated: replicas, progress: Progress::Settled, since: Some(NOW_S - 3600 * (1 + h % 70)) })
}

/// The demo's clock (epoch seconds): ages and history are computed against it, so
/// snapshots don't change from one run to the next.
pub const NOW_S: u64 = 1_762_500_000;

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
                age_s: NOW_S.saturating_sub(d.since.unwrap_or(NOW_S)),
                node: format!("gke-{env}-pool-{}", h % 4),
                tag: d.tag(),
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
        assert!(w.default_env.as_deref().is_some_and(|d| w.env(d).is_some()));
        assert!(w.envs.windows(2).all(|p| p[0].name < p[1].name), "envs alphabetical");
        assert_eq!(deployment("dev35", "entityservice"), deployment("dev35", "entityservice"), "deterministic");
        assert!(c.services.iter().any(|s| deployment("dls", s).is_none()), "some services aren't on dls");
        assert!(catalog("personal").cluster.is_none());
        assert_eq!(w.tools.missing_for(w.app("remotedesktopapp").unwrap()), ["docker not found"]);
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
