//! Typed view addresses. A [`ViewRef`] is the string saved in layouts; `ViewAddr` is
//! what it means. Parts are escaped so ids may contain `/` (`front-end/mastertadaapp`).

use crate::layout::ViewRef;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViewAddr {
    App { app: String },
    Run { app: String, cmd: String },
    Svc { env: String, name: String },
    Logs { env: String, name: String },
    Pod { env: String, pod: String },
    Matrix,
    Port { port: String },
    /// What discovery and the config reported for the workspace.
    Problems,
    /// The first-run screen: no workspace configured.
    Welcome,
    /// A shell in a folder: terminal `n` of the workspace.
    Terminal { n: u32, dir: String },
    /// The git panel of the repository rooted at `root` (an absolute path).
    Git { root: String },
    /// A text file, view only (R12.1): its absolute path.
    File { path: String },
    /// Anything else (demo views, refs from older versions).
    Other(String),
}

fn esc(s: &str) -> String {
    s.replace('%', "%25").replace('/', "%2F")
}

fn unesc(s: &str) -> String {
    s.replace("%2F", "/").replace("%25", "%")
}

impl ViewAddr {
    pub fn to_ref(&self) -> ViewRef {
        ViewRef::new(match self {
            ViewAddr::App { app } => format!("app/{}", esc(app)),
            ViewAddr::Run { app, cmd } => format!("run/{}/{}", esc(app), esc(cmd)),
            ViewAddr::Svc { env, name } => format!("svc/{}/{}", esc(env), esc(name)),
            ViewAddr::Logs { env, name } => format!("logs/{}/{}", esc(env), esc(name)),
            ViewAddr::Pod { env, pod } => format!("pod/{}/{}", esc(env), esc(pod)),
            ViewAddr::Matrix => "matrix".to_string(),
            ViewAddr::Port { port } => format!("port/{}", esc(port)),
            ViewAddr::Problems => "problems".to_string(),
            ViewAddr::Welcome => "welcome".to_string(),
            ViewAddr::Terminal { n, dir } => format!("term/{n}/{}", esc(dir)),
            ViewAddr::Git { root } => format!("git/{}", esc(root)),
            ViewAddr::File { path } => format!("file/{}", esc(path)),
            ViewAddr::Other(s) => s.clone(),
        })
    }

    pub fn parse(r: &ViewRef) -> ViewAddr {
        let s = r.as_str();
        let parts: Vec<String> = s.split('/').map(unesc).collect();
        let p: Vec<&str> = parts.iter().map(String::as_str).collect();
        match p.as_slice() {
            ["app", app] => ViewAddr::App { app: app.to_string() },
            ["run", app, cmd] => ViewAddr::Run { app: app.to_string(), cmd: cmd.to_string() },
            ["svc", env, name] => ViewAddr::Svc { env: env.to_string(), name: name.to_string() },
            ["logs", env, name] => ViewAddr::Logs { env: env.to_string(), name: name.to_string() },
            ["pod", env, pod] => ViewAddr::Pod { env: env.to_string(), pod: pod.to_string() },
            ["matrix"] => ViewAddr::Matrix,
            ["port", port] => ViewAddr::Port { port: port.to_string() },
            ["problems"] => ViewAddr::Problems,
            ["welcome"] => ViewAddr::Welcome,
            ["git", root] => ViewAddr::Git { root: root.to_string() },
            ["file", path] => ViewAddr::File { path: path.to_string() },
            ["term", n, dir] if n.parse::<u32>().is_ok() => ViewAddr::Terminal { n: n.parse().unwrap_or_default(), dir: dir.to_string() },
            _ => ViewAddr::Other(s.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_with_slashes_and_percent() {
        let all = [
            ViewAddr::App { app: "front-end/mastertadaapp".into() },
            ViewAddr::Run { app: "front-end/mastertadaapp".into(), cmd: "start:dev".into() },
            ViewAddr::Run { app: "odd%2Fname".into(), cmd: "a/b".into() },
            ViewAddr::Svc { env: "dev35".into(), name: "entityservice".into() },
            ViewAddr::Git { root: "/home/me/repos/front-end".into() },
            ViewAddr::File { path: "/home/me/repos/a b%/notes.md".into() },
            ViewAddr::Logs { env: "hades".into(), name: "graphservice".into() },
            ViewAddr::Pod { env: "dev35".into(), pod: "entityservice-7d9f-x2k4p".into() },
            ViewAddr::Matrix,
            ViewAddr::Port { port: "5800".into() },
            ViewAddr::Other("demo/logs".into()),
            ViewAddr::Terminal { n: 3, dir: "/home/me/repos/a b%/c".into() },
        ];
        for a in all {
            assert_eq!(ViewAddr::parse(&a.to_ref()), a, "{:?}", a.to_ref());
        }
        assert_eq!(ViewAddr::App { app: "front-end/x".into() }.to_ref().as_str(), "app/front-end%2Fx");
        assert_eq!(ViewAddr::parse(&ViewRef::new("deploy/dev35/x")), ViewAddr::Other("deploy/dev35/x".into()));
    }
}
