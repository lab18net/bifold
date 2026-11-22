//! UI-agnostic application state plus the keyboard contract for the TUI.

use std::path::PathBuf;

use bifold_core::error::Result;
use bifold_core::fsops::{self, DeleteMode};
use bifold_core::help;
use bifold_core::keys::{Action, Key, KeySpec, ViewerAction};
use bifold_core::preview::{Preview, PreviewKind};
use bifold_core::session::Session;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptKind {
    Filter,
    GoTo,
    MkDir,
    NewFile,
    Rename,
    CopyTo,
    MoveTo,
    /// Command line to run in the active folder (Enter on an executable).
    Run,
}

impl PromptKind {
    fn title(self) -> &'static str {
        match self {
            Self::Filter => "Filter",
            Self::GoTo => "Go to",
            Self::MkDir => "New folder",
            Self::NewFile => "New file",
            Self::Rename => "Rename",
            Self::CopyTo => "Copy to",
            Self::MoveTo => "Move to",
            Self::Run => "Run",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Filter => "pattern",
            Self::GoTo => "path",
            Self::MkDir | Self::NewFile | Self::Rename => "name",
            Self::CopyTo | Self::MoveTo => "destination",
            Self::Run => "$",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Prompt {
    pub kind: PromptKind,
    pub value: String,
    pub title: String,
}

#[derive(Debug, Clone)]
pub enum ConfirmAction {
    Delete {
        targets: Vec<PathBuf>,
        mode: DeleteMode,
    },
}

#[derive(Debug, Clone)]
pub struct Confirm {
    pub title: String,
    pub body: String,
    pub action: ConfirmAction,
}

#[derive(Debug, Clone)]
pub struct Viewer {
    pub preview: Preview,
    pub scroll: usize,
    pub wrap: bool,
}

#[derive(Debug, Clone, Default)]
pub enum Mode {
    #[default]
    Normal,
    Prompt(Prompt),
    Confirm(Confirm),
    Progress,
    Viewer(Viewer),
    Help(usize),
}

pub struct App {
    pub session: Session,
    pub mode: Mode,
    pub should_quit: bool,
    /// Rows visible per pane, published by the renderer.
    pub viewport: [usize; 2],
    /// False when started with `--no-config`: leave the config file alone.
    pub persist: bool,
    /// A command the user confirmed in the run prompt. The event loop owns the
    /// terminal, so it takes this, suspends the UI and runs it.
    pub pending_run: Option<(String, PathBuf)>,
}

impl App {
    pub fn new(session: Session) -> Self {
        Self {
            session,
            mode: Mode::Normal,
            should_quit: false,
            viewport: [20, 20],
            persist: true,
            pending_run: None,
        }
    }

    pub fn tick(&mut self) {
        if self.session.poll_jobs().is_some() && matches!(self.mode, Mode::Progress) {
            self.mode = Mode::Normal;
        }
    }

    pub fn set_message(&mut self, message: impl Into<String>) {
        self.session.message = Some(message.into());
    }

    pub fn save_state(&mut self) {
        if !self.persist {
            return;
        }
        let _ = self.session.save_config();
    }

    // -- key dispatch -------------------------------------------------------

    pub fn on_key(&mut self, key: KeyEvent) {
        let mode = std::mem::take(&mut self.mode);
        self.mode = match mode {
            Mode::Normal => self.handle_normal(key),
            Mode::Prompt(prompt) => self.handle_prompt(key, prompt),
            Mode::Confirm(confirm) => self.handle_confirm(key, confirm),
            Mode::Progress => self.handle_progress(key),
            Mode::Viewer(viewer) => self.handle_viewer(key, viewer),
            Mode::Help(tab) => self.handle_help(key, tab),
        };
    }

    fn handle_normal(&mut self, key: KeyEvent) -> Mode {
        let spec = key_spec(key);
        // Fixed key: works whatever the keymap says.
        if spec.as_ref().is_some_and(help::is_help_key) {
            return Mode::Help(0);
        }
        let Some(action) = spec.and_then(|spec| self.session.keymap.action(&spec)) else {
            return Mode::Normal;
        };
        let pane_height = self.viewport[self.session.active].max(1);
        if self.session.apply(action, pane_height) {
            return Mode::Normal;
        }

        match action {
            Action::Quit => {
                self.request_quit();
                Mode::Normal
            }
            Action::Open => self.open_cursor(),
            Action::View => self.open_viewer(false),
            Action::ViewHex => self.open_viewer(true),
            Action::LastOutput => self.open_last_output(),
            Action::Rename => self.prompt(PromptKind::Rename, None),
            Action::Copy => self.prompt(PromptKind::CopyTo, None),
            Action::Move => self.prompt(PromptKind::MoveTo, None),
            Action::MkDir => self.prompt(PromptKind::MkDir, None),
            Action::NewFile => self.prompt(PromptKind::NewFile, None),
            Action::Delete => self.confirm_delete(),
            Action::Filter => {
                let current = self.session.pane().filter.clone();
                self.prompt(PromptKind::Filter, Some(current))
            }
            Action::GoTo => {
                let here = self.session.pane().path.display().to_string();
                self.prompt(PromptKind::GoTo, Some(here))
            }
            Action::Help => Mode::Help(0),
            _ => Mode::Normal,
        }
    }

    fn handle_prompt(&mut self, key: KeyEvent, mut prompt: Prompt) -> Mode {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => {
                if prompt.kind == PromptKind::Filter {
                    self.session.pane_mut().set_filter("");
                }
                Mode::Normal
            }
            KeyCode::Enter => self.submit_prompt(&prompt),
            KeyCode::Backspace => {
                prompt.value.pop();
                Mode::Prompt(prompt)
            }
            KeyCode::Char('u') if ctrl => {
                prompt.value.clear();
                Mode::Prompt(prompt)
            }
            KeyCode::Char(ch) if !ctrl => {
                prompt.value.push(ch);
                Mode::Prompt(prompt)
            }
            _ => Mode::Prompt(prompt),
        }
    }

    fn handle_confirm(&mut self, key: KeyEvent, confirm: Confirm) -> Mode {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                let result = match confirm.action {
                    ConfirmAction::Delete { targets, mode } => {
                        self.session.start_delete(&targets, mode)
                    }
                };
                match result {
                    Ok(()) => Mode::Progress,
                    Err(err) => {
                        self.set_message(format!("error: {err}"));
                        Mode::Normal
                    }
                }
            }
            _ => Mode::Normal,
        }
    }

    fn handle_progress(&mut self, key: KeyEvent) -> Mode {
        match key.code {
            KeyCode::Esc | KeyCode::Char('c') => {
                self.session.cancel_job();
                Mode::Progress
            }
            KeyCode::Enter | KeyCode::Char('q') => {
                if self.session.jobs.is_busy() {
                    self.set_message("cancel the transfer first (Esc)");
                    Mode::Progress
                } else {
                    Mode::Normal
                }
            }
            _ => Mode::Progress,
        }
    }

    fn handle_viewer(&mut self, key: KeyEvent, mut viewer: Viewer) -> Mode {
        let page = self.viewport[self.session.active].max(1);
        let max_scroll = viewer.preview.lines.len().saturating_sub(1);
        let Some(action) = key_spec(key).and_then(|spec| self.session.keymap.viewer_action(&spec))
        else {
            return Mode::Viewer(viewer);
        };
        match action {
            ViewerAction::Close => return Mode::Normal,
            ViewerAction::Down => viewer.scroll = (viewer.scroll + 1).min(max_scroll),
            ViewerAction::Up => viewer.scroll = viewer.scroll.saturating_sub(1),
            ViewerAction::PageDown => viewer.scroll = (viewer.scroll + page).min(max_scroll),
            ViewerAction::PageUp => viewer.scroll = viewer.scroll.saturating_sub(page),
            ViewerAction::Top => viewer.scroll = 0,
            ViewerAction::Bottom => viewer.scroll = max_scroll,
            ViewerAction::ToggleWrap => viewer.wrap = !viewer.wrap,
            ViewerAction::ToggleHex if viewer.preview.kind == PreviewKind::Output => {}
            ViewerAction::ToggleHex => {
                let want_hex = viewer.preview.kind != PreviewKind::Hex;
                let (width, height) = viewer_area();
                match self
                    .session
                    .preview(&viewer.preview.path.clone(), width, height, want_hex)
                {
                    Ok(preview) => {
                        viewer.preview = preview;
                        viewer.scroll = 0;
                    }
                    Err(err) => self.set_message(format!("error: {err}")),
                }
            }
        }
        Mode::Viewer(viewer)
    }

    fn handle_help(&mut self, key: KeyEvent, tab: usize) -> Mode {
        let count = help::TITLES.len();
        let spec = key_spec(key);
        let closes = spec.as_ref().is_some_and(|s| {
            help::is_help_key(s)
                || matches!(
                    s.key,
                    Key::Esc | Key::Char('q') | Key::Char('?') | Key::Enter
                )
                || self.session.keymap.action(s) == Some(Action::Help)
        });
        if closes {
            return Mode::Normal;
        }
        let tab = match key.code {
            KeyCode::Right | KeyCode::Tab | KeyCode::Char('l') => help::step(tab, 1, count),
            KeyCode::Left | KeyCode::BackTab => help::step(tab, -1, count),
            KeyCode::Home => 0,
            KeyCode::End => count - 1,
            KeyCode::Char(c @ '1'..='9') if (c as usize - '1' as usize) < count => {
                c as usize - '1' as usize
            }
            _ => tab,
        };
        Mode::Help(tab)
    }

    // -- actions ------------------------------------------------------------

    fn request_quit(&mut self) {
        if self.session.jobs.is_busy() {
            self.set_message("transfer running — press Esc in the progress dialog to cancel");
            return;
        }
        self.should_quit = true;
    }

    fn prompt(&mut self, kind: PromptKind, prefill: Option<String>) -> Mode {
        let value = prefill.unwrap_or_else(|| match kind {
            PromptKind::MkDir | PromptKind::NewFile => String::new(),
            PromptKind::Rename => self
                .session
                .pane()
                .cursor_entry()
                .map(|entry| entry.name.clone())
                .unwrap_or_default(),
            PromptKind::CopyTo | PromptKind::MoveTo => {
                self.session.target_dir().display().to_string()
            }
            _ => String::new(),
        });
        let title = match kind {
            PromptKind::CopyTo | PromptKind::MoveTo => {
                format!(
                    "{} {} item(s)",
                    kind.title(),
                    self.session.pane().targets().len()
                )
            }
            PromptKind::Run => format!("Run in {}", self.session.pane().path.display()),
            _ => kind.title().to_string(),
        };
        Mode::Prompt(Prompt { kind, value, title })
    }

    fn submit_prompt(&mut self, prompt: &Prompt) -> Mode {
        let value = prompt.value.trim().to_string();
        let mut starts_job = false;

        let result: Result<()> = match prompt.kind {
            PromptKind::Filter => {
                self.session.pane_mut().set_filter(value);
                Ok(())
            }
            PromptKind::GoTo => match fsops::resolve_dir(&value) {
                Ok(dir) => self.session.pane_mut().navigate(&dir),
                Err(err) => Err(err),
            },
            PromptKind::MkDir => {
                if value.is_empty() {
                    return self.fail("folder name cannot be empty");
                }
                self.session.make_dir(&value).map(|_| ())
            }
            PromptKind::Run => {
                if value.is_empty() {
                    return self.fail("command cannot be empty");
                }
                self.pending_run = Some((value, self.session.pane().path.clone()));
                Ok(())
            }
            PromptKind::NewFile => {
                if value.is_empty() {
                    return self.fail("file name cannot be empty");
                }
                self.session.make_file(&value).map(|_| ())
            }
            PromptKind::Rename => {
                if value.is_empty() {
                    return self.fail("new name cannot be empty");
                }
                self.session.rename_cursor(&value).map(|_| ())
            }
            PromptKind::CopyTo | PromptKind::MoveTo => match fsops::resolve_dir(&value) {
                Ok(dest) => {
                    let targets = self.session.pane().targets();
                    starts_job = true;
                    match prompt.kind {
                        PromptKind::CopyTo => self.session.start_copy(&targets, &dest),
                        _ => self.session.start_move(&targets, &dest),
                    }
                }
                Err(err) => Err(err),
            },
        };

        match result {
            Ok(()) => {
                if starts_job && self.session.jobs.is_busy() {
                    Mode::Progress
                } else {
                    Mode::Normal
                }
            }
            Err(err) => self.fail(format!("error: {err}")),
        }
    }

    fn fail(&mut self, message: impl Into<String>) -> Mode {
        self.set_message(message);
        Mode::Normal
    }

    fn open_cursor(&mut self) -> Mode {
        let Some(entry) = self.session.pane().cursor_entry().cloned() else {
            return Mode::Normal;
        };
        if entry.is_dir() {
            if let Err(err) = self.session.pane_mut().descend() {
                self.set_message(format!("error: {err}"));
            }
            Mode::Normal
        } else if let Some(command) = self.session.run_suggestion() {
            // Executable: offer an editable run line, like a shell prompt.
            self.prompt(PromptKind::Run, Some(command))
        } else {
            self.viewer_for(&entry.path, false)
        }
    }

    /// Ctrl+O: the output of the last command run from the file list.
    fn open_last_output(&mut self) -> Mode {
        match self.session.last_run_preview() {
            Some(preview) => Mode::Viewer(Viewer {
                preview,
                scroll: 0,
                wrap: false,
            }),
            None => self.fail("no command has been run yet (Enter on an executable runs one)"),
        }
    }

    fn open_viewer(&mut self, hex: bool) -> Mode {
        let Some(entry) = self.session.pane().cursor_entry().cloned() else {
            return Mode::Normal;
        };
        if entry.is_dir() {
            self.set_message("F3 works on files; use Enter to open a folder");
            return Mode::Normal;
        }
        self.viewer_for(&entry.path, hex)
    }

    fn viewer_for(&mut self, path: &std::path::Path, hex: bool) -> Mode {
        let (width, height) = viewer_area();
        match self.session.preview(path, width, height, hex) {
            Ok(preview) => Mode::Viewer(Viewer {
                preview,
                scroll: 0,
                wrap: false,
            }),
            Err(err) => self.fail(format!("error: {err}")),
        }
    }

    fn confirm_delete(&mut self) -> Mode {
        let targets = self.session.pane().targets();
        if targets.is_empty() {
            return self.fail("nothing to delete");
        }
        let mode = self.session.config.delete_mode();
        if !self.session.config.confirm_delete {
            return match self.session.start_delete(&targets, mode) {
                Ok(()) => Mode::Progress,
                Err(err) => self.fail(format!("error: {err}")),
            };
        }
        let where_to = match mode {
            DeleteMode::Trash => "the trash",
            DeleteMode::Permanent => "permanent deletion",
        };
        Mode::Confirm(Confirm {
            title: "Delete".to_string(),
            body: format!(
                "{} item(s) — {}.\n\n[y] confirm    [n] cancel",
                targets.len(),
                where_to
            ),
            action: ConfirmAction::Delete { targets, mode },
        })
    }
}

