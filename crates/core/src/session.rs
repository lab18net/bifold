//! Two panes plus a job runner: the whole state a front end has to render.

use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::error::{Error, Result};
use crate::fsops::{self, DeleteMode, OpStats};
use crate::job::{JobResult, JobRunner, Progress};
use crate::keys::{Action, Keymap};
use crate::pane::Pane;
use crate::preview::{self, Preview, PreviewOptions};
use crate::sort::{SortDir, SortKey};
use crate::theme::{self, Theme};

pub const LEFT: usize = 0;
pub const RIGHT: usize = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferKind {
    Copy,
    Move,
}

impl TransferKind {
    pub fn verb(self) -> &'static str {
        match self {
            Self::Copy => "Copy",
            Self::Move => "Move",
        }
    }
}

pub struct Session {
    pub panes: [Pane; 2],
    pub active: usize,
    pub jobs: JobRunner,
    pub config: Config,
    pub keymap: Keymap,
    pub theme: Theme,
    /// Where user themes are read from (`None` in tests).
    pub themes_dir: Option<PathBuf>,
    /// Last status-line message (result of the previous operation).
    pub message: Option<String>,
    /// The last command run from the file list, with its output (`Ctrl+O`).
    pub last_run: Option<crate::runcmd::RunOutcome>,
}

impl Session {
    pub fn new(config: Config) -> Self {
        Self::with_themes_dir(config, Config::themes_dir())
    }

    /// Like `new`, with an explicit user-theme directory (`None` = built-ins only).
    pub fn with_themes_dir(config: Config, themes_dir: Option<PathBuf>) -> Self {
        let left = config.left_path();
        let right = config.right_path();
        let (keymap, mut warnings) = config.keymap();
        let (theme, theme_warnings) =
            theme::resolve(&config.theme, themes_dir.as_deref(), &config.theme_custom);
        warnings.extend(theme_warnings);
        let mut session = Self {
            panes: [Pane::new(left), Pane::new(right)],
            active: LEFT,
            jobs: JobRunner::new(),
            config,
            keymap,
            theme,
            themes_dir,
            message: (!warnings.is_empty()).then(|| format!("config: {}", warnings.join("; "))),
            last_run: None,
        };
        session.apply_preferences();
        session
    }

    pub fn new_at(left: &Path, right: &Path) -> Self {
        let mut session = Self::with_themes_dir(Config::default(), None);
        let _ = session.panes[LEFT].navigate(left);
        let _ = session.panes[RIGHT].navigate(right);
        session
    }

    /// Switch to another theme by name (user file or built-in). On failure the
    /// current theme stays and the reason goes to the status line.
    pub fn set_theme(&mut self, name: &str) -> bool {
        let (theme, warnings) =
            theme::resolve(name, self.themes_dir.as_deref(), &self.config.theme_custom);
        if theme.name != theme::canonical_name(name) {
            self.message = Some(format!("config: {}", warnings.join("; ")));
            return false;
        }
        self.config.theme = theme.name.clone();
        self.message = Some(if warnings.is_empty() {
            format!("theme: {}", theme.name)
        } else {
            format!("theme: {} (config: {})", theme.name, warnings.join("; "))
        });
        self.theme = theme;
        true
    }

    /// Pages of the `h` help screen for the current keymap, config and theme.
    pub fn help_pages(&self) -> Vec<crate::help::HelpPage> {
        let config_dir = Config::config_dir()
            .map(|dir| dir.display().to_string())
            .unwrap_or_else(|| "~/.config/bifold".to_string());
        let ctx = crate::help::HelpContext {
            config_dir,
            theme: self.theme.name.clone(),
            themes: self.theme_names(),
        };
        crate::help::pages(&self.keymap, &ctx)
    }

    /// The last run command as viewer content, or `None` before any run.
    pub fn last_run_preview(&self) -> Option<Preview> {
        let run = self.last_run.as_ref()?;
        Some(Preview::from_output(
            &format!("$ {}", run.command),
            &run.cwd,
            &run.output,
            run.truncated,
            Some(run.summary()),
        ))
    }

    pub fn run_suggestion(&self) -> Option<String> {
        if !self.config.run_executables {
            return None;
        }
        let entry = self.pane().cursor_entry()?;
        entry
            .is_executable()
            .then(|| crate::runcmd::command_for(&entry.name))
    }

