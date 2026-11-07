//! The action registry (R5.1): every action with its id, label, group, key context and
//! default keys. Key bindings are generated from it, and everything that names an
//! action or its key (menus, palette, cheat sheet, hint bar, tooltips) reads it here.
//! Actions themselves are GPUI actions declared next to the code that handles them.

use crate::actions::*;
use crate::shell::ConfirmQuit;
use crate::sidebar as sb;
use crate::views::{app_view as av, logs_view as lv, matrix_view as mv, run_view as rv, service_view as sv};
use devdeck_ui::overlay as ov;
use devdeck_ui::text_input as ti;
use gpui::{Action, App, KeyBinding, KeyBindingContextPredicate};
use std::rc::Rc;
use std::sync::LazyLock;

pub struct ActionSpec {
    pub id: &'static str,
    pub label: &'static str,
    pub group: &'static str,
    /// Where it applies (a key context predicate); `None` everywhere.
    pub context: Option<&'static str>,
    /// Default keys, GPUI syntax (`ctrl-shift-d`).
    pub keys: &'static [&'static str],
    /// Shown in the hint bar when available.
    pub hint: bool,
    pub build: fn() -> Box<dyn Action>,
}

impl ActionSpec {
    /// The first key, as shown to people (`ctrl shift d`).
    pub fn key_text(&self) -> Option<String> {
        self.keys.first().map(|k| key_text(k))
    }
}

/// `ctrl-shift-d` → `ctrl shift d`; `ctrl--` → `ctrl -`.
pub fn key_text(key: &str) -> String {
    key.split(' ')
        .map(|stroke| {
            let mut parts: Vec<&str> = Vec::new();
            let mut rest = stroke;
            while let Some(i) = rest.find('-').filter(|&i| i > 0) {
                parts.push(&rest[..i]);
                rest = &rest[i + 1..];
            }
            parts.push(rest);
            parts.join(" ")
        })
        .collect::<Vec<_>>()
        .join(", ")
}

const PANES: Option<&str> = Some("Panes && !TextInput");
/// App-wide, but not under a modal or a menu (they own the keyboard while open).
const DECK: Option<&str> = Some("Deck && !Modal && !Menu");
const ANYWHERE: Option<&str> = None;
const SIDEBAR: Option<&str> = Some("Sidebar");
const SIDEBAR_LIST: Option<&str> = Some("Sidebar && !TextInput");
const TEXT: Option<&str> = Some("TextInput");

macro_rules! spec {
    ($id:literal, $label:literal, $group:literal, $ctx:expr, [$($k:literal),*], $hint:literal, $action:expr) => {
        ActionSpec { id: $id, label: $label, group: $group, context: $ctx, keys: &[$($k),*], hint: $hint, build: || Box::new($action) }
    };
}

