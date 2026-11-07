//! "Send to…" (R3.11): while a pick runs, every pane of the workspace shows its number
//! (in every window) and this list shows them as "n · window · active tab", plus "new
//! window". Pick by number key, by clicking the list, or by clicking a pane's badge
//! (any window); Esc cancels. The pick lives in the [`Deck`], so it ends wherever it
//! is completed, and it's cancelled when its source goes away or the workspace hides.

use crate::deck::{Deck, SendSource};
use crate::views;
use devdeck_core::layout::MAIN_WINDOW;
use devdeck_ui::overlay::ModalClose;
use devdeck_ui::overlay::PRIORITY_MODAL;
use devdeck_ui::{label, pop_modal, push_modal, theme};
use gpui::{
    Action, App, AppContext, Context, Entity, FocusHandle, InteractiveElement, IntoElement, ParentElement, Render, StatefulInteractiveElement, Styled,
    Subscription, WeakFocusHandle, Window, actions, anchored, deferred, div, point, prelude::*,
};

/// Picks the pane with this number (1-based).
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = send_to, no_json)]
pub struct PickTarget(pub usize);

actions!(send_to, [PickNewWindow]);

/// Starts a pick from `window` (a tab or a whole pane).
pub fn open(deck: Entity<Deck>, source: SendSource, window: &mut Window, cx: &mut App) {
    deck.update(cx, |d, cx| d.start_send(source, cx));
    let restore = window.focused(cx).map(|f| f.downgrade());
    let modal = cx.new(|cx| SendTo::new(deck, restore, window, cx));
    push_modal(modal.clone().into(), window, cx);
    let f = modal.read(cx).focus.clone();
    f.focus(window, cx);
}

pub struct SendTo {
    deck: Entity<Deck>,
    focus: FocusHandle,
    restore: Option<WeakFocusHandle>,
    closed: bool,
    _observe: Subscription,
}

impl SendTo {
    fn new(deck: Entity<Deck>, restore: Option<WeakFocusHandle>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Completed elsewhere (a badge in another window) or cancelled: close.
        let observe = cx.observe_in(&deck, window, |this, deck, window, cx| {
            if deck.read(cx).send.is_none() {
                this.close(window, cx);
            }
        });
        Self { deck, focus: cx.focus_handle(), restore, closed: false, _observe: observe }
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if std::mem::replace(&mut self.closed, true) {
            return;
        }
        pop_modal(window, cx);
        if let Some(h) = self.restore.as_ref().and_then(WeakFocusHandle::upgrade) {
            h.focus(window, cx);
        }
    }

    fn pick(&mut self, n: Option<usize>, cx: &mut Context<Self>) {
        let target = match n {
            None => None,
            Some(n) => match self.deck.read(cx).layout().numbered_panes().get(n.wrapping_sub(1)) {
                Some((p, _)) => Some(*p),
                None => return,
            },
        };
        if self.deck.read(cx).send_refusal(target).is_some() {
            return;
        }
        self.deck.update(cx, |d, cx| d.send_to(target, cx));
    }
}

impl Render for SendTo {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let d = self.deck.read(cx);
        let what = match &d.send {
            Some(SendSource::Tab(v)) => format!("Send {} to…", views::title(v)),
            Some(SendSource::Pane(_)) => "Send this pane (all its tabs) to…".into(),
            None => String::new(),
        };
        let mut list = div().flex().flex_col();
        let numbered = d.layout().numbered_panes();
        let mut detached: Vec<_> = numbered.iter().map(|(_, w)| *w).filter(|w| *w != MAIN_WINDOW).collect();
        detached.dedup();
        for (i, (pane, win)) in numbered.iter().enumerate() {
            let n = i + 1;
            let active = d.layout().pane(*pane).ok().and_then(|p| p.active_view().cloned()).map(|r| views::title(&r)).unwrap_or_else(|| "empty".into());
            let place = if *win == MAIN_WINDOW {
                "main window".to_string()
            } else {
                let nth = detached.iter().position(|w| w == win).map_or(0, |i| i + 1);
                format!("detached window {nth}")
            };
            let refusal = d.send_refusal(Some(*pane));
            list = list.child(target_row(("target", n), format!("{n}"), format!("{place} · {active}"), refusal, &t).on_click(cx.listener(move |this, _, _, cx| this.pick(Some(n), cx))));
        }
        let refusal = d.send_refusal(None);
        list = list.child(target_row("target-new", "n".into(), "new window".into(), refusal, &t).on_click(cx.listener(|this, _, _, cx| this.pick(None, cx))));
        let content = div()
            .key_context("Modal SendTo")
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .gap_2()
            .on_action(cx.listener(|this, a: &PickTarget, _, cx| this.pick(Some(a.0), cx)))
            .on_action(cx.listener(|this, _: &PickNewWindow, _, cx| this.pick(None, cx)))
            .on_action(cx.listener(|this, _: &ModalClose, window, cx| {
                this.deck.update(cx, |d, cx| d.cancel_send(cx));
                this.close(window, cx);
            }))
            .child(div().text_color(t.hi).child(what))
            .child(list)
            .child(label("press a number or n · click a row or a pane's badge · esc cancels").text_color(t.dim));
        // A floating panel without a backdrop: the panes stay clickable (R3.11).
        let viewport = window.viewport_size();
        let width = t.size(560.);
        let panel = div()
            .occlude()
            .w(width)
            .bg(t.raised)
            .border_1()
            .border_color(t.line)
            .rounded(t.radius_m)
            .p_4()
            .font_family(t.font.clone())
            .text_size(t.text)
            .text_color(t.fg)
            .child(content);
        deferred(anchored().position(point((viewport.width - width) * 0.5, t.title_h + t.size(48.))).child(panel)).with_priority(PRIORITY_MODAL)
    }
}

fn target_row(id: impl Into<gpui::ElementId>, key: String, text: String, refusal: Option<&'static str>, t: &devdeck_ui::Theme) -> gpui::Stateful<gpui::Div> {
    let hover = t.hover;
    let enabled = refusal.is_none();
    div()
        .id(id)
        .h(t.row_h)
        .px_2()
        .flex()
        .items_center()
        .gap_3()
        .rounded(t.radius_s)
        .when(enabled, |d| d.hover(move |s| s.bg(hover)).cursor_pointer())
        .child(div().flex_none().w(t.size(24.)).text_color(if enabled { t.accent } else { t.dim }).child(key))
        .child(label(text).flex_1().text_color(if enabled { t.fg } else { t.dim }))
        .children(refusal.map(|r| div().flex_none().text_color(t.mute).child(r)))
}
