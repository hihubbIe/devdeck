//! Keyboard actions. Every layout operation has one (R5.1); buttons and drags call
//! the same `Deck::apply` paths. Pane actions only fire while focus is in the pane
//! area (`Panes` key context), so they never act behind the Settings view.

use crate::shell::{self, ConfirmQuit};
use gpui::{Action, App, KeyBinding, actions};

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
    ]
);

/// Shows the workspace at this position (alphabetical, 0-based).
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = devdeck, no_json)]
pub struct SwitchWorkspace(pub usize);

pub fn bind(cx: &mut App) {
    // Pane keys never fire while typing in an input.
    let panes = Some("Panes && !TextInput");
    cx.bind_keys([
        KeyBinding::new("alt-left", FocusLeft, panes),
        KeyBinding::new("alt-right", FocusRight, panes),
        KeyBinding::new("alt-up", FocusUp, panes),
        KeyBinding::new("alt-down", FocusDown, panes),
        KeyBinding::new("alt-shift-left", MoveTabLeft, panes),
        KeyBinding::new("alt-shift-right", MoveTabRight, panes),
        KeyBinding::new("alt-shift-up", MoveTabUp, panes),
        KeyBinding::new("alt-shift-down", MoveTabDown, panes),
        KeyBinding::new("ctrl-tab", NextTab, panes),
        KeyBinding::new("ctrl-shift-tab", PrevTab, panes),
        KeyBinding::new("ctrl-w", CloseTab, panes),
        KeyBinding::new("ctrl-\\", SplitRight, panes),
        KeyBinding::new("ctrl-alt-\\", SplitDown, panes),
        KeyBinding::new("ctrl-shift-c", ToggleCollapse, panes),
        KeyBinding::new("ctrl-shift-m", ToggleMaximize, panes),
        KeyBinding::new("ctrl-shift-d", DetachTab, panes),
        KeyBinding::new("ctrl-shift-a", Reattach, None),
        KeyBinding::new("ctrl-alt-=", GrowPane, panes),
        KeyBinding::new("ctrl-alt--", ShrinkPane, panes),
        KeyBinding::new("alt-f", ToggleFollow, panes),
        KeyBinding::new("ctrl-=", IncreaseScale, None),
        KeyBinding::new("ctrl-+", IncreaseScale, None),
        KeyBinding::new("ctrl--", DecreaseScale, None),
        KeyBinding::new("ctrl-0", ResetScale, None),
        KeyBinding::new("ctrl-,", ToggleSettings, None),
        KeyBinding::new("escape", CloseSettings, Some("Settings")),
        KeyBinding::new("ctrl-q", Quit, None),
        KeyBinding::new("enter", ConfirmQuit, Some("QuitGuard")),
    ]);
    const WORKSPACE_KEYS: [&str; 9] =
        ["ctrl-alt-1", "ctrl-alt-2", "ctrl-alt-3", "ctrl-alt-4", "ctrl-alt-5", "ctrl-alt-6", "ctrl-alt-7", "ctrl-alt-8", "ctrl-alt-9"];
    cx.bind_keys(WORKSPACE_KEYS.iter().enumerate().map(|(i, k)| KeyBinding::new(k, SwitchWorkspace(i), None)));
    // App level, so they work from any window, detached ones included.
    cx.on_action(|_: &Quit, cx| shell::request_quit(cx));
    cx.on_action(|a: &SwitchWorkspace, cx| shell::switch(a.0, cx));
}
