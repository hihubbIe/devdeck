//! Fake catalog for the M1 prototype: what discovery (M2) and the cluster (M3) will
//! provide later. Deterministic, so snapshots and tests are stable.

use crate::list::ListItem;

#[derive(Clone, Debug, PartialEq)]
pub struct Env {
    pub name: String,
    /// Writes need an explicit unlock (R7.6).
    pub protected: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Cluster {
    /// Alphabetical.
    pub envs: Vec<Env>,
    pub default_env: String,
    pub services: Vec<String>,
}

impl Cluster {
    /// Deployments of one env: label is the service, detail its image tag, running when
    /// all replicas are ready.
    pub fn deployments(&self, env: &str) -> Vec<ListItem> {
        self.services
            .iter()
            .map(|s| {
                let h = hash(&format!("{env}/{s}"));
                let tag = match h % 5 {
                    0 => format!("lan-{}", 100 + h % 90),
                    1 => "main".to_string(),
                    _ => format!("1.{}.{}", 60 + h % 30, h % 9),
                };
                ListItem::new(s.clone(), s.clone()).detail(tag).running(!h.is_multiple_of(11))
            })
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Catalog {
    pub apps: Vec<ListItem>,
    /// `None`: the workspace has no cluster, so no Cluster activity (R2.3).
    pub cluster: Option<Cluster>,
    pub ports: Vec<ListItem>,
}

fn hash(s: &str) -> u64 {
    // FNV-1a: stable across runs and platforms.
    s.bytes().fold(0xcbf29ce484222325, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3))
}

pub fn catalog(workspace: &str) -> Catalog {
    match workspace {
        "work" => work(),
        "personal" => personal(),
        _ => Catalog { apps: Vec::new(), cluster: None, ports: Vec::new() },
    }
}

fn work() -> Catalog {
    let running = ["entityservice", "front-end/mastertadaapp"];
    let apps = [
        "boardservice",
        "contentverticalservice",
        "eventservice",
        "entityservice",
        "front-end/controlboardapp",
        "front-end/mastertadaapp",
        "front-end/storeadminapp",
        "geodataservice",
        "graphservice",
        "mastertadaservice",
        "pagerendererservice",
        "proxyapi",
        "remotedesktopapp",
        "replayerservicev2",
        "reportservicev2",
        "schedulerservice",
        "tadawebapi",
        "vesselservice",
        "visualizerservice",
        "a-service-with-a-name-long-enough-to-truncate-in-the-sidebar",
    ]
    .into_iter()
    .map(|a| {
        let detail = if a.starts_with("front-end/") { "pnpm" } else { "go" };
        ListItem::new(a, a).detail(detail).running(running.contains(&a))
    })
    .collect();
    let services = [
        "boardservice",
        "contentverticalservice",
        "entityservice",
        "eventservice",
        "geodataservice",
        "graphservice",
        "mastertadaservice",
        "pagerendererservice",
        "proxyapi",
        "reportservicev2",
        "schedulerservice",
        "tadawebapi",
        "vesselservice",
        "visualizerservice",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    let mut envs: Vec<Env> = (31..=36).map(|n| Env { name: format!("dev{n}"), protected: false }).collect();
    envs.extend(["dls", "hades", "lab"].map(|n| Env { name: n.into(), protected: true }));
    envs.sort_by(|a, b| a.name.cmp(&b.name));
    let ports = [("3000", "front-end/mastertadaapp · node"), ("5800", "entityservice · go"), ("5900", "port-forward dev35/graphservice")]
        .into_iter()
        .map(|(p, who)| ListItem::new(p, format!(":{p}")).detail(who).running(true))
        .collect();
    Catalog { apps, cluster: Some(Cluster { envs, default_env: "dev35".into(), services }), ports }
}

fn personal() -> Catalog {
    let apps = [("blog", "pnpm", true), ("devdeck", "cargo", false), ("dotfiles-sync", "make", false)]
        .into_iter()
        .map(|(a, d, r)| ListItem::new(a, a).detail(d).running(r))
        .collect();
    let ports = vec![ListItem::new("4321", ":4321").detail("blog · node").running(true)];
    Catalog { apps, cluster: None, ports }
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
        assert_eq!(c.deployments("dev35"), c.deployments("dev35"), "deterministic");
        assert!(catalog("personal").cluster.is_none());
    }
}
