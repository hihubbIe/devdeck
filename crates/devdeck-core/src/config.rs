//! Workspace configuration (R10.1, R1.5, R6.4, R6.6).
//!
//! Files, all TOML, in the config folder (`~/.config/devdeck`):
//! - `workspaces/<id>.toml`, one per workspace, shareable (no machine paths):
//!   ```toml
//!   name = "tadaweb"
//!   folders = ["~/Documents/repos"]   # each child folder is a project; monorepos expand
//!   envs = ["dev33", "dev35", "dls"]  # optional
//!   protected = ["dls"]               # writes need an unlock (R7.6)
//!   default_env = "dev35"
//!
//!   [apps.entityservice]              # key: the app id (its folder, relative)
//!   name = "entity"                   # display name; the id never changes
//!   hidden = false
//!   env = { argument = "-s {env}" }   # or { variable = "DEV_ENV" }, { file = "x.yml" }, "none"
//!   urls = ["http://localhost:5720/health"]
//!   [apps.entityservice.commands."task:run"]
//!   line = "go run main.go serve -c configs"  # may hold `{env}` itself
//!   ports = [5720]
//!   tier = "primary"                  # or "secondary", "hidden"
//!   ```
//! - `machine.toml`, per machine (never shared), written only by the user: devdeck
//!   rewrites `settings.toml` itself, which would drop comments and anything it can't
//!   read.
//!   ```toml
//!   node_dirs = ["~/.nvm/versions/node"]  # where `.nvmrc` versions are looked up
//!   path = ["~/go/bin"]                   # added in front of the login shell's PATH
//!   ```
//!
//! Rules: `~` expands to the home folder; unknown keys are errors (a typo in an
//! override must not be silently ignored); a broken file is reported with its path and
//! line and the other files still load; overrides naming an app that wasn't found are
//! warnings.

use crate::apps::{App, Command, EnvApply, Tier, name_of};
use crate::catalog::Env;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use toml::Spanned;

/// The machine file, inside the config folder.
pub const MACHINE_FILE: &str = "machine.toml";

/// The folder holding workspace files, inside the config folder.
pub const WORKSPACES_DIR: &str = "workspaces";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WorkspaceConfig {
    /// The file name without `.toml`.
    pub id: String,
    pub name: String,
    /// Expanded (no `~`).
    pub folders: Vec<PathBuf>,
    /// Alphabetical.
    pub envs: Vec<Env>,
    pub default_env: Option<String>,
    /// By app id.
    pub apps: BTreeMap<String, AppOverride>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AppOverride {
    pub name: Option<String>,
    pub hidden: bool,
    pub env: Option<EnvApply>,
    pub urls: Vec<String>,
    /// By command id (`task:run`); an id that discovery didn't find adds a command.
    pub commands: BTreeMap<String, CommandOverride>,
}

/// Changes to a found command, or a new one: a command id discovery didn't find needs a
/// `line` (an error otherwise) and gets the tier of its name unless one is given.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CommandOverride {
    pub line: Option<String>,
    pub ports: Option<Vec<u16>>,
    pub tier: Option<Tier>,
    /// Programs it needs, for a new command (found commands keep theirs).
    pub tools: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MachineConfig {
    /// Folders holding one folder per Node version (`v22.23.3/bin/node`).
    pub node_dirs: Vec<PathBuf>,
    /// Added in front of the login shell's PATH.
    pub path: Vec<PathBuf>,
}

/// A problem in a config file, shown to the user.
#[derive(Clone, Debug, PartialEq)]
pub struct ConfigError {
    pub file: PathBuf,
    pub line: Option<usize>,
    pub message: String,
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.line {
            Some(line) => write!(f, "{}:{line}: {}", self.file.display(), self.message),
            None => write!(f, "{}: {}", self.file.display(), self.message),
        }
    }
}

/// Every `workspaces/*.toml` in `config_dir`, alphabetical by id, with the problems of
/// the files that couldn't be read (those are left out).
pub fn load_workspaces(config_dir: &Path) -> (Vec<WorkspaceConfig>, Vec<ConfigError>) {
    load_workspaces_with(config_dir, home().as_deref())
}

/// `machine.toml` (defaults when missing).
pub fn load_machine(config_dir: &Path) -> (MachineConfig, Vec<ConfigError>) {
    load_machine_with(config_dir, home().as_deref())
}

