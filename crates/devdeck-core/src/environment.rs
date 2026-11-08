//! The environment runs get (R6.5, R1.4): the login shell's variables and the PATH, and
//! the tool check.
//!
//! An app started from the desktop doesn't see what `~/.bashrc` sets (nvm, pnpm,
//! `~/go/bin`), so the user's login shell is asked once (`$SHELL -lic env`, with a
//! timeout) and its variables are the base for every run; the machine config's `path`
//! goes in front. What toolchains find on the machine (Node installs) loads with it.
//!
//! The tool check, for a command of an app: each toolchain gives the folders to put
//! first on the app's PATH (Node: the app's Node); a command needing a program from a
//! toolchain that can't serve the app is refused with its reason; then the command's
//! programs are looked up on the run's PATH (those folders, then the base); then its
//! toolchain says whether the project is installed.

use crate::apps::{App, Command};
use crate::toolchains::{self, Machine};
use std::io::Read;
use std::path::PathBuf;
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

/// Variables describing the shell that was asked, not the environment.
const SHELL_ONLY: [&str; 4] = ["PWD", "OLDPWD", "SHLVL", "_"];

/// What runs need, found once per workspace load.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Toolbox {
    /// The login shell's variables (PATH included, machine `path` in front).
    pub vars: Vec<(String, String)>,
    /// What toolchains found on this machine.
    pub machine: Machine,
}

impl Toolbox {
    /// Asks the login shell for its environment, and what toolchains find.
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

