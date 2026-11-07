//! The environment runs get (R6.5, R6.8, R1.4): the login shell's variables, the
//! programs on its PATH, and the Node version each app asks for. Builder C of M2 step 2
//! implements this file; the interface is fixed here.
//!
//! An app started from the desktop doesn't see what `~/.bashrc` sets (nvm, pnpm,
//! `~/go/bin`), so the user's login shell is asked once (`$SHELL -lic env`, with a
//! timeout) and its variables are the base for every run; the machine config's `path`
//! goes in front. Node: `.nvmrc` is searched from the app's folder upward to its
//! workspace folder; versions (`20`, `v20.11.1`, `lts/*`, `lts/iron`, using
//! `~/.nvm/alias` for aliases) resolve against the installs found in the machine
//! config's `node_dirs`; the chosen Node's `bin` goes first on the run's PATH, which
//! also decides whether `pnpm` and `npm` exist for that app.

use crate::apps::{App, Command};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

/// How long the login shell may take to print its environment.
const SHELL_TIMEOUT: Duration = Duration::from_secs(5);
/// How long output may still come once the shell has exited.
const AFTER_EXIT: Duration = Duration::from_millis(300);

/// Printed by the shell right before its environment; what rc files print comes before
/// it. The command prints it in two halves, so a shell echoing its command can't fake it.
const MARKER: &str = "__devdeck_env__\n";
const PRINT: &str = "printf '%s%s\\n' __devdeck_ env__; env -0";

/// Programs that come with a Node install or are installed into one, so they're looked
/// up with the app's Node first on the PATH.
const NODE_TOOLS: [&str; 6] = ["node", "npm", "npx", "pnpm", "yarn", "corepack"];

/// Variables describing the shell that was asked, not the environment.
const SHELL_ONLY: [&str; 4] = ["PWD", "OLDPWD", "SHLVL", "_"];

#[derive(Clone, Debug, Default, PartialEq)]
pub struct NodeInstall {
    /// `22.23.3`.
    pub version: String,
    /// Its `bin` folder.
    pub bin: PathBuf,
}

/// What runs need, found once per workspace load.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Toolbox {
    /// The login shell's variables (PATH included, machine `path` in front).
    pub vars: Vec<(String, String)>,
    pub node: Vec<NodeInstall>,
    /// Where nvm keeps aliases (`~/.nvm/alias`), when it exists.
    pub aliases: Option<PathBuf>,
}

impl Toolbox {
    /// Asks the login shell for its environment and finds Node installs.
    pub fn load(machine: &crate::config::MachineConfig) -> Result<Toolbox, String> {
        let shell = std::env::var("SHELL").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/bin/sh".into());
        let out = capture(shell_command(&shell), SHELL_TIMEOUT).map_err(|e| format!("login shell {shell}: {e}"))?;
        let vars = parse_env(&out).ok_or_else(|| format!("login shell {shell}: printed no environment"))?;
        Ok(Self::from_vars(vars, machine))
    }

    /// devdeck's own environment, when the login shell couldn't be asked (it may lack
    /// what `~/.bashrc` sets: the caller says so).
    pub fn current(machine: &crate::config::MachineConfig) -> Toolbox {
        Self::from_vars(std::env::vars().collect(), machine)
    }

