//! Component gallery (`demo/gallery`): every shared component in its states, for the
//! visual check and the manual test. Not part of the product.

use devdeck_ui::Scrolls;
use devdeck_ui::overlay::{ModalClose, ModalNext, ModalPrev};
use devdeck_ui::{
    InputEvent, MenuItem, ModalFocus, TextInput, ToastKind, button, label, modal_frame, open_menu, pop_modal, push_modal,
    section, theme, toasts,
};
use gpui::{
    App, AppContext, Context, Entity, FocusHandle, Focusable, IntoElement, ParentElement, Render, SharedString, Styled,
    Subscription, WeakFocusHandle, Window, div, prelude::*,
};

/// Opens the demo modal in `window` (gallery button, snapshots).
pub fn open_demo_modal(window: &mut Window, cx: &mut App) {
    let restore = window.focused(cx).map(|f| f.downgrade());
    let modal = cx.new(|cx| DemoModal::new(restore, cx));
    push_modal(modal.clone().into(), window, cx);
    let first = modal.read(cx).focus.0[0].clone();
    first.focus(window, cx);
}

pub struct Gallery {
    pub empty: Entity<TextInput>,
    pub filled: Entity<TextInput>,
    last_event: SharedString,
    _subs: Vec<Subscription>,
}

impl Gallery {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let empty = cx.new(|cx| TextInput::new("filter services or tags", cx));
        let filled = cx.new(|cx| {
            let mut i = TextInput::new("", cx);
            i.set_text("europe-west1-docker.pkg.dev/nomadic-ocean-281413/registry06/entityservice:lan-160-a-very-long-branch-name", cx);
            i
        });
        let subs = [&empty, &filled]
            .into_iter()
            .map(|input| {
                cx.subscribe(input, |this, _, e: &InputEvent, cx| {
                    this.last_event = format!("{e:?}").into();
                    cx.notify();
                })
            })
            .collect();
        Self { empty, filled, last_event: "type something".into(), _subs: subs }
    }
}

pub fn demo_menu_items() -> Vec<MenuItem> {
    vec![
        MenuItem::new("Inspect", |_, _| {}),
        MenuItem::new("Pin logs to the right", |_, _| {}).hint("secondary text"),
        MenuItem::separator(),
        MenuItem::new("Restart", |_, _| {}).disabled("hades is protected"),
        MenuItem::new("Follow output", |_, _| {}).checked(true),
        MenuItem::new("Show a toast", |_, cx| {
            toasts(cx).update(cx, |t, cx| t.push(ToastKind::Ok, "Picked from the menu.", vec![], cx));
        }),
    ]
}

impl Render for Gallery {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let w = t.size(420.);
        div()
            .id("gallery")
            .size_full()
            .scroll_y()
            .p_3()
            .flex()
            .flex_col()
            .gap_1()
            .child(section("Text input", &t))
            .child(div().w(w).debug_selector(|| "empty-input".into()).child(self.empty.clone()))
            .child(div().w(w).child(self.filled.clone()))
            .child(label(self.last_event.clone()).text_color(t.mute))
            .child(section("Menu", &t))
            .child(
                div().flex().gap_2().child(button("open-menu", "open a menu", &t).on_click(|e, window, cx| {
                    open_menu(e.position(), demo_menu_items(), window, cx);
                })),
            )
            .child(section("Modal", &t))
            .child(div().flex().gap_2().child(button("open-modal", "open a modal", &t).debug_selector(|| "open-modal".into()).on_click(|_, window, cx| open_demo_modal(window, cx))))
            .child(section("Toasts", &t))
            .child(div().flex().gap_2().children([
                (ToastKind::Info, "info", "Layout: Geo work."),
                (ToastKind::Ok, "ok", "dev35 › entityservice now runs lan-160."),
                (ToastKind::Warn, "warn", "Port 5800 is held by node (pid 7712)."),
                (ToastKind::Err, "err", "boardservice · start:watch exited 1."),
            ]
            .into_iter()
            .enumerate()
            .map(|(i, (kind, name, text))| {
                button(("toast", i), name, &t).on_click(move |_, _, cx| {
                    let actions: Vec<(SharedString, devdeck_ui::overlay::MenuAction)> = if kind == ToastKind::Err {
                        vec![("View output".into(), std::rc::Rc::new(|_, _| {}))]
                    } else {
                        vec![]
                    };
                    toasts(cx).update(cx, |s, cx| s.push(kind, text, actions, cx));
                })
            })))
    }
}

/// A modal with an input and two buttons, to show focus trapping.
pub struct DemoModal {
    pub focus: ModalFocus,
    input: Entity<TextInput>,
    restore: Option<WeakFocusHandle>,
}

impl DemoModal {
    fn new(restore: Option<WeakFocusHandle>, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| TextInput::new("layout name", cx));
        let handles = vec![input.read(cx).focus_handle(cx), cx.focus_handle(), cx.focus_handle()];
        Self { focus: ModalFocus(handles), input, restore }
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        pop_modal(window, cx);
        if let Some(h) = self.restore.as_ref().and_then(WeakFocusHandle::upgrade) {
            h.focus(window, cx);
        }
    }
}

fn focus_button(id: &'static str, text: &'static str, handle: &FocusHandle, t: &devdeck_ui::Theme) -> gpui::Stateful<gpui::Div> {
    let ring = t.focus;
    button(id, text, t).border_1().border_color(t.line).track_focus(handle).focus(move |s| s.border_color(ring))
}

impl Render for DemoModal {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let content = div()
            .key_context("Modal")
            .flex()
            .flex_col()
            .gap_2()
            .on_action(cx.listener(|this, _: &ModalNext, window, cx| this.focus.cycle(1, window, cx)))
            .on_action(cx.listener(|this, _: &ModalPrev, window, cx| this.focus.cycle(-1, window, cx)))
            .on_action(cx.listener(|this, _: &ModalClose, window, cx| this.close(window, cx)))
            .child(div().text_color(t.hi).child("Save current layout"))
            .child(self.input.clone())
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(focus_button("modal-cancel", "Cancel", &self.focus.0[1], &t).on_click(cx.listener(|this, _, w, cx| this.close(w, cx))))
                    .child(focus_button("modal-save", "Save", &self.focus.0[2], &t).on_click(cx.listener(|this, _, w, cx| this.close(w, cx)))),
            );
        modal_frame(content.into_any_element(), window, &t)
    }
}
