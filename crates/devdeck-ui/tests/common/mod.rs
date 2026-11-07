//! A host window for testing one component on its own, on GPUI's test platform: the
//! theme, the component, and the window's overlay layer (menus, modals) and toasts, as
//! the app's windows draw them. Tests drive it with the mouse and keys through a
//! `VisualTestContext`, or dispatch the components' actions directly (key bindings are
//! the app's, not the library's).

#![allow(dead_code)]

use devdeck_ui::overlay::layer;
use devdeck_ui::{Theme, render_layer, theme, toasts};
use gpui::{AnyElement, AnyWindowHandle, App, Context, IntoElement, ParentElement, Render, Styled, Subscription, TestAppContext, Window, div};

/// What a host window draws.
type Content = Box<dyn Fn(&mut Window, &mut App) -> AnyElement>;

pub struct Host {
    content: Content,
    _subs: Vec<Subscription>,
}

impl Render for Host {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        div()
            .size_full()
            .bg(t.bg)
            .text_color(t.fg)
            .font_family(t.font.clone())
            .text_size(t.text)
            .child((self.content)(window, cx))
            .children(render_layer(window, cx))
            .child(toasts(cx))
    }
}

/// Opens a host window drawing `content` with the default theme.
pub fn host(cx: &mut TestAppContext, content: impl Fn(&mut Window, &mut App) -> AnyElement + 'static) -> AnyWindowHandle {
    cx.update(|cx| {
        if !cx.has_global::<Theme>() {
            cx.set_global(Theme::devdeck_dark(1.0));
        }
    });
    let window = cx.add_window(move |window, cx| {
        let overlays = layer(window.window_handle(), cx);
        let toasts = toasts(cx);
        let subs = vec![cx.observe(&overlays, |_, _, cx| cx.notify()), cx.observe(&toasts, |_, _, cx| cx.notify())];
        Host { content: Box::new(content), _subs: subs }
    });
    cx.run_until_parked();
    window.into()
}

/// The theme the host draws with.
pub fn the_theme(cx: &mut TestAppContext) -> Theme {
    cx.update(|cx| theme(cx).clone())
}
