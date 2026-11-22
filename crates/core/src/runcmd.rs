//! Running a command line from the file manager (Enter on an executable).
//!
//! Two ways to run, one per front end:
//! * [`run_inherited`] gives the command the real terminal (the TUI suspends
//!   itself first), like a shell does — interactive programs work.
//! * [`run_captured`] collects stdout+stderr with a timeout, for the desktop
//!   front end, which has no terminal to hand over. A command still running at
//!   the deadline is left running ("detached"), never killed: it may be a GUI
//!   app the user just launched.

use std::io::{Read, Write};
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
    /// What the program wrote to its terminal, byte for byte (colours, `\r`
    /// rewrites…), so a terminal front end can show it again. Empty when the
    /// command did not run on a pty.
    pub raw: Vec<u8>,
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

    /// Bytes to write to a real terminal to show this output again: the raw
    /// pty stream when there is one, else the plain text. Terminal *mode*
    /// switches are removed (see [`strip_private_modes`]) so replaying a
    /// full-screen program cannot take the screen over.
    ///
    /// Line ends are `\r\n`, as a raw terminal needs. A terminal in normal
    /// (cooked) mode adds the `\r` itself; use [`RunOutcome::replay_bytes_cooked`] there.
    pub fn replay_bytes(&self) -> Vec<u8> {
        if self.raw.is_empty() {
            self.output.clone().into_bytes()
        } else {
            strip_private_modes(&self.raw)
        }
    }

    /// [`RunOutcome::replay_bytes`] for a terminal in cooked mode: `\r\n` becomes
    /// `\n` (the tty driver adds the `\r` back), a lone `\r` is kept.
    pub fn replay_bytes_cooked(&self) -> Vec<u8> {
        let bytes = self.replay_bytes();
        let mut out = Vec::with_capacity(bytes.len());
        for (i, &b) in bytes.iter().enumerate() {
            if b == b'\r' && bytes.get(i + 1) == Some(&b'\n') {
                continue;
            }
            out.push(b);
        }
        out
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
        raw: Vec::new(),
        truncated,
        waited: started.elapsed(),
    })
}

/// Run `command` on a pseudo-terminal wired to the real terminal: everything
/// it prints is shown live on `out` *and* recorded, so it can be viewed again
/// (`Action::LastOutput`). The program still sees a tty — colours, prompts and
/// Ctrl+C behave as in a shell. Keys typed meanwhile go to the program.
///
/// Unix only. The size of the pty is the terminal's size when the command
/// starts; resizing the window while it runs is not forwarded.
#[cfg(unix)]
pub fn run_in_pty(command: &str, cwd: &Path) -> Result<RunOutcome> {
    run_in_pty_to(command, cwd, &mut std::io::stdout())
}

#[cfg(not(unix))]
pub fn run_in_pty(command: &str, cwd: &Path) -> Result<RunOutcome> {
    let started = Instant::now();
    let exit = run_inherited(command, cwd)?;
    Ok(RunOutcome {
        command: command.to_string(),
        cwd: cwd.to_path_buf(),
        exit,
        detached: false,
        output: String::new(),
        raw: Vec::new(),
        truncated: false,
        waited: started.elapsed(),
    })
}

