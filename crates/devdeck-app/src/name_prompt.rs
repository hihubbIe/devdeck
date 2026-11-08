//! The modal that asks for a name (a layout's, a group's): a title, a text field, and
//! what the caller does with the name. A refusal (empty name, name taken) shows in the
//! modal; Enter submits, Esc cancels, focus goes back where it was.

use devdeck_ui::overlay::{ModalClose, ModalNext, ModalPrev};
use devdeck_ui::{InputEvent, ModalFocus, TextInput, label, modal_frame, pop_modal, push_modal, theme};
use gpui::{
    App, AppContext, Context, Entity, Focusable, InteractiveElement, IntoElement, ParentElement, Render, SharedString, Styled, Subscription,
    WeakFocusHandle, Window, div,
};
use std::rc::Rc;

/// What a prompt does with the name; `Err` stays in the modal.
pub type Submit = Rc<dyn Fn(&str, &mut App) -> Result<(), String>>;

/// Opens a prompt titled `title`, its field holding `initial`. `selector` names the
/// field for tests and snapshots.
pub fn open(title: impl Into<SharedString>, initial: &str, selector: &'static str, submit: Submit, window: &mut Window, cx: &mut App) -> Entity<NamePrompt> {
    let restore = window.focused(cx).map(|f| f.downgrade());
    let title = title.into();
    let modal = cx.new(|cx| NamePrompt::new(title, initial, selector, submit, restore, window, cx));
    push_modal(modal.clone().into(), window, cx);
    let f = modal.read(cx).input.read(cx).focus_handle(cx);
    f.focus(window, cx);
    modal
}

pub struct NamePrompt {
    title: SharedString,
    selector: &'static str,
    submit: Submit,
    pub input: Entity<TextInput>,
    pub error: Option<SharedString>,
    focus: ModalFocus,
    restore: Option<WeakFocusHandle>,
    _sub: Subscription,
}

impl NamePrompt {
    fn new(
        title: SharedString,
        initial: &str,
        selector: &'static str,
        submit: Submit,
        restore: Option<WeakFocusHandle>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| {
            let mut i = TextInput::new("name", cx);
            if !initial.is_empty() {
                i.set_text(initial, cx);
            }
            i
        });
        let sub = cx.subscribe_in(&input, window, |this, _, e: &InputEvent, window, cx| match e {
            InputEvent::Submit(_) => this.submit(window, cx),
            InputEvent::Changed(_) => {
                this.error = None;
                cx.notify();
            }
            InputEvent::Cancel => {}
        });
        let focus = ModalFocus(vec![input.read(cx).focus_handle(cx)]);
        Self { title, selector, submit, input, error: None, focus, restore, _sub: sub }
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        pop_modal(window, cx);
        if let Some(h) = self.restore.as_ref().and_then(WeakFocusHandle::upgrade) {
            h.focus(window, cx);
        }
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.input.read(cx).text().to_string();
        let submit = self.submit.clone();
        match submit(&name, cx) {
            Ok(()) => self.close(window, cx),
            Err(e) => {
                self.error = Some(e.into());
                cx.notify();
            }
        }
    }
}

impl Render for NamePrompt {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let selector = self.selector;
        let content = div()
            .key_context("Modal")
            .flex()
            .flex_col()
            .gap_2()
            .on_action(cx.listener(|this, _: &ModalNext, window, cx| this.focus.cycle(1, window, cx)))
            .on_action(cx.listener(|this, _: &ModalPrev, window, cx| this.focus.cycle(-1, window, cx)))
            .on_action(cx.listener(|this, _: &ModalClose, window, cx| this.close(window, cx)))
            .child(div().text_color(t.hi).child(self.title.clone()))
            .child(div().w(t.size(360.)).debug_selector(move || selector.into()).child(self.input.clone()))
            .children(self.error.clone().map(|e| label(e).text_color(t.err)));
        modal_frame(content.into_any_element(), window, &t)
    }
}
