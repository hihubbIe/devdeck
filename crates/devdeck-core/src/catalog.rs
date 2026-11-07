//! What one workspace offers: its apps, the tools found, its envs, its cluster and the
//! ports held outside devdeck. Discovery and config (M2) build it for a real workspace;
//! `demo` builds it for the demo.

use crate::apps::{App, Tools};

/// A target environment (`dev35`). Envs belong to the workspace: runs use them even
/// without a cluster (R6.6).
#[derive(Clone, Debug, PartialEq)]
pub struct Env {
    pub name: String,
    /// Writes need an explicit unlock (R7.6).
    pub protected: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ClusterSpec {
    pub services: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PortSpec {
    pub port: u16,
    pub holder: String,
    pub command: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Catalog {
    pub apps: Vec<App>,
    pub tools: Tools,
    /// Alphabetical; empty when the workspace has none (no env selector, R2.3).
    pub envs: Vec<Env>,
    pub default_env: Option<String>,
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

    pub fn env(&self, name: &str) -> Option<&Env> {
        self.envs.iter().find(|e| e.name == name)
    }

    /// The env to use: `wanted` when this workspace has it, else the default.
    pub fn pick_env(&self, wanted: Option<String>) -> Option<String> {
        match wanted {
            Some(e) if self.env(&e).is_some() => Some(e),
            _ => self.default_env.clone().filter(|d| self.env(d).is_some()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_choice_falls_back_to_the_default() {
        let env = |n: &str| Env { name: n.into(), protected: false };
        let c = Catalog { envs: vec![env("dev33"), env("dev35")], default_env: Some("dev35".into()), ..Default::default() };
        assert_eq!(c.pick_env(Some("dev33".into())).as_deref(), Some("dev33"));
        assert_eq!(c.pick_env(Some("gone".into())).as_deref(), Some("dev35"));
        assert_eq!(c.pick_env(None).as_deref(), Some("dev35"));
        assert_eq!(Catalog::default().pick_env(Some("dev33".into())), None, "no envs, no env");
    }
}