static SPECS: LazyLock<Vec<ActionSpec>> = LazyLock::new(|| {
    vec![
        // App
        spec!("app.palette", "command palette", "App", DECK, ["ctrl-k"], true, Palette),
        spec!("app.cheat_sheet", "keys", "App", DECK, ["ctrl-/"], true, CheatSheet),
        spec!("app.cheat_sheet_q", "keys", "App", Some("Deck && !TextInput && !Modal && !Menu"), ["?"], false, CheatSheet),
        spec!("app.settings", "settings", "App", DECK, ["ctrl-,"], false, ToggleSettings),
        spec!("app.close_settings", "close settings", "App", Some("Settings"), ["escape"], true, CloseSettings),
        spec!("app.scale_up", "bigger", "App", DECK, ["ctrl-=", "ctrl-+"], false, IncreaseScale),
        spec!("app.scale_down", "smaller", "App", DECK, ["ctrl--"], false, DecreaseScale),
        spec!("app.scale_reset", "reset size", "App", DECK, ["ctrl-0"], false, ResetScale),
        spec!("app.quit", "quit", "App", ANYWHERE, ["ctrl-q"], false, Quit),
        spec!("app.confirm_quit", "pick", "App", Some("QuitGuard"), ["enter"], true, ConfirmQuit),
        // Workspaces
        spec!("ws.1", "workspace 1", "Workspaces", DECK, ["ctrl-alt-1"], false, SwitchWorkspace(0)),
        spec!("ws.2", "workspace 2", "Workspaces", DECK, ["ctrl-alt-2"], false, SwitchWorkspace(1)),
        spec!("ws.3", "workspace 3", "Workspaces", DECK, ["ctrl-alt-3"], false, SwitchWorkspace(2)),
        spec!("ws.4", "workspace 4", "Workspaces", DECK, ["ctrl-alt-4"], false, SwitchWorkspace(3)),
        spec!("ws.5", "workspace 5", "Workspaces", DECK, ["ctrl-alt-5"], false, SwitchWorkspace(4)),
        spec!("ws.6", "workspace 6", "Workspaces", DECK, ["ctrl-alt-6"], false, SwitchWorkspace(5)),
        spec!("ws.7", "workspace 7", "Workspaces", DECK, ["ctrl-alt-7"], false, SwitchWorkspace(6)),
        spec!("ws.8", "workspace 8", "Workspaces", DECK, ["ctrl-alt-8"], false, SwitchWorkspace(7)),
        spec!("ws.9", "workspace 9", "Workspaces", DECK, ["ctrl-alt-9"], false, SwitchWorkspace(8)),
        spec!("ws.env", "env", "Workspaces", DECK, ["ctrl-e"], false, OpenEnvMenu),
        // Sidebar
        spec!("sidebar.apps", "apps", "Sidebar", DECK, ["ctrl-1"], false, ShowApps),
        spec!("sidebar.cluster", "cluster", "Sidebar", DECK, ["ctrl-2"], false, ShowCluster),
        spec!("sidebar.ports", "ports", "Sidebar", DECK, ["ctrl-3"], false, ShowPorts),
        spec!("sidebar.layouts", "layouts", "Sidebar", DECK, ["ctrl-4"], false, ShowLayouts),
        spec!("sidebar.toggle", "sidebar", "Sidebar", DECK, ["ctrl-b"], false, ToggleSidebar),
        spec!("layout.save_as", "save layout as…", "Layouts", DECK, [], false, SaveLayoutAs),
        spec!("sidebar.next", "down", "Sidebar", SIDEBAR, ["down"], false, sb::SelectNext),
        spec!("sidebar.prev", "up", "Sidebar", SIDEBAR, ["up"], false, sb::SelectPrev),
        spec!("sidebar.first", "first", "Sidebar", SIDEBAR_LIST, ["home"], false, sb::SelectFirst),
        spec!("sidebar.last", "last", "Sidebar", SIDEBAR_LIST, ["end"], false, sb::SelectLast),
        spec!("sidebar.open", "open", "Sidebar", SIDEBAR_LIST, ["enter"], true, sb::OpenSelected),
        spec!("sidebar.pin", "pin", "Sidebar", SIDEBAR_LIST, ["p"], true, sb::TogglePin),
        spec!("sidebar.filter", "filter", "Sidebar", SIDEBAR_LIST, ["/"], true, sb::FocusFilter),
        spec!("sidebar.running", "running only", "Sidebar", SIDEBAR_LIST, ["r"], true, sb::ToggleRunningOnly),
        spec!("sidebar.escape", "back", "Sidebar", SIDEBAR, ["escape"], true, sb::SidebarEscape),
        // Panes
        spec!("pane.focus_left", "focus left", "Panes", PANES, ["alt-left"], false, FocusLeft),
        spec!("pane.focus_right", "focus right", "Panes", PANES, ["alt-right"], false, FocusRight),
        spec!("pane.focus_up", "focus up", "Panes", PANES, ["alt-up"], false, FocusUp),
        spec!("pane.focus_down", "focus down", "Panes", PANES, ["alt-down"], false, FocusDown),
        spec!("pane.move_left", "move tab left", "Panes", PANES, ["alt-shift-left"], false, MoveTabLeft),
        spec!("pane.move_right", "move tab right", "Panes", PANES, ["alt-shift-right"], false, MoveTabRight),
        spec!("pane.move_up", "move tab up", "Panes", PANES, ["alt-shift-up"], false, MoveTabUp),
        spec!("pane.move_down", "move tab down", "Panes", PANES, ["alt-shift-down"], false, MoveTabDown),
        spec!("pane.next_tab", "next tab", "Panes", PANES, ["ctrl-tab"], false, NextTab),
        spec!("pane.prev_tab", "previous tab", "Panes", PANES, ["ctrl-shift-tab"], false, PrevTab),
        spec!("pane.close_tab", "close tab", "Panes", PANES, ["ctrl-w"], true, CloseTab),
        spec!("pane.split_right", "split right", "Panes", PANES, ["ctrl-\\"], true, SplitRight),
        spec!("pane.split_down", "split down", "Panes", PANES, ["ctrl-alt-\\"], false, SplitDown),
        spec!("pane.collapse", "collapse", "Panes", PANES, ["ctrl-shift-c"], false, ToggleCollapse),
        spec!("pane.maximize", "maximize", "Panes", PANES, ["ctrl-shift-m"], true, ToggleMaximize),
        spec!("pane.detach", "detach", "Panes", PANES, ["ctrl-shift-d"], false, DetachTab),
        spec!("pane.send_tab", "send to…", "Panes", PANES, ["ctrl-shift-s"], true, SendTab),
        spec!("pane.send_pane", "send pane to…", "Panes", PANES, ["ctrl-alt-shift-s"], false, SendPane),
        spec!("pane.reattach", "re-attach", "Panes", DECK, ["ctrl-shift-a"], false, Reattach),
        spec!("pane.grow", "grow", "Panes", PANES, ["ctrl-alt-="], false, GrowPane),
        spec!("pane.shrink", "shrink", "Panes", PANES, ["ctrl-alt--"], false, ShrinkPane),
        spec!("pane.follow", "follow", "Panes", PANES, ["alt-f"], false, ToggleFollow),
        spec!("pane.menu", "menu", "Panes", DECK, ["shift-f10", "menu"], true, ContextMenu),
        // App view
        spec!("app_view.next", "down", "App view", Some("AppView && !TextInput"), ["down"], false, av::SelectNext),
        spec!("app_view.prev", "up", "App view", Some("AppView && !TextInput"), ["up"], false, av::SelectPrev),
        spec!("app_view.run", "run", "App view", Some("AppView && !TextInput"), ["enter"], true, av::RunSelected),
        spec!("app_view.stop", "stop", "App view", Some("AppView && !TextInput"), ["s"], true, av::StopSelected),
        spec!("app_view.restart", "restart", "App view", Some("AppView && !TextInput"), ["r"], true, av::RestartSelected),
        spec!("app_view.output", "output", "App view", Some("AppView && !TextInput"), ["o"], true, av::OpenOutput),
        spec!("app_view.hidden", "hidden commands", "App view", Some("AppView && !TextInput"), ["h"], true, av::ToggleHidden),
        // Run view
        spec!("run.run", "run", "Run output", Some("RunView && !TextInput"), ["enter"], true, rv::RunAgain),
        spec!("run.stop", "stop", "Run output", Some("RunView && !TextInput"), ["s"], true, rv::Stop),
        spec!("run.restart", "restart", "Run output", Some("RunView && !TextInput"), ["r"], true, rv::Restart),
        // Service
        spec!("svc.save", "save tag", "Service", Some("ServiceView"), ["ctrl-s"], true, sv::SaveTag),
        spec!("svc.undo", "undo rollout", "Service", Some("ServiceView && !TextInput"), ["u"], true, sv::Undo),
        spec!("svc.logs", "logs", "Service", Some("ServiceView && !TextInput"), ["l"], true, sv::OpenLogs),
        spec!("svc.tag", "edit tag", "Service", Some("ServiceView && !TextInput"), ["t"], true, sv::EditTag),
        spec!("svc.confirm", "confirm", "Service", Some("Modal && ServiceModal"), ["enter"], true, sv::ConfirmModal),
        // Logs
        spec!("logs.find", "find", "Logs", Some("LogsView && !TextInput"), ["/"], true, lv::Find),
        spec!("logs.next", "next match", "Logs", Some("LogsView && !TextInput"), ["n"], true, lv::NextMatch),
        spec!("logs.prev", "previous match", "Logs", Some("LogsView && !TextInput"), ["shift-n"], false, lv::PrevMatch),
        spec!("logs.pause", "pause", "Logs", Some("LogsView && !TextInput"), ["p"], true, lv::TogglePause),
        spec!("logs.errors", "errors", "Logs", Some("LogsView && !TextInput"), ["1"], true, lv::OnlyErrors),
        spec!("logs.warn", "warn+", "Logs", Some("LogsView && !TextInput"), ["2"], false, lv::WarnAndUp),
        spec!("logs.info", "info+", "Logs", Some("LogsView && !TextInput"), ["3"], false, lv::InfoAndUp),
        spec!("logs.all", "all levels", "Logs", Some("LogsView && !TextInput"), ["4"], false, lv::Everything),
        // Matrix
        spec!("matrix.clear", "clear highlight", "Branch matrix", Some("MatrixView"), ["escape"], true, mv::ClearHighlight),
        // Palette
        spec!("palette.down", "down", "Palette", Some("Palette"), ["down"], false, crate::palette::PaletteDown),
        spec!("palette.up", "up", "Palette", Some("Palette"), ["up"], false, crate::palette::PaletteUp),
        spec!("palette.drill", "commands", "Palette", Some("Palette"), ["tab"], true, crate::palette::PaletteDrill),
        // Send to
        spec!("send.1", "pane 1", "Send to", Some("SendTo"), ["1"], false, crate::send_to::PickTarget(1)),
        spec!("send.2", "pane 2", "Send to", Some("SendTo"), ["2"], false, crate::send_to::PickTarget(2)),
        spec!("send.3", "pane 3", "Send to", Some("SendTo"), ["3"], false, crate::send_to::PickTarget(3)),
        spec!("send.4", "pane 4", "Send to", Some("SendTo"), ["4"], false, crate::send_to::PickTarget(4)),
        spec!("send.5", "pane 5", "Send to", Some("SendTo"), ["5"], false, crate::send_to::PickTarget(5)),
        spec!("send.6", "pane 6", "Send to", Some("SendTo"), ["6"], false, crate::send_to::PickTarget(6)),
        spec!("send.7", "pane 7", "Send to", Some("SendTo"), ["7"], false, crate::send_to::PickTarget(7)),
        spec!("send.8", "pane 8", "Send to", Some("SendTo"), ["8"], false, crate::send_to::PickTarget(8)),
        spec!("send.9", "pane 9", "Send to", Some("SendTo"), ["9"], false, crate::send_to::PickTarget(9)),
        spec!("send.new", "new window", "Send to", Some("SendTo"), ["n"], true, crate::send_to::PickNewWindow),
        // Menus and modals
        spec!("menu.down", "down", "Menus", Some("Menu"), ["down"], false, ov::MenuDown),
        spec!("menu.up", "up", "Menus", Some("Menu"), ["up"], false, ov::MenuUp),
        spec!("menu.confirm", "pick", "Menus", Some("Menu"), ["enter"], true, ov::MenuConfirm),
        spec!("menu.close", "close", "Menus", Some("Menu"), ["escape"], true, ov::MenuClose),
        spec!("modal.next", "next", "Menus", Some("Modal"), ["tab"], true, ov::ModalNext),
        spec!("modal.prev", "previous", "Menus", Some("Modal"), ["shift-tab"], false, ov::ModalPrev),
        spec!("modal.close", "cancel", "Menus", Some("Modal"), ["escape"], true, ov::ModalClose),
        // Text inputs
        spec!("text.backspace", "delete left", "Text", TEXT, ["backspace"], false, ti::Backspace),
        spec!("text.delete", "delete right", "Text", TEXT, ["delete"], false, ti::Delete),
        spec!("text.delete_word", "delete word", "Text", TEXT, ["ctrl-backspace"], false, ti::DeleteWordLeft),
        spec!("text.left", "left", "Text", TEXT, ["left"], false, ti::Left),
        spec!("text.right", "right", "Text", TEXT, ["right"], false, ti::Right),
        spec!("text.select_left", "select left", "Text", TEXT, ["shift-left"], false, ti::SelectLeft),
        spec!("text.select_right", "select right", "Text", TEXT, ["shift-right"], false, ti::SelectRight),
        spec!("text.word_left", "word left", "Text", TEXT, ["ctrl-left"], false, ti::WordLeft),
        spec!("text.word_right", "word right", "Text", TEXT, ["ctrl-right"], false, ti::WordRight),
        spec!("text.select_word_left", "select word left", "Text", TEXT, ["ctrl-shift-left"], false, ti::SelectWordLeft),
        spec!("text.select_word_right", "select word right", "Text", TEXT, ["ctrl-shift-right"], false, ti::SelectWordRight),
        spec!("text.home", "start", "Text", TEXT, ["home"], false, ti::Home),
        spec!("text.end", "end", "Text", TEXT, ["end"], false, ti::End),
        spec!("text.select_home", "select to start", "Text", TEXT, ["shift-home"], false, ti::SelectHome),
        spec!("text.select_end", "select to end", "Text", TEXT, ["shift-end"], false, ti::SelectEnd),
        spec!("text.select_all", "select all", "Text", TEXT, ["ctrl-a"], false, ti::SelectAll),
        spec!("text.copy", "copy", "Text", TEXT, ["ctrl-c"], false, ti::Copy),
        spec!("text.cut", "cut", "Text", TEXT, ["ctrl-x"], false, ti::Cut),
        spec!("text.paste", "paste", "Text", TEXT, ["ctrl-v"], false, ti::Paste),
        spec!("text.undo", "undo", "Text", TEXT, ["ctrl-z"], false, ti::Undo),
        spec!("text.redo", "redo", "Text", TEXT, ["ctrl-shift-z"], false, ti::Redo),
        spec!("text.submit", "submit", "Text", TEXT, ["enter"], true, ti::Submit),
        spec!("text.cancel", "cancel", "Text", TEXT, ["escape"], true, ti::Cancel),
    ]
});

