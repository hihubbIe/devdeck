//! The action registry (R5.1): every action with its id, label, group, key context and
//! default keys. Key bindings are generated from it, and everything that names an
//! action or its key (menus, palette, cheat sheet) reads it here.
//! Actions themselves are GPUI actions declared next to the code that handles them.
//!
//! In a focused terminal, keys go to the process (M2 decision): every binding whose key a
//! terminal has bytes for is off inside `Terminal` ([`terminal_takes`]), so devdeck keeps
//! only `alt+…`, `ctrl+shift+…` and keys a process can't receive there.

use crate::actions::*;
use crate::shell::ConfirmQuit;
use crate::sidebar as sb;
use crate::views::{app_view as av, git_view as gv, logs_view as lv, matrix_view as mv, run_view as rv, service_view as sv};
use devdeck_ui::overlay as ov;
use devdeck_ui::terminal as term;
use devdeck_ui::text_input as ti;
use gpui::{Action, App, KeyBinding, KeyBindingContextPredicate, Keystroke};
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
    ($id:literal, $label:literal, $group:literal, $ctx:expr, [$($k:literal),*], $action:expr) => {
        ActionSpec { id: $id, label: $label, group: $group, context: $ctx, keys: &[$($k),*], build: || Box::new($action) }
    };
}

