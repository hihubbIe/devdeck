//! Dangerous actions (R5.6): what is hard to undo runs only once confirmed in its modal.
//!
//! A dangerous action takes a [`Confirmed`], and only a [`DangerModal`] hands one out
//! ([`confirmed`]): calling the action without its modal doesn't compile. The modals that
//! may are listed here and nowhere else (the trait is sealed). Each opens with Cancel
//! focused ([`show`]) and acts on Enter only from its action button, so a stray Enter
//! cancels.
//!
//! Dangerous: a deploy or a redeploy (`Cluster::deploy`, `Cluster::redeploy`), a git
//! replace push or an abort (`Op::dangerous`, `Repo::run_confirmed`; checked when it
//! runs), stopping a process devdeck didn't start (`port_scan::stop_process`), deleting a
//! note (`Notes::delete`). `clippy.toml` bans calling what lies under them (git, the
//! signal, the cluster driver's writes) and [`for_tests`] anywhere else.

use devdeck_ui::push_modal;
use gpui::{App, Entity, FocusHandle, Render, Window};

/// Proof that the user confirmed a dangerous action in its modal.
#[must_use = "a dangerous action takes it"]
pub struct Confirmed(());

mod sealed {
    pub trait Sealed {}
}

impl sealed::Sealed for crate::views::service_view::ConfirmDeploy {}
impl sealed::Sealed for crate::views::git_view::ConfirmModal {}
impl sealed::Sealed for crate::port_scan::StopProcess {}
impl sealed::Sealed for crate::views::note_view::DeleteNote {}

/// A modal confirming a dangerous action.
pub trait DangerModal: sealed::Sealed + Render {
    /// Its Cancel button, focused when it opens.
    fn cancel(&self) -> &FocusHandle;
}

/// The token, from the modal the user confirmed in.
pub fn confirmed<M: DangerModal>(_modal: &M) -> Confirmed {
    Confirmed(())
}

/// Opens `modal` with Cancel focused.
pub fn show<M: DangerModal>(modal: Entity<M>, window: &mut Window, cx: &mut App) {
    push_modal(modal.clone().into(), window, cx);
    let cancel = modal.read(cx).cancel().clone();
    cancel.focus(window, cx);
}

/// A token without a modal: tests and the snapshot tool only (`clippy.toml` bans it
/// anywhere else).
#[doc(hidden)]
pub fn for_tests() -> Confirmed {
    Confirmed(())
}