pub fn all() -> &'static [ActionSpec] {
    &SPECS
}

pub fn by_id(id: &str) -> Option<&'static ActionSpec> {
    all().iter().find(|s| s.id == id)
}

/// The registry entry of an action (same type and data).
pub fn of(action: &dyn Action) -> Option<&'static ActionSpec> {
    all().iter().find(|s| (s.build)().partial_eq(action))
}

/// The key that runs an action where focus is now (the highest-precedence binding).
pub fn active_key(action: &dyn gpui::Action, window: &gpui::Window, cx: &App) -> Option<String> {
    window.highest_precedence_binding_for_action(action).map(|b| {
        b.keystrokes().iter().map(|k| k.display(cx.platform_info().os).to_string()).collect::<Vec<_>>().join(" ")
    })
}

/// The first key of an action, for labels and tooltips.
pub fn key_for(id: &str) -> String {
    by_id(id).and_then(ActionSpec::key_text).unwrap_or_default()
}

/// Each key context and the group of actions it brings into play. A new view type adds
/// its context here, next to its actions.
pub const CONTEXT_GROUPS: &[(&str, &str)] = &[
    ("TextInput", "Text"),
    ("Menu", "Menus"),
    ("Modal", "Menus"),
    ("Palette", "Palette"),
    ("SendTo", "Send to"),
    ("QuitGuard", "App"),
    ("Settings", "App"),
    ("Sidebar", "Sidebar"),
    ("AppView", "App view"),
    ("RunView", "Run output"),
    ("ServiceView", "Service"),
    ("LogsView", "Logs"),
    ("MatrixView", "Branch matrix"),
    ("Panes", "Panes"),
];

