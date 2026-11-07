//! `.vscode/launch.json` configurations (`vscode:<name>`), turned into shell lines: Go
//! (`go run`, `go test`, or the binary for `exec`) and Node. Other types, `attach`
//! requests and configurations using variables other than `${workspaceFolder}`
//! (`${file}`, `${env:X}`, `${input:x}`) are skipped quietly. `envFile` isn't read.

use super::{Cx, Listing, Part, quote};
use serde_json::Value;
use std::path::Path;

pub(super) fn scan(dir: &Path, listing: &Listing, cx: &mut Cx) -> Option<Part> {
    if !listing.dir(".vscode") {
        return None;
    }
    cx.watch(&dir.join(".vscode"));
    let path = dir.join(".vscode/launch.json");
    let text = cx.read(&path)?;
    let json: Value = match jsonc_parser::parse_to_serde_value(&text, &Default::default()) {
        Ok(v) => v,
        Err(e) => {
            cx.problem(&path, e);
            return None;
        }
    };
    let configs = json.get("configurations").and_then(Value::as_array)?;
    let names: Vec<&str> = configs.iter().filter_map(|c| c.get("name")?.as_str()).collect();
    let mut part = Part::new("vscode");
    for c in configs {
        let Some(name) = c.get("name").and_then(Value::as_str) else { continue };
        if c.get("request").and_then(Value::as_str).is_some_and(|r| r != "launch") {
            continue;
        }
        let Some(l) = launch(c, dir) else { continue };
        let tools: Vec<&str> = l.tools.iter().map(String::as_str).collect();
        part.add("vscode", name, l.line, &names, &tools);
        part.runs.push(l.run);
    }
    (!part.commands.is_empty()).then_some(part)
}

struct Launch {
    line: String,
    /// The line as if run from the app's folder, for precedence (`go run ./cmd/api`).
    run: String,
    tools: Vec<String>,
}

/// A configuration as a shell line. With a `cwd` other than the workspace folder, the
/// line changes folder first and paths under the workspace folder become absolute.
fn launch(c: &Value, dir: &Path) -> Option<Launch> {
    let cwd = match c.get("cwd") {
        None => None,
        Some(v) => Some(subst(v.as_str()?, ".")?),
    };
    let moved = cwd.as_deref().is_some_and(|d| !matches!(d.trim_end_matches('/'), "" | "."));
    let (run, tools) = command(c, ".")?;
    if !moved {
        return Some(Launch { line: run.clone(), run, tools });
    }
    let (line, _) = command(c, &dir.display().to_string())?;
    let cd = quote(cwd.as_deref().unwrap_or("."));
    Some(Launch { line: format!("cd {cd} && {line}"), run, tools })
}

/// `env` prefixes, the program and its arguments, with `${workspaceFolder}` as `root`.
fn command(c: &Value, root: &str) -> Option<(String, Vec<String>)> {
    let (program, tools) = match c.get("type")?.as_str()? {
        "go" => go(c, root)?,
        "node" | "pwa-node" => node(c, root)?,
        _ => return None,
    };
    let mut words = env(c.get("env"), root)?;
    words.push(program);
    words.extend(args(c.get("args"), root)?);
    Some((words.join(" "), tools))
}

fn go(c: &Value, root: &str) -> Option<(String, Vec<String>)> {
    let program = quote(&subst(c.get("program")?.as_str()?, root)?);
    let flags = match c.get("buildFlags") {
        None => String::new(),
        Some(Value::String(s)) => format!(" {}", subst(s, root)?),
        Some(v) => args(Some(v), root)?.iter().map(|a| format!(" {a}")).collect(),
    };
    let go = vec!["go".to_string()];
    match c.get("mode").and_then(Value::as_str).unwrap_or("auto") {
        "auto" | "debug" => Some((format!("go run{flags} {program}"), go)),
        "test" => Some((format!("go test{flags} {program}"), go)),
        "exec" => Some((program, Vec::new())),
        _ => None,
    }
}