/// Discovered apps with the workspace's overrides applied: names, hidden apps removed,
/// env bindings, URLs, command lines, ports and tiers changed or added. Returns
/// warnings (shown like discovery problems, no file line) for overrides that name no
/// discovered app and for added commands without a line.
pub fn apply_overrides(apps: Vec<App>, overrides: &BTreeMap<String, AppOverride>) -> (Vec<App>, Vec<String>) {
    let mut apps = apps;
    apps.sort_by(|a, b| a.id.cmp(&b.id));
    let found: BTreeSet<&str> = apps.iter().map(|a| a.id.as_str()).collect();
    let mut warnings: Vec<String> =
        overrides.keys().filter(|id| !found.contains(id.as_str())).map(|id| format!("apps.{id}: no app with this id was found")).collect();
    let apps = apps
        .into_iter()
        .filter_map(|app| match overrides.get(&app.id) {
            Some(o) => apply_app(app, o, &mut warnings),
            None => Some(app),
        })
        .collect();
    (apps, warnings)
}

/// One app with its override, `None` when hidden.
fn apply_app(mut app: App, o: &AppOverride, warnings: &mut Vec<String>) -> Option<App> {
    if o.hidden {
        return None;
    }
    if let Some(name) = &o.name {
        app.name = name.clone();
    }
    if let Some(env) = &o.env {
        app.env = env.clone();
    }
    if !o.urls.is_empty() {
        app.urls = o.urls.clone();
    }
    let mut added = Vec::new();
    for (id, c) in &o.commands {
        let at = format!("apps.{}.commands.\"{id}\"", app.id);
        if let Some(cmd) = app.commands.iter_mut().find(|cmd| cmd.id == *id) {
            if let Some(line) = &c.line {
                cmd.line = line.clone();
            }
            if let Some(ports) = &c.ports {
                cmd.ports = ports.clone();
            }
            if let Some(tier) = c.tier {
                cmd.tier = tier;
            }
            if !c.tools.is_empty() {
                warnings.push(format!("{at}: tools are only for new commands, a found command keeps its own (ignored)"));
            }
        } else if let Some(line) = &c.line {
            added.push((id, line, c));
        } else {
            warnings.push(format!("{at}: not found, and a new command needs a line (skipped)"));
        }
    }
    // Every id of the app, found and added, so a new `prebuild` is the hook of a `build`.
    let ids: Vec<&str> = app.commands.iter().map(|c| c.id.as_str()).chain(added.iter().map(|(id, _, _)| id.as_str())).collect();
    let new: Vec<Command> = added
        .iter()
        .map(|(id, line, c)| {
            let siblings: Vec<&str> = ids.iter().filter(|s| **s != id.as_str() && source_of(s) == source_of(id)).map(|s| name_of(s)).collect();
            Command {
                id: id.to_string(),
                line: line.to_string(),
                tier: c.tier.unwrap_or_else(|| Tier::of(name_of(id), &siblings)),
                tools: c.tools.clone(),
                from_install: Vec::new(), setup: false,
                ports: c.ports.clone().unwrap_or_default(),
            }
        })
        .collect();
    app.commands.extend(new);
    // A primary command added or promoted makes a library something to run.
    app.library &= !app.commands.iter().any(|c| c.tier == Tier::Primary);
    Some(app)
}

/// Where a command id comes from (`task` in `task:run`), like `Command::source`.
fn source_of(id: &str) -> &str {
    id.split_once(':').map_or("", |(s, _)| s)
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").filter(|h| !h.is_empty()).map(PathBuf::from)
}

/// `~` and `~/…` with the home folder; anything else (and everything without a home)
/// unchanged. `~user` isn't supported.
fn expand_home(path: &str, home: Option<&Path>) -> PathBuf {
    let Some(home) = home else { return path.into() };
    if path == "~" {
        return home.into();
    }
    match path.strip_prefix("~/") {
        Some(rest) => home.join(rest.trim_start_matches('/')),
        None => path.into(),
    }
}

/// A problem inside one file: its line, when known, and what is wrong.
type Problem = (Option<usize>, String);

