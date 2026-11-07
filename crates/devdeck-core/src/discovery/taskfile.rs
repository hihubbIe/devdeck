//! Taskfile tasks (`task:`). Included Taskfiles are not followed.

use super::{Cx, Listing, Part, quote, yaml};
use serde::Deserialize;
use serde::de::IgnoredAny;
use std::collections::BTreeMap;
use std::path::Path;

/// The names `task` looks for, in its order.
const NAMES: [&str; 8] = [
    "Taskfile.yml",
    "taskfile.yml",
    "Taskfile.yaml",
    "taskfile.yaml",
    "Taskfile.dist.yml",
    "taskfile.dist.yml",
    "Taskfile.dist.yaml",
    "taskfile.dist.yaml",
];

#[derive(Default, Deserialize)]
struct Taskfile {
    #[serde(default)]
    tasks: BTreeMap<String, Task>,
}

/// A task in any of its forms; one of another shape is still a task, without commands.
#[derive(Deserialize)]
#[serde(untagged)]
enum Task {
    Line(String),
    Lines(Vec<Cmd>),
    Full(Full),
    #[allow(dead_code)]
    Other(IgnoredAny),
}

#[derive(Deserialize)]
struct Full {
    #[serde(default)]
    cmds: Vec<Cmd>,
    #[serde(default)]
    cmd: Option<Cmd>,
    /// Can't be called from the command line.
    #[serde(default)]
    internal: bool,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Cmd {
    Line(String),
    Map {
        cmd: String,
    },
    #[allow(dead_code)]
    Other(IgnoredAny),
}

impl Cmd {
    fn line(&self) -> Option<&str> {
        match self {
            Cmd::Line(l) | Cmd::Map { cmd: l } => Some(l),
            Cmd::Other(_) => None,
        }
    }
}

pub(super) fn scan(dir: &Path, listing: &Listing, cx: &mut Cx) -> Option<Part> {
    let name = NAMES.into_iter().find(|n| listing.file(n))?;
    let mut part = Part::new("task");
    let path = dir.join(name);
    let Some(text) = cx.read(&path) else { return Some(part) };
    let tasks = match yaml::<Taskfile>(&text) {
        Ok(t) => t.tasks,
        Err(e) => {
            cx.problem(&path, e);
            return Some(part);
        }
    };
    let visible: Vec<(&String, Vec<&str>)> = tasks
        .iter()
        .filter_map(|(name, task)| {
            let lines = match task {
                Task::Line(l) => vec![l.as_str()],
                Task::Lines(cmds) => cmds.iter().filter_map(Cmd::line).collect(),
                Task::Full(f) if f.internal => return None,
                Task::Full(f) => f.cmds.iter().chain(&f.cmd).filter_map(Cmd::line).collect(),
                Task::Other(_) => Vec::new(),
            };
            Some((name, lines))
        })
        .collect();
    let names: Vec<&str> = visible.iter().map(|(n, _)| n.as_str()).collect();
    for (name, lines) in &visible {
        part.add("task", name, format!("task {}", quote(name)), &names, &["task"]);
        part.runs.push(lines.join("\n"));
    }
    Some(part)
}
