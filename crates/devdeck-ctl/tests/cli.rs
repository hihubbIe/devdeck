//! The CLI against a stub server on a temporary socket (never a real devdeck: the
//! environment's `DEVDECK_SOCKET` is removed for the child).

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::process::Command;

struct Stub {
    dir: PathBuf,
    path: PathBuf,
}

impl Drop for Stub {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Answers each request by its method: a result, or a permission error.
fn stub(tag: &str) -> Stub {
    let dir = std::env::temp_dir().join(format!("ddctl-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("s.sock");
    let listener = UnixListener::bind(&path).unwrap();
    std::thread::spawn(move || {
        for s in listener.incoming() {
            let Ok(s) = s else { break };
            let mut r = BufReader::new(s.try_clone().unwrap());
            let mut w = s;
            let mut line = String::new();
            while r.read_line(&mut line).unwrap_or(0) > 0 {
                let req: serde_json::Value = serde_json::from_str(&line).unwrap();
                let id = req["id"].clone();
                let answer = match req["method"].as_str().unwrap() {
                    "session.whoami" => serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": {
                        "caller": { "kind": "terminal", "workspace": "work", "n": 3, "uid": "u-1" },
                        "workspace": "work", "terminal": { "n": 3, "uid": "u-1" },
                        "shown": [{ "layout": "default", "window": 0, "pane": 2 }] } }),
                    "run.get" => serde_json::json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32000, "message": "no", "data": { "kind": "permission_denied" } } }),
                    m => serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": { "method": m, "params": req["params"] } }),
                };
                writeln!(w, "{answer}").unwrap();
                line.clear();
            }
        }
    });
    Stub { dir, path }
}

fn ctl(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_devdeck-ctl")).args(args).env_remove("DEVDECK_SOCKET").output().unwrap();
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
}

#[test]
fn whoami_reads_as_text_or_json() {
    let s = stub("who");
    let sock = s.path.to_str().unwrap();
    let (code, out, _) = ctl(&["--socket", sock, "whoami"]);
    assert_eq!(code, 0);
    assert_eq!(out, "caller: terminal 3 of workspace work\nterminal: 3 (uid u-1)\nshown: layout default · window 0 · pane 2\n");
    let (code, out, _) = ctl(&["--socket", sock, "--json", "whoami"]);
    assert_eq!(code, 0);
    assert_eq!(serde_json::from_str::<serde_json::Value>(&out).unwrap()["terminal"]["n"], 3);
}

#[test]
fn calls_pass_their_params_and_errors_their_kind() {
    let s = stub("call");
    let sock = s.path.to_str().unwrap();
    let (code, out, _) = ctl(&["--socket", sock, "call", "app.get", r#"{"app":"front"}"#]);
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!((v["method"].as_str(), v["params"]["app"].as_str()), (Some("app.get"), Some("front")));
    let (code, _, err) = ctl(&["--socket", sock, "call", "run.get", "{}"]);
    assert_eq!(code, 1);
    assert!(err.contains("permission_denied"), "{err}");
}

#[test]
fn no_devdeck_and_bad_usage() {
    let (code, _, err) = ctl(&["--socket", "/nonexistent/devdeck.sock", "whoami"]);
    assert_eq!(code, 2);
    assert!(err.contains("is devdeck running"), "{err}");
    let (code, _, err) = ctl(&["teleport"]);
    assert_eq!(code, 64);
    assert!(err.contains("usage"));
}
