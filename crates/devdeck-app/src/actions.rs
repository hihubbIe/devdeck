//! App and pane actions. Every layout operation has one (R5.1); buttons and drags call
//! the same `Deck::apply` paths. Labels, groups, contexts and keys of every action live
//! in the [registry](crate::registry).

use crate::shell;
use gpui::{Action, App, actions};

actions!(
    devdeck,
    [
        FocusLeft,
        FocusRight,
        FocusUp,
        FocusDown,
        MoveTabLeft,
        MoveTabRight,
        MoveTabUp,
        MoveTabDown,
        NextTab,
        PrevTab,
        CloseTab,
        SplitRight,
        SplitDown,
        ToggleCollapse,
        ToggleMaximize,
        DetachTab,
        Reattach,
        GrowPane,
        ShrinkPane,
        ToggleFollow,
        IncreaseScale,
        DecreaseScale,
        ResetScale,
        ToggleSettings,
        CloseSettings,
        Quit,
        ShowApps,
        ShowCluster,
        ShowPorts,
        ToggleSidebar,
        OpenEnvMenu,
        Palette,
        CheatSheet,
        SendTab,
        SendPane,
        ContextMenu,
        ShowLayouts,
        SaveLayoutAs,
    ]
);

/// Shows the workspace at this position (alphabetical, 0-based).
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = devdeck, no_json)]
pub struct SwitchWorkspace(pub usize);

/// App-level handlers, so these work from any window (detached ones included). Keys
/// come from the registry.
pub fn bind(cx: &mut App) {
    crate::registry::bind(cx);
    cx.on_action(|_: &Quit, cx| shell::request_quit(cx));
    cx.on_action(|a: &SwitchWorkspace, cx| shell::switch(a.0, cx));
}