    fn from_vars(mut vars: Vec<(String, String)>, machine: &crate::config::MachineConfig) -> Toolbox {
        prepend_path(&mut vars, &machine.path);
        let home = var(&vars, "HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(PathBuf::from));
        let aliases = home.as_ref().map(|h| h.join(".nvm/alias")).filter(|a| a.is_dir());
        // Without configured folders, nvm's own.
        let dirs = match machine.node_dirs.is_empty() {
            true => home.map(|h| vec![h.join(".nvm/versions/node")]).unwrap_or_default(),
            false => machine.node_dirs.clone(),
        };
        Toolbox { vars, node: node_installs(&dirs), aliases }
    }

    /// The Node install for an app, or why there is none (`.nvmrc wants node 20, which
    /// isn't installed`). `Ok(None)` when the app asks for no version.
    pub fn node_for(&self, app: &App) -> Result<Option<&NodeInstall>, String> {
        let Some(wanted) = app.node.as_deref().map(str::trim).filter(|w| !w.is_empty()) else { return Ok(None) };
        self.resolve(wanted).map(Some).ok_or_else(|| format!(".nvmrc wants node {wanted}, which isn't installed"))
    }

    /// The PATH for a run of `app`: its Node first, then the base PATH.
    pub fn path_for(&self, app: &App) -> Vec<PathBuf> {
        let node = self.node_for(app).ok().flatten().map(|n| n.bin.clone());
        node.into_iter().chain(self.base_path()).collect()
    }

    /// Why `cmd` of `app` can't run (`go not found`, the Node reason), if it can't.
    pub fn missing(&self, app: &App, cmd: &Command) -> Option<String> {
        let (mut node_path, mut base) = (None, None);
        for tool in &cmd.tools {
            let path: &[PathBuf] = if NODE_TOOLS.contains(&tool.as_str()) {
                if let Err(why) = self.node_for(app) {
                    return Some(why);
                }
                node_path.get_or_insert_with(|| self.path_for(app))
            } else {
                base.get_or_insert_with(|| self.base_path())
            };
            if which(tool, path).is_none() {
                return Some(format!("{tool} not found"));
            }
        }
        None
    }

    /// The login shell's PATH (machine `path` in front), as folders.
    fn base_path(&self) -> Vec<PathBuf> {
        var(&self.vars, "PATH").unwrap_or("").split(':').filter(|d| !d.is_empty()).map(PathBuf::from).collect()
    }

    /// The newest install `wanted` names (`20`, `v20.11`, `node`), following nvm aliases
    /// (`lts/*` → `lts/iron` → `v20.11.1`) a few levels deep, so a loop ends.
    fn resolve(&self, wanted: &str) -> Option<&NodeInstall> {
        let mut wanted = wanted.to_string();
        for _ in 0..8 {
            if wanted == "node" || wanted == "stable" {
                return self.node.first();
            }
            if let Some(parts) = version_parts(&wanted) {
                return self.node.iter().find(|n| version_parts(&n.version).is_some_and(|v| v.starts_with(&parts)));
            }
            wanted = self.alias(&wanted)?;
        }
        None
    }

    /// What the nvm alias `name` (`lts/iron`, `lts/*`, `default`) points to: the first
    /// line of its file in the alias folder.
    fn alias(&self, name: &str) -> Option<String> {
        if name.split('/').any(|p| p.is_empty() || p == "." || p == "..") {
            return None;
        }
        let text = std::fs::read_to_string(self.aliases.as_ref()?.join(name)).ok()?;
        text.lines().map(str::trim).find(|l| !l.is_empty()).map(str::to_string)
    }
}

/// The program `name` in `path`, when it's there and executable.
pub fn which(name: &str, path: &[PathBuf]) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    if name.is_empty() || name.contains('/') {
        return None;
    }
    path.iter()
        .map(|d| d.join(name))
        .find(|p| std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0))
}

/// The version an `.nvmrc` at `dir` or above (up to `stop`, included) asks for, trimmed
/// (`20`, `v20.11.1`, `lts/iron`). Discovery sets `App::node` with it.
pub fn nvmrc(dir: &Path, stop: &Path) -> Option<String> {
    let mut at = Some(dir);
    while let Some(d) = at {
        // A file with only comments or blank lines asks for nothing: keep looking up.
        if let Ok(text) = std::fs::read_to_string(d.join(".nvmrc"))
            && let Some(v) = text.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with('#'))
        {
            return Some(v.to_string());
        }
        if d == stop {
            break;
        }
        at = d.parent();
    }
    None
}

/// `shell` as a login, interactive shell (so it reads the rc files that set nvm, pnpm
/// and PATH) printing the marker, then its environment.
fn shell_command(shell: &str) -> std::process::Command {
    let mut cmd = std::process::Command::new(shell);
    cmd.args(["-l", "-i", "-c", PRINT]);
    cmd
}

