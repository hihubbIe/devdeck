//! Real runs (R6.7): the [`RunDriver`] for config workspaces, on `core::process`.
//! Builder R1 of M2 step 3 implements this file; the interface is fixed here.
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
use crate::runner::{Launch, RunDriver, Runner, Started};
use devdeck_core::process::GroupLog;
use gpui::{Context, Entity};

pub struct PtyDriver {
    pub env: Entity<Environment>,
    /// Where started groups are recorded (none in tests).
    pub log: Option<GroupLog>,
}

impl RunDriver for PtyDriver {
    fn spawn(&self, launch: Launch, cx: &mut Context<Runner>) -> Result<Started, String> {
        let _ = (launch, cx, &self.env, &self.log);
        Err("real runs aren't wired yet (M2 step 3, builder R1)".into())
    }
}
