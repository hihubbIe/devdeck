//! `devdeck-ctl`: devdeck's control API from a shell (R13.11). A client like any other:
//! it only knows `devdeck-api`.
//!
//! ```text
//! devdeck-ctl [--socket PATH] [--json] whoami
//! devdeck-ctl [--socket PATH] [--json] describe [METHOD]
//! devdeck-ctl [--socket PATH] call METHOD [PARAMS-JSON]
//! devdeck-ctl [--socket PATH] [--json] watch TOPIC...
//! ```
//!
//! `watch` follows topics (`runs`, `output`, `notes`, `pulse`…: `api.describe
//! events.subscribe`) and prints each event as it comes, one per line, until devdeck goes
//! or it's interrupted.
//!
//! The socket: `--socket`, else `DEVDECK_SOCKET` (set in devdeck's shells), else the main
//! devdeck's. Exit codes: 0 done, 1 devdeck refused (its kind printed), 2 no connection,
//! 64 a usage mistake.

use devdeck_api::client::ClientError;
use devdeck_api::types::{Caller, Describe, Whoami};
use devdeck_api::{Client, default_socket};
use serde_json::Value;
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "usage: devdeck-ctl [--socket PATH] [--json] (whoami | describe [METHOD] | call METHOD [PARAMS-JSON] | watch TOPIC...)";

#[derive(Debug, PartialEq)]
enum Command {
    Whoami,
    Describe(Option<String>),
    Call(String, Value),
    Watch(Vec<String>),
}

#[derive(Debug, PartialEq)]
struct Args {
    socket: Option<PathBuf>,
    json: bool,
    command: Command,
}

fn parse(args: impl IntoIterator<Item = String>) -> Result<Args, String> {
    let mut socket = None;
    let mut json = false;
    let mut rest = Vec::new();
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--socket" => socket = Some(PathBuf::from(it.next().ok_or("--socket needs a path")?)),
            "--json" => json = true,
            "-h" | "--help" => return Err(String::new()),
            _ => rest.push(a),
        }
    }
    if rest.first().map(String::as_str) == Some("watch") {
        let topics: Vec<String> = rest.into_iter().skip(1).collect();
        if topics.is_empty() {
            return Err("watch needs a topic".into());
        }
        return Ok(Args { socket, json, command: Command::Watch(topics) });
    }
    let mut rest = rest.into_iter();
    let command = match (rest.next().as_deref(), rest.next(), rest.next()) {
        (Some("whoami"), None, None) => Command::Whoami,
        (Some("describe"), method, None) => Command::Describe(method),
        (Some("call"), Some(method), params) => {
            let params = match params {
                Some(p) => serde_json::from_str(&p).map_err(|e| format!("params: {e}"))?,
                None => Value::Object(Default::default()),
            };
            if !params.is_object() {
                return Err("params: a JSON object ({} when none)".into());
            }
            if rest.next().is_some() {
                return Err("call takes a method and one JSON object".into());
            }
            Command::Call(method, params)
        }
        _ => return Err(String::new()),
    };
    Ok(Args { socket, json, command })
}

fn caller(c: &Caller) -> String {
    match c {
        Caller::Terminal { workspace, n, .. } => format!("terminal {n} of workspace {workspace}"),
        Caller::Outside { pid: Some(p), program } => format!("outside: {} (pid {p})", program.as_str()),
        Caller::Outside { pid: None, program } => format!("outside: {} (not pinned)", program.as_str()),
        Caller::Unknown => "unknown".into(),
    }
}

/// How a result reads as plain text.
fn plain(command: &Command, v: &Value) -> String {
    match command {
        Command::Whoami => match serde_json::from_value::<Whoami>(v.clone()) {
            Ok(w) => {
                let mut out = format!("caller: {}\n", caller(&w.caller));
                if let Some(t) = w.terminal {
                    out += &format!("terminal: {} (uid {})\n", t.n, t.uid);
                }
                for p in w.shown {
                    out += &format!("shown: pulse {} · window {} · pane {}\n", p.pulse, p.window, p.pane);
                }
                out
            }
            Err(_) => pretty(v),
        },
        Command::Describe(Some(_)) => pretty(v),
        Command::Describe(None) => match serde_json::from_value::<Describe>(v.clone()) {
            Ok(d) => {
                let width = d.methods.iter().map(|m| m.name.len()).max().unwrap_or(0);
                let mut out = format!("API level {}\n", d.api_level);
                for m in d.methods {
                    let mark = if m.allowed { " " } else { "×" };
                    out += &format!("{mark} {:width$}  {:11}  {}\n", m.name, m.class.as_str(), m.summary);
                }
                out + "× : not allowed for this caller\n"
            }
            Err(_) => pretty(v),
        },
        Command::Call(..) | Command::Watch(_) => pretty(v),
    }
}

