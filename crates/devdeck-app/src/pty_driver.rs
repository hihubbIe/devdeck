//! Real runs (R6.7): the [`RunDriver`] for config workspaces, on `core::process`.
//!
//! - A run's environment is the toolbox's variables, with PATH replaced by
//!   `Toolbox::path_for(app)` (the app's Node first), then the launch's own variables
//!   (the env binding), then `TERM=xterm-256color` and `COLORTERM=truecolor`.
//! - It runs `launch.line` in `launch.app.dir` at `launch.size`, with
//!   `StopKind::of(&launch.cmd)`, recorded in the [`GroupLog`] when there is one.
//! - Output and the exit reach the runner as `Runner::output` / `Runner::exited`, tagged
//!   with the launch's `RunId`; reading the process's channel happens on a task, and
//!   output is coalesced so a fast process costs one runner update per frame or so.
//! - The returned control writes input, resizes and starts the stop sequence; dropping
//!   it (the run ended or was replaced) drops the `Process`, which kills what's left.
//! - Without a toolbox yet (still loading), a start is refused by the runner before it
//!   reaches the driver; the driver can assume one.

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
}

impl RunDriver for PtyDriver {
    fn spawn(&self, launch: Launch, cx: &mut Context<Runner>) -> Result<Started, String> {
        // The runner refuses starts until the toolbox is there; this is a backstop.
        let toolbox = self.env.read(cx).toolbox().ok_or("the environment is still loading")?;
        let spawn = Spawn {
            vars: run_vars(toolbox, &launch),
            line: launch.line,
            dir: launch.app.dir,
            size: launch.size,
            stop: StopKind::of(&launch.cmd),
        };
        let (process, rx) = Process::spawn(spawn, self.log.clone())?;
        let id = launch.id;
        let task = cx.spawn(async move |this, cx| report(id, rx, this, cx).await);
        Ok(Started { control: Box::new(PtyControl(process)), task })
    }
}

/// A run's whole environment: the toolbox's variables, the app's PATH, the launch's own
/// variables, then the terminal's. A later value replaces an earlier one of the same name.
fn run_vars(toolbox: &Toolbox, launch: &Launch) -> Vec<(String, String)> {
    let path: Vec<String> = toolbox.path_for(&launch.app).iter().map(|p| p.to_string_lossy().into_owned()).collect();
    let terminal = [("TERM", "xterm-256color"), ("COLORTERM", "truecolor")].map(|(k, v)| (k.to_string(), v.to_string()));
    let own = std::iter::once(("PATH".to_string(), path.join(":"))).chain(launch.vars.iter().cloned()).chain(terminal);
    let mut vars = toolbox.vars.clone();
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
