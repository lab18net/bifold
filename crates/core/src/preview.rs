//! Text preview for the built-in viewer. Binary files are detected, not dumped.

use std::fs::File;
use std::io::{Read, Take};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::error::{Error, Result};

/// How much of a file the viewer holds in memory.
pub const MAX_BYTES: u64 = 1024 * 1024;
/// Hard cap on rendered lines (protects the UI from minified files).
pub const MAX_LINES: usize = 50_000;

#[derive(Debug, Clone)]
pub struct Preview {
    pub path: PathBuf,
    pub lines: Vec<String>,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub binary: bool,
    /// File was larger than the read budget.
    pub truncated: bool,
}

impl Preview {
    pub fn title(&self) -> String {
        let mut title = self
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.display().to_string());
        if self.binary {
            title.push_str("  [binary]");
        } else if self.truncated {
            title.push_str("  [truncated]");
        }
        title
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }
}

pub fn load(path: &Path, max_bytes: u64) -> Result<Preview> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| Error::io(path, e))?;
    if meta.is_dir() {
        return Err(Error::invalid("directories cannot be viewed"));
    }
    let file = File::open(path).map_err(|e| Error::io(path, e))?;
    let size = meta.len();
    let mut handle: Take<File> = file.take(max_bytes);
    let mut bytes = Vec::with_capacity(size.min(max_bytes) as usize);
    handle
        .read_to_end(&mut bytes)
        .map_err(|e| Error::io(path, e))?;

    let binary = bytes.iter().take(8192).any(|byte| *byte == 0);
    let lines = if binary {
        Vec::new()
    } else {
        let text = String::from_utf8_lossy(&bytes);
        let mut lines: Vec<String> = text
            .split('\n')
            .map(|line| line.trim_end_matches('\r').to_string())
            .collect();
        if lines.len() > MAX_LINES {
            lines.truncate(MAX_LINES);
        }
        lines
    };

    Ok(Preview {
        path: path.to_path_buf(),
        lines,
        size,
        modified: meta.modified().ok(),
        binary,
        truncated: size > max_bytes,
    })
}

pub fn load_default(path: &Path) -> Result<Preview> {
    load(path, MAX_BYTES)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_text() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hello.txt");
        std::fs::write(&path, "one\ntwo\r\nthree\n").unwrap();
        let preview = load_default(&path).unwrap();
        assert!(!preview.binary);
        assert_eq!(preview.lines, ["one", "two", "three", ""]);
        assert_eq!(preview.line_count(), 4);
        assert!(preview.title().starts_with("hello.txt"));
    }

    #[test]
    fn detects_binary() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("blob.bin");
        std::fs::write(&path, [0u8, 1, 2, 3, 0, 255]).unwrap();
        let preview = load_default(&path).unwrap();
        assert!(preview.binary);
        assert!(preview.lines.is_empty());
        assert!(preview.title().contains("[binary]"));
    }

    #[test]
    fn truncates_at_budget() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.txt");
        std::fs::write(&path, "x".repeat(4096)).unwrap();
        let preview = load(&path, 100).unwrap();
        assert!(preview.truncated);
        assert_eq!(preview.lines.join("").len(), 100);
        assert!(preview.title().contains("[truncated]"));
    }

    #[test]
    fn directories_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_default(dir.path()).is_err());
    }
}