/// A notification as one line: `12 runs {"app":…}`, or that some were missed.
fn event_line(n: &Value, json: bool) -> String {
    if json {
        return serde_json::to_string(n).unwrap_or_default();
    }
    let p = &n["params"];
    match n["method"].as_str() {
        Some("events.lagged") => format!("lagged: events from {} were missed (read the state again)", p["seq"]),
        _ => format!("{} {} {}", p["seq"], p["topic"].as_str().unwrap_or("?"), p["data"]),
    }
}

/// Follows `topics` and prints their events until the connection ends.
fn watch(client: &mut Client, topics: &[String], json: bool) -> ExitCode {
    if let Err(e) = client.call_raw("events.subscribe", serde_json::json!({ "topics": topics })) {
        eprintln!("devdeck-ctl: {e}");
        return ExitCode::from(if matches!(e, ClientError::Api(_)) { 1 } else { 2 });
    }
    loop {
        match client.next_notification() {
            Ok(n) => println!("{}", event_line(&n, json)),
            Err(e) => {
                eprintln!("devdeck-ctl: {e}");
                return ExitCode::from(2);
            }
        }
    }
}

fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_default() + "\n"
}

fn main() -> ExitCode {
    let args = match parse(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(why) => {
            if !why.is_empty() {
                eprintln!("devdeck-ctl: {why}");
            }
            eprintln!("{USAGE}");
            return ExitCode::from(64);
        }
    };
    let socket = match args.socket.clone().map_or_else(|| default_socket(|k| std::env::var(k).ok()), Ok) {
        Ok(s) => s,
        Err(why) => {
            eprintln!("devdeck-ctl: no socket ({why}): set DEVDECK_SOCKET or pass --socket");
            return ExitCode::from(2);
        }
    };
    let mut client = match Client::connect(&socket) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("devdeck-ctl: {}: {e} (is devdeck running?)", socket.display());
            return ExitCode::from(2);
        }
    };
    if let Command::Watch(topics) = &args.command {
        return watch(&mut client, topics, args.json);
    }
    let (method, params) = match &args.command {
        Command::Whoami => ("session.whoami".to_string(), Value::Object(Default::default())),
        Command::Describe(m) => ("api.describe".to_string(), m.as_ref().map_or_else(|| serde_json::json!({}), |m| serde_json::json!({ "method": m }))),
        Command::Call(m, p) => (m.clone(), p.clone()),
        Command::Watch(_) => unreachable!("watched above"),
    };
    match client.call_raw(&method, params) {
        Ok(v) => {
            print!("{}", if args.json { pretty(&v) } else { plain(&args.command, &v) });
            ExitCode::SUCCESS
        }
        Err(ClientError::Api(e)) => {
            eprintln!("devdeck-ctl: {e}");
            ExitCode::from(1)
        }
        Err(e) => {
            eprintln!("devdeck-ctl: {e}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(args: &[&str]) -> Result<Args, String> {
        parse(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn arguments() {
        assert_eq!(p(&["whoami"]).unwrap().command, Command::Whoami);
        assert_eq!(p(&["--json", "describe", "run.get"]).unwrap(), Args { socket: None, json: true, command: Command::Describe(Some("run.get".into())) });
        let a = p(&["call", "run.get", r#"{"app":"a","cmd":"c"}"#, "--socket", "/s"]).unwrap();
        assert_eq!(a.socket, Some("/s".into()));
        assert_eq!(a.command, Command::Call("run.get".into(), serde_json::json!({ "app": "a", "cmd": "c" })));
        assert_eq!(p(&["call", "app.list"]).unwrap().command, Command::Call("app.list".into(), serde_json::json!({})));
        assert!(p(&["call"]).is_err());
        assert!(p(&["call", "x", "{bad"]).unwrap_err().starts_with("params"));
        assert!(p(&["call", "x", "[1]"]).unwrap_err().starts_with("params"), "an object only");
        assert!(p(&["whoami", "extra"]).is_err());
        assert!(p(&["--socket"]).is_err());
        assert_eq!(p(&["watch", "runs", "notes"]).unwrap().command, Command::Watch(vec!["runs".into(), "notes".into()]));
        assert!(p(&["watch"]).is_err());
    }

    #[test]
    fn events_read_as_one_line_each() {
        let e = serde_json::json!({"jsonrpc": "2.0", "method": "event", "params": {"seq": 3, "topic": "runs", "data": {"app": "web", "state": "running"}}});
        assert_eq!(event_line(&e, false), r#"3 runs {"app":"web","state":"running"}"#);
        let l = serde_json::json!({"method": "events.lagged", "params": {"seq": 9}});
        assert!(event_line(&l, false).starts_with("lagged: events from 9"));
        assert!(event_line(&e, true).starts_with('{'));
    }
}
