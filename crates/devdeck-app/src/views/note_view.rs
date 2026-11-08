//! Note view (`note/<id>`, R12.3, R12.4): a note's Markdown source in the note editor, or
//! rendered; an icon in its header (or its tab's menu, the palette, ctrl+shift+e)
//! switches. It opens rendered when the note has text, as source when it's empty. The
//! text belongs to the workspace's [`Notes`]: typing goes there, which writes it. The tab
//! is named "app · name" and follows a rename; a deleted note closes its tabs.
//!
//! Also here: making, renaming and deleting a note (the app view's Notes section).

use super::{KeepsState, Live, ToggleRendered, ViewAction, ViewCx};
use crate::notes::{NoteDeleted, Notes};
use devdeck_ui::code::{Code, CodeEvent};
use devdeck_ui::overlay::{ModalClose, ModalNext, ModalPrev};
use devdeck_ui::{Icon, ModalFocus, Rendered, Scrolls, button, label, modal_frame, pop_modal, push_modal, theme};
use gpui::{
    App, AppContext, Context, Entity, FocusHandle, Focusable, InteractiveElement, IntoElement, ParentElement, Render, StatefulInteractiveElement, Styled,
    Subscription, WeakFocusHandle, Window, actions, div,
};

actions!(note_view, [ConfirmDeleteNote]);

pub struct NoteView {
    pub id: String,
    ctx: ViewCx,
    notes: Entity<Notes>,
    /// The editor, made on the first draw of the source (it needs a window).
    pub code: Option<Entity<Code>>,
    /// The rendered note, made when first shown.
    pub rendered_view: Option<Entity<Rendered>>,
    pub rendered: bool,
    focus: FocusHandle,
    /// Esc in the editor gives the keys back to the view: focus stays here then.
    leaving: bool,
    _subs: Vec<Subscription>,
}

impl NoteView {
    pub fn new(id: String, ctx: ViewCx, cx: &mut Context<Self>) -> Self {
        let notes = ctx.services.read(cx).notes.clone();
        let rendered = !notes.update(cx, |n, _| n.text(&id)).trim().is_empty();
        let mine = id.clone();
        let (deck, me) = (ctx.deck.clone(), ctx.view.clone());
        let keep = ctx.clone();
        let subs = vec![
            cx.observe(&notes, |_, _, cx| cx.notify()),
            cx.subscribe(&notes, move |_, _, e: &NoteDeleted, cx| {
                if e.0 == mine {
                    let me = me.clone();
                    let _ = deck.update(cx, |d, cx| d.apply(cx, |l| l.close_view(&me)));
                }
            }),
        ];
        Self { id, ctx: keep, notes, code: None, rendered_view: None, rendered, focus: cx.focus_handle(), leaving: false, _subs: subs }
    }

    /// The editor, made now if it isn't yet, holding the note's text. Focus on the view
    /// goes on to it while the source shows.
    fn editor(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<Code> {
        if let Some(c) = &self.code {
            return c.clone();
        }
        let text = self.notes.update(cx, |n, _| n.text(&self.id));
        let code = cx.new(|cx| Code::note(window, cx));
        // Right-click and the menu key: Cut, Copy, Paste, Share with… (R5.7).
        let menu = crate::context_menu::text_menu(&self.ctx);
        code.update(cx, |c, cx| {
            c.set_text(&text, "markdown", window, cx);
            c.set_menu(menu);
        });
        let (notes, id) = (self.notes.clone(), self.id.clone());
        self._subs.push(cx.subscribe(&code, move |_, code, _: &CodeEvent, cx| {
            let text = code.read(cx).text(cx).to_string();
            notes.update(cx, |n, cx| n.set_text(&id, text, cx));
        }));
        // Esc left the editor for the view; once the view loses the focus, coming back
        // to it goes to the text again.
        self._subs.push(cx.on_blur(&self.focus, window, |this, _, _| this.leaving = false));
        self.code = Some(code.clone());
        code
    }

    /// The rendered note, made now if it isn't yet, showing `text`.
    fn rendered_view(&mut self, text: &str, cx: &mut Context<Self>) -> Entity<Rendered> {
        match &self.rendered_view {
            Some(r) => {
                r.update(cx, |r, cx| r.set_text(text, cx));
                r.clone()
            }
            None => {
                let menu = crate::context_menu::text_menu(&self.ctx);
                let r = cx.new(|cx| {
                    let mut r = Rendered::new(text, cx);
                    r.set_menu(menu);
                    r
                });
                self.rendered_view = Some(r.clone());
                r
            }
        }
    }

    /// The menu key (R5.7): the menu for the selection, else the key goes on (the tab's
    /// menu).
    fn menu_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let opened = match (&self.code, &self.rendered_view) {
            (_, Some(r)) if self.rendered => Rendered::menu_key(r, window, cx),
            (Some(c), _) if !self.rendered => Code::menu_key(c, window, cx),
            _ => false,
        };
        if !opened {
            cx.propagate();
        }
    }