fn node(c: &Value, root: &str) -> Option<(String, Vec<String>)> {
    let runtime = match c.get("runtimeExecutable") {
        None => "node".to_string(),
        Some(v) => subst(v.as_str()?, root)?,
    };
    let mut words = vec![quote(&runtime)];
    words.extend(args(c.get("runtimeArgs"), root)?);
    if let Some(program) = c.get("program") {
        words.push(quote(&subst(program.as_str()?, root)?));
    }
    if words.len() == 1 {
        return None;
    }
    let mut tools = vec!["node".to_string()];
    if runtime != "node" && !runtime.contains('/') {
        tools.insert(0, runtime);
    }
    Some((words.join(" "), tools))
}

/// Arguments: a list (each quoted) or a string (already a command line).
fn args(v: Option<&Value>, root: &str) -> Option<Vec<String>> {
    match v {
        None => Some(Vec::new()),
        Some(Value::String(s)) if s.trim().is_empty() => Some(Vec::new()),
        Some(Value::String(s)) => Some(vec![subst(s, root)?]),
        Some(Value::Array(a)) => a.iter().map(|w| Some(quote(&subst(w.as_str()?, root)?))).collect(),
        Some(_) => None,
    }
}

/// `VAR=value` words for the `env` object, by name: a JSON object's order depends on
/// serde_json's features, which other crates of the build may turn on.
fn env(v: Option<&Value>, root: &str) -> Option<Vec<String>> {
    let Some(v) = v else { return Some(Vec::new()) };
    let mut words = Vec::new();
    let mut vars: Vec<(&String, &Value)> = v.as_object()?.iter().collect();
    vars.sort_by_key(|(k, _)| *k);
    for (k, v) in vars {
        if k.is_empty() || k.starts_with(|c: char| c.is_ascii_digit()) || !k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return None;
        }
        let value = match v {
            Value::Null => continue,
            Value::String(s) => subst(s, root)?,
            Value::Bool(_) | Value::Number(_) => v.to_string(),
            _ => return None,
        };
        words.push(format!("{k}={}", quote(&value)));
    }
    Some(words)
}

/// `s` with `${workspaceFolder}` replaced by `root`; `None` when another variable is left.
fn subst(s: &str, root: &str) -> Option<String> {
    let s = s.replace("${workspaceFolder}", root).replace("${workspaceRoot}", root);
    (!s.contains("${")).then_some(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn go_configurations_become_lines() {
        let c = json!({"type": "go", "request": "launch", "program": "${workspaceFolder}/cmd/api", "args": ["serve", "-s", "dev 35"], "env": {"PORT": 8080, "MODE": "a b"}});
        let l = launch(&c, Path::new("/w/x")).unwrap();
        assert_eq!(l.line, "MODE='a b' PORT=8080 go run ./cmd/api serve -s 'dev 35'");
        assert_eq!(l.tools, ["go"]);
        let c = json!({"type": "go", "program": "${workspaceFolder}", "mode": "test", "buildFlags": "-tags=it"});
        assert_eq!(launch(&c, Path::new("/w/x")).unwrap().line, "go test -tags=it .");
        let c = json!({"type": "go", "program": "${workspaceFolder}/main.go", "cwd": "${workspaceFolder}/configs"});
        let l = launch(&c, Path::new("/w/x")).unwrap();
        assert_eq!(l.line, "cd ./configs && go run /w/x/main.go");
        assert_eq!(l.run, "go run ./main.go");
    }

    #[test]
    fn node_configurations_become_lines() {
        let c = json!({"type": "node", "program": "${workspaceFolder}/index.js", "args": ["--port", "3000"]});
        assert_eq!(launch(&c, Path::new("/w")).unwrap().line, "node ./index.js --port 3000");
        let c = json!({"type": "node", "runtimeExecutable": "pnpm", "runtimeArgs": ["run", "dev"]});
        let l = launch(&c, Path::new("/w")).unwrap();
        assert_eq!((l.line.as_str(), l.tools), ("pnpm run dev", vec!["pnpm".to_string(), "node".to_string()]));
    }

    #[test]
    fn unresolvable_configurations_are_skipped() {
        assert!(launch(&json!({"type": "go", "program": "${file}"}), Path::new("/w")).is_none());
        assert!(launch(&json!({"type": "go", "program": ".", "env": {"PATH": "${env:PATH}"}}), Path::new("/w")).is_none());
        assert!(launch(&json!({"type": "python", "program": "a.py"}), Path::new("/w")).is_none());
        assert!(launch(&json!({"type": "node"}), Path::new("/w")).is_none(), "nothing to run");
    }
}