/// [`run_in_pty`] with the live mirror going to `out` (tests pass a buffer).
#[cfg(unix)]
pub fn run_in_pty_to(command: &str, cwd: &Path, out: &mut dyn Write) -> Result<RunOutcome> {
    use std::os::fd::{FromRawFd, OwnedFd};
    use std::os::unix::process::CommandExt;

    let started = Instant::now();
    let io_err =
        |what: &str| Error::invalid(format!("{what}: {}", std::io::Error::last_os_error()));

    // SAFETY: zeroed termios/winsize are valid plain-old-data; every libc call
    // below is given pointers to live locals, and fds are owned right away.
    let (master, slave, have_tty, saved) = unsafe {
        let have_tty = libc::isatty(0) == 1 && libc::isatty(1) == 1;
        let mut term: libc::termios = std::mem::zeroed();
        let mut size: libc::winsize = std::mem::zeroed();
        if have_tty {
            if libc::tcgetattr(0, &mut term) != 0 {
                return Err(io_err("cannot read terminal settings"));
            }
            libc::ioctl(1, libc::TIOCGWINSZ as _, &mut size);
        }
        let (mut m, mut sl) = (-1, -1);
        let rc = libc::openpty(
            &mut m,
            &mut sl,
            std::ptr::null_mut(),
            if have_tty {
                &mut term
            } else {
                std::ptr::null_mut()
            },
            if have_tty && size.ws_col > 0 {
                &mut size
            } else {
                std::ptr::null_mut()
            },
        );
        if rc != 0 {
            return Err(io_err("cannot open a pseudo-terminal"));
        }
        (
            OwnedFd::from_raw_fd(m),
            OwnedFd::from_raw_fd(sl),
            have_tty,
            term,
        )
    };
    let dup = |fd: &OwnedFd| {
        fd.try_clone()
            .map_err(|e| Error::invalid(format!("dup failed: {e}")))
    };

    let mut cmd = Command::new("sh");
    cmd.arg("-c")
        .arg(command)
        .current_dir(cwd)
        .stdin(Stdio::from(dup(&slave)?))
        .stdout(Stdio::from(dup(&slave)?))
        .stderr(Stdio::from(slave));
    // SAFETY: setsid/ioctl/signal are async-signal-safe, as `pre_exec` requires.
    unsafe {
        cmd.pre_exec(|| {
            // Its own session with the pty as controlling terminal: Ctrl+C
            // typed on the real keyboard becomes SIGINT for the program only.
            libc::setsid();
            libc::ioctl(0, libc::TIOCSCTTY as _, 0);
            libc::signal(libc::SIGINT, libc::SIG_DFL);
            Ok(())
        });
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| Error::invalid(format!("cannot run sh: {e}")))?;
    // Our copies of the slave end live in `cmd`; drop them or the master never sees EOF.
    drop(cmd);

    // Real terminal -> raw, so every key (Ctrl+C = 0x03) reaches the pty's own
    // line discipline. Restored on every exit path by the guard.
    struct Restore(Option<libc::termios>, libc::sighandler_t);
    impl Drop for Restore {
        fn drop(&mut self) {
            // SAFETY: restoring settings captured at the start.
            unsafe {
                if let Some(term) = &self.0 {
                    libc::tcsetattr(0, libc::TCSANOW, term);
                }
                libc::signal(libc::SIGINT, self.1);
            }
        }
    }
    // SAFETY: as above; SIGINT is ignored in this process while the program owns the keyboard.
    let _restore = unsafe {
        let previous = libc::signal(libc::SIGINT, libc::SIG_IGN);
        if have_tty {
            let mut raw = saved;
            libc::cfmakeraw(&mut raw);
            libc::tcsetattr(0, libc::TCSANOW, &raw);
        }
        Restore(have_tty.then_some(saved), previous)
    };

    let master_fd = std::os::fd::AsRawFd::as_raw_fd(&master);
    let mut recorded: Vec<u8> = Vec::new();
    let mut truncated = false;
    let mut stdin_open = have_tty;
    let mut buf = [0u8; 16 * 1024];
    loop {
        let mut fds = [
            libc::pollfd {
                fd: master_fd,
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: 0,
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        let count = if stdin_open { 2 } else { 1 };
        // SAFETY: `fds` outlives the call and `count` <= its length.
        let ready = unsafe { libc::poll(fds.as_mut_ptr(), count, 100) };
        if ready < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            break;
        }
        if stdin_open && fds[1].revents & (libc::POLLIN | libc::POLLHUP) != 0 {
            // SAFETY: reading into `buf`, writing back from it.
            let n = unsafe { libc::read(0, buf.as_mut_ptr().cast(), buf.len()) };
            if n <= 0 {
                stdin_open = false;
            } else {
                let _ = unsafe { libc::write(master_fd, buf.as_ptr().cast(), n as usize) };
            }
        }
        if fds[0].revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0 {
            // SAFETY: reading into `buf`.
            let n = unsafe { libc::read(master_fd, buf.as_mut_ptr().cast(), buf.len()) };
            if n <= 0 {
                break; // EIO / 0: every holder of the slave end is gone
            }
            let chunk = &buf[..n as usize];
            let _ = out.write_all(chunk).and_then(|_| out.flush());
            let room = MAX_OUTPUT.saturating_sub(recorded.len());
            recorded.extend_from_slice(&chunk[..chunk.len().min(room)]);
            truncated |= chunk.len() > room;
        }
    }
    let exit = child.wait().ok().and_then(|s| s.code());
    Ok(RunOutcome {
        command: command.to_string(),
        cwd: cwd.to_path_buf(),
        exit,
        detached: false,
        output: tidy_terminal_text(&String::from_utf8_lossy(&recorded)),
        raw: recorded,
        truncated,
        waited: started.elapsed(),
    })
}

/// Remove DEC private mode switches (`ESC [ ? … h` / `l`): alternate screen,
/// mouse reporting, bracketed paste, cursor hiding… Everything else (colours,
/// cursor movement, text) is kept.
pub fn strip_private_modes(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b && bytes.get(i + 1) == Some(&b'[') && bytes.get(i + 2) == Some(&b'?') {
            let mut j = i + 3;
            while j < bytes.len() && (bytes[j].is_ascii_digit() || bytes[j] == b';') {
                j += 1;
            }
            if matches!(bytes.get(j), Some(b'h') | Some(b'l')) {
                i = j + 1;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

/// A pty reports `\r\n`; a bare `\r` rewinds the line (progress bars), so only
/// what follows the last one survives. Colours (SGR) are kept for the viewer.
fn tidy_terminal_text(raw: &str) -> String {
    raw.replace("\r\n", "\n")
        .split('\n')
        .map(|line| line.rsplit('\r').next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[cfg(unix)]
    #[test]
    fn pty_run_mirrors_and_records_both_streams_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let mut mirror = Vec::new();
        let out = run_in_pty_to(
            "echo one; echo two 1>&2; echo three; exit 5",
            dir.path(),
            &mut mirror,
        )
        .unwrap();
        assert_eq!(out.output, "one\ntwo\nthree\n");
        assert_eq!(out.exit, Some(5));
        assert_eq!(out.cwd, dir.path());
        assert_eq!(
            String::from_utf8(mirror).unwrap(),
            "one\r\ntwo\r\nthree\r\n",
            "live copy is the raw terminal stream"
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_program_really_sees_a_terminal_in_its_folder() {
        let dir = tempfile::tempdir().unwrap();
        let out = run_in_pty_to(
            "test -t 0 && test -t 1 && test -t 2 && pwd",
            dir.path(),
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(
            out.exit,
            Some(0),
            "stdin/stdout/stderr are ttys: {:?}",
            out.output
        );
        assert_eq!(
            std::fs::canonicalize(out.output.trim()).unwrap(),
            std::fs::canonicalize(dir.path()).unwrap()
        );
    }

    #[cfg(unix)]
    #[test]
    fn pty_output_keeps_colours_and_drops_carriage_return_rewrites() {
        let dir = tempfile::tempdir().unwrap();
        let out = run_in_pty_to(
            "printf '\\033[31mred\\033[0m\\n10%%\\r50%%\\r100%%\\ndone\\n'",
            dir.path(),
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(out.output, "\x1b[31mred\x1b[0m\n100%\ndone\n");
    }

    #[cfg(unix)]
    #[test]
    fn pty_output_is_capped_and_the_program_runs_to_the_end() {
        let dir = tempfile::tempdir().unwrap();
        let out = run_in_pty_to(
            "head -c 6000000 /dev/zero | tr '\\0' x; echo end",
            dir.path(),
            &mut std::io::sink(),
        )
        .unwrap();
        assert!(out.truncated);
        assert_eq!(out.output.len(), MAX_OUTPUT);
        assert_eq!(out.exit, Some(0));
    }

    #[cfg(unix)]
    #[test]
    fn pty_run_reports_a_syntax_error_and_a_signal() {
        let dir = tempfile::tempdir().unwrap();
        let bad = run_in_pty_to("if then fi", dir.path(), &mut Vec::new()).unwrap();
        assert_ne!(bad.exit, Some(0));
        assert!(!bad.output.is_empty());
        let killed = run_in_pty_to("kill -KILL $$", dir.path(), &mut Vec::new()).unwrap();
        assert_eq!(killed.exit, None);
        assert!(killed.summary().contains("signal"), "{}", killed.summary());
    }

    #[test]
    fn replay_drops_mode_switches_and_keeps_text_and_colours() {
        let raw = b"\x1b[?1049h\x1b[?25l\x1b[31mred\x1b[0m\r\nok\x1b[?2004h\x1b[?1000;1006l\x1b[2J";
        assert_eq!(strip_private_modes(raw), b"\x1b[31mred\x1b[0m\r\nok\x1b[2J");
        assert_eq!(
            strip_private_modes(b"plain \x1b[?x"),
            b"plain \x1b[?x",
            "malformed sequences are left alone"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_pty_run_keeps_the_raw_stream_for_replay() {
        let dir = tempfile::tempdir().unwrap();
        let out = run_in_pty_to(
            "printf '\\033[?1049h\\033[32mgreen\\033[0m\\nprog\\r50%%\\n'",
            dir.path(),
            &mut Vec::new(),
        )
        .unwrap();
        assert!(
            out.raw.starts_with(b"\x1b[?1049h\x1b[32mgreen"),
            "{:?}",
            String::from_utf8_lossy(&out.raw)
        );
        let replay = String::from_utf8(out.replay_bytes()).unwrap();
        assert_eq!(replay, "\x1b[32mgreen\x1b[0m\r\nprog\r50%\r\n");
        // Without a recording (the GUI path) the plain text is the replay.
        let plain = run_captured("echo hi", dir.path(), DEFAULT_TIMEOUT).unwrap();
        assert!(plain.raw.is_empty());
        assert_eq!(plain.replay_bytes(), b"hi\n");
        assert_eq!(
            out.replay_bytes_cooked(),
            b"\x1b[32mgreen\x1b[0m\nprog\r50%\n",
            "\\r\\n -> \\n, a lone \\r stays"
        );
    }
}