static SPECS: LazyLock<Vec<ActionSpec>> = LazyLock::new(|| {
    vec![
        // App
        spec!("app.palette", "command palette", "App", DECK, ["ctrl-k", "ctrl-shift-p"], Palette),
        spec!("app.cheat_sheet", "keys", "App", DECK, ["ctrl-/"], CheatSheet),
        spec!("app.cheat_sheet_q", "keys", "App", Some("Deck && !TextInput && !Modal && !Menu"), ["?"], CheatSheet),
        spec!("app.settings", "settings", "App", DECK, ["ctrl-,"], ToggleSettings),
        spec!("app.close_settings", "close settings", "App", Some("Settings"), ["escape"], CloseSettings),
        spec!("app.scale_up", "bigger", "App", DECK, ["ctrl-=", "ctrl-+"], IncreaseScale),
        spec!("app.scale_down", "smaller", "App", DECK, ["ctrl--"], DecreaseScale),
        spec!("app.scale_reset", "reset size", "App", DECK, ["ctrl-0"], ResetScale),
        spec!("app.quit", "quit", "App", ANYWHERE, ["ctrl-q"], Quit),
        spec!("app.confirm_quit", "pick", "App", Some("QuitGuard"), ["enter"], ConfirmQuit),
        // Workspaces
        spec!("ws.1", "workspace 1", "Workspaces", DECK, ["ctrl-alt-1"], SwitchWorkspace(0)),
        spec!("ws.2", "workspace 2", "Workspaces", DECK, ["ctrl-alt-2"], SwitchWorkspace(1)),
        spec!("ws.3", "workspace 3", "Workspaces", DECK, ["ctrl-alt-3"], SwitchWorkspace(2)),
        spec!("ws.4", "workspace 4", "Workspaces", DECK, ["ctrl-alt-4"], SwitchWorkspace(3)),
        spec!("ws.5", "workspace 5", "Workspaces", DECK, ["ctrl-alt-5"], SwitchWorkspace(4)),
        spec!("ws.6", "workspace 6", "Workspaces", DECK, ["ctrl-alt-6"], SwitchWorkspace(5)),
        spec!("ws.7", "workspace 7", "Workspaces", DECK, ["ctrl-alt-7"], SwitchWorkspace(6)),
        spec!("ws.8", "workspace 8", "Workspaces", DECK, ["ctrl-alt-8"], SwitchWorkspace(7)),
        spec!("ws.9", "workspace 9", "Workspaces", DECK, ["ctrl-alt-9"], SwitchWorkspace(8)),
        spec!("ws.env", "env", "Workspaces", DECK, ["ctrl-e"], OpenEnvMenu),
        // Sidebar
        spec!("sidebar.apps", "apps", "Sidebar", DECK, ["ctrl-1"], ShowApps),
        spec!("sidebar.cluster", "cluster", "Sidebar", DECK, ["ctrl-2"], ShowCluster),
        spec!("sidebar.ports", "ports", "Sidebar", DECK, ["ctrl-3"], ShowPorts),
        spec!("sidebar.layouts", "layouts", "Sidebar", DECK, ["ctrl-4"], ShowLayouts),
        spec!("sidebar.terminals", "terminals", "Sidebar", DECK, ["ctrl-5"], ShowTerminals),
        spec!("sidebar.toggle", "sidebar", "Sidebar", DECK, ["ctrl-b"], ToggleSidebar),
        spec!("layout.save_as", "save layout as…", "Layouts", DECK, [], SaveLayoutAs),
        spec!("sidebar.next", "down", "Sidebar", SIDEBAR, ["down"], sb::SelectNext),
        spec!("sidebar.prev", "up", "Sidebar", SIDEBAR, ["up"], sb::SelectPrev),
        spec!("sidebar.first", "first", "Sidebar", SIDEBAR_LIST, ["home"], sb::SelectFirst),
        spec!("sidebar.last", "last", "Sidebar", SIDEBAR_LIST, ["end"], sb::SelectLast),
        spec!("sidebar.open", "open", "Sidebar", SIDEBAR_LIST, ["enter"], sb::OpenSelected),
        spec!("sidebar.expand", "open folder", "Sidebar", SIDEBAR_LIST, ["right"], sb::ExpandSelected),
        spec!("sidebar.collapse", "close folder", "Sidebar", SIDEBAR_LIST, ["left"], sb::CollapseSelected),
        spec!("sidebar.pin", "pin", "Sidebar", SIDEBAR_LIST, ["p"], sb::TogglePin),
        spec!("sidebar.filter", "filter", "Sidebar", SIDEBAR_LIST, ["/"], sb::FocusFilter),
        spec!("sidebar.running", "running only", "Sidebar", SIDEBAR_LIST, ["r"], sb::ToggleRunningOnly),
        spec!("sidebar.watch_pods", "watch pods", "Sidebar", SIDEBAR_LIST, [], sb::ToggleWatchPods),
        spec!("sidebar.escape", "back", "Sidebar", SIDEBAR, ["escape"], sb::SidebarEscape),
        // Panes
        spec!("pane.focus_left", "focus left", "Panes", PANES, ["alt-left"], FocusLeft),
        spec!("pane.focus_right", "focus right", "Panes", PANES, ["alt-right"], FocusRight),
        spec!("pane.focus_up", "focus up", "Panes", PANES, ["alt-up"], FocusUp),
        spec!("pane.focus_down", "focus down", "Panes", PANES, ["alt-down"], FocusDown),
        spec!("pane.move_left", "move tab left", "Panes", PANES, ["alt-shift-left"], MoveTabLeft),
        spec!("pane.move_right", "move tab right", "Panes", PANES, ["alt-shift-right"], MoveTabRight),
        spec!("pane.move_up", "move tab up", "Panes", PANES, ["alt-shift-up"], MoveTabUp),
        spec!("pane.move_down", "move tab down", "Panes", PANES, ["alt-shift-down"], MoveTabDown),
        spec!("pane.next_tab", "next tab", "Panes", PANES, ["ctrl-tab"], NextTab),
        spec!("pane.prev_tab", "previous tab", "Panes", PANES, ["ctrl-shift-tab"], PrevTab),
        spec!("pane.close_tab", "close tab", "Panes", PANES, ["ctrl-w"], CloseTab),
        spec!("pane.terminal", "new terminal", "Panes", PANES, ["ctrl-shift-t"], NewTerminal),
        spec!("pane.split_right", "split right", "Panes", PANES, ["ctrl-\\"], SplitRight),
        spec!("pane.split_down", "split down", "Panes", PANES, ["ctrl-alt-\\"], SplitDown),
        spec!("pane.collapse", "collapse", "Panes", PANES, ["ctrl-shift-h"], ToggleCollapse),
        spec!("pane.maximize", "maximize", "Panes", PANES, ["ctrl-shift-m"], ToggleMaximize),
        spec!("pane.detach", "detach", "Panes", PANES, ["ctrl-shift-d"], DetachTab),
        spec!("pane.send_tab", "send to…", "Panes", PANES, ["ctrl-shift-s"], SendTab),
        spec!("pane.send_pane", "send pane to…", "Panes", PANES, ["ctrl-alt-shift-s"], SendPane),
        spec!("pane.reattach", "re-attach", "Panes", DECK, ["ctrl-shift-a"], Reattach),
        spec!("pane.grow", "grow", "Panes", PANES, ["ctrl-alt-="], GrowPane),
        spec!("pane.shrink", "shrink", "Panes", PANES, ["ctrl-alt--"], ShrinkPane),
        spec!("pane.follow", "follow", "Panes", PANES, ["alt-f"], ToggleFollow),
        spec!("pane.menu", "menu", "Panes", DECK, ["shift-f10", "menu"], ContextMenu),
        // App view
        spec!("app_view.next", "down", "App view", Some("AppView && !TextInput"), ["down"], av::SelectNext),
        spec!("app_view.prev", "up", "App view", Some("AppView && !TextInput"), ["up"], av::SelectPrev),
        spec!("app_view.run", "run", "App view", Some("AppView && !TextInput"), ["enter"], av::RunSelected),
        spec!("app_view.stop", "stop", "App view", Some("AppView && !TextInput"), ["s"], av::StopSelected),
        spec!("app_view.restart", "restart", "App view", Some("AppView && !TextInput"), ["r"], av::RestartSelected),
        spec!("app_view.output", "output", "App view", Some("AppView && !TextInput"), ["o"], av::OpenOutput),
        spec!("app_view.hidden", "hidden commands", "App view", Some("AppView && !TextInput"), ["h"], av::ToggleHidden),
        spec!("app_view.git", "git panel", "App view", Some("AppView && !TextInput"), ["g"], av::OpenGit),
        // Run view
        spec!("run.run", "run", "Run output", Some("RunView && !TextInput"), ["enter", "ctrl-shift-enter"], rv::RunAgain),
        spec!("run.stop", "stop", "Run output", Some("RunView && !TextInput"), ["s", "ctrl-shift-x"], rv::Stop),
        spec!("run.restart", "restart", "Run output", Some("RunView && !TextInput"), ["r", "ctrl-shift-r"], rv::Restart),
        spec!("run.terminal", "type into the terminal", "Run output", Some("RunView && !TextInput"), ["i"], rv::FocusTerminal),
        spec!("run.args", "arguments", "Run output", Some("RunView && !TextInput"), ["a", "ctrl-shift-a"], rv::ChooseArgs),
        // Terminal
        spec!("term.copy", "copy the selection", "Run output", Some("RunView && !TextInput"), ["ctrl-shift-c", "ctrl-c"], term::Copy),
        spec!("term.paste", "paste", "Terminal", Some("Terminal"), ["ctrl-shift-v"], term::Paste),
        spec!("term.leave", "leave the terminal", "Terminal", Some("Terminal"), ["ctrl-shift-escape"], rv::LeaveTerminal),
        // Service
        spec!("svc.save", "save tag", "Service", Some("ServiceView"), ["ctrl-s"], sv::SaveTag),
        spec!("svc.undo", "undo rollout", "Service", Some("ServiceView && !TextInput"), ["u"], sv::Undo),
        spec!("svc.logs", "logs", "Service", Some("ServiceView && !TextInput"), ["l"], sv::OpenLogs),
        spec!("svc.tag", "edit tag", "Service", Some("ServiceView && !TextInput"), ["t"], sv::EditTag),
        spec!("svc.confirm", "confirm", "Service", Some("Modal && ServiceModal"), ["enter"], sv::ConfirmModal),
        spec!("git.next", "next branch", "Git", Some("GitView && !TextInput"), ["down"], gv::SelectNext),
        spec!("git.prev", "previous branch", "Git", Some("GitView && !TextInput"), ["up"], gv::SelectPrev),
        spec!("git.switch", "switch to branch", "Git", Some("GitView && !TextInput"), ["enter"], gv::SwitchSelected),
        spec!("git.fetch", "fetch", "Git", Some("GitView && !TextInput"), ["f"], gv::Fetch),
        spec!("git.pull", "pull", "Git", Some("GitView && !TextInput"), ["p"], gv::Pull),
        spec!("git.push", "push", "Git", Some("GitView && !TextInput"), ["shift-p"], gv::Push),
        spec!("git.new", "new branch", "Git", Some("GitView && !TextInput"), ["n"], gv::NewBranch),
        spec!("git.refresh", "refresh", "Git", Some("GitView && !TextInput"), ["r"], gv::Refresh),
        spec!("git.confirm", "confirm", "Git", Some("Modal && GitModal"), ["enter"], gv::ConfirmGit),
        spec!("lifecycle.kill_leftovers", "pick", "App", Some("Modal && Leftovers"), ["enter"], crate::lifecycle::KillLeftovers),
        spec!("ports.confirm_stop", "stop", "Ports", Some("Modal && StopHolder"), ["enter"], crate::port_scan::ConfirmStopHolder),
        // Logs
        spec!("logs.find", "find", "Logs", Some("LogsView && !TextInput"), ["/"], lv::Find),
        spec!("logs.next", "next match", "Logs", Some("LogsView && !TextInput"), ["n"], lv::NextMatch),
        spec!("logs.prev", "previous match", "Logs", Some("LogsView && !TextInput"), ["shift-n"], lv::PrevMatch),
        spec!("logs.pause", "pause", "Logs", Some("LogsView && !TextInput"), ["p"], lv::TogglePause),
        spec!("logs.errors", "errors", "Logs", Some("LogsView && !TextInput"), ["1"], lv::OnlyErrors),
        spec!("logs.warn", "warn+", "Logs", Some("LogsView && !TextInput"), ["2"], lv::WarnAndUp),
        spec!("logs.info", "info+", "Logs", Some("LogsView && !TextInput"), ["3"], lv::InfoAndUp),
        spec!("logs.all", "all levels", "Logs", Some("LogsView && !TextInput"), ["4"], lv::Everything),
        // Matrix
        spec!("matrix.clear", "clear highlight", "Branch matrix", Some("MatrixView"), ["escape"], mv::ClearHighlight),
        // Palette
        spec!("palette.down", "down", "Palette", Some("Palette"), ["down"], crate::palette::PaletteDown),
        spec!("palette.up", "up", "Palette", Some("Palette"), ["up"], crate::palette::PaletteUp),
        spec!("palette.drill", "commands", "Palette", Some("Palette"), ["tab"], crate::palette::PaletteDrill),
        // Send to
        spec!("send.1", "pane 1", "Send to", Some("SendTo"), ["1"], crate::send_to::PickTarget(1)),
        spec!("send.2", "pane 2", "Send to", Some("SendTo"), ["2"], crate::send_to::PickTarget(2)),
        spec!("send.3", "pane 3", "Send to", Some("SendTo"), ["3"], crate::send_to::PickTarget(3)),
        spec!("send.4", "pane 4", "Send to", Some("SendTo"), ["4"], crate::send_to::PickTarget(4)),
        spec!("send.5", "pane 5", "Send to", Some("SendTo"), ["5"], crate::send_to::PickTarget(5)),
        spec!("send.6", "pane 6", "Send to", Some("SendTo"), ["6"], crate::send_to::PickTarget(6)),
        spec!("send.7", "pane 7", "Send to", Some("SendTo"), ["7"], crate::send_to::PickTarget(7)),
        spec!("send.8", "pane 8", "Send to", Some("SendTo"), ["8"], crate::send_to::PickTarget(8)),
        spec!("send.9", "pane 9", "Send to", Some("SendTo"), ["9"], crate::send_to::PickTarget(9)),
        spec!("send.new", "new window", "Send to", Some("SendTo"), ["n"], crate::send_to::PickNewWindow),
        // Menus and modals
        spec!("menu.down", "down", "Menus", Some("Menu"), ["down"], ov::MenuDown),
        spec!("menu.up", "up", "Menus", Some("Menu"), ["up"], ov::MenuUp),
        spec!("menu.confirm", "pick", "Menus", Some("Menu"), ["enter"], ov::MenuConfirm),
        spec!("menu.close", "close", "Menus", Some("Menu"), ["escape"], ov::MenuClose),
        spec!("menu.open_sub", "places", "Menus", Some("Menu"), ["right"], ov::MenuOpenSub),
        spec!("menu.close_sub", "back", "Menus", Some("Menu"), ["left"], ov::MenuCloseSub),
        spec!("modal.next", "next", "Menus", Some("Modal"), ["tab"], ov::ModalNext),
        spec!("modal.prev", "previous", "Menus", Some("Modal"), ["shift-tab"], ov::ModalPrev),
        spec!("modal.close", "cancel", "Menus", Some("Modal"), ["escape"], ov::ModalClose),
        // Text inputs
        spec!("text.backspace", "delete left", "Text", TEXT, ["backspace"], ti::Backspace),
        spec!("text.delete", "delete right", "Text", TEXT, ["delete"], ti::Delete),
        spec!("text.delete_word", "delete word", "Text", TEXT, ["ctrl-backspace"], ti::DeleteWordLeft),
        spec!("text.left", "left", "Text", TEXT, ["left"], ti::Left),
        spec!("text.right", "right", "Text", TEXT, ["right"], ti::Right),
        spec!("text.select_left", "select left", "Text", TEXT, ["shift-left"], ti::SelectLeft),
        spec!("text.select_right", "select right", "Text", TEXT, ["shift-right"], ti::SelectRight),
        spec!("text.word_left", "word left", "Text", TEXT, ["ctrl-left"], ti::WordLeft),
        spec!("text.word_right", "word right", "Text", TEXT, ["ctrl-right"], ti::WordRight),
        spec!("text.select_word_left", "select word left", "Text", TEXT, ["ctrl-shift-left"], ti::SelectWordLeft),
        spec!("text.select_word_right", "select word right", "Text", TEXT, ["ctrl-shift-right"], ti::SelectWordRight),
        spec!("text.home", "start", "Text", TEXT, ["home"], ti::Home),
        spec!("text.end", "end", "Text", TEXT, ["end"], ti::End),
        spec!("text.select_home", "select to start", "Text", TEXT, ["shift-home"], ti::SelectHome),
        spec!("text.select_end", "select to end", "Text", TEXT, ["shift-end"], ti::SelectEnd),
        spec!("text.select_all", "select all", "Text", TEXT, ["ctrl-a"], ti::SelectAll),
        spec!("text.copy", "copy", "Text", TEXT, ["ctrl-c"], ti::Copy),
        spec!("text.cut", "cut", "Text", TEXT, ["ctrl-x"], ti::Cut),
        spec!("text.paste", "paste", "Text", TEXT, ["ctrl-v"], ti::Paste),
        spec!("text.undo", "undo", "Text", TEXT, ["ctrl-z"], ti::Undo),
        spec!("text.redo", "redo", "Text", TEXT, ["ctrl-shift-z"], ti::Redo),
        spec!("text.submit", "submit", "Text", TEXT, ["enter"], ti::Submit),
        spec!("text.cancel", "cancel", "Text", TEXT, ["escape"], ti::Cancel),
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

/// True when a focused terminal sends `key` (a binding's key, `ctrl-k`) to its process.
pub fn terminal_takes(key: &'static str) -> bool {
    Keystroke::parse(key).is_ok_and(|k| term::takes(&k))
}

/// Where `key` of `spec` applies: its context, outside terminals when a terminal takes it.
fn context_of(spec: &ActionSpec, key: &'static str) -> Option<String> {
    let in_terminals = spec.context.is_some_and(|c| c.contains("Terminal"));
    match (spec.context, terminal_takes(key) && !in_terminals) {
        (c, false) => c.map(String::from),
        (Some(c), true) => Some(format!("({c}) && !Terminal")),
        (None, true) => Some("!Terminal".into()),
    }
}

thread_local! {
    /// Every key's context, parsed once (GPUI predicates aren't thread-safe).
    static CONTEXTS: Vec<Vec<Option<Rc<KeyBindingContextPredicate>>>> = all()
        .iter()
        .map(|s| {
            s.keys
                .iter()
                .map(|k| context_of(s, k).map(|c| Rc::new(KeyBindingContextPredicate::parse(&c).unwrap_or_else(|e| panic!("{}: bad context {c}: {e}", s.id)))))
                .collect()
        })
        .collect();
}

/// Binds every key of every action.
pub fn bind(cx: &mut App) {
    let mut bindings = Vec::new();
    let contexts = CONTEXTS.with(|c| c.clone());
    for (spec, predicates) in all().iter().zip(contexts) {
        for (key, predicate) in spec.keys.iter().zip(predicates) {
            let b = KeyBinding::load(key, (spec.build)(), predicate, false, None, cx).unwrap_or_else(|e| panic!("{}: bad key {key}: {e:?}", spec.id));
            bindings.push(b);
        }
    }
    cx.bind_keys(bindings);
}
