//! Real runs (R6.7): the [`RunDriver`] for config workspaces, on `core::process`.
//!
//! - A run's environment is the toolbox's variables, with PATH replaced by
//!   `Toolbox::path_for(app)` (the app's Node first), then the launch's own variables
//!   (the env binding), then `TERM=xterm-256color` and `COLORTERM=truecolor`.
//! - It runs `launch.line` in `launch.app.dir` at `launch.size`, with
//!   `StopKind::of(&launch.cmd)`, in a scope of its own (a terminal's shell: none),
//!   recorded in the [`GroupLog`] when there is one.
//! - Output and the exit reach the runner as `Runner::output` / `Runner::exited`, tagged
//!   with the launch's `RunId`; reading the process's channel happens on a task, and
//!   output is coalesced so a fast process costs one runner update per frame or so.
//! - The returned control writes input, resizes and starts the stop sequence; dropping
//!   it (the run ended or was replaced) drops the `Process`, which kills what's left of
//!   its group (and its scope, unless it ended on its own).
//! - Without a toolbox yet (still loading), a start is refused by the runner before it
//!   reaches the driver; the driver can assume one.
//! - A terminal's shell is told where the control API is (`DEVDECK_SOCKET`) and which
//!   terminal it is (`DEVDECK_TERMINAL`, `<workspace>/<n>`), while the API is on; app runs
//!   are never told (R13.4). Neither grants anything: devdeck finds callers from their
//!   processes.

use crate::environment::Environment;
use crate::runner::{Launch, RunControl, RunDriver, RunId, Runner, Started};
use async_channel::{Receiver, TryRecvError};
use devdeck_core::environment::Toolbox;
use devdeck_core::process::{GroupLog, Process, ProcessEvent, Spawn, StopKind};
use devdeck_core::runs::Exit;
use devdeck_core::terminal::TermSize;
use gpui::{AsyncApp, Context, Entity, WeakEntity};
use std::time::Duration;

/// After handing output to the runner, the driver waits this long before the next batch,
/// so what arrives meanwhile goes in one update (tests advance their clock by it).
pub const FRAME: Duration = Duration::from_millis(16);

pub struct PtyDriver {
    pub env: Entity<Environment>,
    /// The workspace's id (a shell's `DEVDECK_TERMINAL`).
    pub workspace: String,
    /// Where started groups are recorded (none in tests).
    pub log: Option<GroupLog>,
}

struct PtyControl(Process);

impl RunControl for PtyControl {
    fn write(&self, bytes: &[u8]) {
        self.0.write(bytes);
    }

    fn resize(&self, size: TermSize) {
        self.0.resize(size);
    }

    fn stop(&self) {
        self.0.stop();
    }

    fn pid(&self) -> Option<u32> {
        Some(self.0.pid())
    }

    fn foreground_program(&self) -> Option<String> {
        self.0.foreground_program()
    }
}

impl RunDriver for PtyDriver {
    fn spawn(&self, launch: Launch, cx: &mut Context<Runner>) -> Result<Started, String> {
        // The runner refuses starts until the toolbox is there; this is a backstop.
        let toolbox = self.env.read(cx).toolbox().ok_or("the environment is still loading")?;
        let spawn = Spawn {
            vars: run_vars(toolbox, &launch, crate::api::socket_path(cx).as_deref(), &self.workspace),
            line: launch.line,
            dir: launch.app.dir,
            size: launch.size,
            stop: StopKind::of(&launch.cmd),
            // A terminal's shell: what's detached from it on purpose stays (R6.12).
            scope: launch.cmd.source() != "shell",
            label: format!("{} · {}", launch.app.name, launch.cmd.name()),
        };
        let (process, rx) = Process::spawn(spawn, self.log.clone())?;
        let id = launch.id;
        let task = cx.spawn(async move |this, cx| report(id, rx, this, cx).await);
        Ok(Started { control: Box::new(PtyControl(process)), task })
    }
}

/// Variables that describe the terminal devdeck itself was started from.
const LEFT_BEHIND: [&str; 5] = ["TMUX", "TMUX_PANE", "VTE_VERSION", "TERM_PROGRAM", "TERM_PROGRAM_VERSION"];

