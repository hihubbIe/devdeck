//! "Open in" (R6.13, R6.14): one item in every menu that opens something, and one modal
//! (`devdeck_ui::chooser`) to say what and where, never a submenu. WHAT: the thing itself
//! (from a full view: a list row, the details' header), then, with a folder, a shell there
//! and a session of each agent of the machine. WHERE: Terminals or Agents first (their own
//! place in the app), then the pulses from a full view, or this pulse's panes and a new
//! window from a tab or a pane: a pane is only offered inside a pulse.

use crate::deck::Deck;
use crate::places::{self, Place};
use crate::sidebar::{Activity, Sidebar};
use devdeck_core::layout::{MAIN_WINDOW, PaneId, ViewRef, WindowId};
use devdeck_core::view_addr::ViewAddr;
use devdeck_ui::{Choice, ChoiceRow, ChooserSpec, MenuItem, ToastKind, open_chooser, toasts};
use gpui::{App, Entity, Window};
use std::path::PathBuf;
use std::rc::Rc;

/// Where the menu was opened: a full view, or a pane (of a window) of the pulse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenedFrom {
    Outside,
    Pane(PaneId, WindowId),
}

/// What the menu was opened on: the view itself, its own folder.
#[derive(Clone, Debug, Default)]
pub struct Subject {
    pub view: Option<ViewRef>,
    pub dir: Option<PathBuf>,
}

impl Subject {
    pub fn view(view: ViewRef, dir: Option<PathBuf>) -> Self {
        Self { view: Some(view), dir }
    }

    pub fn folder(dir: PathBuf) -> Self {
        Self { view: None, dir: Some(dir) }
    }
}

/// WHAT values: the view, a shell, `agent:<id>`.
const VIEW: &str = "view";
const TERMINAL: &str = "terminal";
const AGENT: &str = "agent:";
/// WHERE values: Terminals or Agents, `pulse:<id>`, `pane:<id>`, a new window.
const HOME: &str = "home";
const PULSE: &str = "pulse:";
const PANE: &str = "pane:";
const NEW_WINDOW: &str = "window";

/// The menu item opening it; `None` when there's nothing to open.
pub fn item(deck: &Entity<Deck>, subject: Subject, from: OpenedFrom, cx: &App) -> Option<MenuItem> {
    if choices(deck, &subject, from, cx).is_empty() {
        return None;
    }
    let deck = deck.clone();
    Some(MenuItem::new("Open in", move |window, cx| open(&deck, subject.clone(), from, window, cx)))
}

/// Opens the chooser; the first WHERE selected is the current pulse, or the menu's pane.
pub fn open(deck: &Entity<Deck>, subject: Subject, from: OpenedFrom, window: &mut Window, cx: &mut App) {
    let choices = choices(deck, &subject, from, cx);
    if choices.is_empty() {
        return;
    }
    let start_where = Some(match from {
        OpenedFrom::Outside => format!("{PULSE}{}", deck.read(cx).book.current),
        OpenedFrom::Pane(p, _) => format!("{PANE}{}", p.0),
    });
    let title = match &subject.view {
        Some(v) if from == OpenedFrom::Outside => format!("Open {} in", deck.read(cx).title(v, cx)),
        _ => "Open in".to_string(),
    };
    let (d, s) = (deck.clone(), subject);
    let spec = ChooserSpec {
        title: title.into(),
        what_title: "WHAT".into(),
        where_title: "WHERE".into(),
        choices,
        confirm: "Open".into(),
        start_where,
        on_choose: Rc::new(move |what, place, window, cx| choose(&d, &s, from, &what, &place, window, cx)),
    };
    open_chooser(spec, window, cx);
}

/// The folder a shell or a session starts in: the subject's own, and inside a pulse the
/// pane's as a new terminal's (`Deck::terminal_folder`).
fn folder(deck: &Entity<Deck>, subject: &Subject, from: OpenedFrom, cx: &App) -> Option<PathBuf> {
    match from {
        OpenedFrom::Outside => subject.dir.clone(),
        OpenedFrom::Pane(p, _) => Some(subject.dir.clone().unwrap_or_else(|| deck.read(cx).terminal_folder(Some(p), cx))),
    }
}

/// What the chooser lists, each WHAT with its WHERE.
pub fn choices(deck: &Entity<Deck>, subject: &Subject, from: OpenedFrom, cx: &App) -> Vec<Choice> {
    let d = deck.read(cx);
    let targets: Vec<ChoiceRow> = match from {
        OpenedFrom::Outside => d
            .book
            .entries()
            .into_iter()
            .map(|(id, name)| {
                let row = ChoiceRow::new(format!("{PULSE}{id}"), name);
                if id == d.book.current { row.detail("current") } else { row }
            })
            .collect(),
        OpenedFrom::Pane(..) => places::panes(d.layout(), |r| d.title(r, cx))
            .into_iter()
            .enumerate()
            .map(|(i, (pane, name))| ChoiceRow::new(format!("{PANE}{}", pane.0), format!("{} · {name}", i + 1)))
            .chain(std::iter::once(ChoiceRow::new(NEW_WINDOW, "new window")))
            .collect(),
    };
    let mut out = Vec::new();
    // Inside a pulse the view is there already: Send to moves it.
    if from == OpenedFrom::Outside
        && let Some(v) = &subject.view
    {
        out.push(Choice { what: ChoiceRow::new(VIEW, d.title(v, cx)), wheres: targets.clone() });
    }
    if folder(deck, subject, from, cx).is_some() {
        let home = |name: &str| std::iter::once(ChoiceRow::new(HOME, name.to_string()));
        out.push(Choice { what: ChoiceRow::new(TERMINAL, "Terminal"), wheres: home("Terminals").chain(targets.iter().cloned()).collect() });
        for kind in crate::environment::agent_kinds(cx) {
            out.push(Choice { what: ChoiceRow::new(format!("{AGENT}{}", kind.id), kind.name), wheres: home("Agents").chain(targets.iter().cloned()).collect() });
        }
    }
    out
}

