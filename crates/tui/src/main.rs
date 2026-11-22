mod app;
mod ui;

use std::io::{self, Stdout, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use bifold_core::config::Config;
use bifold_core::runcmd;
use bifold_core::session::Session;
use bifold_core::{APP_NAME, VERSION};
use crossterm::cursor;
use crossterm::event::{self, Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::app::App;

pub(crate) const HELP: &str = "\
Bifold — two-pane file manager (terminal)

USAGE:
    bifold [PATH] [OPTIONS]

OPTIONS:
    -l, --left <PATH>    directory for the left pane
    -r, --right <PATH>   directory for the right pane
    -p, --path <PATH>    directory for both panes
        --dump           print both panes as plain text and exit
        --dump-keys      print the effective key bindings as TOML and exit
        --theme <NAME>   colour theme for this run (see --list-themes)
        --list-themes    list built-in and user themes and exit
        --no-config      ignore ~/.config/bifold/config.toml
    -h, --help           show this help
    -V, --version        show the version

KEYS (defaults — rebind in [keys] / [viewer_keys] of ~/.config/bifold/config.toml):
    Tab switch pane          Enter / → enter directory or view file
    ↑ ↓ move cursor          ← / Backspace go up
    Space mark item          F2 rename      F3 view      F4 hex view   F5 copy
    F6 move                  F7 new folder  F8 delete
    Ctrl+A mark all          Ctrl+D clear marks
    . hidden files           s sort key     o sort order
    / filter                 g go to path   r refresh     h / ? help (h is fixed)   q quit
    t cycle colour theme     Ctrl+O output of the last run command
    H show / hide the key-hint line (or show_hints = false in the config)
    In the viewer: x toggles hex, w wraps, Esc closes.
";

#[derive(Debug, Default)]
struct Args {
    left: Option<PathBuf>,
    right: Option<PathBuf>,
    path: Option<PathBuf>,
    dump: bool,
    dump_keys: bool,
    theme: Option<String>,
    no_config: bool,
}

fn parse_args() -> Result<Option<Args>, String> {
    let mut args = Args::default();
    let mut raw = std::env::args().skip(1);
    while let Some(arg) = raw.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{HELP}");
                return Ok(None);
            }
            "-V" | "--version" => {
                println!("{APP_NAME} {VERSION}");
                return Ok(None);
            }
            "--dump" => args.dump = true,
            "--dump-keys" => args.dump_keys = true,
            "--theme" => {
                args.theme = Some(raw.next().ok_or("--theme needs a name")?);
            }
            "--list-themes" => {
                print!(
                    "{}",
                    bifold_core::theme::list_text(Config::themes_dir().as_deref())
                );
                return Ok(None);
            }
            "--no-config" => args.no_config = true,
            "-l" | "--left" => {
                args.left = Some(take_path(&mut raw, "--left")?);
            }
            "-r" | "--right" => {
                args.right = Some(take_path(&mut raw, "--right")?);
            }
            "-p" | "--path" => {
                args.path = Some(take_path(&mut raw, "--path")?);
            }
            other if other.starts_with('-') => {
                return Err(format!("unknown option: {other} (try --help)"));
            }
            positional => {
                args.path = Some(PathBuf::from(positional));
            }
        }
    }
    Ok(Some(args))
}