/// A run's whole environment: the toolbox's variables, the app's PATH, the launch's own
/// variables, then the terminal's (and a shell's, the API's `socket` and its terminal in
/// `workspace`). A later value replaces an earlier one of the same name.
fn run_vars(toolbox: &Toolbox, launch: &Launch, socket: Option<&std::path::Path>, workspace: &str) -> Vec<(String, String)> {
    let path: Vec<String> = toolbox.path_for(&launch.app).iter().map(|p| p.to_string_lossy().into_owned()).collect();
    let terminal = [("TERM", "xterm-256color"), ("COLORTERM", "truecolor")].map(|(k, v)| (k.to_string(), v.to_string()));
    let shell = launch.cmd.source() == "shell";
    let api = socket.filter(|_| shell).and_then(|s| {
        let n = launch.id.key.terminal_number()?;
        Some([(devdeck_api::SOCKET_ENV.to_string(), s.to_string_lossy().into_owned()), (devdeck_api::TERMINAL_ENV.to_string(), format!("{workspace}/{n}"))])
    });
    let own = std::iter::once(("PATH".to_string(), path.join(":"))).chain(launch.vars.iter().cloned()).chain(terminal).chain(api.into_iter().flatten());
    let mut vars = toolbox.vars.clone();
    // Only what devdeck says here: never a parent devdeck's API.
    vars.retain(|(n, _)| n != devdeck_api::SOCKET_ENV && n != devdeck_api::TERMINAL_ENV);
    // A terminal's shell isn't inside the terminal devdeck was started from.
    if shell {
        vars.retain(|(n, _)| !LEFT_BEHIND.contains(&n.as_str()));
    }
    for (name, value) in own {
        match vars.iter_mut().find(|(n, _)| *n == name) {
            Some(var) => var.1 = value,
            None => vars.push((name, value)),
        }
    }
    vars
}

/// Hands a run's output and exit to the runner: waits for an event, takes every event
/// already queued, and makes one `output` call of them all. After a batch it waits a
/// [`FRAME`], so a fast process costs about one update per frame, while output after a
/// quiet spell shows at once.
async fn report(id: RunId, rx: Receiver<ProcessEvent>, this: WeakEntity<Runner>, cx: &mut AsyncApp) {
    loop {
        let mut out = Vec::new();
        // A channel closed without an exit (its thread died): the status can't be known.
        let mut exit = match rx.recv().await {
            Ok(event) => take(event, &mut out),
            Err(_) => Some(Exit::Code(-1)),
        };
        while exit.is_none() {
            match rx.try_recv() {
                Ok(event) => exit = take(event, &mut out),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Closed) => exit = Some(Exit::Code(-1)),
            }
        }
        if !out.is_empty() && this.update(cx, |r, cx| r.output(&id, &out, cx)).is_err() {
            return;
        }
        if let Some(exit) = exit {
            let _ = this.update(cx, |r, cx| r.exited(&id, exit, cx));
            return;
        }
        cx.background_executor().timer(FRAME).await;
    }
}

/// Adds an event's output to `out`; the exit, when it is one.
fn take(event: ProcessEvent, out: &mut Vec<u8>) -> Option<Exit> {
    match event {
        ProcessEvent::Output(bytes) if out.is_empty() => *out = bytes,
        ProcessEvent::Output(bytes) => out.extend_from_slice(&bytes),
        ProcessEvent::Exited(exit) => return Some(exit),
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::RunKey;
    use devdeck_core::apps::{App, Command, EnvApply, Tier};

    fn launch(key: RunKey, source: &str) -> Launch {
        let app = App {
            id: "front".into(),
            name: "front".into(),
            folder: "~/front".into(),
            dir: "/tmp".into(),
            kind: "pnpm".into(),
            library: false,
            facts: Default::default(),
            commands: Vec::new(),
            env: EnvApply::None,
            urls: Vec::new(),
            images: Vec::new(),
        };
        let cmd = Command { id: format!("{source}:x"), line: "x".into(), tier: Tier::Primary, tools: Vec::new(), from_install: Vec::new(), setup: false, ports: Vec::new() };
        Launch { id: RunId { key, n: 1 }, app, cmd, line: "x".into(), vars: Vec::new(), env: None, size: TermSize { cols: 80, rows: 24 } }
    }

    fn get<'a>(vars: &'a [(String, String)], name: &str) -> Option<&'a str> {
        vars.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }

    #[test]
    fn only_shells_are_told_where_the_api_is() {
        // A parent devdeck's variables, as a login shell started from one would pass on.
        let toolbox = Toolbox { vars: vec![("DEVDECK_SOCKET".into(), "/parent.sock".into()), ("DEVDECK_TERMINAL".into(), "old/1".into())], ..Default::default() };
        let sock = std::path::Path::new("/run/user/1000/devdeck/main.sock");
        let shell = run_vars(&toolbox, &launch(RunKey::terminal(3), "shell"), Some(sock), "work");
        assert_eq!(get(&shell, "DEVDECK_SOCKET"), Some("/run/user/1000/devdeck/main.sock"));
        assert_eq!(get(&shell, "DEVDECK_TERMINAL"), Some("work/3"));
        assert_eq!(shell.iter().filter(|(n, _)| n == "DEVDECK_SOCKET").count(), 1);
        let app = run_vars(&toolbox, &launch(RunKey::new("front", "pnpm:x"), "pnpm"), Some(sock), "work");
        assert_eq!((get(&app, "DEVDECK_SOCKET"), get(&app, "DEVDECK_TERMINAL")), (None, None), "app runs are never told");
        let off = run_vars(&toolbox, &launch(RunKey::terminal(3), "shell"), None, "work");
        assert_eq!(get(&off, "DEVDECK_SOCKET"), None, "the API is off");
    }
}