/// Does what was chosen: `what` and `place` are the chooser's values.
pub fn choose(deck: &Entity<Deck>, subject: &Subject, from: OpenedFrom, what: &str, place: &str, window: &mut Window, cx: &mut App) {
    if what == VIEW {
        if let (Some(view), Some(id)) = (subject.view.clone(), place.strip_prefix(PULSE)) {
            into_pulse(deck, id, view, cx);
        }
        return;
    }
    let Some(dir) = folder(deck, subject, from, cx) else { return };
    let agent = what.strip_prefix(AGENT);
    if place == HOME {
        match agent {
            None => terminal_home(deck, &dir, cx),
            Some(a) => crate::sidebar::new_agent_session(deck, a, &dir, cx),
        }
        // Terminals and Agents are the main window's: brought forward from a detached one.
        if let OpenedFrom::Pane(_, w) = from
            && w != MAIN_WINDOW
            && let Some(main) = crate::shell::try_shell(cx).and_then(|s| s.read(cx).main_window())
        {
            cx.defer(move |cx| {
                let _ = main.update(cx, |_, window, _| window.activate_window());
            });
        }
        return;
    }
    if let Some(id) = place.strip_prefix(PULSE) {
        let Some(view) = new_view(deck, agent, &dir, cx) else { return };
        into_pulse(deck, id, view, cx);
        return;
    }
    let at = match place.strip_prefix(PANE).and_then(|p| p.parse().ok()) {
        Some(p) => Place::Pane(PaneId(p)),
        None => Place::NewWindow,
    };
    let Some(pane) = deck.update(cx, |d, cx| match agent {
        None => d.open_terminal(&dir, at, cx),
        Some(a) => d.open_agent(a, &dir, at, cx),
    }) else {
        return;
    };
    let shown = deck.read(cx).layout().window_of(pane).unwrap_or(MAIN_WINDOW);
    // A main-window pane while the main window shows a full view: Pulses show.
    if shown == MAIN_WINDOW
        && let Some(s) = sidebar_of(deck, cx)
        && s.read(cx).activity().full_view()
    {
        s.update(cx, |s, cx| s.show_pulses(cx));
    }
    let here = match from {
        OpenedFrom::Pane(_, w) => w,
        OpenedFrom::Outside => MAIN_WINDOW,
    };
    if shown == here
        && let Some(f) = deck.read(cx).view_focus(pane)
    {
        f.focus(window, cx);
    }
}

/// A new shell in `dir` shown in Terminals: known to the runner and selected there in one
/// go, so no sync forgets it before its view starts it.
fn terminal_home(deck: &Entity<Deck>, dir: &std::path::Path, cx: &mut App) {
    let Some(services) = deck.read(cx).services() else { return };
    let n = deck.read(cx).next_terminal(cx);
    let runner = services.read(cx).runner.clone();
    runner.update(cx, |r, _| {
        r.open_terminal(n, dir);
    });
    if let Some(s) = sidebar_of(deck, cx) {
        s.update(cx, |s, cx| s.show_item(Activity::Terminals, &n.to_string(), cx));
    }
}

/// A new shell (`agent` `None`) or agent session in `dir`, as a view to open.
fn new_view(deck: &Entity<Deck>, agent: Option<&str>, dir: &std::path::Path, cx: &mut App) -> Option<ViewRef> {
    let n = deck.read(cx).next_terminal(cx);
    Some(match agent {
        None => ViewAddr::Terminal { n, dir: dir.display().to_string() }.to_ref(),
        Some(a) => {
            let agents = deck.read(cx).services()?.read(cx).agents.clone();
            let id = agents.update(cx, |s, cx| s.create(a, dir, n, cx));
            ViewAddr::Agent { id }.to_ref()
        }
    })
}

/// `view` into pulse `id`'s first pane ([`Deck::open_in_pulse`]); a toast says where.
fn into_pulse(deck: &Entity<Deck>, id: &str, view: ViewRef, cx: &mut App) {
    let title = deck.read(cx).title(&view, cx);
    let done = deck.update(cx, |d, cx| d.open_in_pulse(id, view, cx));
    let (kind, text) = match done {
        Ok(name) => (ToastKind::Info, format!("{title} opened in {name}")),
        Err(why) => (ToastKind::Warn, format!("{title} wasn't opened: {why}")),
    };
    toasts(cx).update(cx, |t, cx| t.push(kind, text, vec![], cx));
}

/// The sidebar of `deck`'s workspace.
fn sidebar_of(deck: &Entity<Deck>, cx: &App) -> Option<Entity<Sidebar>> {
    crate::shell::try_shell(cx).and_then(|s| s.read(cx).workspaces.iter().find(|w| w.deck == *deck).map(|w| w.sidebar.clone()))
}
