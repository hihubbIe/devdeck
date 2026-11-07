//! The machine's environment for runs (R1.4, R6.5, R6.8): the login shell's variables
//! and the Node installs, found once at start off the UI thread and shared by every
//! workspace. Until it's there, apps say "checking tools…"; if the login shell can't be
//! asked, devdeck's own environment is used and that's reported.

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
    /// Machine-wide problems (config files, the login shell), listed with each
    /// workspace's own in the problems view.
    pub problems: Vec<String>,
}

impl Environment {
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
    let e = cx.new(|_| Environment { state: EnvState::Loading, problems: Vec::new() });
    cx.set_global(EnvHandle(e.clone()));
    e
}

/// Asks the login shell in the background (it can take a moment: rc files run).
pub fn load(machine: MachineConfig, cx: &mut App) -> Entity<Environment> {
    let env = environment(cx);
    let target = env.clone();
    cx.spawn(async move |cx| {
        let found = cx
            .background_executor()
            .spawn(async move { Toolbox::load(&machine).map_err(|why| (why, Toolbox::current(&machine))) })
            .await;
        target.update(cx, |e, cx| {
            match found {
                Ok(tb) => e.state = EnvState::Ready(tb),
                Err((why, own)) => {
                    let text = format!("{why}; runs use devdeck's own environment");
                    devdeck_ui::toasts(cx).update(cx, |t, cx| t.push(devdeck_ui::ToastKind::Warn, text.clone(), vec![], cx));
                    e.problems.push(text);
                    e.state = EnvState::Ready(own);
                }
            }
            cx.notify();
        });
    })
    .detach();
    env
}