fn take_path(raw: &mut impl Iterator<Item = String>, flag: &str) -> Result<PathBuf, String> {
    raw.next()
        .map(PathBuf::from)
        .ok_or_else(|| format!("{flag} needs a directory"))
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(message) => {
            eprintln!("{APP_NAME}: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<ExitCode, String> {
    let Some(args) = parse_args()? else {
        return Ok(ExitCode::SUCCESS);
    };

    let (mut config, config_problem) = if args.no_config {
        (Config::default(), None)
    } else {
        Config::load_checked()
    };
    if let Some(path) = &args.path {
        let text = path.display().to_string();
        config.left = text.clone();
        config.right = text;
    }
    if let Some(left) = &args.left {
        config.left = left.display().to_string();
    }
    if let Some(right) = &args.right {
        config.right = right.display().to_string();
    }
    if let Some(theme) = &args.theme {
        config.theme = theme.clone();
    }

    if args.dump_keys {
        let (keymap, warnings) = config.keymap();
        for warning in warnings {
            eprintln!("{APP_NAME}: {warning}");
        }
        print!("{}", keymap.to_toml());
        return Ok(ExitCode::SUCCESS);
    }

    let mut session = Session::new(config);
    session.report_config_problem(config_problem);
    if args.dump {
        dump(&session);
        return Ok(ExitCode::SUCCESS);
    }

    let mut app = App::new(session);
    app.persist = !args.no_config;
    let mut guard = TerminalGuard::new().map_err(|e| e.to_string())?;
    let outcome = event_loop(&mut guard, &mut app);
    drop(guard);
    app.save_state();
    outcome
        .map(|()| ExitCode::SUCCESS)
        .map_err(|e| e.to_string())
}

fn event_loop(guard: &mut TerminalGuard, app: &mut App) -> io::Result<()> {
    while !app.should_quit {
        guard.terminal.draw(|frame| ui::draw(frame, app))?;
        if event::poll(Duration::from_millis(120))? {
            match event::read()? {
                Event::Key(key)
                    if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) =>
                {
                    app.on_key(key);
                }
                Event::Resize(_, _) => {}
                _ => {}
            }
        }
        if let Some((command, cwd)) = app.pending_run.take() {
            run_in_terminal(guard, app, &command, &cwd)?;
        }
        app.tick();
    }
    Ok(())
}

/// Hand the terminal to `command` like a shell would: leave the alternate
/// screen, run it with the real stdin/stdout, wait for Enter so its output can
/// be read, then come back and refresh both panes.
fn run_in_terminal(
    guard: &mut TerminalGuard,
    app: &mut App,
    command: &str,
    cwd: &Path,
) -> io::Result<()> {
    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen, cursor::Show)?;
    println!("$ {command}");
    // On a pty, so the output is shown live *and* kept for Ctrl+O.
    let result = runcmd::run_in_pty(command, cwd);
    let (note, message) = match &result {
        Ok(out) => {
            let note = match out.exit {
                Some(code) => format!("exit {code}"),
                None => "killed by a signal".to_string(),
            };
            (note, format!("{}  (Ctrl+O: output)", out.summary()))
        }
        Err(err) => (format!("error: {err}"), format!("error: {err}")),
    };
    if let Ok(out) = result {
        app.session.last_run = Some(out);
    }
    print!("\n[{note}] press Enter to return to {APP_NAME}… ");
    io::stdout().flush()?;
    let mut line = String::new();
    let _ = io::stdin().read_line(&mut line);

    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen, cursor::Hide)?;
    guard.terminal.clear()?;
    // Keys typed while the command ran must not drive the file manager.
    while event::poll(Duration::ZERO)? {
        let _ = event::read()?;
    }
    app.session.reload_all();
    app.set_message(message);
    Ok(())
}

fn dump(session: &Session) {
    for (label, pane) in [("LEFT", &session.panes[0]), ("RIGHT", &session.panes[1])] {
        println!("{label}  {}", pane.path.display());
        println!("  {:<40} {:>9}  modified", "name", "size");
        for entry in &pane.entries {
            let kind = if entry.is_dir() {
                format!("{}/", entry.name)
            } else {
                entry.name.clone()
            };
            let size = if entry.is_dir() {
                "<dir>".to_string()
            } else {
                bifold_core::format::human_size(entry.size)
            };
            let time = entry
                .modified
                .map(bifold_core::format::fmt_time)
                .unwrap_or_default();
            println!("  {kind:<40} {size:>9}  {time}");
        }
        if let Some(error) = &pane.error {
            println!("  error: {error}");
        }
        println!();
    }
}

/// Owns the alternate screen and restores the terminal on drop (including panics).
struct TerminalGuard {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl TerminalGuard {
    fn new() -> io::Result<Self> {
        install_panic_hook();
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, cursor::Hide)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;
        terminal.clear()?;
        Ok(Self { terminal })
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            self.terminal.backend_mut(),
            LeaveAlternateScreen,
            cursor::Show
        );
        let _ = self.terminal.show_cursor();
    }
}

fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, cursor::Show);
        previous(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_text_documents_the_keys() {
        for key in [
            "F5 copy",
            "F8 delete",
            "F4 hex",
            "--dump",
            "--dump-keys",
            "--theme",
            "--list-themes",
            "t cycle colour theme",
            "h / ? help",
            "Tab switch pane",
        ] {
            assert!(HELP.contains(key), "help must mention {key}");
        }
    }
}