    fn from_vars(mut vars: Vec<(String, String)>, config: &crate::config::MachineConfig) -> Toolbox {
        prepend_path(&mut vars, &config.path);
        let home = var(&vars, "HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(PathBuf::from));
        let machine = Machine::load(config, home.as_deref(), &path_of(&vars));
        Toolbox { vars, machine }
    }

    /// The PATH for a run of `app`: what its toolchains put first, then the base PATH.
    pub fn path_for(&self, app: &App) -> Vec<PathBuf> {
        let front = toolchains::ALL.iter().filter_map(|t| t.front(app, &self.machine).ok()).flatten();
        front.chain(self.base_path()).collect()
    }

    /// Why `cmd` of `app` can't run (`go not found`, `.nvmrc wants node 20, which isn't
    /// installed`, `vite isn't installed: run pnpm install`), if it can't.
    pub fn missing(&self, app: &App, cmd: &Command) -> Option<String> {
        let mut front: Vec<PathBuf> = Vec::new();
        for t in toolchains::ALL {
            match t.front(app, &self.machine) {
                Ok(dirs) => front.extend(dirs),
                Err(why) if cmd.tools.iter().any(|tool| t.provides(tool)) => return Some(why),
                Err(_) => {}
            }
        }
        let path: Vec<PathBuf> = front.into_iter().chain(self.base_path()).collect();
        if let Some(tool) = cmd.tools.iter().find(|tool| which(tool, &path).is_none()) {
            return Some(format!("{tool} not found"));
        }
        toolchains::of(cmd).and_then(|t| t.installed(app, cmd, &path))
    }

    /// The login shell's PATH (machine `path` in front), as folders.
    fn base_path(&self) -> Vec<PathBuf> {
        path_of(&self.vars)
    }
}

/// The PATH in `vars`, as folders.
fn path_of(vars: &[(String, String)]) -> Vec<PathBuf> {
    var(vars, "PATH").unwrap_or("").split(':').filter(|d| !d.is_empty()).map(PathBuf::from).collect()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::{EnvApply, Tier};
    use crate::toolchains::node;
    use std::path::Path;

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

    /// An app whose `.nvmrc` asks for `nvmrc`.
    fn app(nvmrc: Option<&str>) -> App {
        let facts = toolchains::Facts { node: nvmrc.map(|v| node::Facts { nvmrc: Some(v.into()), ..Default::default() }) };
        App { id: "a".into(), name: "a".into(), folder: "/a".into(), dir: "/a".into(), kind: "pnpm".into(), library: false, facts, commands: vec![], env: EnvApply::None, urls: vec![], images: vec![] }
    }

    fn cmd(tools: &[&str]) -> Command {
        Command { id: "pnpm:dev".into(), line: "x".into(), tier: Tier::Primary, tools: tools.iter().map(|t| t.to_string()).collect(), from_install: vec![], setup: false, ports: vec![] }
    }

    /// Node installs in `node_dirs`, and `path` as the PATH.
    fn toolbox(node_dirs: &[&Path], path: &[&Path]) -> Toolbox {
        let node_dirs: Vec<PathBuf> = node_dirs.iter().map(|d| d.to_path_buf()).collect();
        let vars = vec![("PATH".to_string(), path.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>().join(":"))];
        let config = crate::config::MachineConfig { node_dirs, path: vec![] };
        let machine = Machine::load(&config, None, &path_of(&vars));
        Toolbox { vars, machine }
    }

    #[test]
    fn a_program_from_a_toolchain_that_cant_serve_the_app_refuses() {
        let d = tmp("tools");
        let installs = d.join("node");
        node(&installs, "20.11.1", &["npm", "pnpm"]);
        node(&installs, "22.1.0", &["npm"]);
        let bin = d.join("bin");
        exe(&bin.join("go"));
        let tb = toolbox(&[&installs], &[&bin]);
        assert_eq!(tb.missing(&app(Some("20")), &cmd(&["pnpm", "node"])), None);
        assert_eq!(tb.missing(&app(Some("22")), &cmd(&["node", "pnpm"])).as_deref(), Some("pnpm not found"), "pnpm only under 20");
        assert_eq!(tb.missing(&app(Some("19")), &cmd(&["pnpm"])).as_deref(), Some(".nvmrc wants node 19, which isn't installed"));
        assert_eq!(tb.missing(&app(Some("19")), &cmd(&["go"])), None, "other programs don't need the Node");
        assert_eq!(tb.missing(&app(None), &cmd(&["go", "docker"])).as_deref(), Some("docker not found"));
        assert_eq!(tb.missing(&app(None), &cmd(&["node"])).as_deref(), Some("node not found"), "no Node asked, none on the PATH");
        exe(&bin.join("pnpm"));
        assert_eq!(tb.missing(&app(Some("22")), &cmd(&["pnpm"])), None, "pnpm on the base PATH");
        assert_eq!(tb.missing(&app(None), &cmd(&[])), None);
        // Programs are looked up on the run's PATH: the app's Node folder counts for all.
        exe(&installs.join("v20.11.1/bin/tsx"));
        assert_eq!(tb.missing(&app(Some("20")), &cmd(&["tsx"])), None, "found in the app's Node folder");
        assert_eq!(tb.missing(&app(None), &cmd(&["tsx"])).as_deref(), Some("tsx not found"));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_commands_toolchain_says_whether_the_project_is_installed() {
        let d = tmp("installed");
        let bin = d.join("bin");
        for p in ["npm", "node", "tsc"] {
            exe(&bin.join(p));
        }
        let tb = toolbox(&[], &[&bin]);
        let with = |from_install: &[&str]| Command { id: "npm:dev".into(), from_install: from_install.iter().map(|d| d.to_string()).collect(), ..cmd(&["npm", "node"]) };
        assert_eq!(tb.missing(&app(None), &with(&["vite"])).as_deref(), Some("vite isn't installed: run npm install"));
        assert_eq!(tb.missing(&app(None), &with(&["tsc"])), None, "on the PATH (installed globally)");
        assert_eq!(tb.missing(&app(None), &Command { tools: vec!["pnpm".into()], ..with(&["vite"]) }).as_deref(), Some("pnpm not found"), "programs first");
        let config_only = Command { id: "sh:x".into(), from_install: vec!["vite".into()], ..cmd(&["npm"]) };
        assert_eq!(tb.missing(&app(None), &config_only), None, "no toolchain owns it: the generic check only");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_apps_node_comes_first_on_the_path() {
        let d = tmp("path");
        node(&d, "20.11.1", &[]);
        let tb = toolbox(&[&d], &[Path::new("/a"), Path::new("/b")]);
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
