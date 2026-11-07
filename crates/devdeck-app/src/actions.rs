//! Keyboard actions. Every layout operation has one (R5.1); buttons and drags call
//! the same `Deck::apply` paths.

use gpui::{App, KeyBinding, actions};

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
        Quit,
    ]
);

pub fn bind(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("alt-left", FocusLeft, None),
        KeyBinding::new("alt-right", FocusRight, None),
        KeyBinding::new("alt-up", FocusUp, None),
        KeyBinding::new("alt-down", FocusDown, None),
        KeyBinding::new("alt-shift-left", MoveTabLeft, None),
        KeyBinding::new("alt-shift-right", MoveTabRight, None),
        KeyBinding::new("alt-shift-up", MoveTabUp, None),
        KeyBinding::new("alt-shift-down", MoveTabDown, None),
        KeyBinding::new("ctrl-tab", NextTab, None),
        KeyBinding::new("ctrl-shift-tab", PrevTab, None),
        KeyBinding::new("ctrl-w", CloseTab, None),
        KeyBinding::new("ctrl-\\", SplitRight, None),
        KeyBinding::new("ctrl-alt-\\", SplitDown, None),
        KeyBinding::new("ctrl-shift-c", ToggleCollapse, None),
        KeyBinding::new("ctrl-shift-m", ToggleMaximize, None),
        KeyBinding::new("ctrl-shift-d", DetachTab, None),
        KeyBinding::new("ctrl-shift-a", Reattach, None),
        KeyBinding::new("ctrl-alt-=", GrowPane, None),
        KeyBinding::new("ctrl-alt--", ShrinkPane, None),
        KeyBinding::new("alt-f", ToggleFollow, None),
        KeyBinding::new("ctrl-q", Quit, None),
    ]);
}