fn load_workspaces_with(config_dir: &Path, home: Option<&Path>) -> (Vec<WorkspaceConfig>, Vec<ConfigError>) {
    let dir = config_dir.join(WORKSPACES_DIR);
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (Vec::new(), Vec::new()),
        Err(e) => return (Vec::new(), vec![ConfigError { file: dir, line: None, message: e.to_string() }]),
    };
    let mut errors = Vec::new();
    let mut files = Vec::new();
    for entry in entries {
        let path = match entry {
            Ok(entry) => entry.path(),
            Err(e) => {
                errors.push(ConfigError { file: dir.clone(), line: None, message: e.to_string() });
                continue;
            }
        };
        // Hidden files (`.x.toml`) are editors' and tools' own.
        if path.extension().is_none_or(|e| e != "toml") || path.file_name().is_some_and(|n| n.as_encoded_bytes().starts_with(b".")) {
            continue;
        }
        match path.file_stem().and_then(|s| s.to_str()) {
            Some(id) => files.push((id.to_string(), path)),
            None => errors.push(ConfigError { file: path, line: None, message: "the file name isn't valid UTF-8".into() }),
        }
    }
    files.sort();
    let mut workspaces = Vec::new();
    for (id, path) in files {
        let problems = match std::fs::read_to_string(&path) {
            Ok(text) => match parse_workspace(&id, &text, home) {
                Ok(w) => {
                    workspaces.push(w);
                    continue;
                }
                Err(problems) => problems,
            },
            Err(e) => vec![(None, e.to_string())],
        };
        errors.extend(problems.into_iter().map(|(line, message)| ConfigError { file: path.clone(), line, message }));
    }
    (workspaces, errors)
}

fn parse_workspace(id: &str, text: &str, home: Option<&Path>) -> Result<WorkspaceConfig, Vec<Problem>> {
    let raw: RawWorkspace = toml::from_str(text).map_err(|e| vec![toml_problem(text, &e)])?;
    let names: BTreeSet<&str> = raw.envs.iter().map(String::as_str).collect();
    let unknown = |env: &Spanned<String>, key: &str| {
        (!names.contains(env.get_ref().as_str())).then(|| (Some(line_of(text, env.span().start)), format!("{key}: \"{}\" is not one of envs", env.get_ref())))
    };
    let problems: Vec<Problem> = raw.protected.iter().filter_map(|e| unknown(e, "protected")).chain(raw.default_env.iter().filter_map(|e| unknown(e, "default_env"))).collect();
    if !problems.is_empty() {
        return Err(problems);
    }
    let protected: BTreeSet<&str> = raw.protected.iter().map(|e| e.get_ref().as_str()).collect();
    Ok(WorkspaceConfig {
        id: id.into(),
        name: raw.name.unwrap_or_else(|| id.into()),
        folders: raw.folders.iter().map(|f| expand_home(f, home)).collect(),
        envs: names.iter().map(|n| Env { name: n.to_string(), protected: protected.contains(n) }).collect(),
        default_env: raw.default_env.map(Spanned::into_inner),
        apps: raw.apps.into_iter().map(|(id, app)| (id, app.into())).collect(),
    })
}

fn load_machine_with(config_dir: &Path, home: Option<&Path>) -> (MachineConfig, Vec<ConfigError>) {
    let path = config_dir.join(MACHINE_FILE);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (MachineConfig::default(), Vec::new()),
        Err(e) => return (MachineConfig::default(), vec![ConfigError { file: path, line: None, message: e.to_string() }]),
    };
    match toml::from_str::<RawMachine>(&text) {
        Ok(m) => {
            let expand = |v: &[String]| v.iter().map(|p| expand_home(p, home)).collect();
            (MachineConfig { node_dirs: expand(&m.node_dirs), path: expand(&m.path) }, Vec::new())
        }
        Err(e) => {
            let (line, message) = toml_problem(&text, &e);
            (MachineConfig::default(), vec![ConfigError { file: path, line, message }])
        }
    }
}

fn toml_problem(text: &str, e: &toml::de::Error) -> Problem {
    (e.span().map(|s| line_of(text, s.start)), e.message().trim().to_string())
}

/// The 1-based line holding byte `offset`.
fn line_of(text: &str, offset: usize) -> usize {
    text.as_bytes()[..offset.min(text.len())].iter().filter(|b| **b == b'\n').count() + 1
}