thread_local! {
    /// Every entry's context, parsed once (GPUI predicates aren't thread-safe).
    static CONTEXTS: Vec<Option<Rc<KeyBindingContextPredicate>>> = all()
        .iter()
        .map(|s| s.context.map(|c| Rc::new(KeyBindingContextPredicate::parse(c).unwrap_or_else(|e| panic!("{}: bad context {c}: {e}", s.id)))))
        .collect();
}

/// The entries whose binding is enabled for this key context stack (GPUI's own test,
/// `depth_of`), ordered by group: the deepest context's group first, app-wide last. A
/// deeper binding on the same key can still shadow one of them (Esc in an input).
pub fn applicable(stack: &[gpui::KeyContext]) -> Vec<&'static ActionSpec> {
    let groups: Vec<&str> = stack
        .iter()
        .rev()
        .flat_map(|c| CONTEXT_GROUPS.iter().filter(move |(k, _)| c.contains(k)).map(|(_, g)| *g))
        .collect();
    let rank = |g: &str| groups.iter().position(|x| *x == g).unwrap_or(if g == "App" { usize::MAX } else { usize::MAX - 1 });
    let mut out: Vec<&'static ActionSpec> = CONTEXTS.with(|ctx| {
        all().iter().zip(ctx.iter()).filter(|(_, p)| p.as_ref().is_none_or(|p| p.depth_of(stack).is_some())).map(|(s, _)| s).collect()
    });
    out.sort_by_key(|s| rank(s.group));
    out
}

/// Binds every key of every action.
pub fn bind(cx: &mut App) {
    let mut bindings = Vec::new();
    let contexts = CONTEXTS.with(|c| c.clone());
    for (spec, predicate) in all().iter().zip(contexts) {
        for key in spec.keys {
            let b = KeyBinding::load(key, (spec.build)(), predicate.clone(), false, None, cx).unwrap_or_else(|e| panic!("{}: bad key {key}: {e:?}", spec.id));
            bindings.push(b);
        }
    }
    cx.bind_keys(bindings);
}
