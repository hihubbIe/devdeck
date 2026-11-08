//! Edit mode (R5.6): potentially destructive actions run only while it's on — a deploy
//! (set image, undo, restart, replicas), a git replace push or abort, stopping a process
//! devdeck didn't start, a shell in a pod; k9s opens read only otherwise. A title bar
//! button turns it on after a modal saying what it allows, and off at once. Off at every
//! start; one switch for the whole app.
//!
//! The gated places check [`refusal`] themselves, below the UI, so no button, menu, key or
//! modal left open can get around it.

use devdeck_ui::{ModalFocus, Theme, button, label, modal_frame, pop_modal, push_modal, theme};
use gpui::{App, AppContext, Context, Entity, FocusHandle, Global, IntoElement, ParentElement, Render, Styled, WeakFocusHandle, Window, div, prelude::*};

/// Why an action is refused while edit mode is off.
pub const OFF: &str = "turn on edit mode first (the warning button in the title bar)";

/// What it allows, said before turning it on.
pub const ALLOWS: [&str; 5] = [
    "Deploys: setting an image, undoing a rollout, restarts and replicas.",
    "Git: replacing a rewritten branch on the remote, aborting a rebase, merge, cherry-pick, revert or am.",
    "Stopping a process devdeck didn't start (one holding a port).",
    "A shell in a pod, and k9s with its write actions.",
    "Protected envs still ask for their name; main and master are never pushed.",
];

pub struct EditMode {
    pub on: bool,
}

struct Global_(Entity<EditMode>);

impl Global for Global_ {}

/// The app's edit mode (off until turned on).
pub fn edit_mode(cx: &mut App) -> Entity<EditMode> {
    if let Some(g) = cx.try_global::<Global_>() {
        return g.0.clone();
    }
    let e = cx.new(|_| EditMode { on: false });
    cx.set_global(Global_(e.clone()));
    e
}

pub fn is_on(cx: &App) -> bool {
    cx.try_global::<Global_>().is_some_and(|g| g.0.read(cx).on)
}

/// Why a destructive action can't run now, if it can't.
pub fn refusal(cx: &App) -> Option<String> {
    (!is_on(cx)).then(|| OFF.to_string())
}

pub fn set(on: bool, cx: &mut App) {
    let e = edit_mode(cx);
    e.update(cx, |m, cx| {
        if m.on != on {
            m.on = on;
            cx.notify();
        }
    });
}

/// The title bar button: on, it turns edit mode off at once; off, it asks first.
pub fn toggle(window: &mut Window, cx: &mut App) {
    if is_on(cx) {
        return set(false, cx);
    }
    let restore = window.focused(cx).map(|f| f.downgrade());
    let modal = cx.new(|cx| TurnOn { focus: ModalFocus(vec![cx.focus_handle(), cx.focus_handle()]), restore });
    push_modal(modal.clone().into(), window, cx);
    let first = modal.read(cx).focus.0[0].clone();
    first.focus(window, cx);
}

/// "Turn on edit mode?" with what it allows; Enter turns it on, Esc cancels.
pub struct TurnOn {
    focus: ModalFocus,
    restore: Option<WeakFocusHandle>,
}

gpui::actions!(edit_mode, [ConfirmEditMode]);

impl TurnOn {
    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        pop_modal(window, cx);
        if let Some(h) = self.restore.as_ref().and_then(WeakFocusHandle::upgrade) {
            h.focus(window, cx);
        }
    }

    fn turn_on(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close(window, cx);
        set(true, cx);
    }

    /// Enter: on whichever button has focus (Cancel first, so a stray Enter can't turn it
    /// on).
    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.focus.0[0].is_focused(window) {
            return self.close(window, cx);
        }
        self.turn_on(window, cx);
    }
}

fn focus_button(id: &'static str, text: &'static str, handle: &FocusHandle, t: &Theme) -> gpui::Stateful<gpui::Div> {
    let ring = t.focus;
    button(id, text, t).border_1().border_color(t.line).track_focus(handle).focus(move |s| s.border_color(ring))
}

impl Render for TurnOn {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let content = div()
            .key_context("Modal EditModeModal")
            .flex()
            .flex_col()
            .gap_2()
            .max_w(t.size(680.))
            .on_action(cx.listener(|this, _: &devdeck_ui::overlay::ModalNext, window, cx| this.focus.cycle(1, window, cx)))
            .on_action(cx.listener(|this, _: &devdeck_ui::overlay::ModalPrev, window, cx| this.focus.cycle(-1, window, cx)))
            .on_action(cx.listener(|this, _: &devdeck_ui::overlay::ModalClose, window, cx| this.close(window, cx)))
            .on_action(cx.listener(|this, _: &ConfirmEditMode, window, cx| this.confirm(window, cx)))
            .child(div().text_color(t.hi).debug_selector(|| "edit-mode-title".into()).child("Turn on edit mode?"))
            .child(label("While it's on, devdeck can do things that are hard to undo:").text_color(t.mute))
            .children(ALLOWS.iter().map(|l| div().text_color(t.fg).child(format!("· {l}"))))
            .child(label("Turn it off with the same button when you're done; it's off at every start.").text_color(t.mute))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(focus_button("edit-mode-cancel", "Cancel", &self.focus.0[0], &t).on_click(cx.listener(|this, _, w, cx| this.close(w, cx))))
                    .child(
                        focus_button("edit-mode-confirm", "Turn on", &self.focus.0[1], &t)
                            .debug_selector(|| "edit-mode-confirm".into())
                            .on_click(cx.listener(|this, _, w, cx| this.turn_on(w, cx))),
                    ),
            );
        modal_frame(content.into_any_element(), window, &t)
    }
}
