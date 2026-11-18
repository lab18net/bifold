//! Two panes plus a job runner: the whole state a front end has to render.

use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::error::{Error, Result};
use crate::fsops::{self, DeleteMode, OpStats};
use crate::job::{JobResult, JobRunner, Progress};
use crate::pane::Pane;
use crate::sort::{SortDir, SortKey};

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
    /// Last status-line message (result of the previous operation).
    pub message: Option<String>,
}

impl Session {
    pub fn new(config: Config) -> Self {
        let left = config.left_path();
        let right = config.right_path();
        let mut session = Self {
            panes: [Pane::new(left), Pane::new(right)],
            active: LEFT,
            jobs: JobRunner::new(),
            config,
            message: None,
        };
        session.apply_preferences();
        session
    }

    pub fn new_at(left: &Path, right: &Path) -> Self {
        let mut session = Self::new(Config::default());
        let _ = session.panes[LEFT].navigate(left);
        let _ = session.panes[RIGHT].navigate(right);
        session
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
}