/// Translate a terminal key event into the shared key vocabulary.
pub fn key_spec(key: KeyEvent) -> Option<KeySpec> {
    let code = match key.code {
        KeyCode::Char(c) => Key::Char(c),
        KeyCode::Enter => Key::Enter,
        KeyCode::Esc => Key::Esc,
        KeyCode::Tab | KeyCode::BackTab => Key::Tab,
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Delete => Key::Delete,
        KeyCode::Insert => Key::Insert,
        KeyCode::Up => Key::Up,
        KeyCode::Down => Key::Down,
        KeyCode::Left => Key::Left,
        KeyCode::Right => Key::Right,
        KeyCode::Home => Key::Home,
        KeyCode::End => Key::End,
        KeyCode::PageUp => Key::PageUp,
        KeyCode::PageDown => Key::PageDown,
        KeyCode::F(n) => Key::F(n),
        _ => return None,
    };
    Some(KeySpec::new(
        code,
        key.modifiers.contains(KeyModifiers::CONTROL),
        key.modifiers.contains(KeyModifiers::ALT),
        key.modifiers.contains(KeyModifiers::SHIFT) || key.code == KeyCode::BackTab,
    ))
}

/// Cells inside the viewer's border, passed to the previewer as width/height.
fn viewer_area() -> (u16, u16) {
    let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
    (cols.saturating_sub(2).max(1), rows.saturating_sub(2).max(1))
}