    pub fn toggle_rendered(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.rendered = !self.rendered;
        if !self.rendered {
            let code = self.editor(window, cx);
            let f = code.read(cx).focus_handle(cx);
            f.focus(window, cx);
        } else {
            self.focus.focus(window, cx);
        }
        cx.notify();
    }

    fn leave_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.leaving = true;
        self.focus.focus(window, cx);
        cx.notify();
    }
}

impl Live for NoteView {
    fn title(&self, cx: &App) -> Option<String> {
        let meta = self.notes.read(cx).meta(&self.id)?;
        let app = meta.app.rsplit('/').next().unwrap_or(&meta.app);
        Some(format!("{app} · {}", meta.name))
    }

    fn actions(&self, _cx: &App) -> Vec<ViewAction> {
        vec![ViewAction { id: super::file_view::TOGGLE_RENDERED, icon: Icon::Rendered, on: self.rendered }]
    }

    fn run(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if id == super::file_view::TOGGLE_RENDERED {
            self.toggle_rendered(window, cx);
        }
    }
}

impl KeepsState for NoteView {
    fn save(&self, _cx: &App) -> serde_json::Value {
        serde_json::json!({ "rendered": self.rendered })
    }

    fn restore(&mut self, state: &serde_json::Value, cx: &mut Context<Self>) {
        if let Some(r) = state.get("rendered").and_then(serde_json::Value::as_bool) {
            self.rendered = r;
        }
        cx.notify();
    }
}

impl Focusable for NoteView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for NoteView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let root = div()
            .size_full()
            .flex()
            .flex_col()
            .key_context("NoteView")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &ToggleRendered, window, cx| this.toggle_rendered(window, cx)))
            .on_action(cx.listener(|this, _: &devdeck_ui::code::EditorEscape, window, cx| this.leave_editor(window, cx)))
            .on_action(cx.listener(|this, _: &crate::actions::ContextMenu, window, cx| this.menu_key(window, cx)));
        // A file another program wrote, not as text: said, never edited (it would be
        // written over).
        if let Some(why) = self.notes.read(cx).unreadable(&self.id) {
            return root
                .justify_center()
                .px_3()
                .text_color(t.mute)
                .debug_selector(|| "note-unreadable".into())
                .child(div().w_full().min_w_0().flex().justify_center().child(label(format!("This note can't be read: {why}")).min_w_0()))
                .into_any_element();
        }
        if self.rendered {
            let text = self.notes.update(cx, |n, _| n.text(&self.id));
            let body = if text.trim().is_empty() {
                div().text_color(t.mute).child(label("Empty note. ctrl shift e to write.")).into_any_element()
            } else {
                self.rendered_view(&text, cx).into_any_element()
            };
            return root
                .child(div().id("note-rendered").debug_selector(|| "note-rendered".into()).flex_1().min_h_0().scroll_y().px_3().py_2().child(body))
                .into_any_element();
        }
        let code = self.editor(window, cx);
        // Focus given to the view (its pane focused, the note just made) goes on to the
        // text, unless Esc just left it.
        if self.focus.is_focused(window) && !self.leaving {
            let f = code.read(cx).focus_handle(cx);
            window.defer(cx, move |window, cx| f.focus(window, cx));
        }
        root.child(div().debug_selector(|| "note-source".into()).flex_1().min_h_0().pl_2().child(code)).into_any_element()
    }
}

