//! UI-agnostic application state plus the keyboard contract for the TUI.

use std::path::PathBuf;

use bifold_core::error::Result;
use bifold_core::fsops::{self, DeleteMode};
use bifold_core::preview::{self, Preview};
use bifold_core::session::{LEFT, RIGHT, Session};
use bifold_core::sort::SortDir;
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
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Filter => "pattern",
            Self::GoTo => "path",
            Self::MkDir | Self::NewFile | Self::Rename => "name",
            Self::CopyTo | Self::MoveTo => "destination",
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
    Help,
}

pub struct App {
    pub session: Session,
    pub mode: Mode,
    pub should_quit: bool,
    /// Rows visible per pane, published by the renderer.
    pub viewport: [usize; 2],
    /// False when started with `--no-config`: leave the config file alone.
    pub persist: bool,
}

impl App {
    pub fn new(session: Session) -> Self {
        Self {
            session,
            mode: Mode::Normal,
            should_quit: false,
            viewport: [20, 20],
            persist: true,
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
        let (show_hidden, sort, sort_desc) = {
            let pane = self.session.pane();
            (
                pane.show_hidden,
                pane.sort_key.label().to_string(),
                pane.sort_dir == SortDir::Desc,
            )
        };
        self.session.config.show_hidden = show_hidden;
        self.session.config.sort = sort;
        self.session.config.sort_desc = sort_desc;
        let left = self.session.panes[LEFT].path.clone();
        let right = self.session.panes[RIGHT].path.clone();
        self.session.config.remember_dirs(&left, &right);
        let _ = self.session.config.save();
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
            Mode::Help => self.handle_help(key),
        };
    }

    fn handle_normal(&mut self, key: KeyEvent) -> Mode {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let pane_height = self.viewport[self.session.active].max(1);

        match key.code {
            KeyCode::Char('c') if ctrl => {
                self.request_quit();
                Mode::Normal
            }
            KeyCode::Char('q') => {
                self.request_quit();
                Mode::Normal
            }
            KeyCode::Tab | KeyCode::BackTab => {
                self.session.switch();
                Mode::Normal
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.session.pane_mut().move_cursor(-1);
                Mode::Normal
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.session.pane_mut().move_cursor(1);
                Mode::Normal
            }
            KeyCode::PageUp => {
                self.session.pane_mut().page(-1, pane_height);
                Mode::Normal
            }
            KeyCode::PageDown => {
                self.session.pane_mut().page(1, pane_height);
                Mode::Normal
            }
            KeyCode::Home => {
                self.session.pane_mut().home();
                Mode::Normal
            }
            KeyCode::End => {
                self.session.pane_mut().end();
                Mode::Normal
            }
            KeyCode::Left | KeyCode::Backspace => {
                if !self.session.pane_mut().up() {
                    self.set_message("already at the filesystem root");
                }
                Mode::Normal
            }
            KeyCode::Right | KeyCode::Enter | KeyCode::Char('l') => self.open_cursor(),
            KeyCode::Char(' ') | KeyCode::Insert => {
                let pane = self.session.pane_mut();
                pane.toggle_select();
                pane.move_cursor(1);
                Mode::Normal
            }
            KeyCode::F(2) => self.prompt(PromptKind::Rename, None),
            KeyCode::F(3) => self.open_viewer(),
            KeyCode::F(5) => self.prompt(PromptKind::CopyTo, None),
            KeyCode::F(6) => self.prompt(PromptKind::MoveTo, None),
            KeyCode::F(7) => self.prompt(PromptKind::MkDir, None),
            KeyCode::F(8) => self.confirm_delete(),
            KeyCode::Char('n') if ctrl => self.prompt(PromptKind::NewFile, None),
            KeyCode::Char('a') if ctrl => {
                self.session.pane_mut().select_all();
                Mode::Normal
            }
            KeyCode::Char('d') if ctrl => {
                self.session.pane_mut().clear_selection();
                Mode::Normal
            }
            KeyCode::Char('.') => {
                self.session.pane_mut().toggle_hidden();
                Mode::Normal
            }
            KeyCode::Char('s') => {
                self.session.pane_mut().cycle_sort();
                Mode::Normal
            }
            KeyCode::Char('o') => {
                self.session.pane_mut().toggle_sort_dir();
                Mode::Normal
            }
            KeyCode::Char('/') => {
                self.prompt(PromptKind::Filter, Some(self.session.pane().filter.clone()))
            }
            KeyCode::Char('g') => self.prompt(
                PromptKind::GoTo,
                Some(self.session.pane().path.display().to_string()),
            ),
            KeyCode::Char('r') => {
                self.session.reload_all();
                self.set_message("refreshed");
                Mode::Normal
            }
            KeyCode::Char('b') => {
                self.session.pane_mut().back();
                Mode::Normal
            }
            KeyCode::Char('f') => {
                self.session.pane_mut().forward();
                Mode::Normal
            }
            KeyCode::Char('?') => Mode::Help,
            KeyCode::Esc => {
                let pane = self.session.pane_mut();
                pane.clear_selection();
                if !pane.filter.is_empty() {
                    pane.set_filter("");
                }
                Mode::Normal
            }
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
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::F(3) => return Mode::Normal,
            KeyCode::Down | KeyCode::Char('j') => {
                viewer.scroll = (viewer.scroll + 1).min(max_scroll)
            }
            KeyCode::Up | KeyCode::Char('k') => viewer.scroll = viewer.scroll.saturating_sub(1),
            KeyCode::PageDown | KeyCode::Char(' ') => {
                viewer.scroll = (viewer.scroll + page).min(max_scroll)
            }
            KeyCode::PageUp => viewer.scroll = viewer.scroll.saturating_sub(page),
            KeyCode::Home | KeyCode::Char('g') => viewer.scroll = 0,
            KeyCode::End | KeyCode::Char('G') => viewer.scroll = max_scroll,
            KeyCode::Char('w') => viewer.wrap = !viewer.wrap,
            _ => {}
        }
        Mode::Viewer(viewer)
    }

    fn handle_help(&mut self, _key: KeyEvent) -> Mode {
        Mode::Normal
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
        } else {
            self.viewer_for(&entry.path)
        }
    }

    fn open_viewer(&mut self) -> Mode {
        let Some(entry) = self.session.pane().cursor_entry().cloned() else {
            return Mode::Normal;
        };
        if entry.is_dir() {
            self.set_message("F3 works on files; use Enter to open a folder");
            return Mode::Normal;
        }
        self.viewer_for(&entry.path)
    }

    fn viewer_for(&mut self, path: &std::path::Path) -> Mode {
        match preview::load_default(path) {
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

#[cfg(test)]
mod tests {
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
            Mode::Viewer(viewer) => assert!(viewer.preview.lines.iter().any(|l| l == "hello")),
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
}
