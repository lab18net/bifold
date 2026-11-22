//! File viewer content, modelled on lf's previewer.
//!
//! 1. If a `previewer` command is configured it is run as
//!    `previewer <file> <width> <height> <x> <y>` (lf's argument order) and its
//!    stdout — ANSI colours allowed — is the preview. Any lf previewer script
//!    works unchanged.
//! 2. Otherwise (or when the previewer fails or times out) the built-in viewer
//!    is used: text as text, binary content as a hex dump.
//!
//! The viewer can also be forced into hex mode for any file.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime};

use crate::error::{Error, Result};
use crate::fsops;
use crate::style::{Color, StyledLine, parse_ansi};

/// How much of a file the viewer holds in memory.
pub const MAX_BYTES: u64 = 1024 * 1024;
/// Hard cap on rendered lines (protects the UI from minified files).
pub const MAX_LINES: usize = 50_000;
/// Cap on previewer output.
pub const MAX_OUTPUT: usize = 4 * 1024 * 1024;
const HEX_COLUMNS: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewKind {
    Text,
    Hex,
    /// Produced by the configured previewer command.
    External,
}

#[derive(Debug, Clone)]
pub struct Preview {
    pub path: PathBuf,
    pub lines: Vec<StyledLine>,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub kind: PreviewKind,
    /// Content was cut at the read or line budget.
    pub truncated: bool,
    /// Why the requested mode was not used (e.g. previewer failed).
    pub notice: Option<String>,
}

impl Preview {
    pub fn title(&self) -> String {
        let mut title = self
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.display().to_string());
        match self.kind {
            PreviewKind::Text => {}
            PreviewKind::Hex => title.push_str("  [hex]"),
            PreviewKind::External => title.push_str("  [previewer]"),
        }
        if self.truncated {
            title.push_str("  [truncated]");
        }
        if let Some(notice) = &self.notice {
            title.push_str(&format!("  ({notice})"));
        }
        title
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    pub fn plain_lines(&self) -> Vec<String> {
        self.lines.iter().map(StyledLine::plain).collect()
    }
}

#[derive(Debug, Clone)]
pub struct PreviewOptions {
    pub max_bytes: u64,
    /// Command line of the previewer (program plus optional leading arguments).
    pub previewer: Option<String>,
    pub timeout: Duration,
    /// Size of the area the preview will be drawn in, passed to the previewer.
    pub width: u16,
    pub height: u16,
    pub x: u16,
    pub y: u16,
    pub force_hex: bool,
}

impl Default for PreviewOptions {
    fn default() -> Self {
        Self {
            max_bytes: MAX_BYTES,
            previewer: None,
            timeout: Duration::from_millis(3000),
            width: 80,
            height: 24,
            x: 0,
            y: 0,
            force_hex: false,
        }
    }
}

pub fn load_default(path: &Path) -> Result<Preview> {
    load(path, &PreviewOptions::default())
}

pub fn load(path: &Path, options: &PreviewOptions) -> Result<Preview> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| Error::io(path, e))?;
    if meta.is_dir() {
        return Err(Error::invalid("directories cannot be viewed"));
    }
    let mut preview = Preview {
        path: path.to_path_buf(),
        lines: Vec::new(),
        size: meta.len(),
        modified: meta.modified().ok(),
        kind: PreviewKind::Text,
        truncated: false,
        notice: None,
    };

    if !options.force_hex
        && let Some(command) = options
            .previewer
            .as_deref()
            .filter(|c| !c.trim().is_empty())
    {
        match run_previewer(command, path, options) {
            Ok(lines) => {
                preview.lines = lines;
                preview.kind = PreviewKind::External;
                return Ok(preview);
            }
            Err(reason) => preview.notice = Some(format!("previewer: {reason}")),
        }
    }

    let bytes = read_head(path, options.max_bytes)?;
    preview.truncated = meta.len() > options.max_bytes;

    if options.force_hex || looks_binary(&bytes) {
        preview.kind = PreviewKind::Hex;
        let (lines, cut) = hex_dump(&bytes);
        preview.lines = lines;
        preview.truncated |= cut;
    } else {
        let text = String::from_utf8_lossy(&bytes);
        let mut lines: Vec<StyledLine> = text
            .split('\n')
            .map(|line| StyledLine::plain_text(line.trim_end_matches('\r')))
            .collect();
        if lines.len() > MAX_LINES {
            lines.truncate(MAX_LINES);
            preview.truncated = true;
        }
        preview.lines = lines;
    }
    Ok(preview)
}