/// The output of `cmd`, run in its own session: an interactive shell would otherwise
/// take over the terminal devdeck was started from. Reading stops at the end of the
/// output, or shortly after the shell exited (a job its rc files started in the
/// background may keep the output open), or after `timeout`; then what it started is
/// killed. What was read is kept: the caller decides whether it's complete.
fn capture(mut cmd: std::process::Command, timeout: Duration) -> Result<Vec<u8>, String> {
    use std::os::unix::process::CommandExt;
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    // SAFETY: `setsid` is async-signal-safe and touches no memory of the parent.
    unsafe {
        cmd.pre_exec(|| rustix::process::setsid().map(|_| ()).map_err(Into::into));
    }
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    let mut stdout = child.stdout.take().expect("stdout is piped");
    let out = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let (tx, rx) = std::sync::mpsc::channel();
    // Read on a thread so the wait can stop early; it ends at the end of the output (when
    // the shell and what it started have exited or been killed).
    let sink = out.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        while let Ok(n @ 1..) = stdout.read(&mut buf) {
            sink.lock().unwrap_or_else(|e| e.into_inner()).extend_from_slice(&buf[..n]);
        }
        let _ = tx.send(());
    });
    let start = Instant::now();
    let mut exited: Option<Instant> = None;
    let ended = loop {
        if rx.recv_timeout(Duration::from_millis(50)).is_ok() {
            break true;
        }
        if exited.is_none() && child.try_wait().ok().flatten().is_some() {
            exited = Some(Instant::now());
        }
        if exited.is_some_and(|t| t.elapsed() >= AFTER_EXIT) || start.elapsed() >= timeout {
            break false;
        }
    };
    let group = rustix::process::Pid::from_child(&child);
    let _ = rustix::process::kill_process_group(group, rustix::process::Signal::KILL);
    let _ = child.wait();
    let out = std::mem::take(&mut *out.lock().unwrap_or_else(|e| e.into_inner()));
    if !ended && exited.is_none() && out.is_empty() {
        return Err(format!("no answer within {} s", timeout.as_secs_f32()));
    }
    Ok(out)
}

/// The variables `env -0` printed after the marker, without the shell's own; `None`
/// without the marker or variables (`env -0` unsupported). What follows the last NUL
/// (a logout script's output) is ignored.
fn parse_env(out: &[u8]) -> Option<Vec<(String, String)>> {
    let marker = MARKER.as_bytes();
    let start = out.windows(marker.len()).position(|w| w == marker)? + marker.len();
    let body = &out[start..];
    let end = body.iter().rposition(|b| *b == 0).map_or(0, |i| i + 1);
    let vars: Vec<(String, String)> = body[..end]
        .split(|b| *b == 0)
        .filter_map(|entry| {
            let entry = String::from_utf8_lossy(entry);
            let (k, v) = entry.split_once('=')?;
            (!k.is_empty() && !SHELL_ONLY.contains(&k)).then(|| (k.to_string(), v.to_string()))
        })
        .collect();
    (!vars.is_empty()).then_some(vars)
}

/// Puts `front` before the PATH in `vars` (one is added when there is none), each
/// folder once.
fn prepend_path(vars: &mut Vec<(String, String)>, front: &[PathBuf]) {
    if front.is_empty() {
        return;
    }
    let mut dirs: Vec<String> = front.iter().map(|p| p.to_string_lossy().into_owned()).collect();
    for d in var(vars, "PATH").unwrap_or("").split(':').filter(|d| !d.is_empty()) {
        if !dirs.iter().any(|x| x == d) {
            dirs.push(d.to_string());
        }
    }
    let path = dirs.join(":");
    match vars.iter_mut().find(|(k, _)| k == "PATH") {
        Some((_, v)) => *v = path,
        None => vars.push(("PATH".into(), path)),
    }
}

fn var<'a>(vars: &'a [(String, String)], key: &str) -> Option<&'a str> {
    vars.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

/// The Node installs in `dirs` (`v22.23.3/bin/node`), newest first. A version found in
/// two folders keeps the first.
fn node_installs(dirs: &[PathBuf]) -> Vec<NodeInstall> {
    let mut found: Vec<(Vec<u64>, NodeInstall)> = Vec::new();
    for entry in dirs.iter().filter_map(|d| std::fs::read_dir(d).ok()).flatten().flatten() {
        let name = entry.file_name();
        let Some(version) = name.to_str().and_then(|n| n.strip_prefix('v')) else { continue };
        let Some(parts) = version_parts(version).filter(|p| p.len() == 3) else { continue };
        let bin = entry.path().join("bin");
        if found.iter().any(|(p, _)| *p == parts) || which("node", std::slice::from_ref(&bin)).is_none() {
            continue;
        }
        found.push((parts, NodeInstall { version: version.to_string(), bin }));
    }
    found.sort_by(|a, b| b.0.cmp(&a.0));
    found.into_iter().map(|(_, n)| n).collect()
}

