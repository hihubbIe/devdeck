//! The picker: pick one of many in a modal (a branch to deploy, a port…). It opens with its
//! search focused and filters as one types (`devdeck_core::fuzzy`); options come in groups
//! with their heading, each with a label, chips and a detail; filter chips narrow them to a
//! kind. Up and down choose, Enter or a click picks, Esc closes; the focus goes back where
//! it was. The caller gives the options and what a pick does, and may let what's typed be
//! picked as it is.

use crate::overlay::ModalClose;
use crate::text_input::{InputEvent, TextInput};
use crate::theme::theme;
use crate::scroll::Scrolls;
use crate::{label, modal_frame, pop_modal, push_modal, toggle_button};
use devdeck_core::fuzzy;
use gpui::{
    App, AppContext, Context, Entity, Focusable, InteractiveElement, IntoElement, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled,
    Subscription, WeakFocusHandle, Window, actions, div, prelude::*,
};
use std::rc::Rc;

actions!(picker, [PickerUp, PickerDown]);

/// One option: `value` is what a pick hands back.
#[derive(Clone, Debug, PartialEq)]
pub struct PickOption {
    pub value: String,
    pub label: SharedString,
    pub detail: SharedString,
    pub chips: Vec<SharedString>,
    /// Its kind, for the filter chips (`None`: only under "all").
    pub kind: Option<&'static str>,
}

impl PickOption {
    pub fn new(value: impl Into<String>) -> Self {
        let value = value.into();
        Self { label: value.clone().into(), value, detail: SharedString::default(), chips: Vec::new(), kind: None }
    }

    pub fn detail(mut self, detail: impl Into<SharedString>) -> Self {
        self.detail = detail.into();
        self
    }

    pub fn chip(mut self, chip: impl Into<SharedString>) -> Self {
        self.chips.push(chip.into());
        self
    }

