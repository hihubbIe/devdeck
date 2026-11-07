//! Information views: what discovery and the config reported (`problems`), and the
//! first-run screen when no workspace is configured (`welcome`).

use super::ViewCx;
use devdeck_core::config::{MACHINE_FILE, WORKSPACES_DIR};
use devdeck_ui::{label, section, theme};
use gpui::{Context, IntoElement, ParentElement, Render, Styled, Subscription, Window, div, prelude::*};

/// The workspace's problems, live: fixing a manifest or the config clears them.
pub struct ProblemsView {
    ctx: ViewCx,
    _observe: Subscription,
}

impl ProblemsView {
    pub fn new(ctx: ViewCx, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&ctx.services, |_, _, cx| cx.notify());
        Self { ctx, _observe: observe }
    }
}

impl Render for ProblemsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let mut problems = self.ctx.services.read(cx).problems.clone();
        problems.extend(crate::environment::environment(cx).read(cx).problems.iter().cloned());
        let body = if problems.is_empty() {
            div().text_color(t.mute).child(label("No problems."))
        } else {
            // Problems name files and reasons in full: they wrap rather than truncate.
            div().flex().flex_col().gap_1().children(problems.into_iter().map(|p| div().text_color(t.warn).child(format!("⚠ {p}"))))
        };
        div().id("problems").size_full().overflow_y_scroll().p_3().child(section("Problems", &t)).child(body)
    }
}

/// No workspace configured yet: where the files go, and an example.
pub struct WelcomeView;

impl Render for WelcomeView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let dir = devdeck_core::settings::default_config_dir();
        let example = [
            "name = \"work\"",
            "folders = [\"~/repos\"]          # each child folder is a project",
            "envs = [\"dev\", \"prod\"]         # optional",
            "protected = [\"prod\"]",
            "default_env = \"dev\"",
            "",
            "[apps.api]                      # an app's id is its folder",
            "env = { argument = \"--env {env}\" }",
        ];
        div()
            .id("welcome")
            .size_full()
            .overflow_y_scroll()
            .p_3()
            .flex()
            .flex_col()
            .gap_2()
            .child(div().text_color(t.hi).child("No workspace yet"))
            .child(label(format!("Add one file per workspace in {}/{WORKSPACES_DIR}/, for example work.toml:", dir.display())).text_color(t.mute))
            .child(div().p_2().bg(t.raised).rounded(t.radius_m).flex().flex_col().children(example.map(|l| div().whitespace_nowrap().child(l))))
            .child(label(format!("Machine paths (Node folders, PATH additions) go in {}/{MACHINE_FILE}.", dir.display())).text_color(t.mute))
            .child(label("devdeck finds the apps in the folders when it starts. `devdeck --demo` shows the demo.").text_color(t.mute))
    }
}
