//! The method catalog: one type per method, naming it, its class and what it takes and
//! gives. The server builds its handlers from the same list ([`visit_all`]), so a method
//! can't be listed without being served (a test in the app checks it).

use crate::types::*;
use schemars::JsonSchema;
use serde::Serialize;
use serde::de::DeserializeOwned;

pub trait Method: 'static {
    /// `domain.verb`.
    const NAME: &'static str;
    /// One line, for people and agents choosing a method.
    const SUMMARY: &'static str;
    /// Its class for any target.
    const CLASS: Class;
    /// Classes it rises to with some targets (stopping a foreign process).
    const ESCALATES_TO: &'static [Class] = &[];
    /// The API level it appeared at.
    const SINCE: u32 = 1;
    /// May still change; outside the "only grows" promise.
    const EXPERIMENTAL: bool = false;
    type Params: Serialize + DeserializeOwned + JsonSchema;
    type Result: Serialize + DeserializeOwned + JsonSchema;
}

/// Something done for every method, typed (the server registering handlers, the catalog).
pub trait Visit {
    fn visit<M: Method>(&mut self);
}

macro_rules! methods {
    (@since) => { 1 };
    (@since $since:literal) => { $since };
    ($( $(#[$doc:meta])* $ty:ident = $name:literal, $class:ident, $params:ty => $result:ty, $summary:literal $(, since $since:literal)?; )*) => {
        $(
            $(#[$doc])*
            pub struct $ty;
            impl Method for $ty {
                const NAME: &'static str = $name;
                const SUMMARY: &'static str = $summary;
                const CLASS: Class = Class::$class;
                const SINCE: u32 = methods!(@since $($since)?);
                type Params = $params;
                type Result = $result;
            }
        )*

        /// Calls `v` for every method.
        pub fn visit_all(v: &mut impl Visit) {
            $( v.visit::<$ty>(); )*
        }
    };
}

methods! {
    SessionHello = "session.hello", Observe, HelloParams => Hello,
        "Introduces the client: the API level, who devdeck takes the caller for, and what it may do per class.";
    SessionWhoami = "session.whoami", Observe, NoParams => Whoami,
        "Who devdeck takes the caller for: its terminal, workspace, and where that terminal shows.";
    ApiDescribe = "api.describe", Observe, DescribeParams => Describe,
        "Every method with its class and whether the caller may use it; give `method` for its parameter and result schemas.";
    DevdeckInfo = "devdeck.info", Observe, NoParams => Info,
        "devdeck's version, API level, instance and process.";
    DevdeckProblems = "devdeck.problems", Observe, InWorkspace => Problems,
        "What discovery, the config and the machine reported for the workspace.";
    WorkspaceList = "workspace.list", Observe, NoParams => Workspaces,
        "The workspaces the caller can see.";
    WorkspaceGet = "workspace.get", Observe, InWorkspace => Workspace,
        "A workspace: folders, envs, current env.";
    AppList = "app.list", Observe, InWorkspace => Apps,
        "The workspace's apps, with whether they run and can be used.";
    AppGet = "app.get", Observe, AppParams => App,
        "An app: its commands (id, line, ports, running), links, why it can't be used, notes count.";
    RunList = "run.list", Observe, InWorkspace => Runs,
        "The workspace's runs (commands started at least once), with their state.";
    RunGet = "run.get", Observe, RunParams => Run,
        "A run: state, exit, env, command line, arguments, process, ports.";
    TerminalList = "terminal.list", Observe, InWorkspace => Terminals,
        "The workspace's terminals (shells): folder, running, program in the foreground.";
    TerminalGet = "terminal.get", Observe, TerminalParams => Terminal,
        "A terminal, and where it shows.";
    UiGet = "ui.get", Observe, InWorkspace => Ui,
        "What's on screen: windows, the pane tree, tabs with their view and title, focused and maximized panes.";
    LayoutList = "layout.list", Observe, InWorkspace => Layouts,
        "The workspace's saved layouts, and which is current.";
    LayoutGet = "layout.get", Observe, IdParams => Layout,
        "A layout's windows, panes and tabs.";
    NoteList = "note.list", Observe, NoteListParams => Notes,
        "The workspace's notes (names only), or one app's.";
    NoteGet = "note.get", Observe, IdParams => Note,
        "A note's name and app.";
    EnvList = "env.list", Observe, InWorkspace => Envs,
        "The workspace's envs, protected or not, and the current one.";
    EnvGet = "env.get", Observe, EnvParams => Env,
        "An env.";
    EditModeGet = "editmode.get", Observe, NoParams => EditMode,
        "Whether edit mode is on. Only the user turns it on.";
    SettingsGet = "settings.get", Observe, NoParams => Settings,
        "UI scale and theme.";
    RunStart = "run.start", Run, RunStartParams => Run,
        "Starts one of an app's main commands with the arguments the user set; already running, nothing changes.", since 2;
    RunStop = "run.stop", Run, RunParams => Run,
        "Stops a run the caller started (the user's runs need the user's approval).", since 2;
    RunRestart = "run.restart", Run, RunStartParams => Run,
        "Restarts a run the caller started, or starts it when it isn't running.", since 2;
    RunOutput = "run.output", Read, OutputParams => Output,
        "A run's output as lines: the last ones, or what came after `since` (the `next` of an earlier answer).", since 3;
    RunSearch = "run.search", Read, SearchParams => Matches,
        "Lines of a run's output matching a regular expression, with context.", since 3;
    TerminalOutput = "terminal.output", Read, TerminalOutputParams => Output,
        "A terminal's output as lines: the last ones, or what came after `since`.", since 3;
    TerminalSearch = "terminal.search", Read, TerminalSearchParams => Matches,
        "Lines of a terminal's output matching a regular expression, with context.", since 3;
    TerminalScreen = "terminal.screen", Read, TerminalParams => Screen,
        "A terminal's screen as shown (a full-screen program's), and its cursor.", since 3;
    NoteRead = "note.read", Read, IdParams => NoteText,
        "A note's text (Markdown).", since 3;
    FileRead = "file.read", Read, FileParams => FileLines,
        "Lines of a text file in one of the workspace's folders.", since 3;
    RunWait = "run.wait", Read, RunWaitParams => Waited,
        "Waits until a run exits or writes a matching line, 50 s at most; then its state and what it wrote meanwhile, or a job to wait on.", since 4;
    TerminalWait = "terminal.wait", Read, TerminalWaitParams => Waited,
        "Waits until the caller's terminal is idle or writes a matching line, 50 s at most; then its state and what it wrote, or a job.", since 4;
    JobGet = "job.get", Read, JobParams => Waited,
        "A job's wait, looked at now without waiting.", since 4;
    JobWait = "job.wait", Read, JobWaitParams => Waited,
        "Waits on a job again, 50 s at most.", since 4;
    JobCancel = "job.cancel", Read, JobParams => Cancelled,
        "Forgets a job.", since 4;
    JobList = "job.list", Read, NoParams => Jobs,
        "The caller's jobs, oldest first.", since 4;
    EventsSubscribe = "events.subscribe", Observe, TopicsParams => Subscribed,
        "Follows topics on this connection: devdeck sends `event` notifications saying what changed (never its content).", since 4;
    EventsUnsubscribe = "events.unsubscribe", Observe, TopicsParams => Subscribed,
        "Stops following topics.", since 4;
    GitRepos = "git.repos", Observe, InWorkspace => Repos,
        "The repositories of the workspace's apps.", since 4;
    GitStatus_ = "git.status", Observe, RepoParams => GitStatus,
        "A repository's branch, upstream, operation under way and changes (names that may hold secrets hidden).", since 4;
    GitBranches = "git.branches", Observe, BranchesParams => Branches,
        "A repository's branches, remote ones when asked.", since 4;
    PortList_ = "port.list", Observe, InWorkspace => PortList,
        "The workspace's ports: those its runs listen on and its commands declare, and who holds each.", since 4;
    PortGet = "port.get", Observe, PortParams => PortEntry,
        "One of the workspace's ports and who holds it.", since 4;
    ViewOpen = "view.open", Present, ViewOpenParams => Ui,
        "Opens an app, a run, a file, a note or a port into a pane or beside it, in the background unless `activate`; never moves the user's keys.", since 4;
    ViewClose = "view.close", Present, ViewParams => Ui,
        "Closes a view's tab (a terminal's shell goes on).", since 4;
    ViewFocus = "view.focus", Present, ViewParams => Ui,
        "Makes a view its pane's active tab (never the keyboard's).", since 4;
    FileOpen = "file.open", Present, FileOpenParams => Ui,
        "Opens a file of the workspace's apps' folders, at a line when given.", since 4;
    FileRendered = "file.rendered", Present, RenderedParams => Done,
        "Shows a Markdown file or a note rendered, or its source.", since 4;
    PaneSplit = "pane.split", Present, PaneSplitParams => Ui,
        "Moves a pane's active tab to a new pane on one of its edges.", since 4;
    PaneClose = "pane.close", Present, PaneParams => Ui,
        "Closes a pane's tabs (terminals' shells go on).", since 4;
    PaneCollapse = "pane.collapse", Present, PaneCollapseParams => Ui,
        "Collapses or expands a pane.", since 4;
    PaneMaximize = "pane.maximize", Present, PaneMaximizeParams => Ui,
        "Maximizes a pane in its window, or not.", since 4;
    PaneResize = "pane.resize", Present, PaneResizeParams => Ui,
        "Moves a boundary of a split.", since 4;
    PaneMoveTab = "pane.move_tab", Present, MoveTabParams => Ui,
        "Moves an open view into a pane or beside it.", since 4;
    WindowAttach = "window.attach", Present, WindowAttachParams => Ui,
        "Moves a detached window's tabs into a pane and closes it.", since 4;
    WindowClose = "window.close", Present, WindowParams => Ui,
        "Closes a detached window and its tabs (terminals' shells go on).", since 4;
    LayoutSwitch = "layout.switch", Present, IdParams => Ui,
        "Shows another saved layout (asks the user while their keys are in a pane).", since 4;
    SettingsSet = "settings.set", Present, SettingsSetParams => Settings,
        "Sets the UI scale or the theme.", since 4;
    WorkspaceRescan = "workspace.rescan", Present, InWorkspace => Done,
        "Finds the workspace's apps again.", since 4;
    UiNotify = "ui.notify", Present, NotifyParams => Notified,
        "Shows the user a one-line toast, labelled with the caller.", since 4;
    UiToasts = "ui.toasts", Read, NoParams => Toasts,
        "The toasts shown lately, oldest first.", since 4;
}

/// A method as the catalog lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct MethodInfo {
    pub name: &'static str,
    pub summary: &'static str,
    pub class: Class,
    pub escalates_to: &'static [Class],
    pub since: u32,
    pub experimental: bool,
    pub params: serde_json::Value,
    pub result: serde_json::Value,
}

impl MethodInfo {
    pub fn of<M: Method>() -> Self {
        let schema = |s: schemars::Schema| serde_json::to_value(s).unwrap_or_default();
        Self {
            name: M::NAME,
            summary: M::SUMMARY,
            class: M::CLASS,
            escalates_to: M::ESCALATES_TO,
            since: M::SINCE,
            experimental: M::EXPERIMENTAL,
            params: schema(schemars::schema_for!(M::Params)),
            result: schema(schemars::schema_for!(M::Result)),
        }
    }

    /// Every class it may be called as: its own and those it rises to.
    pub fn classes(&self) -> impl Iterator<Item = Class> + '_ {
        std::iter::once(self.class).chain(self.escalates_to.iter().copied())
    }
}

/// Every method, alphabetical by name.
pub fn catalog() -> Vec<MethodInfo> {
    struct Collect(Vec<MethodInfo>);
    impl Visit for Collect {
        fn visit<M: Method>(&mut self) {
            self.0.push(MethodInfo::of::<M>());
        }
    }
    let mut c = Collect(Vec::new());
    visit_all(&mut c);
    c.0.sort_by_key(|m| m.name);
    c.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalog_is_well_formed() {
        let all = catalog();
        let mut names: Vec<&str> = all.iter().map(|m| m.name).collect();
        names.dedup();
        assert_eq!(names.len(), all.len(), "names are unique");
        for m in &all {
            let (domain, verb) = m.name.split_once('.').expect("domain.verb");
            assert!(!domain.is_empty() && !verb.is_empty() && !verb.contains('.'), "{}", m.name);
            assert!(m.name.chars().all(|c| c.is_ascii_lowercase() || c == '.' || c == '_'), "{}", m.name);
            assert_ne!(m.class, Class::Unknown, "{}", m.name);
            assert!(!m.escalates_to.contains(&m.class) && !m.escalates_to.contains(&Class::Unknown), "{}", m.name);
            assert!(m.since >= 1 && m.since <= crate::API_LEVEL, "{}", m.name);
            assert!(m.summary.ends_with('.'), "{}: a sentence", m.name);
            assert_eq!(m.params["type"], "object", "{}: parameters are an object", m.name);
            assert_eq!(m.result["type"], "object", "{}: results are an object, so they can grow", m.name);
        }
    }

    #[test]
    fn methods_look_or_run_and_levels_say_when() {
        for m in catalog() {
            assert!(matches!(m.class, Class::Observe | Class::Run | Class::Read | Class::Present) && m.escalates_to.is_empty(), "{}", m.name);
            let since = match m.class {
                Class::Run => 2,
                Class::Read if m.name.ends_with(".wait") || m.name.starts_with("job.") || m.name == "ui.toasts" => 4,
                Class::Read => 3,
                _ if ["events.", "git.", "port."].iter().any(|d| m.name.starts_with(d)) => 4,
                Class::Present => 4,
                _ => 1,
            };
            assert_eq!(m.since, since, "{}", m.name);
        }
    }
}