fn read_head(path: &Path, max_bytes: u64) -> Result<Vec<u8>> {
    let file = File::open(path).map_err(|e| Error::io(path, e))?;
    let mut bytes = Vec::new();
    file.take(max_bytes)
        .read_to_end(&mut bytes)
        .map_err(|e| Error::io(path, e))?;
    Ok(bytes)
}

/// A NUL byte in the first 8 KiB, or content that is not valid UTF-8 text.
fn looks_binary(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(8192)];
    if head.contains(&0) {
        return true;
    }
    match std::str::from_utf8(head) {
        Ok(_) => false,
        // A multi-byte character cut by the window is fine; anything else is not.
        Err(e) => e.error_len().is_some(),
    }
}

/// `00000000  48 65 6c 6c 6f 20 77 6f  72 6c 64 0a              |Hello world.|`
pub fn hex_dump(bytes: &[u8]) -> (Vec<StyledLine>, bool) {
    let dim = Some(Color::Faint);
    let accent = Some(Color::Accent);
    let mut lines = Vec::new();
    let mut cut = false;
    for (row, chunk) in bytes.chunks(HEX_COLUMNS).enumerate() {
        if lines.len() >= MAX_LINES {
            cut = true;
            break;
        }
        let mut line = StyledLine::default();
        line.push(format!("{:08x}", row * HEX_COLUMNS), dim);
        let mut hex = String::from(" ");
        for i in 0..HEX_COLUMNS {
            if i % 8 == 0 {
                hex.push(' ');
            }
            match chunk.get(i) {
                Some(b) => hex.push_str(&format!("{b:02x} ")),
                None => hex.push_str("   "),
            }
        }
        line.push(hex, None);
        let ascii: String = chunk
            .iter()
            .map(|b| {
                if (0x20..0x7f).contains(b) {
                    *b as char
                } else {
                    '.'
                }
            })
            .collect();
        line.push(format!("|{ascii}|"), accent);
        lines.push(line);
    }
    if lines.is_empty() {
        lines.push(StyledLine::plain_text("(empty file)"));
    }
    (lines, cut)
}