/// `20`, `v20.11`, `20.11.1` as numbers (one to three parts); `None` for anything else.
fn version_parts(s: &str) -> Option<Vec<u64>> {
    let s = s.strip_prefix('v').unwrap_or(s);
    let parts: Vec<u64> = s
        .split('.')
        .map(|p| if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) { p.parse().ok() } else { None })
        .collect::<Option<_>>()?;
    (1..=3).contains(&parts.len()).then_some(parts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::{EnvApply, Tier};

    #[test]
    fn nvmrc_is_found_upward_until_the_workspace_folder() {
        let root = crate::testing::TempDir::new("nvmrc");
        let app = root.join("front-end/apps/x");
        std::fs::create_dir_all(&app).unwrap();
        assert_eq!(nvmrc(&app, &root), None);
        std::fs::write(root.join("front-end/.nvmrc"), "# node\n 20 \n").unwrap();
        assert_eq!(nvmrc(&app, &root).as_deref(), Some("20"));
        assert_eq!(nvmrc(&app, &root.join("front-end/apps")), None, "stops at the workspace folder");
        std::fs::write(app.join(".nvmrc"), "# only a comment\n\n").unwrap();
        assert_eq!(nvmrc(&app, &root).as_deref(), Some("20"), "an .nvmrc asking for nothing doesn't stop the search");
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// An empty folder of its own for one test.
    fn tmp(tag: &str) -> crate::testing::TempDir {
        crate::testing::TempDir::new(tag)
    }

    /// An executable file at `path` (folders created).
    fn exe(path: &Path) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// A Node install `v<version>` in `dir` with these programs in its `bin`.
    fn node(dir: &Path, version: &str, tools: &[&str]) {
        for t in ["node"].iter().chain(tools) {
            exe(&dir.join(format!("v{version}/bin/{t}")));
        }
    }

    fn app(node: Option<&str>) -> App {
        App {
            id: "a".into(),
            name: "a".into(),
            folder: "/a".into(),
            dir: "/a".into(),
            kind: "pnpm".into(),
            library: false,
            node: node.map(str::to_string),
            commands: vec![],
            env: EnvApply::None,
            urls: vec![],
        }
    }

    fn cmd(tools: &[&str]) -> Command {
        Command { id: "pnpm:dev".into(), line: "x".into(), tier: Tier::Primary, tools: tools.iter().map(|t| t.to_string()).collect(), ports: vec![] }
    }

    fn toolbox(node_dirs: &[&Path], path: &[&Path], aliases: Option<PathBuf>) -> Toolbox {
        let node_dirs: Vec<PathBuf> = node_dirs.iter().map(|d| d.to_path_buf()).collect();
        let path = path.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>().join(":");
        Toolbox { vars: vec![("PATH".into(), path)], node: node_installs(&node_dirs), aliases }
    }

    fn versions(tb: &Toolbox) -> Vec<&str> {
        tb.node.iter().map(|n| n.version.as_str()).collect()
    }

    fn wants<'a>(tb: &'a Toolbox, v: &str) -> Result<Option<&'a str>, String> {
        tb.node_for(&app(Some(v))).map(|n| n.map(|n| n.version.as_str()))
    }

    #[test]
    fn installs_are_found_newest_first() {
        let d = tmp("installs");
        let (a, b) = (d.join("a"), d.join("b"));
        for v in ["18.20.4", "20.9.0", "20.11.1", "22.1.0"] {
            node(&a, v, &[]);
        }
        std::fs::create_dir_all(a.join("v21.0.0/bin")).unwrap();
        std::fs::create_dir_all(a.join("notes/bin")).unwrap();
        node(&b, "20.11.1", &[]);
        node(&b, "16.0.0", &[]);
        let tb = toolbox(&[&a, &b, &d.join("absent")], &[], None);
        assert_eq!(versions(&tb), ["22.1.0", "20.11.1", "20.9.0", "18.20.4", "16.0.0"], "v21 has no node");
        assert_eq!(tb.node[1].bin, a.join("v20.11.1/bin"), "the first folder wins");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn versions_resolve_to_the_newest_match() {
        let d = tmp("resolve");
        for v in ["18.20.4", "20.9.0", "20.11.1", "22.1.0"] {
            node(&d, v, &[]);
        }
        let tb = toolbox(&[&d], &[], None);
        assert_eq!(tb.node_for(&app(None)), Ok(None), "asks for nothing");
        assert_eq!(wants(&tb, "20"), Ok(Some("20.11.1")));
        assert_eq!(wants(&tb, "v20"), Ok(Some("20.11.1")));
        assert_eq!(wants(&tb, "v20.9.0"), Ok(Some("20.9.0")));
        assert_eq!(wants(&tb, "20.9"), Ok(Some("20.9.0")));
        assert_eq!(wants(&tb, " 18 "), Ok(Some("18.20.4")));
        assert_eq!(wants(&tb, "node"), Ok(Some("22.1.0")));
        assert_eq!(wants(&tb, "stable"), Ok(Some("22.1.0")));
        assert_eq!(wants(&tb, "20").unwrap(), wants(&tb, "20.11").unwrap());
        assert_eq!(wants(&tb, "19"), Err(".nvmrc wants node 19, which isn't installed".into()));
        assert!(wants(&tb, "2").is_err(), "a prefix of numbers, not of text");
        assert!(wants(&tb, "20.9.0.1").is_err());
        assert!(wants(&tb, "iojs").is_err(), "no alias folder");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn nvm_aliases_lead_to_versions() {
        let d = tmp("aliases");
        let installs = d.join("node");
        for v in ["20.11.1", "22.1.0"] {
            node(&installs, v, &[]);
        }
        let alias = d.join("alias");
        std::fs::create_dir_all(alias.join("lts")).unwrap();
        for (name, to) in [("lts/*", "lts/iron\n"), ("lts/iron", "v20.11.1\n"), ("lts/hydrogen", "v18.19.0"), ("default", "22"), ("loop", "loop")] {
            std::fs::write(alias.join(name), to).unwrap();
        }
        let tb = toolbox(&[&installs], &[], Some(alias));
        assert_eq!(wants(&tb, "lts/*"), Ok(Some("20.11.1")));
        assert_eq!(wants(&tb, "lts/iron"), Ok(Some("20.11.1")));
        assert_eq!(wants(&tb, "default"), Ok(Some("22.1.0")));
        assert_eq!(wants(&tb, "lts/hydrogen"), Err(".nvmrc wants node lts/hydrogen, which isn't installed".into()));
        assert!(wants(&tb, "lts/jod").is_err(), "no alias file");
        assert!(wants(&tb, "loop").is_err(), "an alias loop ends");
        assert!(wants(&tb, "../alias/default").is_err(), "stays in the alias folder");
        let without = toolbox(&[&installs], &[], None);
        assert!(wants(&without, "lts/iron").is_err(), "no alias folder");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn node_tools_follow_the_apps_node() {
        let d = tmp("tools");
        let installs = d.join("node");
        node(&installs, "20.11.1", &["npm", "pnpm"]);
        node(&installs, "22.1.0", &["npm"]);
        let bin = d.join("bin");
        exe(&bin.join("go"));
        let tb = toolbox(&[&installs], &[&bin], None);
        assert_eq!(tb.missing(&app(Some("20")), &cmd(&["pnpm", "node"])), None);
        assert_eq!(tb.missing(&app(Some("22")), &cmd(&["node", "pnpm"])).as_deref(), Some("pnpm not found"), "pnpm only under 20");
        assert_eq!(tb.missing(&app(Some("19")), &cmd(&["pnpm"])).as_deref(), Some(".nvmrc wants node 19, which isn't installed"));
        assert_eq!(tb.missing(&app(Some("19")), &cmd(&["go"])), None, "other tools don't need the Node");
        assert_eq!(tb.missing(&app(None), &cmd(&["go", "docker"])).as_deref(), Some("docker not found"));
        assert_eq!(tb.missing(&app(None), &cmd(&["node"])).as_deref(), Some("node not found"), "no Node asked, none on the PATH");
        exe(&bin.join("pnpm"));
        assert_eq!(tb.missing(&app(Some("22")), &cmd(&["pnpm"])), None, "pnpm on the base PATH");
        assert_eq!(tb.missing(&app(None), &cmd(&[])), None);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_apps_node_comes_first_on_the_path() {
        let d = tmp("path");
        node(&d, "20.11.1", &[]);
        let tb = toolbox(&[&d], &[Path::new("/a"), Path::new("/b")], None);
        let base = [PathBuf::from("/a"), PathBuf::from("/b")];
        assert_eq!(tb.path_for(&app(Some("20"))), [d.join("v20.11.1/bin"), base[0].clone(), base[1].clone()]);
        assert_eq!(tb.path_for(&app(None)), base);
        assert_eq!(tb.path_for(&app(Some("19"))), base, "a missing Node adds nothing");
        let mut vars = vec![("HOME".to_string(), "/h".to_string()), ("PATH".to_string(), "/a:/go::/b".to_string())];
        prepend_path(&mut vars, &[PathBuf::from("/go"), PathBuf::from("/c")]);
        assert_eq!(var(&vars, "PATH"), Some("/go:/c:/a:/b"), "machine folders first, once");
        let mut none = vec![];
        prepend_path(&mut none, &[PathBuf::from("/go")]);
        assert_eq!(none, [("PATH".to_string(), "/go".to_string())]);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn which_wants_an_executable_file() {
        let d = tmp("which");
        let (a, b) = (d.join("a"), d.join("b"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::write(a.join("go"), "").unwrap();
        std::fs::create_dir_all(a.join("task")).unwrap();
        exe(&b.join("go"));
        exe(&b.join("task"));
        let path = [a.clone(), b.clone()];
        assert_eq!(which("go", &path), Some(b.join("go")), "not executable in a");
        assert_eq!(which("task", &path), Some(b.join("task")), "a folder in a");
        exe(&a.join("go"));
        assert_eq!(which("go", &path), Some(a.join("go")), "the first folder wins");
        assert_eq!(which("docker", &path), None);
        assert_eq!(which("", &path), None);
        assert_eq!(which("b/go", &[d.to_path_buf()]), None, "a name, not a path");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_environment_follows_the_marker() {
        let out = b"Welcome!\n__devdeck_\n__devdeck_env__\nA=1\0B=x=y\0PWD=/here\0SHLVL=2\0MULTI=l1\nl2\0junk\0logout\x1b[H";
        let vars = parse_env(out).unwrap();
        let expected = [("A", "1"), ("B", "x=y"), ("MULTI", "l1\nl2")].map(|(k, v)| (k.to_string(), v.to_string()));
        assert_eq!(vars, expected, "noise around the variables and the shell's own are dropped");
        assert_eq!(parse_env(b"A=1\0"), None, "no marker");
        assert_eq!(parse_env(b"__devdeck_env__\n"), None, "no variables");
    }

    #[test]
    fn a_real_login_shell_gives_its_environment() {
        let home = tmp("shell");
        std::fs::write(home.join(".profile"), "echo 'hello from .profile'\nexport DEVDECK_PROFILE=read\n").unwrap();
        let mut sh = shell_command("/bin/sh");
        sh.env("HOME", home.path()).env_remove("ENV").env_remove("BASH_ENV");
        let vars = parse_env(&capture(sh, Duration::from_secs(5)).unwrap()).unwrap();
        assert_eq!(var(&vars, "DEVDECK_PROFILE"), Some("read"), "the login profile ran");
        assert!(var(&vars, "PATH").is_some());
        assert_eq!(var(&vars, "PWD"), None);
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn a_shell_that_hangs_is_killed() {
        let mut sh = std::process::Command::new("/bin/sh");
        sh.args(["-c", "sleep 10 & sleep 10"]);
        let start = std::time::Instant::now();
        assert_eq!(capture(sh, Duration::from_millis(200)), Err("no answer within 0.2 s".into()));
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn output_is_kept_when_a_background_job_holds_it_open() {
        let mut sh = std::process::Command::new("/bin/sh");
        sh.args(["-c", "printf done; sleep 10 &"]);
        let start = std::time::Instant::now();
        assert_eq!(capture(sh, Duration::from_secs(5)), Ok(b"done".to_vec()));
        assert!(start.elapsed() < Duration::from_secs(2), "doesn't wait for the job: {:?}", start.elapsed());
    }
}
