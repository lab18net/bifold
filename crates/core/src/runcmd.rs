//! Running a command line from the file manager (Enter on an executable).
//!
//! Two ways to run, one per front end:
//! * [`run_inherited`] gives the command the real terminal (the TUI suspends
//!   itself first), like a shell does — interactive and full-screen programs
//!   work, and its output stays on the terminal for `Ctrl+O` to show.
//! * [`run_captured`] collects stdout+stderr with a timeout, for the desktop
//!   front end, which has no terminal to hand over. A command still running at
//!   the deadline is left running ("detached"), never killed: it may be a GUI
//!   app the user just launched.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::error::{Error, Result};
use crate::preview::MAX_OUTPUT;

pub const DEFAULT_TIMEOUT: Duration = Duration::from_millis(10_000);

/// What to pre-fill in the run line for an executable called `name` in the
/// current folder: `./name`, single-quoted when it has odd characters.
pub fn command_for(name: &str) -> String {
    let plain = !name.is_empty()
        && name.chars().all(|c| {
            c.is_ascii_alphanumeric()
                || matches!(c, '.' | '_' | '-' | '+' | ',' | '@' | '%' | ':' | '=')
        });
    if plain {
        format!("./{name}")
    } else {
        format!("'./{}'", name.replace('\'', "'\\''"))
    }
}

/// One word for `sh`: unchanged when it only has safe characters, otherwise
/// single-quoted (embedded quotes escaped).
pub fn quote_word(word: &str) -> String {
    let plain = !word.is_empty()
        && word.chars().all(|c| {
            c.is_ascii_alphanumeric()
                || matches!(c, '.' | '_' | '-' | '+' | ',' | '@' | '%' | ':' | '=' | '/')
        });
    if plain {
        word.to_string()
    } else {
        format!("'{}'", word.replace('\'', "'\\''"))
    }
}

/// The terminal editor to use: the `editor` setting, else `$VISUAL`, else
/// `$EDITOR`, else `vi`. Blank values count as unset.
pub fn resolve_editor(configured: &str, visual: Option<&str>, editor_env: Option<&str>) -> String {
    [Some(configured), visual, editor_env]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|s| !s.is_empty())
        .unwrap_or("vi")
        .to_string()
}

/// What the desktop app uses when `gui_editor` is empty: the system's default
/// text editor / handler.
pub fn default_gui_editor() -> &'static str {
    if cfg!(target_os = "macos") {
        "open -t"
    } else {
        "xdg-open"
    }
}

/// `editor` followed by the file name as a single shell word. A name starting
/// with `-` gets `./` so the editor cannot read it as an option.
pub fn edit_line(editor: &str, name: &str) -> String {
    let arg = if name.starts_with('-') {
        format!("./{name}")
    } else {
        name.to_string()
    };
    format!("{} {}", editor.trim(), quote_word(&arg))
}

#[derive(Debug, Clone)]
pub struct RunOutcome {
    pub command: String,
    /// Folder it ran in.
    pub cwd: PathBuf,
    /// `None`: killed by a signal, or still running (`detached`).
    pub exit: Option<i32>,
    /// Still running when the deadline passed; output is what arrived so far.
    pub detached: bool,
    pub output: String,
    /// More output arrived than the cap kept.
    pub truncated: bool,
    pub waited: Duration,
}

impl RunOutcome {
    pub fn summary(&self) -> String {
        match (self.exit, self.detached) {
            (_, true) => format!(
                "still running after {}s (left running): {}",
                self.waited.as_secs(),
                self.command
            ),
            (Some(0), _) => format!("ran: {} (exit 0)", self.command),
            (Some(code), _) => format!("ran: {} — exit {code}", self.command),
            (None, _) => format!("ran: {} — killed by a signal", self.command),
        }
    }

    pub fn succeeded(&self) -> bool {
        self.exit == Some(0) || self.detached
    }
}