// The files as written; converted to the public types once read.

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawWorkspace {
    name: Option<String>,
    #[serde(default)]
    folders: Vec<String>,
    #[serde(default)]
    envs: Vec<String>,
    #[serde(default)]
    protected: Vec<Spanned<String>>,
    default_env: Option<Spanned<String>>,
    #[serde(default)]
    apps: BTreeMap<String, RawApp>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawApp {
    name: Option<String>,
    #[serde(default)]
    hidden: bool,
    env: Option<RawEnv>,
    #[serde(default)]
    urls: Vec<String>,
    #[serde(default)]
    commands: BTreeMap<String, RawCommand>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCommand {
    line: Option<String>,
    ports: Option<Vec<u16>>,
    tier: Option<RawTier>,
    #[serde(default)]
    tools: Vec<String>,
}

/// `{ argument = "-s {env}" }`, `{ variable = "DEV_ENV" }`, `{ file = "x.yml" }` or `"none"`.
#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum RawEnv {
    Argument(String),
    Variable(String),
    File(String),
    None,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum RawTier {
    Primary,
    Secondary,
    Hidden,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMachine {
    #[serde(default)]
    node_dirs: Vec<String>,
    #[serde(default)]
    path: Vec<String>,
}

impl From<RawApp> for AppOverride {
    fn from(raw: RawApp) -> Self {
        AppOverride {
            name: raw.name,
            hidden: raw.hidden,
            env: raw.env.map(|e| match e {
                RawEnv::Argument(a) => EnvApply::Argument(a),
                RawEnv::Variable(v) => EnvApply::Variable(v),
                RawEnv::File(f) => EnvApply::FileRewrite(f),
                RawEnv::None => EnvApply::None,
            }),
            urls: raw.urls,
            commands: raw.commands.into_iter().map(|(id, c)| (id, c.into())).collect(),
        }
    }
}

impl From<RawCommand> for CommandOverride {
    fn from(raw: RawCommand) -> Self {
        let tier = raw.tier.map(|t| match t {
            RawTier::Primary => Tier::Primary,
            RawTier::Secondary => Tier::Secondary,
            RawTier::Hidden => Tier::Hidden,
        });
        CommandOverride { line: raw.line, ports: raw.ports, tier, tools: raw.tools }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: &str = "/home/test";

    fn tmp_dir(name: &str) -> crate::testing::TempDir {
        let d = crate::testing::TempDir::new(name);
        std::fs::create_dir_all(d.join(WORKSPACES_DIR)).unwrap();
        d
    }

    fn workspaces(name: &str, files: &[(&str, &str)]) -> (crate::testing::TempDir, Vec<WorkspaceConfig>, Vec<ConfigError>) {
        let d = tmp_dir(name);
        for (file, text) in files {
            std::fs::write(d.join(WORKSPACES_DIR).join(file), text).unwrap();
        }
        let (w, e) = load_workspaces_with(&d, Some(Path::new(HOME)));
        (d, w, e)
    }

    /// The single error of a load, as (file name, line, message).
    fn only_error(errors: &[ConfigError]) -> (String, Option<usize>, String) {
        assert_eq!(errors.len(), 1, "{errors:?}");
        let e = &errors[0];
        (e.file.file_name().unwrap().to_string_lossy().into_owned(), e.line, e.message.clone())
    }

    const TADAWEB: &str = r#"name = "tadaweb"
folders = ["~/Documents/repos", "/srv/other"]
envs = ["dls", "dev35", "dev33"]
protected = ["dls"]
default_env = "dev35"

[apps.entityservice]
name = "entity"
env = { argument = "-s {env}" }
urls = ["http://localhost:5720/health"]
[apps.entityservice.commands."task:run"]
line = "go run main.go serve -c configs"
ports = [5720]
tier = "primary"

[apps.web]
hidden = true
env = "none"

[apps.api]
env = { variable = "DEV_ENV" }
[apps.api.commands."task:seed"]
line = "task seed"
tools = ["task"]

[apps.legacy]
env = { file = "config/local.yml" }
"#;

    #[test]
    fn a_valid_file_loads() {
        let (_, w, e) = workspaces("valid", &[("tadaweb.toml", TADAWEB)]);
        assert!(e.is_empty(), "{e:?}");
        let w = &w[0];
        assert_eq!((w.id.as_str(), w.name.as_str()), ("tadaweb", "tadaweb"));
        assert_eq!(w.folders, [PathBuf::from("/home/test/Documents/repos"), PathBuf::from("/srv/other")]);
        let env = |n: &str, protected| Env { name: n.into(), protected };
        assert_eq!(w.envs, [env("dev33", false), env("dev35", false), env("dls", true)], "alphabetical, protected marked");
        assert_eq!(w.default_env.as_deref(), Some("dev35"));
        let entity = &w.apps["entityservice"];
        assert_eq!(entity.name.as_deref(), Some("entity"));
        assert!(!entity.hidden);
        assert_eq!(entity.env, Some(EnvApply::Argument("-s {env}".into())));
        assert_eq!(entity.urls, ["http://localhost:5720/health"]);
        let run = &entity.commands["task:run"];
        assert_eq!(
            run,
            &CommandOverride { line: Some("go run main.go serve -c configs".into()), ports: Some(vec![5720]), tier: Some(Tier::Primary), tools: vec![] }
        );
        assert!(w.apps["web"].hidden);
        assert_eq!(w.apps["web"].env, Some(EnvApply::None));
        assert_eq!(w.apps["api"].env, Some(EnvApply::Variable("DEV_ENV".into())));
        assert_eq!(w.apps["api"].commands["task:seed"].tools, ["task"]);
        assert_eq!(w.apps["api"].commands["task:seed"].tier, None);
        assert_eq!(w.apps["legacy"].env, Some(EnvApply::FileRewrite("config/local.yml".into())));
    }

    #[test]
    fn a_minimal_file_takes_its_name_from_the_id() {
        let (_, w, e) = workspaces("minimal", &[("personal.toml", "")]);
        assert!(e.is_empty(), "{e:?}");
        assert_eq!(w, [WorkspaceConfig { id: "personal".into(), name: "personal".into(), ..Default::default() }]);
    }

    #[test]
    fn workspaces_are_alphabetical_and_other_files_are_skipped() {
        let files = [("zeta.toml", ""), ("alpha.toml", ""), ("notes.txt", "x"), ("alpha.toml~", "x"), (".hidden.toml", "x =")];
        let (_, w, e) = workspaces("order", &files);
        assert!(e.is_empty(), "{e:?}");
        let ids: Vec<&str> = w.iter().map(|w| w.id.as_str()).collect();
        assert_eq!(ids, ["alpha", "zeta"]);
    }

    #[test]
    fn no_workspaces_folder_is_no_workspace_and_no_error() {
        let d = crate::testing::TempDir::new("config-none");
        assert_eq!(load_workspaces_with(&d, Some(Path::new(HOME))), (vec![], vec![]));
    }

    #[test]
    fn a_broken_file_is_reported_with_its_line_and_the_others_load() {
        let (d, w, e) = workspaces("syntax", &[("bad.toml", "name = \"x\"\nfolders = [\n\"a\",\n= 3\n"), ("good.toml", "")]);
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].id, "good");
        let (file, line, _) = only_error(&e);
        assert_eq!((file.as_str(), line), ("bad.toml", Some(4)));
        assert_eq!(e[0].file, d.join(WORKSPACES_DIR).join("bad.toml"));
    }

    #[test]
    fn unknown_keys_are_errors_with_their_line() {
        let (_, _, e) = workspaces("unknown-top", &[("w.toml", "name = \"w\"\n\nfloders = []\n")]);
        let (_, line, message) = only_error(&e);
        assert_eq!(line, Some(3));
        assert!(message.contains("floders"), "{message}");

        let text = "[apps.x]\nname = \"x\"\n[apps.x.commands.\"task:run\"]\nline = \"task run\"\nport = [1]\n";
        let (_, _, e) = workspaces("unknown-command", &[("w.toml", text)]);
        let (_, line, message) = only_error(&e);
        assert_eq!(line, Some(5));
        assert!(message.contains("port"), "{message}");
    }

    #[test]
    fn bad_values_are_errors_with_their_line() {
        let cases = [
            ("tier", "[apps.x.commands.\"a:b\"]\nline = \"a\"\ntier = \"top\"\n", 3, "top"),
            ("binding", "[apps.x]\n\nenv = { argument = \"-s\", variable = \"E\" }\n", 3, ""),
            ("binding-name", "[apps.x]\nenv = { flag = \"-s\" }\n", 2, "flag"),
            ("binding-string", "[apps.x]\nenv = \"always\"\n", 2, "always"),
            ("port", "[apps.x.commands.\"a:b\"]\nline = \"a\"\nports = [70000]\n", 3, ""),
            ("type", "\nfolders = \"~/repos\"\n", 2, ""),
        ];
        for (name, text, want, word) in cases {
            let (_, w, e) = workspaces(&format!("value-{name}"), &[("w.toml", text)]);
            assert!(w.is_empty(), "{name}");
            let (_, line, message) = only_error(&e);
            assert_eq!(line, Some(want), "{name}: {message}");
            assert!(message.contains(word), "{name}: {message}");
        }
    }

    #[test]
    fn envs_named_elsewhere_must_exist() {
        let text = "envs = [\"dev33\"]\nprotected = [\"dev33\", \"prod\"]\ndefault_env = \"dev35\"\n";
        let (_, w, e) = workspaces("envs", &[("w.toml", text)]);
        assert!(w.is_empty());
        let got: Vec<(Option<usize>, &str)> = e.iter().map(|e| (e.line, e.message.as_str())).collect();
        assert_eq!(got, [(Some(2), "protected: \"prod\" is not one of envs"), (Some(3), "default_env: \"dev35\" is not one of envs")]);

        let (_, w, e) = workspaces("no-envs", &[("w.toml", "default_env = \"dev35\"\n")]);
        assert!(w.is_empty());
        assert_eq!(only_error(&e).1, Some(1));
    }

    #[test]
    fn home_expands_only_at_the_start() {
        let home = Some(Path::new(HOME));
        assert_eq!(expand_home("~", home), PathBuf::from(HOME));
        assert_eq!(expand_home("~/go/bin", home), PathBuf::from("/home/test/go/bin"));
        assert_eq!(expand_home("/opt/~/x", home), PathBuf::from("/opt/~/x"));
        assert_eq!(expand_home("~other/x", home), PathBuf::from("~other/x"), "~user isn't supported");
        assert_eq!(expand_home("repos", home), PathBuf::from("repos"));
        assert_eq!(expand_home("~/x", None), PathBuf::from("~/x"), "no home: unchanged");
    }

    #[test]
    fn the_public_loader_uses_home() {
        let (d, _, _) = workspaces("home", &[("w.toml", "folders = [\"~/repos\"]\n")]);
        let (w, e) = load_workspaces(&d);
        assert!(e.is_empty(), "{e:?}");
        let want = match home() {
            Some(h) => h.join("repos"),
            None => PathBuf::from("~/repos"),
        };
        assert_eq!(w[0].folders, [want]);
    }

    fn machine(name: &str, text: Option<&str>) -> (MachineConfig, Vec<ConfigError>) {
        let d = tmp_dir(name);
        if let Some(text) = text {
            std::fs::write(d.join(MACHINE_FILE), text).unwrap();
        }
        load_machine_with(&d, Some(Path::new(HOME)))
    }

    #[test]
    fn the_machine_file_is_read_and_expanded() {
        let text = "# this machine\nnode_dirs = [\"~/.nvm/versions/node\"]\npath = [\"~/go/bin\", \"/opt/bin\"]\n";
        let (m, e) = machine("machine", Some(text));
        assert!(e.is_empty(), "{e:?}");
        assert_eq!(m.node_dirs, [PathBuf::from("/home/test/.nvm/versions/node")]);
        assert_eq!(m.path, [PathBuf::from("/home/test/go/bin"), PathBuf::from("/opt/bin")]);
    }

    #[test]
    fn no_machine_file_gives_defaults() {
        assert_eq!(machine("machine-nofile", None), (MachineConfig::default(), vec![]));
        assert_eq!(machine("machine-empty", Some("")), (MachineConfig::default(), vec![]));
        assert_eq!(machine("machine-partial", Some("path = [\"/x\"]\n")).0.path, [PathBuf::from("/x")]);
    }

    #[test]
    fn machine_problems_have_their_line() {
        let (m, e) = machine("machine-unknown", Some("# typo below\npath = []\nnode_dir = []\n"));
        assert_eq!(m, MachineConfig::default());
        let (file, line, message) = only_error(&e);
        assert_eq!((file.as_str(), line), (MACHINE_FILE, Some(3)));
        assert!(message.contains("node_dir"), "{message}");

        let (_, e) = machine("machine-syntax", Some("path = [\n"));
        assert_eq!(e.len(), 1);
        assert!(e[0].line.is_some());
    }

    #[test]
    fn a_primary_command_makes_a_library_runnable() {
        let mut lib = app("lib", vec![cmd("npm:build")]);
        lib.library = true;
        let promote = CommandOverride { tier: Some(Tier::Primary), ..Default::default() };
        let o = AppOverride { commands: BTreeMap::from([("npm:build".to_string(), promote)]), ..Default::default() };
        let (apps, _) = apply_overrides(vec![lib.clone()], &BTreeMap::from([("lib".to_string(), o)]));
        assert!(!apps[0].library);
        let (apps, _) = apply_overrides(vec![lib], &BTreeMap::new());
        assert!(apps[0].library, "unchanged without an override");
    }

    fn cmd(id: &str) -> Command {
        Command { id: id.into(), line: format!("{id} line"), tier: Tier::Secondary, tools: vec!["tool".into()], from_install: vec![], setup: false, ports: vec![1] }
    }

    fn app(id: &str, commands: Vec<Command>) -> App {
        App {
            id: id.into(),
            name: id.into(),
            folder: format!("~/{id}"),
            dir: PathBuf::from("/").join(id),
            kind: "go".into(),
            library: false,
            facts: Default::default(),
            commands,
            env: EnvApply::None,
            urls: vec!["http://found".into()],
        }
    }

    fn overrides(list: Vec<(&str, AppOverride)>) -> BTreeMap<String, AppOverride> {
        list.into_iter().map(|(id, o)| (id.to_string(), o)).collect()
    }

    fn command(list: Vec<(&str, CommandOverride)>) -> BTreeMap<String, CommandOverride> {
        list.into_iter().map(|(id, o)| (id.to_string(), o)).collect()
    }

    #[test]
    fn no_overrides_change_nothing_but_the_order() {
        let apps = vec![app("b", vec![cmd("task:run")]), app("a", vec![])];
        let (out, warnings) = apply_overrides(apps.clone(), &BTreeMap::new());
        assert!(warnings.is_empty());
        assert_eq!(out, [apps[1].clone(), apps[0].clone()]);
    }

    #[test]
    fn app_overrides_apply() {
        let apps = vec![app("a", vec![]), app("b", vec![]), app("c", vec![])];
        let o = overrides(vec![
            ("a", AppOverride { name: Some("Alpha".into()), env: Some(EnvApply::Argument("-s {env}".into())), urls: vec!["http://a".into()], ..Default::default() }),
            ("b", AppOverride { hidden: true, name: Some("ignored".into()), ..Default::default() }),
        ]);
        let (out, warnings) = apply_overrides(apps, &o);
        assert!(warnings.is_empty(), "{warnings:?}");
        let ids: Vec<&str> = out.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["a", "c"], "hidden apps are removed");
        assert_eq!(out[0].name, "Alpha");
        assert_eq!(out[0].env, EnvApply::Argument("-s {env}".into()));
        assert_eq!(out[0].urls, ["http://a"], "configured URLs replace the found ones");
        assert_eq!(out[1].urls, ["http://found"], "no URLs configured: the found ones stay");
        assert_eq!((out[1].name.as_str(), &out[1].env), ("c", &EnvApply::None));
    }

    #[test]
    fn found_commands_change() {
        let apps = vec![app("a", vec![cmd("task:run"), cmd("task:build")])];
        let changes = command(vec![
            ("task:run", CommandOverride { line: Some("task run -- -s {env}".into()), ports: Some(vec![5720, 5721]), tier: Some(Tier::Primary), tools: vec![] }),
            ("task:build", CommandOverride { tier: Some(Tier::Hidden), ..Default::default() }),
        ]);
        let (out, warnings) = apply_overrides(apps, &overrides(vec![("a", AppOverride { commands: changes, ..Default::default() })]));
        assert!(warnings.is_empty(), "{warnings:?}");
        let run = out[0].command("task:run").unwrap();
        assert_eq!((run.line.as_str(), run.ports.as_slice(), run.tier), ("task run -- -s {env}", &[5720, 5721][..], Tier::Primary));
        assert_eq!(run.tools, ["tool"]);
        let build = out[0].command("task:build").unwrap();
        assert_eq!((build.line.as_str(), build.ports.as_slice(), build.tier), ("task:build line", &[1][..], Tier::Hidden), "only the tier changes");
    }

    #[test]
    fn tools_of_a_found_command_are_kept_with_a_warning() {
        let changes = command(vec![("task:run", CommandOverride { tools: vec!["docker".into()], ..Default::default() })]);
        let (out, warnings) = apply_overrides(vec![app("a", vec![cmd("task:run")])], &overrides(vec![("a", AppOverride { commands: changes, ..Default::default() })]));
        assert_eq!(out[0].commands[0].tools, ["tool"]);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].starts_with("apps.a.commands.\"task:run\""), "{warnings:?}");
    }

    #[test]
    fn unknown_commands_are_added() {
        let apps = vec![app("a", vec![cmd("npm:build")])];
        let changes = command(vec![
            ("npm:prebuild", CommandOverride { line: Some("npm run prebuild".into()), ..Default::default() }),
            ("npm:preview", CommandOverride { line: Some("npm run preview".into()), ..Default::default() }),
            ("task:dev", CommandOverride { line: Some("task dev".into()), ports: Some(vec![3000]), tools: vec!["task".into()], ..Default::default() }),
            ("task:prebuild", CommandOverride { line: Some("task prebuild".into()), ..Default::default() }),
            ("task:seed", CommandOverride { line: Some("task seed".into()), tier: Some(Tier::Primary), ..Default::default() }),
        ]);
        let (out, warnings) = apply_overrides(apps, &overrides(vec![("a", AppOverride { commands: changes, ..Default::default() })]));
        assert!(warnings.is_empty(), "{warnings:?}");
        let a = &out[0];
        let tier = |id: &str| a.command(id).unwrap().tier;
        assert_eq!(tier("npm:prebuild"), Tier::Hidden, "a hook of the found npm:build");
        assert_eq!(tier("npm:preview"), Tier::Secondary);
        assert_eq!(tier("task:prebuild"), Tier::Secondary, "no task:build: not a hook");
        assert_eq!(tier("task:dev"), Tier::Primary, "the tier of its name");
        assert_eq!(tier("task:seed"), Tier::Primary, "the configured tier");
        let dev = a.command("task:dev").unwrap();
        assert_eq!(dev, &Command { id: "task:dev".into(), line: "task dev".into(), tier: Tier::Primary, tools: vec!["task".into()], from_install: vec![], setup: false, ports: vec![3000] });
        assert_eq!(a.command("task:seed").unwrap().ports, Vec::<u16>::new());
    }

    #[test]
    fn a_new_hook_sees_new_siblings() {
        let changes = command(vec![
            ("task:build", CommandOverride { line: Some("task build".into()), ..Default::default() }),
            ("task:postbuild", CommandOverride { line: Some("task postbuild".into()), ..Default::default() }),
        ]);
        let (out, _) = apply_overrides(vec![app("a", vec![])], &overrides(vec![("a", AppOverride { commands: changes, ..Default::default() })]));
        assert_eq!(out[0].command("task:postbuild").unwrap().tier, Tier::Hidden);
    }

    #[test]
    fn a_new_command_without_a_line_is_skipped_with_a_warning() {
        let changes = command(vec![("task:dev", CommandOverride { ports: Some(vec![3000]), ..Default::default() })]);
        let (out, warnings) = apply_overrides(vec![app("a", vec![])], &overrides(vec![("a", AppOverride { commands: changes, ..Default::default() })]));
        assert!(out[0].commands.is_empty());
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].starts_with("apps.a.commands.\"task:dev\"") && warnings[0].contains("line"), "{warnings:?}");
    }

    #[test]
    fn overrides_of_missing_apps_are_warnings() {
        let o = overrides(vec![("gone", AppOverride { name: Some("x".into()), ..Default::default() }), ("old", AppOverride { hidden: true, ..Default::default() })]);
        let (out, warnings) = apply_overrides(vec![app("a", vec![])], &o);
        assert_eq!(out.len(), 1);
        assert_eq!(warnings, ["apps.gone: no app with this id was found", "apps.old: no app with this id was found"]);
    }

    #[test]
    fn overrides_from_a_file_apply() {
        let (_, w, e) = workspaces("end-to-end", &[("w.toml", TADAWEB)]);
        assert!(e.is_empty(), "{e:?}");
        let apps = vec![app("web", vec![]), app("entityservice", vec![cmd("task:run")]), app("api", vec![])];
        let (out, warnings) = apply_overrides(apps, &w[0].apps);
        assert_eq!(warnings, ["apps.legacy: no app with this id was found"]);
        let ids: Vec<&str> = out.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["api", "entityservice"]);
        let applied = out[1].env.apply(&out[1].command("task:run").unwrap().line, Some("dev35"));
        assert_eq!(applied.line, "go run main.go serve -c configs -s dev35");
        assert_eq!(out[0].command("task:seed").unwrap().tools, ["task"]);
    }
}