#[cfg(test)]
mod tests {
    use bifold_core::session::{LEFT, RIGHT};
    use std::fs;

    use bifold_core::session::Session;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn fixture() -> (tempfile::TempDir, App) {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("left")).unwrap();
        fs::create_dir(dir.path().join("right")).unwrap();
        fs::write(dir.path().join("left/a.txt"), b"hello").unwrap();
        let session = Session::new_at(&dir.path().join("left"), &dir.path().join("right"));
        (dir, App::new(session))
    }

    #[test]
    fn tab_switches_panes() {
        let (_dir, mut app) = fixture();
        assert_eq!(app.session.active, LEFT);
        app.on_key(key(KeyCode::Tab));
        assert_eq!(app.session.active, RIGHT);
        app.on_key(key(KeyCode::Tab));
        assert_eq!(app.session.active, LEFT);
    }

    #[test]
    fn enter_opens_directory_and_backspace_returns() {
        let (dir, mut app) = fixture();
        app.session.pane_mut().navigate(dir.path()).unwrap();
        let sub = app
            .session
            .pane()
            .entries
            .iter()
            .position(|e| e.name == "left")
            .unwrap();
        app.session.pane_mut().cursor = sub;
        app.on_key(key(KeyCode::Enter));
        assert_eq!(
            app.session.pane().path,
            fs::canonicalize(dir.path().join("left")).unwrap()
        );
        app.on_key(key(KeyCode::Backspace));
        assert_eq!(
            app.session.pane().path,
            fs::canonicalize(dir.path()).unwrap()
        );
    }

    #[test]
    fn typing_in_prompt_builds_a_folder() {
        let (dir, mut app) = fixture();
        app.on_key(key(KeyCode::F(7)));
        for ch in "newdir".chars() {
            app.on_key(key(KeyCode::Char(ch)));
        }
        app.on_key(key(KeyCode::Enter));
        assert!(
            fs::canonicalize(dir.path().join("left/newdir"))
                .unwrap()
                .is_dir(),
            "folder created by the prompt"
        );
        assert!(matches!(app.mode, Mode::Normal));
    }

    #[test]
    fn escape_cancels_a_prompt() {
        let (_dir, mut app) = fixture();
        app.on_key(key(KeyCode::F(7)));
        app.on_key(key(KeyCode::Char('x')));
        app.on_key(key(KeyCode::Esc));
        assert!(matches!(app.mode, Mode::Normal));
    }

    #[test]
    fn filter_prompt_narrows_the_listing() {
        let (dir, mut app) = fixture();
        fs::write(dir.path().join("left/b.md"), b"x").unwrap();
        app.session.pane_mut().reload();
        app.on_key(key(KeyCode::Char('/')));
        for ch in "*.md".chars() {
            app.on_key(key(KeyCode::Char(ch)));
        }
        app.on_key(key(KeyCode::Enter));
        assert_eq!(app.session.pane().entries.len(), 1);
        assert_eq!(app.session.pane().entries[0].name, "b.md");
        app.on_key(key(KeyCode::Esc));
        assert_eq!(app.session.pane().entries.len(), 2);
    }

    #[test]
    fn delete_asks_for_confirmation_and_starts_a_job() {
        let (_dir, mut app) = fixture();
        app.on_key(key(KeyCode::F(8)));
        assert!(matches!(app.mode, Mode::Confirm(_)));
        app.on_key(key(KeyCode::Char('y')));
        assert!(matches!(app.mode, Mode::Progress));
    }

    #[test]
    fn quit_is_blocked_while_a_job_runs() {
        let (_dir, mut app) = fixture();
        app.session.jobs.start("fake", 0, 0, |_ctx| {
            std::thread::sleep(std::time::Duration::from_millis(300));
            Ok(bifold_core::fsops::OpStats::default())
        });
        app.on_key(key(KeyCode::Char('q')));
        assert!(!app.should_quit);
        app.session.cancel_job();
        app.session.jobs.poll();
    }

    #[test]
    fn viewer_opens_for_files() {
        let (_dir, mut app) = fixture();
        app.on_key(key(KeyCode::F(3)));
        match &app.mode {
            Mode::Viewer(viewer) => {
                assert!(viewer.preview.plain_lines().iter().any(|l| l == "hello"))
            }
            other => panic!("expected viewer, got {other:?}"),
        }
        app.on_key(key(KeyCode::Esc));
        assert!(matches!(app.mode, Mode::Normal));
    }

    #[test]
    fn copy_prompt_prefills_the_other_pane() {
        let (_dir, mut app) = fixture();
        app.on_key(key(KeyCode::F(5)));
        let Mode::Prompt(prompt) = &app.mode else {
            panic!("expected prompt");
        };
        assert_eq!(
            prompt.value,
            fs::canonicalize(app.session.other().path.clone())
                .unwrap()
                .display()
                .to_string()
        );
    }

    #[test]
    fn binary_files_open_as_hex_and_toggle() {
        let (dir, mut app) = fixture();
        fs::write(dir.path().join("left/z.bin"), [0u8, 1, 2, 3, 0xff]).unwrap();
        app.session.pane_mut().reload();
        let at = app
            .session
            .pane()
            .entries
            .iter()
            .position(|e| e.name == "z.bin")
            .unwrap();
        app.session.pane_mut().cursor = at;
        app.on_key(key(KeyCode::F(3)));
        match &app.mode {
            Mode::Viewer(v) => {
                assert_eq!(v.preview.kind, PreviewKind::Hex);
                assert!(v.preview.lines[0].plain().contains("00 01 02 03 ff"));
            }
            other => panic!("expected viewer, got {other:?}"),
        }
        app.on_key(key(KeyCode::Esc));

        // F4 forces hex on a text file; `x` toggles back to text.
        let at = app
            .session
            .pane()
            .entries
            .iter()
            .position(|e| e.name == "a.txt")
            .unwrap();
        app.session.pane_mut().cursor = at;
        app.on_key(key(KeyCode::F(4)));
        assert!(matches!(&app.mode, Mode::Viewer(v) if v.preview.kind == PreviewKind::Hex));
        app.on_key(key(KeyCode::Char('x')));
        assert!(matches!(&app.mode, Mode::Viewer(v) if v.preview.kind == PreviewKind::Text));
    }

    #[test]
    fn config_rebinds_keys_in_both_modes() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("left")).unwrap();
        fs::create_dir(dir.path().join("right")).unwrap();
        fs::write(dir.path().join("left/a.txt"), b"hello").unwrap();
        let mut config = bifold_core::config::Config::default();
        config.keys.insert("m".into(), "mkdir".into());
        config.keys.insert("f7".into(), "none".into());
        config.viewer_keys.insert("h".into(), "toggle_hex".into());
        let mut session = Session::new(config);
        let _ = session.panes[LEFT].navigate(&dir.path().join("left"));
        let mut app = App::new(session);

        app.on_key(key(KeyCode::F(7)));
        assert!(matches!(app.mode, Mode::Normal), "f7 was unbound");
        app.on_key(key(KeyCode::Char('m')));
        assert!(matches!(app.mode, Mode::Prompt(_)), "m now opens mkdir");
        app.on_key(key(KeyCode::Esc));

        app.on_key(key(KeyCode::F(3)));
        app.on_key(key(KeyCode::Char('h')));
        assert!(matches!(&app.mode, Mode::Viewer(v) if v.preview.kind == PreviewKind::Hex));
    }

    #[test]
    fn shift_tab_and_uppercase_map_to_specs() {
        let back = KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT);
        assert_eq!(key_spec(back).unwrap().to_string(), "shift+tab");
        let upper = KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT);
        assert_eq!(key_spec(upper).unwrap().to_string(), "G");
        let ctrl = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL);
        assert_eq!(key_spec(ctrl).unwrap().to_string(), "ctrl+a");
    }

    #[test]
    fn h_is_hard_bound_and_ignores_the_keymap() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("left")).unwrap();
        fs::create_dir(dir.path().join("right")).unwrap();
        let mut config = bifold_core::config::Config::default();
        config.keys.insert("?".into(), "none".into());
        config.keys.insert("h".into(), "delete".into());
        let mut session = Session::with_themes_dir(config, None);
        let _ = session.panes[LEFT].navigate(&dir.path().join("left"));
        let mut app = App::new(session);
        assert!(
            app.session.status().contains("reserved"),
            "{}",
            app.session.status()
        );

        app.on_key(key(KeyCode::Char('?')));
        assert!(matches!(app.mode, Mode::Normal), "help action was unbound");
        app.on_key(key(KeyCode::Char('h')));
        assert!(
            matches!(app.mode, Mode::Help(0)),
            "h still opens help, and never deletes"
        );
    }

    #[test]
    fn help_tabs_navigate_wrap_and_close() {
        let (_dir, mut app) = fixture();
        app.on_key(key(KeyCode::Char('h')));
        let tab = |app: &App| match app.mode {
            Mode::Help(t) => t,
            _ => panic!("help closed unexpectedly"),
        };
        app.on_key(key(KeyCode::Right));
        assert_eq!(tab(&app), 1);
        app.on_key(key(KeyCode::Tab));
        assert_eq!(tab(&app), 2);
        app.on_key(key(KeyCode::Left));
        app.on_key(key(KeyCode::Left));
        app.on_key(key(KeyCode::Left));
        assert_eq!(tab(&app), 4, "wraps to the last tab");
        app.on_key(key(KeyCode::Home));
        assert_eq!(tab(&app), 0);
        app.on_key(key(KeyCode::Char('4')));
        assert_eq!(tab(&app), 3);
        app.on_key(key(KeyCode::Char('9')));
        app.on_key(key(KeyCode::Char('x')));
        assert_eq!(tab(&app), 3, "unrelated keys change nothing");
        app.on_key(key(KeyCode::End));
        assert_eq!(tab(&app), 4);

        for close in [
            KeyCode::Esc,
            KeyCode::Char('h'),
            KeyCode::Char('q'),
            KeyCode::Char('?'),
        ] {
            app.on_key(key(KeyCode::Char('h')));
            assert!(matches!(app.mode, Mode::Help(0)) || matches!(app.mode, Mode::Normal));
            app.mode = Mode::Help(2);
            app.on_key(key(close));
            assert!(matches!(app.mode, Mode::Normal), "{close:?} closes help");
        }
    }

    #[test]
    fn typing_h_in_a_prompt_types_h() {
        let (_dir, mut app) = fixture();
        app.on_key(key(KeyCode::F(7)));
        app.on_key(key(KeyCode::Char('h')));
        match &app.mode {
            Mode::Prompt(prompt) => assert_eq!(prompt.value, "h"),
            other => panic!("expected the prompt to stay open, got {other:?}"),
        }
    }

    #[cfg(unix)]
    fn exec_fixture(config: bifold_core::config::Config) -> (tempfile::TempDir, App) {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("left")).unwrap();
        fs::create_dir(dir.path().join("right")).unwrap();
        for (name, mode) in [
            ("run.sh", 0o755),
            ("my tool.sh", 0o755),
            ("notes.txt", 0o644),
        ] {
            let path = dir.path().join("left").join(name);
            fs::write(&path, "#!/bin/sh\necho hi\n").unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
        }
        let mut session = Session::with_themes_dir(config, None);
        let _ = session.panes[LEFT].navigate(&dir.path().join("left"));
        (dir, App::new(session))
    }

    #[cfg(unix)]
    fn cursor_to(app: &mut App, name: &str) {
        let at = app
            .session
            .pane()
            .entries
            .iter()
            .position(|e| e.name == name)
            .unwrap();
        app.session.pane_mut().cursor = at;
    }

    #[cfg(unix)]
    #[test]
    fn enter_on_an_executable_shows_an_editable_run_line() {
        let (dir, mut app) = exec_fixture(Default::default());
        cursor_to(&mut app, "run.sh");
        app.on_key(key(KeyCode::Enter));
        match &app.mode {
            Mode::Prompt(prompt) => {
                assert_eq!(prompt.kind, PromptKind::Run);
                assert_eq!(prompt.value, "./run.sh");
                assert!(prompt.title.starts_with("Run in "), "{}", prompt.title);
            }
            other => panic!("expected the run line, got {other:?}"),
        }
        assert!(
            app.pending_run.is_none(),
            "nothing runs until Enter confirms"
        );

        for ch in " --fast".chars() {
            app.on_key(key(KeyCode::Char(ch)));
        }
        app.on_key(key(KeyCode::Backspace));
        app.on_key(key(KeyCode::Char('t')));
        app.on_key(key(KeyCode::Enter));
        assert!(matches!(app.mode, Mode::Normal));
        let (command, cwd) = app.pending_run.take().expect("queued for the event loop");
        assert_eq!(command, "./run.sh --fast");
        assert_eq!(cwd, fs::canonicalize(dir.path().join("left")).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn escape_cancels_the_run_line_and_odd_names_are_quoted() {
        let (_dir, mut app) = exec_fixture(Default::default());
        cursor_to(&mut app, "my tool.sh");
        app.on_key(key(KeyCode::Right));
        match &app.mode {
            Mode::Prompt(prompt) => assert_eq!(prompt.value, "'./my tool.sh'"),
            other => panic!("{other:?}"),
        }
        app.on_key(key(KeyCode::Esc));
        assert!(matches!(app.mode, Mode::Normal));
        assert!(app.pending_run.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn an_empty_run_line_is_refused() {
        let (_dir, mut app) = exec_fixture(Default::default());
        cursor_to(&mut app, "run.sh");
        app.on_key(key(KeyCode::Enter));
        for _ in 0.."./run.sh".len() {
            app.on_key(key(KeyCode::Backspace));
        }
        app.on_key(key(KeyCode::Enter));
        assert!(app.pending_run.is_none());
        assert!(
            app.session.status().contains("empty"),
            "{}",
            app.session.status()
        );
    }

    #[cfg(unix)]
    #[test]
    fn plain_files_and_disabled_runs_still_open_the_viewer() {
        let (_dir, mut app) = exec_fixture(Default::default());
        cursor_to(&mut app, "notes.txt");
        app.on_key(key(KeyCode::Enter));
        assert!(matches!(app.mode, Mode::Viewer(_)), "not executable");

        let config = bifold_core::config::Config {
            run_executables: false,
            ..Default::default()
        };
        let (_dir, mut app) = exec_fixture(config);
        cursor_to(&mut app, "run.sh");
        app.on_key(key(KeyCode::Enter));
        assert!(
            matches!(app.mode, Mode::Viewer(_)),
            "run_executables = false"
        );
    }

    #[cfg(unix)]
    #[test]
    fn f3_still_views_an_executable() {
        let (_dir, mut app) = exec_fixture(Default::default());
        cursor_to(&mut app, "run.sh");
        app.on_key(key(KeyCode::F(3)));
        assert!(matches!(app.mode, Mode::Viewer(_)));
    }

    fn ctrl(ch: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(ch), crossterm::event::KeyModifiers::CONTROL)
    }

    #[test]
    fn ctrl_o_without_a_previous_run_says_so() {
        let (_dir, mut app) = fixture();
        app.on_key(ctrl('o'));
        assert!(matches!(app.mode, Mode::Normal));
        assert!(
            app.session.status().contains("no command has been run"),
            "{}",
            app.session.status()
        );
    }

    #[test]
    fn ctrl_o_shows_the_last_output_and_q_closes_it() {
        let (dir, mut app) = fixture();
        app.session.last_run = Some(
            bifold_core::runcmd::run_captured(
                "echo first; echo second",
                dir.path(),
                bifold_core::runcmd::DEFAULT_TIMEOUT,
            )
            .unwrap(),
        );
        app.on_key(ctrl('o'));
        match &app.mode {
            Mode::Viewer(viewer) => {
                assert_eq!(viewer.preview.plain_lines(), ["first", "second"]);
                assert!(
                    viewer
                        .preview
                        .title()
                        .starts_with("$ echo first; echo second")
                );
            }
            other => panic!("expected the viewer, got {other:?}"),
        }
        app.on_key(key(KeyCode::Char('q')));
        assert!(matches!(app.mode, Mode::Normal));
    }

    #[test]
    fn ctrl_o_can_be_rebound() {
        let mut config = bifold_core::config::Config::default();
        config.keys.insert("ctrl+o".into(), "none".into());
        config.keys.insert("O".into(), "last_output".into());
        let (_dir, mut app) = {
            let dir = tempfile::tempdir().unwrap();
            fs::create_dir(dir.path().join("left")).unwrap();
            fs::create_dir(dir.path().join("right")).unwrap();
            let mut session = Session::with_themes_dir(config, None);
            let _ = session.panes[LEFT].navigate(&dir.path().join("left"));
            (dir, App::new(session))
        };
        app.on_key(ctrl('o'));
        assert!(
            app.session.status().is_empty() || !app.session.status().contains("no command"),
            "ctrl+o is unbound now"
        );
        app.on_key(key(KeyCode::Char('O')));
        assert!(
            app.session.status().contains("no command has been run"),
            "{}",
            app.session.status()
        );
    }
}
