//! Local apps and their commands (R6). Discovery fills these from the project folders;
//! the demo builds them by hand.

use std::collections::BTreeSet;
use std::path::PathBuf;

/// How prominent a command is (R6.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    Primary,
    Secondary,
    Hidden,
}

impl Tier {
    pub const ALL: [Tier; 3] = [Tier::Primary, Tier::Secondary, Tier::Hidden];

    pub fn title(self) -> &'static str {
        match self {
            Tier::Primary => "Primary",
            Tier::Secondary => "Secondary",
            Tier::Hidden => "Hidden",
        }
    }

    /// The tier a command name gets by default: servers first, chores hidden.
    /// `siblings` are the other names from the same source: `pre<x>` and `post<x>` are
    /// hooks (hidden) only when `<x>` is one of them, so `preview` stays visible.
    pub fn of(name: &str, siblings: &[&str]) -> Tier {
        let base = name.split(':').next().unwrap_or(name);
        let hidden = ["lint", "format", "fmt", "prettier", "eslint", "prepare", "clean", "postinstall", "preinstall"];
        let hook = |prefix: &str| base.strip_prefix(prefix).is_some_and(|x| !x.is_empty() && siblings.contains(&x));
        if hidden.contains(&base) || hook("pre") || hook("post") {
            Tier::Hidden
        } else if ["start", "dev", "serve", "run", "watch", "up"].contains(&base) {
            Tier::Primary
        } else {
            Tier::Secondary
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Command {
    /// Unique within the app and stable (stored in layouts): `source:name`, where the
    /// source is where the command was found (`task:run`, `pnpm:start:dev`, `go:run`).
    pub id: String,
    /// What runs (`pnpm run start:dev`).
    pub line: String,
    pub tier: Tier,
    /// Programs it needs on the PATH (R6.5).
    pub tools: Vec<String>,
    /// Programs its script runs that the project's install provides and that weren't
    /// installed when it was found: its toolchain checks them before a run.
    pub from_install: Vec<String>,
    /// It sets the project up (installs it) rather than using it: an app whose only
    /// runnable commands are these still can't be used.
    pub setup: bool,
    /// Ports it listens on when running (R6.9).
    pub ports: Vec<u16>,
}

impl Command {
    /// The name shown (`start:dev`): the id without its source.
    pub fn name(&self) -> &str {
        name_of(&self.id)
    }

    /// Where it was found (`pnpm`).
    pub fn source(&self) -> &str {
        self.id.split_once(':').map_or("", |(s, _)| s)
    }
}

/// The shown name of a command id (`pnpm:start:dev` → `start:dev`).
pub fn name_of(id: &str) -> &str {
    id.split_once(':').map_or(id, |(_, n)| n)
}

/// How the workspace env reaches a run (R6.6). A command line may also hold `{env}`
/// itself (an override), which is replaced whatever the binding.
#[derive(Clone, Debug, PartialEq)]
pub enum EnvApply {
    /// Appended to the command line with `{env}` replaced (`-s {env}`). Preferred:
    /// nothing is written to disk.
    Argument(String),
    /// Sets an environment variable to the env name.
    Variable(String),
    /// Rewrites a gitignored file before the run (declared; not applied yet).
    FileRewrite(String),
    /// The app doesn't use an env.
    None,
}

/// A command line ready to run for one env, and the variables to set.
#[derive(Clone, Debug, PartialEq)]
pub struct Applied {
    pub line: String,
    pub vars: Vec<(String, String)>,
}

impl EnvApply {
    /// What the UI says about it, for one env.
    pub fn describe(&self, env: &str) -> String {
        match self {
            EnvApply::Variable(v) => format!("{v}={env}"),
            EnvApply::Argument(a) => a.replace("{env}", env),
            EnvApply::FileRewrite(f) => format!("rewrites {f} for {env}"),
            EnvApply::None => "no env".into(),
        }
    }

    /// `line` for `env`: `{env}` replaced, the argument appended or the variable set.
    /// Without an env, `line` is unchanged (an unreplaced `{env}` stays, so the run
    /// fails visibly rather than with an empty argument).
    pub fn apply(&self, line: &str, env: Option<&str>) -> Applied {
        let Some(env) = env else { return Applied { line: line.into(), vars: Vec::new() } };
        let mut line = line.replace("{env}", env);
        let mut vars = Vec::new();
        match self {
            EnvApply::Argument(a) => {
                line.push(' ');
                line.push_str(&a.replace("{env}", env));
            }
            EnvApply::Variable(v) => vars.push((v.clone(), env.to_string())),
            EnvApply::FileRewrite(_) | EnvApply::None => {}
        }
        Applied { line, vars }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct App {
    /// Stable (stored in layouts and pins): the app's folder relative to the workspace
    /// folder it was found in (`front-end/mastertadaapp`), never the display name.
    pub id: String,
    pub name: String,
    /// As shown (`~/Documents/repos/x`).
    pub folder: String,
    /// Where its commands run.
    pub dir: PathBuf,
    /// `go`, `pnpm`, `cargo`…
    pub kind: String,
    /// A library rather than something to run: listed, de-emphasized (R6.2).
    pub library: bool,
    /// What its toolchains keep about it (Node: `.nvmrc`, `engines.node`).
    pub facts: crate::toolchains::Facts,
    pub commands: Vec<Command>,
    pub env: EnvApply,
    /// Links to open (R6.4).
    pub urls: Vec<String>,
    /// Image names it's deployed as (its CI's `IMAGE_NAME`, the config's `deployments`):
    /// links a cluster's service to it (M3).
    pub images: Vec<String>,
}

impl App {
    pub fn command(&self, id: &str) -> Option<&Command> {
        self.commands.iter().find(|c| c.id == id)
    }

    /// Commands of one tier, alphabetical.
    pub fn tier(&self, tier: Tier) -> Vec<&Command> {
        let mut v: Vec<&Command> = self.commands.iter().filter(|c| c.tier == tier).collect();
        v.sort_by(|a, b| a.id.cmp(&b.id));
        v
    }
}

/// Which programs runs can use (R6.5): a fixed list (the demo), or the toolbox found on
/// this machine, which also knows each app's Node.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tools {
    pub found: BTreeSet<String>,
    /// When set, it decides (`found` is ignored).
    pub toolbox: Option<crate::environment::Toolbox>,
    /// The toolbox is still being found: nothing can run yet.
    pub pending: bool,
}

impl Tools {
    pub fn new(found: &[&str]) -> Self {
        Self { found: found.iter().map(|s| s.to_string()).collect(), toolbox: None, pending: false }
    }

    /// Tools still being found (the login shell is being asked).
    pub fn pending() -> Self {
        Self { pending: true, ..Self::default() }
    }

    /// Why a command of `app` can't run, if it can't (`go not found`).
    pub fn missing(&self, app: &App, cmd: &Command) -> Option<String> {
        if self.pending {
            return Some("checking tools…".into());
        }
        match &self.toolbox {
            Some(tb) => tb.missing(app, cmd),
            None => cmd.tools.iter().find(|t| !self.found.contains(*t)).map(|t| format!("{t} not found")),
        }
    }

    /// Why an app can't be used at all: none of its visible (not hidden) commands can
    /// run; the reason of the first one. Setting it up is the remedy, not a use: setup
    /// commands don't count.
    pub fn unusable(&self, app: &App) -> Option<String> {
        let visible: Vec<&Command> = app.commands.iter().filter(|c| c.tier != Tier::Hidden && !c.setup).collect();
        if visible.is_empty() {
            return None;
        }
        visible.iter().map(|c| self.missing(app, c)).collect::<Option<Vec<_>>>().and_then(|v| v.into_iter().next())
    }

    /// Every reason a command of `app` can't run, alphabetical and without repeats.
    pub fn missing_for(&self, app: &App) -> Vec<String> {
        let set: BTreeSet<String> = app.commands.iter().filter_map(|c| self.missing(app, c)).collect();
        set.into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(id: &str, tools: &[&str]) -> Command {
        Command { id: id.into(), line: id.into(), tier: Tier::of(name_of(id), &[]), tools: tools.iter().map(|t| t.to_string()).collect(), from_install: vec![], setup: false, ports: vec![] }
    }

    fn app(kind: &str, commands: Vec<Command>, env: EnvApply) -> App {
        App { id: "a".into(), name: "a".into(), folder: "/a".into(), dir: "/a".into(), kind: kind.into(), library: false, facts: Default::default(), commands, env, urls: vec![], images: vec![] }
    }

    #[test]
    fn tiers_from_names() {
        let scripts = ["build", "test", "prebuild", "preview", "postbuild"];
        assert_eq!(Tier::of("start:dev", &scripts), Tier::Primary);
        assert_eq!(Tier::of("dev", &scripts), Tier::Primary);
        assert_eq!(Tier::of("up", &scripts), Tier::Primary, "docker compose up");
        assert_eq!(Tier::of("build", &scripts), Tier::Secondary);
        assert_eq!(Tier::of("test:unit", &scripts), Tier::Secondary);
        assert_eq!(Tier::of("lint", &scripts), Tier::Hidden);
        assert_eq!(Tier::of("prebuild", &scripts), Tier::Hidden, "a hook of build");
        assert_eq!(Tier::of("postbuild", &scripts), Tier::Hidden);
        assert_eq!(Tier::of("postinstall", &scripts), Tier::Hidden);
        assert_eq!(Tier::of("preview", &scripts), Tier::Secondary, "no `view` script: not a hook");
        assert_eq!(Tier::of("pre", &scripts), Tier::Secondary, "not a pre-hook by itself");
    }

    #[test]
    fn an_app_whose_setup_alone_can_run_is_unusable() {
        use std::os::unix::fs::PermissionsExt;
        let dir = crate::testing::TempDir::new("unusable");
        for p in ["npm", "node"] {
            std::fs::write(dir.join(p), "").unwrap();
            std::fs::set_permissions(dir.join(p), std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let tb = crate::environment::Toolbox { vars: vec![("PATH".into(), dir.display().to_string())], ..Default::default() };
        let tools = Tools { toolbox: Some(tb), ..Tools::default() };
        let dev = Command { from_install: vec!["vite".into()], ..cmd("npm:dev", &["npm", "node"]) };
        let install = Command { setup: true, ..cmd("npm:install", &["npm", "node"]) };
        let a = app("npm", vec![dev, install.clone()], EnvApply::None);
        assert_eq!(tools.unusable(&a).as_deref(), Some("vite isn't installed: run npm install"));
        let installed = app("npm", vec![cmd("npm:dev", &["npm", "node"]), install], EnvApply::None);
        assert_eq!(tools.unusable(&installed), None);
    }

    #[test]
    fn command_ids_carry_their_source() {
        let c = cmd("pnpm:start:dev", &[]);
        assert_eq!((c.source(), c.name()), ("pnpm", "start:dev"));
        assert_eq!(name_of("run"), "run", "no source");
    }

    #[test]
    fn env_reaches_the_command_line_or_a_variable() {
        let arg = EnvApply::Argument("-s {env}".into());
        assert_eq!(arg.apply("task run", Some("dev35")), Applied { line: "task run -s dev35".into(), vars: vec![] });
        assert_eq!(arg.describe("dev35"), "-s dev35");
        let var = EnvApply::Variable("DEV_ENV".into());
        assert_eq!(var.apply("task run", Some("dev35")).vars, [("DEV_ENV".to_string(), "dev35".to_string())]);
        let placeholder = EnvApply::None.apply("go run . serve -s {env}", Some("dev33"));
        assert_eq!(placeholder.line, "go run . serve -s dev33", "a line's own {{env}}");
        assert_eq!(arg.apply("task run", None).line, "task run", "no env: unchanged");
    }

    #[test]
    fn tiers_list_alphabetically() {
        let app = app("pnpm", vec![cmd("test", &[]), cmd("build", &[]), cmd("start", &[])], EnvApply::None);
        let ids: Vec<&str> = app.tier(Tier::Secondary).iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["build", "test"]);
    }

    #[test]
    fn missing_tools_are_named() {
        let tools = Tools::new(&["go", "pnpm"]);
        let any = app("go", vec![], EnvApply::None);
        assert_eq!(tools.missing(&any, &cmd("run", &["go"])), None);
        assert_eq!(tools.missing(&any, &cmd("compose", &["docker", "go"])).as_deref(), Some("docker not found"));
        let app = app("go", vec![cmd("lint", &["golangci-lint"]), cmd("up", &["docker"]), cmd("down", &["docker"])], EnvApply::Variable("ENV".into()));
        assert_eq!(tools.missing_for(&app), ["docker not found", "golangci-lint not found"]);
        assert_eq!(app.env.describe("dev35"), "ENV=dev35");
    }
}