fn run_previewer(
    command: &str,
    path: &Path,
    options: &PreviewOptions,
) -> std::result::Result<Vec<StyledLine>, String> {
    let mut parts = command.split_whitespace();
    let program = parts.next().ok_or("empty command")?;
    let program = fsops::expand_home(program);

    let mut child = Command::new(&program)
        .args(parts)
        .arg(path)
        .arg(options.width.to_string())
        .arg(options.height.to_string())
        .arg(options.x.to_string())
        .arg(options.y.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("{}: {e}", program.display()))?;

    let mut stdout = child.stdout.take().ok_or("no stdout")?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut kept = Vec::new();
        let mut buf = [0u8; 16 * 1024];
        while let Ok(n) = stdout.read(&mut buf) {
            if n == 0 {
                break;
            }
            // Keep draining past the cap so the child never blocks on a full pipe.
            if kept.len() < MAX_OUTPUT {
                let room = MAX_OUTPUT - kept.len();
                kept.extend_from_slice(&buf[..n.min(room)]);
            }
        }
        let _ = tx.send(kept);
    });

    let deadline = Instant::now() + options.timeout;
    let output = loop {
        match rx.recv_timeout(Duration::from_millis(10)) {
            Ok(bytes) => break bytes,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break Vec::new(),
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "timed out after {} ms",
                options.timeout.as_millis()
            ));
        }
    };
    let status = child.wait().map_err(|e| e.to_string())?;
    if output.is_empty() {
        return Err(match status.code() {
            Some(code) => format!("no output (exit {code})"),
            None => "no output (killed by signal)".to_string(),
        });
    }
    let mut lines = parse_ansi(&String::from_utf8_lossy(&output));
    if lines.len() > MAX_LINES {
        lines.truncate(MAX_LINES);
    }
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[cfg(unix)]
    fn script(dir: &Path, body: &str) -> String {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("previewer.sh");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path.display().to_string()
    }

    #[test]
    fn reads_text() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "hello.txt", b"one\ntwo\r\n\tthree\n");
        let preview = load_default(&path).unwrap();
        assert_eq!(preview.kind, PreviewKind::Text);
        assert_eq!(preview.plain_lines(), ["one", "two", "    three", ""]);
        assert!(preview.title().starts_with("hello.txt"));
    }

    #[test]
    fn binary_becomes_hex_instead_of_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let mut data = b"Hello, world!\n".to_vec();
        data.extend_from_slice(&[0, 1, 2, 255]);
        let path = write(dir.path(), "blob.bin", &data);
        let preview = load_default(&path).unwrap();
        assert_eq!(preview.kind, PreviewKind::Hex);
        let first = preview.lines[0].plain();
        assert!(
            first.starts_with("00000000  48 65 6c 6c 6f 2c 20 77  6f 72 6c 64 21 0a 00 01"),
            "{first}"
        );
        assert!(first.ends_with("|Hello, world!...|"), "{first}");
        assert_eq!(preview.lines[1].plain().get(..8), Some("00000010"));
        assert!(preview.title().contains("[hex]"));
    }

    #[test]
    fn invalid_utf8_counts_as_binary_but_split_char_does_not() {
        assert!(looks_binary(&[0xff, 0xfe, b'a']));
        let snowman = "☃".as_bytes();
        assert!(!looks_binary(&snowman[..2]), "window cut mid-character");
        assert!(!looks_binary("héllo".as_bytes()));
    }

    #[test]
    fn force_hex_applies_to_text_and_empty_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "a.txt", b"abc");
        let options = PreviewOptions {
            force_hex: true,
            ..Default::default()
        };
        let preview = load(&path, &options).unwrap();
        assert_eq!(preview.kind, PreviewKind::Hex);
        assert!(preview.lines[0].plain().contains("61 62 63"));

        let empty = write(dir.path(), "empty", b"");
        assert_eq!(
            load(&empty, &options).unwrap().plain_lines(),
            ["(empty file)"]
        );
    }

    #[test]
    fn truncates_at_budget() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "big.txt", "x".repeat(4096).as_bytes());
        let options = PreviewOptions {
            max_bytes: 100,
            ..Default::default()
        };
        let preview = load(&path, &options).unwrap();
        assert!(preview.truncated);
        assert_eq!(preview.plain_lines().join("").len(), 100);
        assert!(preview.title().contains("[truncated]"));
    }

    #[test]
    fn directories_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_default(dir.path()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn previewer_receives_lf_style_arguments_and_ansi_is_parsed() {
        let dir = tempfile::tempdir().unwrap();
        let file = write(dir.path(), "f.bin", &[0, 1, 2]);
        let command = script(
            dir.path(),
            r#"printf '\033[31mred\033[0m %s|%s|%s|%s|%s\n' "$(basename "$1")" "$2" "$3" "$4" "$5""#,
        );
        let options = PreviewOptions {
            previewer: Some(command),
            width: 70,
            height: 20,
            x: 3,
            y: 4,
            ..Default::default()
        };
        let preview = load(&file, &options).unwrap();
        assert_eq!(preview.kind, PreviewKind::External);
        assert_eq!(preview.lines[0].plain(), "red f.bin|70|20|3|4");
        assert_eq!(preview.lines[0].spans[0].fg, Some(Color::Indexed(1)));
        assert!(preview.title().contains("[previewer]"));
    }

    #[cfg(unix)]
    #[test]
    fn previewer_can_be_bypassed_with_hex() {
        let dir = tempfile::tempdir().unwrap();
        let file = write(dir.path(), "f.txt", b"abc");
        let options = PreviewOptions {
            previewer: Some(script(dir.path(), "echo external")),
            force_hex: true,
            ..Default::default()
        };
        assert_eq!(load(&file, &options).unwrap().kind, PreviewKind::Hex);
    }

    #[cfg(unix)]
    #[test]
    fn failing_previewer_falls_back_with_a_notice() {
        let dir = tempfile::tempdir().unwrap();
        let file = write(dir.path(), "f.txt", b"fallback text");

        let missing = PreviewOptions {
            previewer: Some("/no/such/previewer".into()),
            ..Default::default()
        };
        let preview = load(&file, &missing).unwrap();
        assert_eq!(preview.kind, PreviewKind::Text);
        assert!(
            preview
                .notice
                .as_deref()
                .unwrap_or("")
                .contains("previewer")
        );

        let silent = PreviewOptions {
            previewer: Some(script(dir.path(), "exit 3")),
            ..Default::default()
        };
        let preview = load(&file, &silent).unwrap();
        assert_eq!(preview.plain_lines(), ["fallback text"]);
        assert!(preview.notice.unwrap().contains("exit 3"));
    }

    #[cfg(unix)]
    #[test]
    fn slow_previewer_is_killed() {
        let dir = tempfile::tempdir().unwrap();
        let file = write(dir.path(), "f.txt", b"text");
        let options = PreviewOptions {
            previewer: Some(script(dir.path(), "exec sleep 30")),
            timeout: Duration::from_millis(150),
            ..Default::default()
        };
        let started = Instant::now();
        let preview = load(&file, &options).unwrap();
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(preview.kind, PreviewKind::Text);
        assert!(preview.notice.unwrap().contains("timed out"));
    }
}
