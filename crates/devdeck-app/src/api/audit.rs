//! The API's audit (R13.8): JSON lines in the state folder (`api-audit.jsonl`, rotated
//! at 1 MB, one old file kept), written by a thread of its own. Phase 1 records
//! connections, refusals, denials and dropped notifications; later, every call that
//! changes something.

use serde::Serialize;
use std::io::Write;
use std::path::PathBuf;
use std::sync::mpsc;

pub const FILE: &str = "api-audit.jsonl";
const ROTATE_AT: u64 = 1_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Event {
    Connected,
    Refused,
    Denied,
    Notification,
    Expired,
}

impl Event {
    pub fn label(self) -> &'static str {
        match self {
            Event::Connected => "connected",
            Event::Refused => "refused",
            Event::Denied => "denied",
            Event::Notification => "notification dropped",
            Event::Expired => "shell ended",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Entry {
    /// Milliseconds since the Unix epoch.
    pub at_ms: u64,
    pub caller: String,
    pub event: Event,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl Entry {
    pub fn new(caller: String, event: Event, method: Option<String>, detail: Option<String>) -> Self {
        let at_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
        Self { at_ms, caller, event, method, detail }
    }
}

/// Hands entries to the writer thread (which ends when this is dropped).
pub struct Writer(mpsc::Sender<Entry>);

impl Writer {
    pub fn start(path: PathBuf) -> std::io::Result<Self> {
        let (tx, rx) = mpsc::channel::<Entry>();
        std::thread::Builder::new().name("devdeck-api-audit".into()).spawn(move || {
            let mut file = None;
            for e in rx {
                if let Err(err) = write(&path, &mut file, &e) {
                    eprintln!("devdeck: API audit {}: {err}", path.display());
                    file = None;
                }
            }
        })?;
        Ok(Self(tx))
    }

    pub fn write(&self, e: Entry) {
        let _ = self.0.send(e);
    }
}

fn write(path: &PathBuf, file: &mut Option<std::fs::File>, e: &Entry) -> std::io::Result<()> {
    if file.is_none() {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        *file = Some(std::fs::OpenOptions::new().create(true).append(true).open(path)?);
    }
    let f = file.as_mut().expect("opened");
    let mut line = serde_json::to_vec(e).map_err(std::io::Error::other)?;
    line.push(b'\n');
    f.write_all(&line)?;
    if f.metadata()?.len() > ROTATE_AT {
        *file = None;
        std::fs::rename(path, path.with_extension("jsonl.1"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use devdeck_core::testing::TempDir;

    #[test]
    fn entries_are_lines_and_the_file_rotates() {
        let dir = TempDir::new("api-audit");
        let path = dir.join(FILE);
        let mut file = None;
        let e = Entry::new("work/3".into(), Event::Denied, Some("run.get".into()), Some("x".repeat(400)));
        write(&path, &mut file, &e).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let v: serde_json::Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
        assert_eq!(v["event"], "denied");
        assert_eq!(v["method"], "run.get");
        while !dir.join("api-audit.jsonl.1").exists() {
            write(&path, &mut file, &e).unwrap();
        }
        write(&path, &mut file, &e).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap().lines().count(), 1, "a new file after the rotation");
    }
}