    pub fn kind(mut self, kind: &'static str) -> Self {
        self.kind = Some(kind);
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PickGroup {
    pub label: SharedString,
    pub options: Vec<PickOption>,
}

pub type OnPick = Rc<dyn Fn(String, &mut Window, &mut App)>;

/// What a picker shows and does.
#[derive(Clone)]
pub struct PickerSpec {
    pub title: SharedString,
    pub placeholder: SharedString,
    /// The kinds a filter chip narrows to, after "all".
    pub kinds: Vec<&'static str>,
    pub groups: Vec<PickGroup>,
    /// What's typed is an option of its own, with this detail (`None`: only the options).
    pub typed: Option<SharedString>,
    pub on_pick: OnPick,
}

/// Opens a picker over the window, its search focused.
pub fn open_picker(spec: PickerSpec, window: &mut Window, cx: &mut App) -> Entity<Picker> {
    let restore = window.focused(cx).map(|f| f.downgrade());
    let picker = cx.new(|cx| Picker::new(spec, restore, window, cx));
    push_modal(picker.clone().into(), window, cx);
    let f = picker.read(cx).input.read(cx).focus_handle(cx);
    f.focus(window, cx);
    picker
}

pub struct Picker {
    pub input: Entity<TextInput>,
    spec: PickerSpec,
    /// The kind narrowed to (`None`: all).
    pub kind: Option<&'static str>,
    pub selected: usize,
    restore: Option<WeakFocusHandle>,
    /// Its scroll, read by its scrollbar.
    scroll: gpui::ScrollHandle,
    _sub: Subscription,
}

impl Picker {
    fn new(spec: PickerSpec, restore: Option<WeakFocusHandle>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| TextInput::new(spec.placeholder.clone(), cx));
        let sub = cx.subscribe_in(&input, window, |this, _, e: &InputEvent, window, cx| match e {
            InputEvent::Changed(_) => {
                this.selected = 0;
                cx.notify();
            }
            InputEvent::Submit(_) => this.pick(this.selected, window, cx),
            InputEvent::Cancel => this.close(window, cx),
        });
        Self { input, spec, kind: None, selected: 0, restore, scroll: gpui::ScrollHandle::new(), _sub: sub }
    }

    /// The groups as shown: matching what's typed and the kind, empty groups left out;
    /// what's typed first when it may be picked and isn't an option already.
    pub fn shown(&self, cx: &App) -> Vec<PickGroup> {
        let q = self.input.read(cx).text().trim().to_string();
        let mut out: Vec<PickGroup> = Vec::new();
        let exact = self.spec.groups.iter().flat_map(|g| &g.options).any(|o| o.value == q);
        if let Some(detail) = self.spec.typed.as_ref().filter(|_| !q.is_empty() && !exact) {
            out.push(PickGroup { label: SharedString::default(), options: vec![PickOption::new(q.clone()).detail(detail.clone())] });
        }
        for g in &self.spec.groups {
            let options: Vec<PickOption> = g
                .options
                .iter()
                .filter(|o| self.kind.is_none_or(|k| o.kind == Some(k)))
                .filter(|o| fuzzy::score(&q, &format!("{} {} {}", o.label, o.detail, o.chips.join(" "))).is_some())
                .cloned()
                .collect();
            if !options.is_empty() {
                out.push(PickGroup { label: g.label.clone(), options });
            }
        }
        out
    }

    fn options(&self, cx: &App) -> Vec<PickOption> {
        self.shown(cx).into_iter().flat_map(|g| g.options).collect()
    }

    fn step(&mut self, by: isize, cx: &mut Context<Self>) {
        let n = self.options(cx).len();
        if n > 0 {
            self.selected = (self.selected as isize + by).clamp(0, n as isize - 1) as usize;
            cx.notify();
        }
    }

    pub fn set_kind(&mut self, kind: Option<&'static str>, cx: &mut Context<Self>) {
        self.kind = kind;
        self.selected = 0;
        cx.notify();
    }

    fn pick(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(option) = self.options(cx).into_iter().nth(i) else { return };
        let on_pick = self.spec.on_pick.clone();
        self.close(window, cx);
        on_pick(option.value, window, cx);
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        pop_modal(window, cx);
        if let Some(h) = self.restore.as_ref().and_then(WeakFocusHandle::upgrade) {
            h.focus(window, cx);
        }
    }
}

impl Render for Picker {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let shown = self.shown(cx);
        let mut chips = div().flex().items_center().gap_1();
        if !self.spec.kinds.is_empty() {
            chips = chips.child(toggle_button("pick-all", "all", self.kind.is_none(), &t).on_click(cx.listener(|this, _, _, cx| this.set_kind(None, cx))));
            for k in self.spec.kinds.clone() {
                chips = chips.child(
                    toggle_button(SharedString::from(format!("pick-kind-{k}")), k, self.kind == Some(k), &t)
                        .debug_selector(move || format!("pick-kind-{k}"))
                        .on_click(cx.listener(move |this, _, _, cx| this.set_kind(Some(k), cx))),
                );
            }
        }
        let mut list = div().id("pick-list").max_h(t.size(380.)).scroll_y().track_scroll(&self.scroll).flex().flex_col();
        let mut n = 0;
        for g in &shown {
            if !g.label.is_empty() {
                list = list.child(div().px_2().pt_2().text_size(t.text_small).text_color(t.dim).child(g.label.clone()));
            }
            for o in &g.options {
                let (i, selected) = (n, n == self.selected);
                n += 1;
                list = list.child(
                    div()
                        .id(("pick", i))
                        .debug_selector(move || format!("pick-{i}"))
                        .h(t.row_h)
                        .px_2()
                        .flex()
                        .items_center()
                        .gap_2()
                        .rounded(t.radius_s)
                        .cursor_pointer()
                        .when(selected, |d| d.bg(t.selection))
                        .hover(|s| s.bg(t.hover))
                        .on_click(cx.listener(move |this, _, window, cx| this.pick(i, window, cx)))
                        .child(label(o.label.clone()).flex_1().min_w_0().text_color(t.fg))
                        .children(o.chips.iter().map(|c| div().flex_none().px_1().rounded(t.radius_s).bg(t.env_bg).text_color(t.env).child(c.clone())))
                        .child(div().flex_none().max_w(t.size(240.)).truncate().text_color(t.mute).child(o.detail.clone())),
                );
            }
        }
        if n == 0 {
            list = list.child(div().px_2().h(t.row_h).flex().items_center().text_color(t.dim).child("nothing matches"));
        }
        let content = div()
            .key_context("Modal Picker")
            .w(t.size(640.))
            .flex()
            .flex_col()
            .gap_2()
            .on_action(cx.listener(|this, _: &ModalClose, window, cx| this.close(window, cx)))
            .on_action(cx.listener(|this, _: &PickerUp, _, cx| this.step(-1, cx)))
            .on_action(cx.listener(|this, _: &PickerDown, _, cx| this.step(1, cx)))
            .child(div().text_color(t.hi).child(self.spec.title.clone()))
            .child(div().debug_selector(|| "pick-search".into()).child(self.input.clone()))
            .when(!self.spec.kinds.is_empty(), |d| d.child(chips))
            .child(crate::with_scrollbar("pick-scrollbar", list, &self.scroll));
        modal_frame(content.into_any_element(), window, &t)
    }
}
