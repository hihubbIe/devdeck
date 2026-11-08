//! The machine's environment for runs (R1.4, R6.5, R6.8): the login shell's variables
//! and the Node installs, found once at start off the UI thread and shared by every
//! workspace. Until it's there, apps say "checking tools…"; if the login shell can't be
//! asked, devdeck's own environment is used and that's reported. Loading again (a changed
//! `machine.toml`) replaces it; when loads overlap, the last one asked wins.

use devdeck_core::apps::Tools;
use devdeck_core::config::MachineConfig;
use devdeck_core::environment::Toolbox;
use gpui::{App, AppContext, Entity, Global};

pub enum EnvState {
    Loading,
    Ready(Toolbox),
}

pub struct Environment {
    pub state: EnvState,
    /// The config files' problems, as of their last reading.
    pub config_problems: Vec<String>,
    /// The login shell's, as of the last load.
    pub shell_problems: Vec<String>,
    /// Loads asked so far: a load's result is kept only if it's the last one asked.
    loads: u64,
}

impl Environment {
    pub fn new(state: EnvState) -> Self {
        Self { state, config_problems: Vec::new(), shell_problems: Vec::new(), loads: 0 }
    }

    /// Machine-wide problems, listed with each workspace's own in the problems view.
    pub fn problems(&self) -> Vec<String> {
        self.config_problems.iter().chain(&self.shell_problems).cloned().collect()
    }

    /// The tools runs can use now.
    pub fn tools(&self) -> Tools {
        match &self.state {
            EnvState::Loading => Tools::pending(),
            EnvState::Ready(tb) => Tools { toolbox: Some(tb.clone()), ..Tools::default() },
        }
    }

    pub fn toolbox(&self) -> Option<&Toolbox> {
        match &self.state {
            EnvState::Ready(tb) => Some(tb),
            EnvState::Loading => None,
        }
    }
}

struct EnvHandle(Entity<Environment>);
impl Global for EnvHandle {}

/// The app's environment (loading until [`load`] finished).
pub fn environment(cx: &mut App) -> Entity<Environment> {
    if let Some(h) = cx.try_global::<EnvHandle>() {
        return h.0.clone();
    }
    let e = cx.new(|_| Environment::new(EnvState::Loading));
    cx.set_global(EnvHandle(e.clone()));
    e
}

/// Asks the login shell in the background (it can take a moment: rc files run).
pub fn load(machine: MachineConfig, cx: &mut App) -> Entity<Environment> {
    let env = environment(cx);
    let n = env.update(cx, |e, _| {
        e.loads += 1;
        e.loads
    });
    let target = env.clone();
    cx.spawn(async move |cx| {
        let found = cx
            .background_executor()
            .spawn(async move { Toolbox::load(&machine).map_err(|why| (why, Toolbox::current(&machine))) })
            .await;
        target.update(cx, |e, cx| {
            if e.loads != n {
                return;
            }
            match found {
                Ok(tb) => {
                    e.state = EnvState::Ready(tb);
                    e.shell_problems.clear();
                }
                Err((why, own)) => {
                    let text = format!("{why}; runs use Kugelblitz's own environment");
                    devdeck_ui::toasts(cx).update(cx, |t, cx| t.push(devdeck_ui::ToastKind::Warn, text.clone(), vec![], cx));
                    e.shell_problems = vec![text];
                    e.state = EnvState::Ready(own);
                }
            }
            cx.notify();
        });
    })
    .detach();
    env
}