    /// How long the desktop waits for a run command before detaching.
    pub fn run_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.config.run_timeout_ms.max(1))
    }

    pub fn theme_names(&self) -> Vec<String> {
        theme::available(self.themes_dir.as_deref())
    }

    pub fn cycle_theme(&mut self) {
        let names = self.theme_names();
        let at = names
            .iter()
            .position(|n| *n == self.theme.name)
            .unwrap_or(0);
        let next = names[(at + 1) % names.len()].clone();
        self.set_theme(&next);
    }

    /// Copy the live view state (sort, hidden files, theme, directories) into
    /// the config and write it. Both front ends call this on exit.
    pub fn save_config(&mut self) -> Result<PathBuf> {
        let (show_hidden, sort, sort_desc) = {
            let pane = self.pane();
            (
                pane.show_hidden,
                pane.sort_key.label().to_string(),
                pane.sort_dir == SortDir::Desc,
            )
        };
        self.config.show_hidden = show_hidden;
        self.config.sort = sort;
        self.config.sort_desc = sort_desc;
        self.config.theme = self.theme.name.clone();
        let left = self.panes[LEFT].path.clone();
        let right = self.panes[RIGHT].path.clone();
        self.config.remember_dirs(&left, &right);
        self.config.save()
    }

    /// Push config-level preferences into both panes.
    pub fn apply_preferences(&mut self) {
        let sort_key = self.config.sort_key();
        let sort_dir = self.config.sort_dir();
        for pane in &mut self.panes {
            pane.show_hidden = self.config.show_hidden;
            pane.sort_key = sort_key;
            pane.sort_dir = sort_dir;
            pane.dirs_first = self.config.dirs_first;
            pane.reload();
        }
    }

    pub fn pane(&self) -> &Pane {
        &self.panes[self.active]
    }

    pub fn pane_mut(&mut self) -> &mut Pane {
        &mut self.panes[self.active]
    }

    pub fn other(&self) -> &Pane {
        &self.panes[1 - self.active]
    }

    pub fn other_mut(&mut self) -> &mut Pane {
        &mut self.panes[1 - self.active]
    }

    pub fn switch(&mut self) {
        self.active = 1 - self.active;
    }

    /// Where transfers go by default: the passive pane.
    pub fn target_dir(&self) -> PathBuf {
        self.other().path.clone()
    }

    pub fn reload_all(&mut self) {
        for pane in &mut self.panes {
            pane.reload();
        }
    }

    pub fn refresh(&mut self) {
        self.pane_mut().reload();
    }

    pub fn status(&self) -> String {
        match &self.message {
            Some(message) if !message.is_empty() => message.clone(),
            _ => self.pane().status(),
        }
    }

    pub fn job_progress(&self) -> Option<Progress> {
        self.jobs.progress()
    }

    /// Run the actions that only touch pane state (navigation, marks, sort,
    /// hidden files, refresh). Returns false for actions that need UI
    /// (dialogs, viewer, quit) so the front end can handle them.
    pub fn apply(&mut self, action: Action, page_height: usize) -> bool {
        let handled = match action {
            Action::SwitchPane => {
                self.switch();
                true
            }
            Action::Up => {
                self.pane_mut().move_cursor(-1);
                true
            }
            Action::Down => {
                self.pane_mut().move_cursor(1);
                true
            }
            Action::PageUp => {
                self.pane_mut().page(-1, page_height);
                true
            }
            Action::PageDown => {
                self.pane_mut().page(1, page_height);
                true
            }
            Action::Home => {
                self.pane_mut().home();
                true
            }
            Action::End => {
                self.pane_mut().end();
                true
            }
            Action::Parent => {
                if !self.pane_mut().up() {
                    self.message = Some("already at the filesystem root".into());
                    return true;
                }
                true
            }
            Action::Back => {
                self.pane_mut().back();
                true
            }
            Action::Forward => {
                self.pane_mut().forward();
                true
            }
            Action::ToggleMark => {
                let pane = self.pane_mut();
                pane.toggle_select();
                pane.move_cursor(1);
                true
            }
            Action::MarkAll => {
                self.pane_mut().select_all();
                true
            }
            Action::ClearMarks => {
                self.pane_mut().clear_selection();
                true
            }
            Action::ToggleHidden => {
                self.pane_mut().toggle_hidden();
                true
            }
            Action::CycleSort => {
                self.pane_mut().cycle_sort();
                true
            }
            Action::ToggleSortOrder => {
                self.pane_mut().toggle_sort_dir();
                true
            }
            Action::CycleTheme => {
                self.cycle_theme();
                return true;
            }
            Action::Refresh => {
                self.reload_all();
                self.message = Some("refreshed".into());
                return true;
            }
            Action::Cancel => {
                let pane = self.pane_mut();
                pane.clear_selection();
                if !pane.filter.is_empty() {
                    pane.set_filter("");
                }
                true
            }
            _ => false,
        };
        if handled {
            self.message = None;
        }
        handled
    }

    /// Options for the viewer built from config, for a viewport of `width` x `height` cells.
    pub fn preview_options(&self, width: u16, height: u16, force_hex: bool) -> PreviewOptions {
        let previewer = self.config.previewer.trim();
        PreviewOptions {
            previewer: (!previewer.is_empty()).then(|| previewer.to_string()),
            timeout: std::time::Duration::from_millis(self.config.previewer_timeout_ms.max(1)),
            width,
            height,
            force_hex,
            ..Default::default()
        }
    }

    pub fn preview(
        &self,
        path: &Path,
        width: u16,
        height: u16,
        force_hex: bool,
    ) -> Result<Preview> {
        preview::load(path, &self.preview_options(width, height, force_hex))
    }

    // -- operations ---------------------------------------------------------

    pub fn start_copy(&mut self, sources: &[PathBuf], dest: &Path) -> Result<()> {
        self.start_transfer(TransferKind::Copy, sources, dest)
    }

    pub fn start_move(&mut self, sources: &[PathBuf], dest: &Path) -> Result<()> {
        self.start_transfer(TransferKind::Move, sources, dest)
    }

    pub fn start_transfer(
        &mut self,
        kind: TransferKind,
        sources: &[PathBuf],
        dest: &Path,
    ) -> Result<()> {
        if sources.is_empty() {
            return Err(Error::invalid("nothing to transfer"));
        }
        if self.jobs.is_busy() {
            return Err(Error::invalid("a transfer is already running"));
        }
        let dest = fsops::canonical_dir(dest)?;
        for source in sources {
            self.validate_transfer(source, &dest)?;
        }

        let totals = fsops::measure_many(sources)?;
        let label = format!(
            "{} {} item(s) → {}",
            kind.verb(),
            sources.len(),
            dest.display()
        );
        let sources: Vec<PathBuf> = sources.to_vec();
        let destination = dest.clone();
        let started = self
            .jobs
            .start(&label, totals.files, totals.bytes, move |ctx| {
                let mut stats = OpStats::default();
                for source in &sources {
                    let one = match kind {
                        TransferKind::Copy => fsops::copy_entry(source, &destination, ctx)?,
                        TransferKind::Move => fsops::move_entry(source, &destination, ctx)?,
                    };
                    stats.add(one);
                }
                Ok(stats)
            });
        if !started {
            return Err(Error::invalid("a transfer is already running"));
        }
        self.message = Some(label);
        Ok(())
    }

    pub fn start_delete(&mut self, targets: &[PathBuf], mode: DeleteMode) -> Result<()> {
        if targets.is_empty() {
            return Err(Error::invalid("nothing to delete"));
        }
        if self.jobs.is_busy() {
            return Err(Error::invalid("a transfer is already running"));
        }
        let totals = fsops::measure_many(targets)?;
        let verb = match mode {
            DeleteMode::Trash => "Trash",
            DeleteMode::Permanent => "Delete",
        };
        let label = format!("{verb} {} item(s)", targets.len());
        let targets: Vec<PathBuf> = targets.to_vec();
        let started = self
            .jobs
            .start(&label, totals.files, totals.bytes, move |ctx| {
                let mut stats = OpStats::default();
                for target in &targets {
                    stats.add(fsops::delete_entry(target, mode, None, ctx)?);
                }
                Ok(stats)
            });
        if !started {
            return Err(Error::invalid("a transfer is already running"));
        }
        self.message = Some(label);
        Ok(())
    }

    pub fn start_delete_with_mode(&mut self, mode: DeleteMode) -> Result<()> {
        self.start_delete(&self.pane().targets(), mode)
    }

    pub fn make_dir(&mut self, name: &str) -> Result<PathBuf> {
        let parent = self.pane().path.clone();
        let created = fsops::create_dir(&parent, name)?;
        self.reload_all();
        focus(self.pane_mut(), &created);
        self.message = Some(format!("created {}", created.display()));
        Ok(created)
    }

    pub fn make_file(&mut self, name: &str) -> Result<PathBuf> {
        let parent = self.pane().path.clone();
        let created = fsops::create_file(&parent, name)?;
        self.reload_all();
        focus(self.pane_mut(), &created);
        self.message = Some(format!("created {}", created.display()));
        Ok(created)
    }

    pub fn rename_cursor(&mut self, target: &str) -> Result<PathBuf> {
        let origin = self
            .pane()
            .cursor_entry()
            .map(|entry| entry.path.clone())
            .ok_or_else(|| Error::invalid("nothing under the cursor"))?;
        let renamed = fsops::rename(&origin, target)?;
        self.reload_all();
        focus(self.pane_mut(), &renamed);
        self.message = Some(format!("renamed to {}", renamed.display()));
        Ok(renamed)
    }

    /// Poll the job runner; on completion refresh both panes and set a message.
    pub fn poll_jobs(&mut self) -> Option<JobResult> {
        let result = self.jobs.poll()?;
        self.reload_all();
        self.message = Some(result.summary());
        Some(result)
    }

    pub fn cancel_job(&self) {
        self.jobs.cancel();
    }

    fn validate_transfer(&self, source: &Path, dest: &Path) -> Result<()> {
        let canonical = std::fs::canonicalize(source).map_err(|e| Error::io(source, e))?;
        if canonical.as_path() == dest {
            return Err(Error::invalid(format!(
                "destination is the source itself: {}",
                dest.display()
            )));
        }
        if canonical.is_dir() && dest.starts_with(&canonical) {
            return Err(Error::invalid(format!(
                "cannot move {} inside itself",
                canonical.display()
            )));
        }
        Ok(())
    }
}

