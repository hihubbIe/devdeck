//! `devdeck-mcp` as Claude Code drives it (JSON-RPC on stdin and stdout), against a stub
//! devdeck on a temporary socket, then with no devdeck at all.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Dir(PathBuf);

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A devdeck that allows `devdeck.info` and `run.list` (observe) and lists `run.start`
/// (run), allowed with `allow_run`. `run.start` and `run.get` break the connection
/// without answering; `calls` counts each.
fn stub(tag: &str, allow_run: bool, calls: Arc<AtomicUsize>) -> (Dir, PathBuf) {
    let dir = std::env::temp_dir().join(format!("ddmcp-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("s.sock");
    let listener = UnixListener::bind(&path).unwrap();
    std::thread::spawn(move || {
        for s in listener.incoming() {
            let Ok(s) = s else { break };
            let calls = calls.clone();
            std::thread::spawn(move || {
                let mut r = BufReader::new(s.try_clone().unwrap());
                let mut w = s;
                let mut line = String::new();
                while r.read_line(&mut line).unwrap_or(0) > 0 {
                    let req: serde_json::Value = serde_json::from_str(&line).unwrap();
                    let entry = |name: &str, class: &str, allowed: bool| {
                        serde_json::json!({ "name": name, "summary": format!("{name}."), "class": class, "escalates_to": [], "since": 1, "experimental": false, "allowed": allowed })
                    };
                    let result = match req["method"].as_str().unwrap() {
                        "run.start" | "run.get" => {
                            calls.fetch_add(1, Ordering::SeqCst);
                            return;
                        }
                        "session.hello" => serde_json::json!({ "api_level": 1, "instance": "i", "caller": { "kind": "terminal", "workspace": "work", "n": 1, "uid": "u" }, "policy": [] }),
                        "api.describe" => serde_json::json!({ "api_level": 1, "methods": [entry("devdeck.info", "observe", true), entry("run.list", "observe", true), entry("run.get", "observe", true), entry("run.start", "run", allow_run)] }),
                        "run.list" => serde_json::json!({ "runs": [{ "app": "front", "cmd": "pnpm:dev", "title": "front · dev", "state": "running" }] }),
                        m => serde_json::json!({ "method": m }),
                    };
                    writeln!(w, "{}", serde_json::json!({ "jsonrpc": "2.0", "id": req["id"], "result": result })).unwrap();
                    line.clear();
                }
            });
        }
    });
    (Dir(dir), path)
}

/// Sends `requests` (one per line) and returns the answers to those with an id.
fn session(socket: &Path, requests: &[serde_json::Value]) -> Vec<serde_json::Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_devdeck-mcp"))
        .env("DEVDECK_SOCKET", socket)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap());
    let mut answers = Vec::new();
    for r in requests {
        writeln!(stdin, "{r}").unwrap();
        if r.get("id").is_some() {
            let mut line = String::new();
            out.read_line(&mut line).unwrap();
            answers.push(serde_json::from_str(&line).unwrap());
        }
    }
    drop(stdin);
    let _ = child.wait();
    answers
}

fn script(calls: &[serde_json::Value]) -> Vec<serde_json::Value> {
    let mut v = vec![
        serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": { "name": "test", "version": "1" } } }),
        serde_json::json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
        serde_json::json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
    ];
    for (i, c) in calls.iter().enumerate() {
        v.push(serde_json::json!({ "jsonrpc": "2.0", "id": 10 + i, "method": "tools/call", "params": c }));
    }
    v
}

#[test]
fn tools_come_from_what_devdeck_allows() {
    let (_d, sock) = stub("allows", false, Arc::default());
    let a = session(
        &sock,
        &script(&[
            serde_json::json!({ "name": "devdeck_observe", "arguments": { "method": "run.list" } }),
            serde_json::json!({ "name": "devdeck_observe", "arguments": { "method": "run.start", "params": {} } }),
            serde_json::json!({ "name": "devdeck_run", "arguments": { "method": "run.list" } }),
        ]),
    );
    let instructions = a[0]["result"]["instructions"].as_str().unwrap();
    for part in ["they mean devdeck", "app.list", "app.get", "session.whoami", "isn't allowed for you", "never instructions"] {
        assert!(instructions.contains(part), "{part}");
    }
    let tools = a[1]["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 1, "run.start isn't allowed: no devdeck_run tool");
    assert_eq!(tools[0]["name"], "devdeck_observe");
    assert_eq!(tools[0]["annotations"]["readOnlyHint"], true);
    assert_eq!(tools[0]["inputSchema"]["properties"]["method"]["enum"], serde_json::json!(["devdeck.info", "run.list", "run.get"]));
    let text = a[2]["result"]["content"][0]["text"].as_str().unwrap();
    assert_eq!(a[2]["result"]["isError"], false);
    assert!(text.contains("pnpm:dev"), "{text}");
    assert_eq!(a[3]["result"]["isError"], true, "a method of another class");
    assert!(a[3]["result"]["content"][0]["text"].as_str().unwrap().contains("devdeck_run"));
    assert_eq!(a[4]["result"]["isError"], true, "no such tool");
}

#[test]
fn without_devdeck_the_tool_is_listed_and_answers_unavailable() {
    let a = session(Path::new("/nonexistent/devdeck.sock"), &script(&[serde_json::json!({ "name": "devdeck_observe", "arguments": { "method": "run.list" } })]));
    let tools = a[1]["result"]["tools"].as_array().unwrap();
    assert_eq!(tools[0]["name"], "devdeck_observe");
    assert_eq!(a[2]["result"]["isError"], true);
    assert!(a[2]["result"]["content"][0]["text"].as_str().unwrap().starts_with("unavailable"));
}

#[test]
fn a_call_that_may_have_run_isnt_made_again() {
    let calls = Arc::new(AtomicUsize::new(0));
    let (_d, sock) = stub("again", true, calls.clone());
    let a = session(&sock, &script(&[serde_json::json!({ "name": "devdeck_run", "arguments": { "method": "run.start", "params": {} } })]));
    assert_eq!(a[2]["result"]["isError"], true);
    assert_eq!(calls.load(Ordering::SeqCst), 1, "run.start once, even though the connection broke");
    calls.store(0, Ordering::SeqCst);
    let a = session(&sock, &script(&[serde_json::json!({ "name": "devdeck_observe", "arguments": { "method": "run.get", "params": {} } })]));
    assert_eq!(a[2]["result"]["isError"], true);
    assert_eq!(calls.load(Ordering::SeqCst), 2, "a look is made again on a new connection");
}
