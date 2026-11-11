//! The ports each command listened on last time (R6.9), so a conflict can be reported
//! before a start. Which ports a run listens on, and who holds one, are the OS's:
//! `devdeck_sys::ports`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Ports each command listened on last time, in the state folder: a conflict can be
/// reported before a start even when no config names the port. Nested by workspace id,
/// app id, command id (app ids hold `/`, so no joined key).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PortMemory {
    pub path: PathBuf,
    pub ports: BTreeMap<String, BTreeMap<String, BTreeMap<String, Vec<u16>>>>,
}

impl PortMemory {
    /// Reads `ports.json` in `state_dir`. Missing or unreadable means nothing
    /// remembered; a broken file is moved aside (`ports.json.bad`) so the next save
    /// doesn't destroy it.
    pub fn load(state_dir: &Path) -> PortMemory {
        let path = state_dir.join("ports.json");
        let ports = match std::fs::read_to_string(&path) {
            Ok(t) => serde_json::from_str(&t).unwrap_or_else(|_| {
                let _ = std::fs::rename(&path, path.with_extension("json.bad"));
                BTreeMap::new()
            }),
            Err(_) => BTreeMap::new(),
        };
        PortMemory { path, ports }
    }

    pub fn get(&self, workspace: &str, app: &str, cmd: &str) -> &[u16] {
        self.ports.get(workspace).and_then(|a| a.get(app)).and_then(|c| c.get(cmd)).map_or(&[], Vec::as_slice)
    }

    /// Saves at once when they changed. Ports are kept sorted without duplicates; no
    /// ports forgets the command.
    pub fn remember(&mut self, workspace: &str, app: &str, cmd: &str, ports: &[u16]) -> Result<(), String> {
        let ports: Vec<u16> = ports.iter().copied().collect::<BTreeSet<_>>().into_iter().collect();
        if self.get(workspace, app, cmd) == ports.as_slice() {
            return Ok(());
        }
        if ports.is_empty() {
            let apps = self.ports.get_mut(workspace).expect("known, since it changed");
            let cmds = apps.get_mut(app).expect("known, since it changed");
            cmds.remove(cmd);
            if cmds.is_empty() {
                apps.remove(app);
            }
            if apps.is_empty() {
                self.ports.remove(workspace);
            }
        } else {
            self.ports.entry(workspace.into()).or_default().entry(app.into()).or_default().insert(cmd.into(), ports);
        }
        let text = serde_json::to_string_pretty(&self.ports).map_err(|e| e.to_string())?;
        crate::state::write_atomic(&self.path, &text)
    }
}

/// A `kubectl port-forward` command line, read: what it forwards and where.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Forward {
    /// `-n`/`--namespace`, when given.
    pub namespace: Option<String>,
    /// `svc/graphservice`, `pod/x`, `deployment/y` (a bare name is a pod).
    pub target: String,
    pub local: u16,
    pub remote: u16,
}

impl Forward {
    /// The deployment or service it reaches, by name (`svc/graphservice` → `graphservice`).
    pub fn service(&self) -> Option<&str> {
        let (kind, name) = self.target.split_once('/')?;
        matches!(kind, "svc" | "service" | "services" | "deploy" | "deployment" | "deployments").then_some(name)
    }
}

/// Reads `command` as `kubectl … port-forward <target> <local>:<remote>` (or `<port>`,
/// both the same); `None` for anything else. The first port pair is the one shown.
pub fn port_forward(command: &str) -> Option<Forward> {
    let words: Vec<&str> = command.split_whitespace().collect();
    let program = words.first()?.rsplit('/').next()?;
    if program != "kubectl" {
        return None;
    }
    let at = words.iter().position(|w| *w == "port-forward")?;
    let mut namespace = None;
    let mut rest = Vec::new();
    let mut i = 1;
    while i < words.len() {
        let w = words[i];
        match w {
            "-n" | "--namespace" => {
                namespace = words.get(i + 1).map(|n| n.to_string());
                i += 2;
                continue;
            }
            _ if w.starts_with("--namespace=") => namespace = Some(w["--namespace=".len()..].to_string()),
            _ if i > at && !w.starts_with('-') => rest.push(w),
            _ => {}
        }
        i += 1;
    }
    let target = rest.first()?.to_string();
    let (local, remote) = match rest.get(1)?.split_once(':') {
        Some((l, r)) => (if l.is_empty() { 0 } else { l.parse().ok()? }, r.parse().ok()?),
        None => {
            let p = rest[1].parse().ok()?;
            (p, p)
        }
    };
    Some(Forward { namespace, target, local, remote })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_port_forward_is_read_and_anything_else_isnt() {
        let f = port_forward("kubectl -n dev35 port-forward svc/graphservice 5900:80").unwrap();
        assert_eq!(f, Forward { namespace: Some("dev35".into()), target: "svc/graphservice".into(), local: 5900, remote: 80 });
        assert_eq!(f.service(), Some("graphservice"));
        let f = port_forward("/usr/local/bin/kubectl port-forward --namespace=dev33 pod/api-7f9 8080").unwrap();
        assert_eq!((f.namespace.as_deref(), f.local, f.remote, f.service()), (Some("dev33"), 8080, 8080, None));
        assert_eq!(port_forward("kubectl get pods"), None);
        assert_eq!(port_forward("ssh -L 5900:localhost:80 host"), None);
        assert_eq!(port_forward("kubectl port-forward svc/x"), None, "no port");
    }

    fn temp_dir(tag: &str) -> crate::testing::TempDir {
        crate::testing::TempDir::new(tag)
    }

    #[test]
    fn port_memory_round_trip() {
        let dir = temp_dir("memory");
        let mut m = PortMemory::load(&dir);
        assert_eq!(m.get("work", "apps/web", "dev"), [] as [u16; 0]);
        m.remember("work", "apps/web", "dev", &[5173, 3000, 5173]).unwrap();
        m.remember("work", "api", "start", &[8080]).unwrap();
        assert_eq!(m.get("work", "apps/web", "dev"), [3000, 5173], "sorted, no duplicates");
        let back = PortMemory::load(&dir);
        assert_eq!(back, m);

        std::fs::remove_file(&m.path).unwrap();
        m.remember("work", "apps/web", "dev", &[3000, 5173]).unwrap();
        assert!(!m.path.exists(), "unchanged ports are not saved again");

        m.remember("work", "apps/web", "dev", &[]).unwrap();
        assert_eq!(PortMemory::load(&dir).ports, BTreeMap::from([("work".into(), BTreeMap::from([("api".into(), BTreeMap::from([("start".into(), vec![8080])]))]))]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_broken_port_memory_is_empty_and_kept_aside() {
        let dir = temp_dir("broken");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("ports.json"), "{oops").unwrap();
        let m = PortMemory::load(&dir);
        assert!(m.ports.is_empty());
        assert!(dir.join("ports.json.bad").exists(), "broken file kept aside");
        assert!(!dir.join("ports.json").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
