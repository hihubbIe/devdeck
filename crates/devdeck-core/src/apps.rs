//! Local apps and their commands (R6). Discovery (M2) fills these from the project
//! folders; M1 builds them from the demo catalog.

use std::collections::BTreeSet;

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

    /// The tier a script name gets by default: servers first, chores hidden.
    pub fn of(name: &str) -> Tier {
        let base = name.split(':').next().unwrap_or(name);
        let hidden = ["lint", "format", "fmt", "prettier", "eslint", "prepare", "clean", "postinstall", "preinstall"];
        if hidden.contains(&base) || base.starts_with("pre") && base.len() > 3 || base.starts_with("post") {
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
    /// Unique within the app (`start:dev`).
    pub id: String,
    /// What runs (`pnpm run start:dev`).
    pub line: String,
    pub tier: Tier,
    /// Programs it needs on the PATH (R6.5).
    pub tools: Vec<String>,
    /// Ports it listens on when running (R6.9).
    pub ports: Vec<u16>,
}

/// How the workspace env reaches a run (R6.6).
#[derive(Clone, Debug, PartialEq)]
pub enum EnvApply {
    /// Sets an environment variable to the env name.
    Variable(String),
    /// Appends an argument followed by the env name.
    Argument(String),
    /// Rewrites a gitignored file before the run.
    FileRewrite(String),
    /// The app doesn't use an env.
    None,
}

impl EnvApply {
    /// What the UI says about it, for one env.
    pub fn describe(&self, env: &str) -> String {
        match self {
            EnvApply::Variable(v) => format!("{v}={env}"),
            EnvApply::Argument(a) => format!("{a} {env}"),
            EnvApply::FileRewrite(f) => format!("rewrites {f} for {env}"),
            EnvApply::None => "no env".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct App {
    pub id: String,
    pub name: String,
    pub folder: String,
    /// `go`, `pnpm`, `cargo`…
    pub kind: String,
    pub commands: Vec<Command>,
    pub env: EnvApply,
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

/// Programs found on this machine (M2 detects them; M1 lists them).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tools {
    pub found: BTreeSet<String>,
}

impl Tools {
    pub fn new(found: &[&str]) -> Self {
        Self { found: found.iter().map(|s| s.to_string()).collect() }
    }

    /// Why a command can't run, if it can't: the first missing tool, named.
    pub fn missing(&self, cmd: &Command) -> Option<String> {
        cmd.tools.iter().find(|t| !self.found.contains(*t)).map(|t| format!("{t} not found"))
    }

    /// Why an app can't be used at all: none of its visible (not hidden) commands can
    /// run; the reason of the first one.
    pub fn unusable(&self, app: &App) -> Option<String> {
        let visible: Vec<&Command> = app.commands.iter().filter(|c| c.tier != Tier::Hidden).collect();
        if visible.is_empty() {
            return None;
        }
        visible.iter().map(|c| self.missing(c)).collect::<Option<Vec<_>>>().and_then(|v| v.into_iter().next())
    }

    /// Every missing tool of an app, alphabetical and without repeats.
    pub fn missing_for(&self, app: &App) -> Vec<String> {
        let set: BTreeSet<&String> = app.commands.iter().flat_map(|c| c.tools.iter()).filter(|t| !self.found.contains(*t)).collect();
        set.into_iter().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(id: &str, tools: &[&str]) -> Command {
        Command { id: id.into(), line: id.into(), tier: Tier::of(id), tools: tools.iter().map(|t| t.to_string()).collect(), ports: vec![] }
    }

    #[test]
    fn tiers_from_names() {
        assert_eq!(Tier::of("start:dev"), Tier::Primary);
        assert_eq!(Tier::of("dev"), Tier::Primary);
        assert_eq!(Tier::of("up"), Tier::Primary, "docker compose up");
        assert_eq!(Tier::of("build"), Tier::Secondary);
        assert_eq!(Tier::of("test:unit"), Tier::Secondary);
        assert_eq!(Tier::of("lint"), Tier::Hidden);
        assert_eq!(Tier::of("prebuild"), Tier::Hidden);
        assert_eq!(Tier::of("postinstall"), Tier::Hidden);
        assert_eq!(Tier::of("pre"), Tier::Secondary, "not a pre-hook by itself");
    }

    #[test]
    fn tiers_list_alphabetically() {
        let app = App {
            id: "a".into(),
            name: "a".into(),
            folder: "/a".into(),
            kind: "pnpm".into(),
            commands: vec![cmd("test", &[]), cmd("build", &[]), cmd("start", &[])],
            env: EnvApply::None,
        };
        let ids: Vec<&str> = app.tier(Tier::Secondary).iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["build", "test"]);
    }

    #[test]
    fn missing_tools_are_named() {
        let tools = Tools::new(&["go", "pnpm"]);
        assert_eq!(tools.missing(&cmd("run", &["go"])), None);
        assert_eq!(tools.missing(&cmd("compose", &["docker", "go"])).as_deref(), Some("docker not found"));
        let app = App {
            id: "a".into(),
            name: "a".into(),
            folder: "/a".into(),
            kind: "go".into(),
            commands: vec![cmd("lint", &["golangci-lint"]), cmd("up", &["docker"]), cmd("down", &["docker"])],
            env: EnvApply::Variable("ENV".into()),
        };
        assert_eq!(tools.missing_for(&app), ["docker", "golangci-lint"]);
        assert_eq!(app.env.describe("dev35"), "ENV=dev35");
    }
}
