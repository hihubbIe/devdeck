//! Reading a file to show it (R12.1): text up to [`MAX_BYTES`], or why it can't be shown.

use std::io::Read;
use std::path::Path;

/// Bigger files aren't shown.
pub const MAX_BYTES: u64 = 10_000_000;

/// A NUL byte in this much of the start makes a file binary.
const SNIFF: usize = 8 * 1024;

/// What a file holds, as far as showing it goes.
#[derive(Clone, Debug, PartialEq)]
pub enum FileText {
    Text(String),
    /// Over [`MAX_BYTES`]: its size.
    TooBig(u64),
    /// Binary: a NUL byte near the start, or not UTF-8.
    NotText,
    Missing,
    /// Why it can't be read (a folder, no permission…).
    Unreadable(String),
}

impl FileText {
    /// What to say instead of the text, if anything.
    pub fn problem(&self, path: &Path) -> Option<String> {
        let name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
        match self {
            FileText::Text(_) => None,
            FileText::TooBig(size) => Some(format!("{name} is {} MB: files over {} MB aren't shown.", size / 1_000_000, MAX_BYTES / 1_000_000)),
            FileText::NotText => Some(format!("{name} isn't text.")),
            FileText::Missing => Some(format!("{name} isn't there anymore.")),
            FileText::Unreadable(why) => Some(format!("{name} can't be read: {why}.")),
        }
    }
}

/// Reads `path` to show it.
pub fn read_text(path: &Path) -> FileText {
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return FileText::Missing,
        Err(e) => return FileText::Unreadable(e.to_string()),
    };
    if meta.is_dir() {
        return FileText::Unreadable("it's a folder".into());
    }
    if meta.len() > MAX_BYTES {
        return FileText::TooBig(meta.len());
    }
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    let read = std::fs::File::open(path).and_then(|f| f.take(MAX_BYTES + 1).read_to_end(&mut bytes));
    match read {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return FileText::Missing,
        Err(e) => return FileText::Unreadable(e.to_string()),
        Ok(n) if n as u64 > MAX_BYTES => return FileText::TooBig(n as u64),
        Ok(_) => {}
    }
    if bytes[..bytes.len().min(SNIFF)].contains(&0) {
        return FileText::NotText;
    }
    String::from_utf8(bytes).map_or(FileText::NotText, FileText::Text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    #[test]
    fn reads_text_and_says_why_not() {
        let dir = TempDir::new("files");
        let p = |name: &str| dir.join(name);
        std::fs::write(p("a.rs"), "fn main() {}\n").unwrap();
        assert_eq!(read_text(&p("a.rs")), FileText::Text("fn main() {}\n".into()));
        std::fs::write(p("bin"), b"ab\0cd").unwrap();
        assert_eq!(read_text(&p("bin")), FileText::NotText);
        std::fs::write(p("latin1"), [0x63, 0x61, 0x66, 0xe9]).unwrap();
        assert_eq!(read_text(&p("latin1")), FileText::NotText);
        assert_eq!(read_text(&p("gone")), FileText::Missing);
        assert!(matches!(read_text(&dir), FileText::Unreadable(_)));
        let big = std::fs::File::create(p("big")).unwrap();
        big.set_len(MAX_BYTES + 1).unwrap();
        assert_eq!(read_text(&p("big")), FileText::TooBig(MAX_BYTES + 1));
        assert_eq!(read_text(&p("big")).problem(&p("big")).unwrap(), "big is 10 MB: files over 10 MB aren't shown.");
        assert_eq!(FileText::Missing.problem(&p("a.rs")).unwrap(), "a.rs isn't there anymore.");
    }
}