/// Asks a name, then makes a note for `app` and opens it (as source: it's empty).
pub fn new_note(ctx: &ViewCx, app: &str, window: &mut Window, cx: &mut App) {
    let (notes, app, ctx) = (ctx.services.read(cx).notes.clone(), app.to_string(), ctx.clone());
    let submit: crate::name_prompt::Submit = std::rc::Rc::new(move |name, cx| {
        let id = notes.update(cx, |n, cx| n.create(&app, name, cx)).ok_or("the note wasn't made")?;
        ctx.open(devdeck_core::view_addr::ViewAddr::Note { id }, cx);
        Ok(())
    });
    crate::name_prompt::open("New note", "", "note-name", submit, window, cx);
}

/// Asks a new name for a note.
pub fn rename_note(notes: &Entity<Notes>, id: &str, window: &mut Window, cx: &mut App) {
    let Some(name) = notes.read(cx).meta(id).map(|m| m.name.clone()) else { return };
    let (notes, id) = (notes.clone(), id.to_string());
    let submit: crate::name_prompt::Submit = std::rc::Rc::new(move |name, cx| {
        if name.trim().is_empty() {
            return Err("A note needs a name.".into());
        }
        notes.update(cx, |n, cx| n.rename(&id, name, cx));
        Ok(())
    });
    crate::name_prompt::open(format!("Rename note {name}"), &name, "note-name", submit, window, cx);
}

/// Asks before deleting a note: "Delete note todo? Its text is gone for good."
pub fn delete_note(notes: &Entity<Notes>, id: &str, window: &mut Window, cx: &mut App) {
    let Some(name) = notes.read(cx).meta(id).map(|m| m.name.clone()) else { return };
    let restore = window.focused(cx).map(|f| f.downgrade());
    let modal = cx.new(|cx| DeleteNote { notes: notes.clone(), id: id.to_string(), name, focus: ModalFocus(vec![cx.focus_handle(), cx.focus_handle()]), restore });
    push_modal(modal.clone().into(), window, cx);
    let first = modal.read(cx).focus.0[0].clone();
    first.focus(window, cx);
}

pub struct DeleteNote {
    notes: Entity<Notes>,
    id: String,
    name: String,
    focus: ModalFocus,
    restore: Option<WeakFocusHandle>,
}

impl DeleteNote {
    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        pop_modal(window, cx);
        if let Some(h) = self.restore.as_ref().and_then(WeakFocusHandle::upgrade) {
            h.focus(window, cx);
        }
    }

    fn delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close(window, cx);
        let id = self.id.clone();
        self.notes.update(cx, |n, cx| n.delete(&id, cx));
    }

    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.focus.0[1].is_focused(window) {
            self.close(window, cx);
        } else {
            self.delete(window, cx);
        }
    }
}

impl Render for DeleteNote {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let ring = t.focus;
        let focus_button = |id: &'static str, text: &'static str, handle: &FocusHandle| {
            button(id, text, &t).border_1().border_color(t.line).track_focus(handle).focus(move |s| s.border_color(ring))
        };
        let content = div()
            .key_context("Modal DeleteNote")
            .flex()
            .flex_col()
            .gap_2()
            .max_w(t.size(560.))
            .on_action(cx.listener(|this, _: &ModalNext, window, cx| this.focus.cycle(1, window, cx)))
            .on_action(cx.listener(|this, _: &ModalPrev, window, cx| this.focus.cycle(-1, window, cx)))
            .on_action(cx.listener(|this, _: &ModalClose, window, cx| this.close(window, cx)))
            .on_action(cx.listener(|this, _: &ConfirmDeleteNote, window, cx| this.confirm(window, cx)))
            .child(div().text_color(t.hi).child(format!("Delete note {}?", self.name)))
            .child(label("Its text is gone for good.").text_color(t.mute))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(focus_button("delete-note-cancel", "Cancel", &self.focus.0[1]).on_click(cx.listener(|this, _, w, cx| this.close(w, cx))))
                    .child(focus_button("delete-note-confirm", "Delete", &self.focus.0[0]).debug_selector(|| "delete-note-confirm".into()).on_click(cx.listener(|this, _, w, cx| this.delete(w, cx)))),
            );
        modal_frame(content.into_any_element(), window, &t)
    }
}