/// Run `command` with the terminal's stdin/stdout/stderr. Returns the exit
/// code (`None` when a signal ended it).
///
/// While the command owns the terminal, Ctrl+C must stop *it*, not us: SIGINT
/// is ignored here and reset to the default in the child.
pub fn run_inherited(command: &str, cwd: &Path) -> Result<Option<i32>> {
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(command).current_dir(cwd);

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: `signal` is async-signal-safe, which is all `pre_exec` allows.
        unsafe {
            cmd.pre_exec(|| {
                libc::signal(libc::SIGINT, libc::SIG_DFL);
                Ok(())
            });
        }
        // SAFETY: plain signal-disposition changes, restored below.
        let previous = unsafe { libc::signal(libc::SIGINT, libc::SIG_IGN) };
        let status = cmd.status();
        unsafe { libc::signal(libc::SIGINT, previous) };
        status
            .map(|s| s.code())
            .map_err(|e| Error::invalid(format!("cannot run sh: {e}")))
    }
    #[cfg(not(unix))]
    {
        cmd.status()
            .map(|s| s.code())
            .map_err(|e| Error::invalid(format!("cannot run sh: {e}")))
    }
}

/// Run `command` in `cwd`, collecting stdout and stderr together.
pub fn run_captured(command: &str, cwd: &Path, timeout: Duration) -> Result<RunOutcome> {
    let started = Instant::now();
    // One pipe for both streams keeps the output in the order it happened. Wired
    // at the OS level: a shell-side `exec 2>&1` would miss syntax errors, which
    // sh reports before running anything.
    let (mut reader, writer) =
        std::io::pipe().map_err(|e| Error::invalid(format!("cannot create a pipe: {e}")))?;
    let writer_copy = writer
        .try_clone()
        .map_err(|e| Error::invalid(format!("cannot create a pipe: {e}")))?;
    let mut cmd = Command::new("sh");
    cmd.arg("-c")
        .arg(command)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(writer)
        .stderr(writer_copy);
    let mut child = cmd
        .spawn()
        .map_err(|e| Error::invalid(format!("cannot run sh: {e}")))?;
    // The Command still owns our copies of the write ends; without this the
    // reader would never see EOF.
    drop(cmd);

    let shared = Arc::new(Mutex::new((Vec::<u8>::new(), false)));
    let sink = Arc::clone(&shared);
    let (done_tx, done_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = [0u8; 16 * 1024];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 {
                break;
            }
            // Keep draining past the cap (and past our deadline) so the
            // command never blocks, or dies of SIGPIPE, on a full pipe.
            if let Ok(mut guard) = sink.lock() {
                let room = MAX_OUTPUT.saturating_sub(guard.0.len());
                guard.0.extend_from_slice(&buf[..n.min(room)]);
                if n > room {
                    guard.1 = true;
                }
            }
        }
        let _ = done_tx.send(());
    });

    let deadline = started + timeout;
    let finished = loop {
        match done_rx.recv_timeout(Duration::from_millis(10)) {
            Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break true,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if Instant::now() >= deadline {
            break false;
        }
    };

    let exit = if finished {
        child.wait().ok().and_then(|s| s.code())
    } else {
        // Reap it whenever it ends, without blocking the caller.
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        None
    };
    let (bytes, truncated) = shared
        .lock()
        .map(|g| (g.0.clone(), g.1))
        .unwrap_or_default();
    Ok(RunOutcome {
        command: command.to_string(),
        cwd: cwd.to_path_buf(),
        exit,
        detached: !finished,
        output: String::from_utf8_lossy(&bytes).into_owned(),
        truncated,
        waited: started.elapsed(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_is_the_setting_then_visual_then_editor_then_vi() {
        assert_eq!(resolve_editor("hx", Some("nano"), Some("vim")), "hx");
        assert_eq!(resolve_editor("  ", Some("nano"), Some("vim")), "nano");
        assert_eq!(resolve_editor("", None, Some("vim -p")), "vim -p");
        assert_eq!(resolve_editor("", Some(""), Some(" ")), "vi");
        assert_eq!(resolve_editor("", None, None), "vi");
    }

    #[test]
    fn the_edit_line_quotes_the_name_and_guards_option_lookalikes() {
        assert_eq!(edit_line("hx", "main.rs"), "hx main.rs");
        assert_eq!(edit_line(" vim -p ", "my file.txt"), "vim -p 'my file.txt'");
        assert_eq!(edit_line("hx", "it's.md"), "hx 'it'\\''s.md'");
        assert_eq!(edit_line("hx", "-rf"), "hx ./-rf");
        assert_eq!(edit_line("hx", "$(touch x).txt"), "hx '$(touch x).txt'");
    }

    #[test]
    fn quote_word_leaves_safe_words_alone() {
        assert_eq!(quote_word("a/b-c_1.txt"), "a/b-c_1.txt");
        assert_eq!(quote_word(""), "''");
        assert_eq!(quote_word("a b"), "'a b'");
    }

    #[test]
    fn suggests_a_quoted_relative_command() {
        assert_eq!(command_for("run.sh"), "./run.sh");
        assert_eq!(command_for("-rf"), "./-rf");
        assert_eq!(command_for("my file.sh"), "'./my file.sh'");
        assert_eq!(command_for("it's"), "'./it'\\''s'");
        assert_eq!(command_for("a;rm -rf x"), "'./a;rm -rf x'");
        assert_eq!(command_for("$(x)"), "'./$(x)'");
    }

    #[test]
    fn captures_both_streams_in_order_with_the_exit_code() {
        let dir = tempfile::tempdir().unwrap();
        let out = run_captured(
            "echo one; echo two 1>&2; echo three; exit 3",
            dir.path(),
            DEFAULT_TIMEOUT,
        )
        .unwrap();
        assert_eq!(out.output, "one\ntwo\nthree\n");
        assert_eq!(out.exit, Some(3));
        assert!(!out.detached && !out.truncated);
        assert!(out.summary().contains("exit 3"), "{}", out.summary());
        assert!(!out.succeeded());
    }

    #[test]
    fn runs_in_the_given_folder() {
        let dir = tempfile::tempdir().unwrap();
        let out = run_captured("pwd", dir.path(), DEFAULT_TIMEOUT).unwrap();
        assert_eq!(
            std::fs::canonicalize(out.output.trim()).unwrap(),
            std::fs::canonicalize(dir.path()).unwrap()
        );
        assert!(out.succeeded() && out.summary().ends_with("(exit 0)"));
    }

    #[test]
    fn a_syntax_error_is_reported_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let out = run_captured("if then fi", dir.path(), DEFAULT_TIMEOUT).unwrap();
        assert_ne!(out.exit, Some(0));
        assert!(
            !out.output.is_empty(),
            "sh explains itself on the shared pipe"
        );
    }

    #[test]
    fn a_slow_command_is_detached_with_its_partial_output() {
        let dir = tempfile::tempdir().unwrap();
        let started = Instant::now();
        let out = run_captured(
            "echo started; sleep 2",
            dir.path(),
            Duration::from_millis(400),
        )
        .unwrap();
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "returned at the deadline"
        );
        assert!(out.detached && out.exit.is_none());
        assert_eq!(out.output, "started\n");
        assert!(out.summary().contains("left running"), "{}", out.summary());
        assert!(out.succeeded(), "detaching is not a failure");
    }

    #[test]
    fn output_is_capped_without_stalling_the_command() {
        let dir = tempfile::tempdir().unwrap();
        let out = run_captured(
            "head -c 6000000 /dev/zero | tr '\\0' x",
            dir.path(),
            DEFAULT_TIMEOUT,
        )
        .unwrap();
        assert!(out.truncated);
        assert_eq!(out.output.len(), MAX_OUTPUT);
        assert_eq!(out.exit, Some(0), "the command was drained to the end");
    }

    #[cfg(unix)]
    #[test]
    fn inherited_run_returns_the_exit_code_and_restores_sigint() {
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: test-only probe of the process-wide disposition.
        unsafe { libc::signal(libc::SIGINT, libc::SIG_DFL) };
        assert_eq!(run_inherited("exit 4", dir.path()).unwrap(), Some(4));
        assert_eq!(run_inherited("true", dir.path()).unwrap(), Some(0));
        let after = unsafe { libc::signal(libc::SIGINT, libc::SIG_DFL) };
        assert_eq!(
            after,
            libc::SIG_DFL,
            "our SIGINT handling is back to what it was"
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_child_can_be_interrupted_even_though_we_ignore_sigint() {
        let dir = tempfile::tempdir().unwrap();
        // If SIG_IGN leaked into the child it would print and exit 0 instead of dying.
        assert_eq!(
            run_inherited("kill -INT $$; echo survived", dir.path()).unwrap(),
            None
        );
    }
}