/// Put the cursor on `path` if it is in the current listing.
pub fn focus(pane: &mut Pane, path: &Path) {
    if let Some(index) = pane.entries.iter().position(|entry| entry.path == path) {
        pane.cursor = index;
    }
}

/// Sort key/dir currently in force for the active pane.
pub fn active_sort(session: &Session) -> (SortKey, SortDir, bool) {
    let pane = session.pane();
    (pane.sort_key, pane.sort_dir, pane.show_hidden)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{Duration, Instant};

    use super::*;

    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("left")).unwrap();
        fs::create_dir(dir.path().join("right")).unwrap();
        fs::write(dir.path().join("left/a.txt"), b"aaa").unwrap();
        dir
    }

    fn wait_for_job(session: &mut Session) -> JobResult {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(5) {
            if let Some(result) = session.poll_jobs() {
                return result;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("job did not finish");
    }

    #[test]
    fn copy_between_panes() {
        let dir = fixture();
        let mut session = Session::new_at(&dir.path().join("left"), &dir.path().join("right"));
        let source = session.pane().entries[0].path.clone();
        session
            .start_copy(&[source], &dir.path().join("right"))
            .unwrap();
        let result = wait_for_job(&mut session);
        assert!(result.ok(), "{}", result.summary());
        assert!(dir.path().join("right/a.txt").exists());
        assert!(dir.path().join("left/a.txt").exists(), "copy keeps source");
    }

    #[test]
    fn move_between_panes() {
        let dir = fixture();
        let mut session = Session::new_at(&dir.path().join("left"), &dir.path().join("right"));
        let source = session.pane().entries[0].path.clone();
        session
            .start_move(&[source], &dir.path().join("right"))
            .unwrap();
        let result = wait_for_job(&mut session);
        assert!(result.ok(), "{}", result.summary());
        assert!(!dir.path().join("left/a.txt").exists());
        assert!(dir.path().join("right/a.txt").exists());
    }

    #[test]
    fn refuses_to_copy_into_itself() {
        let dir = fixture();
        let mut session = Session::new_at(&dir.path().join("left"), &dir.path().join("right"));
        let source = dir.path().join("left");
        let err = session
            .start_copy(&[source], &dir.path().join("left"))
            .unwrap_err();
        assert!(err.to_string().contains("source itself"), "{err}");
    }

    #[test]
    fn one_job_at_a_time() {
        let dir = fixture();
        let mut session = Session::new_at(&dir.path().join("left"), &dir.path().join("right"));
        session
            .start_copy(&[dir.path().join("left/a.txt")], &dir.path().join("right"))
            .unwrap();
        assert!(
            session
                .start_copy(&[dir.path().join("left")], &dir.path().join("right"))
                .is_err()
        );
        wait_for_job(&mut session);
    }

    #[test]
    fn mkdir_and_rename_move_the_cursor() {
        let dir = fixture();
        let mut session = Session::new_at(&dir.path().join("left"), &dir.path().join("right"));
        session.make_dir("fresh").unwrap();
        assert_eq!(session.pane().cursor_entry().unwrap().name, "fresh");
        session.rename_cursor("renamed").unwrap();
        assert_eq!(session.pane().cursor_entry().unwrap().name, "renamed");
        assert!(dir.path().join("left/renamed").is_dir());
    }

    #[test]
    fn delete_permanently_removes_the_target() {
        let dir = fixture();
        let mut session = Session::new_at(&dir.path().join("left"), &dir.path().join("right"));
        let targets = session.pane().targets();
        session
            .start_delete(&targets, DeleteMode::Permanent)
            .unwrap();
        let result = wait_for_job(&mut session);
        assert!(result.ok(), "{}", result.summary());
        assert!(!dir.path().join("left/a.txt").exists());
    }

    #[test]
    fn apply_handles_navigation_and_leaves_ui_actions() {
        let dir = fixture();
        fs::write(dir.path().join("left/b.txt"), b"b").unwrap();
        let mut session = Session::new_at(&dir.path().join("left"), &dir.path().join("right"));
        assert!(session.apply(Action::Down, 10));
        assert_eq!(session.pane().cursor, 1);
        assert!(session.apply(Action::ToggleMark, 10));
        assert_eq!(session.pane().selection_len(), 1);
        assert!(session.apply(Action::Cancel, 10));
        assert_eq!(session.pane().selection_len(), 0);
        assert!(session.apply(Action::SwitchPane, 10));
        assert_eq!(session.active, RIGHT);
        for ui_action in [
            Action::Copy,
            Action::Quit,
            Action::View,
            Action::Help,
            Action::Open,
            Action::LastOutput,
        ] {
            assert!(!session.apply(ui_action, 10), "{ui_action:?}");
        }
    }

    #[test]
    fn bad_key_config_is_reported_at_startup() {
        let mut config = Config::default();
        config.keys.insert("x".into(), "teleport".into());
        let session = Session::new(config);
        assert!(
            session.status().contains("teleport"),
            "{}",
            session.status()
        );
    }

    #[test]
    fn preview_uses_config_previewer_setting() {
        let dir = fixture();
        let mut session = Session::new_at(&dir.path().join("left"), &dir.path().join("right"));
        assert!(session.preview_options(10, 5, false).previewer.is_none());
        session.config.previewer = " /bin/true ".into();
        session.config.previewer_timeout_ms = 250;
        let options = session.preview_options(10, 5, true);
        assert_eq!(options.previewer.as_deref(), Some("/bin/true"));
        assert_eq!(options.timeout.as_millis(), 250);
        assert!(options.force_hex);
        let preview = session
            .preview(&dir.path().join("left/a.txt"), 10, 5, true)
            .unwrap();
        assert_eq!(preview.kind, preview::PreviewKind::Hex);
    }

    #[test]
    fn switching_panes_changes_target_dir() {
        let dir = fixture();
        let mut session = Session::new_at(&dir.path().join("left"), &dir.path().join("right"));
        assert_eq!(
            session.target_dir(),
            fs::canonicalize(dir.path().join("right")).unwrap()
        );
        session.switch();
        assert_eq!(
            session.target_dir(),
            fs::canonicalize(dir.path().join("left")).unwrap()
        );
    }

    #[test]
    fn set_theme_switches_reports_and_refuses_unknown() {
        let dir = fixture();
        let mut session = Session::new_at(&dir.path().join("left"), &dir.path().join("right"));
        assert_eq!(session.theme.name, theme::DEFAULT_THEME);
        assert!(session.set_theme("Solarized Light"));
        assert_eq!(session.theme.name, "solarized-light");
        assert_eq!(session.config.theme, "solarized-light");
        assert_eq!(session.status(), "theme: solarized-light");
        assert!(!session.set_theme("no-such-theme"));
        assert_eq!(
            session.theme.name, "solarized-light",
            "unchanged on failure"
        );
        assert!(
            session.status().contains("no-such-theme"),
            "{}",
            session.status()
        );
    }

    #[test]
    fn cycle_theme_visits_builtin_and_user_themes_then_wraps() {
        let dir = fixture();
        let themes = dir.path().join("themes");
        fs::create_dir(&themes).unwrap();
        fs::write(
            themes.join("mine.toml"),
            "extends = \"nord\"\naccent = \"#ff8800\"\n",
        )
        .unwrap();
        let mut session = Session::with_themes_dir(Config::default(), Some(themes));
        let names = session.theme_names();
        assert_eq!(names.last().map(String::as_str), Some("mine"));
        let mut seen = vec![session.theme.name.clone()];
        for _ in 0..names.len() {
            assert!(session.apply(Action::CycleTheme, 10));
            seen.push(session.theme.name.clone());
        }
        assert_eq!(seen.first(), seen.last(), "wrapped around");
        assert_eq!(
            &seen[1..=names.len()]
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            &names.len()
        );
        assert!(seen.contains(&"mine".to_string()));
    }

    #[test]
    fn config_theme_and_overrides_reach_the_session() {
        let dir = fixture();
        let themes = dir.path().join("themes");
        fs::create_dir(&themes).unwrap();
        fs::write(
            themes.join("mine.toml"),
            "extends = \"gruvbox\"\nred = \"#010203\"\n",
        )
        .unwrap();
        let mut config = Config {
            theme: "mine".into(),
            ..Default::default()
        };
        config
            .theme_custom
            .insert("accent".into(), "#0a0b0c".into());
        config.theme_custom.insert("nonsense".into(), "#fff".into());
        let session = Session::with_themes_dir(config, Some(themes));
        assert_eq!(session.theme.name, "mine");
        assert_eq!(session.theme.red, Some(crate::style::Color::Rgb(1, 2, 3)));
        assert_eq!(
            session.theme.accent,
            Some(crate::style::Color::Rgb(10, 11, 12))
        );
        assert!(
            session.status().contains("nonsense"),
            "{}",
            session.status()
        );
    }

    #[cfg(unix)]
    #[test]
    fn run_suggestion_only_for_executable_files() {
        use std::os::unix::fs::PermissionsExt;
        let dir = fixture();
        let left = dir.path().join("left");
        let script = left.join("my script.sh");
        fs::write(&script, "#!/bin/sh\n").unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        fs::create_dir(left.join("zdir")).unwrap();
        let mut session = Session::new_at(&left, &dir.path().join("right"));

        let at = |s: &Session, name: &str| {
            s.pane()
                .entries
                .iter()
                .position(|e| e.name == name)
                .unwrap()
        };
        session.pane_mut().cursor = at(&session, "my script.sh");
        assert_eq!(
            session.run_suggestion().as_deref(),
            Some("'./my script.sh'")
        );
        session.pane_mut().cursor = at(&session, "a.txt");
        assert_eq!(session.run_suggestion(), None, "plain file");
        session.pane_mut().cursor = at(&session, "zdir");
        assert_eq!(session.run_suggestion(), None, "folder");

        session.pane_mut().cursor = at(&session, "my script.sh");
        session.config.run_executables = false;
        assert_eq!(session.run_suggestion(), None, "switched off in config");
        session.config.run_timeout_ms = 250;
        assert_eq!(session.run_timeout().as_millis(), 250);
    }

    #[test]
    fn last_run_becomes_a_viewer_preview_only_once_something_ran() {
        let dir = fixture();
        let mut session = Session::new_at(&dir.path().join("left"), &dir.path().join("right"));
        assert!(session.last_run_preview().is_none(), "nothing ran yet");

        session.last_run = Some(
            crate::runcmd::run_captured(
                "echo hi; exit 2",
                &dir.path().join("left"),
                crate::runcmd::DEFAULT_TIMEOUT,
            )
            .unwrap(),
        );
        let preview = session.last_run_preview().unwrap();
        assert_eq!(preview.plain_lines(), ["hi"]);
        assert_eq!(
            preview.title(),
            "$ echo hi; exit 2  [output]  (ran: echo hi; exit 2 — exit 2)"
        );
    }
}
