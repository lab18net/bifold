//! Background transfer jobs: one at a time, cancellable, progress-reporting.
//!
//! A `JobRunner` owns at most one running thread. Both front ends poll it once
//! per frame and show the same numbers.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use crate::error::{Error, Result};
use crate::fsops::OpStats;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Working,
    Done,
    Cancelled,
    Failed,
}

impl Phase {
    pub fn is_finished(self) -> bool {
        !matches!(self, Self::Working)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Working => "working",
            Self::Done => "done",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
        }
    }
}

/// A snapshot of a job's progress, cheap to clone for rendering.
#[derive(Debug, Clone)]
pub struct Progress {
    pub label: String,
    pub current: String,
    pub files_done: u64,
    pub files_total: u64,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub phase: Phase,
    pub message: Option<String>,
}

impl Default for Progress {
    fn default() -> Self {
        Self {
            label: String::new(),
            current: String::new(),
            files_done: 0,
            files_total: 0,
            bytes_done: 0,
            bytes_total: 0,
            phase: Phase::Working,
            message: None,
        }
    }
}

impl Progress {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            ..Default::default()
        }
    }

    pub fn with_totals(mut self, files: u64, bytes: u64) -> Self {
        self.files_total = files;
        self.bytes_total = bytes;
        self
    }

    pub fn ratio(&self) -> f32 {
        let raw = if self.bytes_total > 0 {
            self.bytes_done as f64 / self.bytes_total as f64
        } else if self.files_total > 0 {
            self.files_done as f64 / self.files_total as f64
        } else {
            0.0
        };
        if self.phase.is_finished() && self.phase == Phase::Done {
            1.0
        } else {
            raw.clamp(0.0, 1.0) as f32
        }
    }

    pub fn headline(&self) -> String {
        match self.phase {
            Phase::Working => format!(
                "{} — {} of {} files, {} of {}",
                self.label,
                self.files_done,
                self.files_total,
                crate::format::human_size(self.bytes_done),
                crate::format::human_size(self.bytes_total),
            ),
            Phase::Done => format!("{} — {}", self.label, self.phase.label()),
            Phase::Cancelled => format!("{} — {}", self.label, self.phase.label()),
            Phase::Failed => format!(
                "{} — {}: {}",
                self.label,
                self.phase.label(),
                self.message.clone().unwrap_or_default()
            ),
        }
    }
}

/// Shared job state: progress plus the final stats.
#[derive(Debug, Default)]
pub struct JobState {
    pub progress: Progress,
    pub stats: OpStats,
}

/// Handle given to the worker closure.
#[derive(Clone)]
pub struct OpCtx {
    state: Arc<Mutex<JobState>>,
    cancel: Arc<AtomicBool>,
}

impl OpCtx {
    pub fn new(state: Arc<Mutex<JobState>>, cancel: Arc<AtomicBool>) -> Self {
        Self { state, cancel }
    }

    pub fn check(&self) -> Result<()> {
        if self.cancel.load(Ordering::Relaxed) {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }

    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    pub fn set_current(&self, path: &Path) {
        if let Ok(mut state) = self.state.lock() {
            state.progress.current = path.display().to_string();
        }
    }

    pub fn add_bytes(&self, bytes: u64) {
        if let Ok(mut state) = self.state.lock() {
            state.progress.bytes_done += bytes;
        }
    }

    pub fn add_file(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.progress.files_done += 1;
        }
    }
}

#[derive(Debug, Clone)]
pub struct JobResult {
    pub id: u64,
    pub label: String,
    pub phase: Phase,
    pub message: Option<String>,
    pub stats: OpStats,
}

impl JobResult {
    pub fn ok(&self) -> bool {
        self.phase == Phase::Done
    }

    pub fn summary(&self) -> String {
        match (self.phase, &self.message) {
            (Phase::Done, _) => format!("{}: {}", self.label, self.stats.summary()),
            (Phase::Cancelled, _) => format!("{}: cancelled", self.label),
            (Phase::Failed, Some(msg)) => format!("{}: {}", self.label, msg),
            (Phase::Failed, None) => format!("{}: failed", self.label),
            (Phase::Working, _) => self.label.clone(),
        }
    }
}

pub struct Job {
    pub id: u64,
    pub label: String,
    state: Arc<Mutex<JobState>>,
    cancel: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Job {
    pub fn snapshot(&self) -> Progress {
        self.state
            .lock()
            .map(|state| state.progress.clone())
            .unwrap_or_default()
    }

    pub fn is_finished(&self) -> bool {
        self.snapshot().phase.is_finished()
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    fn result(&self) -> JobResult {
        let state = self
            .state
            .lock()
            .map(|state| (state.progress.clone(), state.stats))
            .unwrap_or_default();
        JobResult {
            id: self.id,
            label: self.label.clone(),
            phase: state.0.phase,
            message: state.0.message,
            stats: state.1,
        }
    }
}

#[derive(Default)]
pub struct JobRunner {
    next_id: u64,
    current: Option<Job>,
}

impl JobRunner {
    pub fn new() -> Self {
        Self {
            next_id: 1,
            current: None,
        }
    }

    pub fn is_busy(&self) -> bool {
        self.current
            .as_ref()
            .map(|job| !job.is_finished())
            .unwrap_or(false)
    }

    pub fn current(&self) -> Option<&Job> {
        self.current.as_ref()
    }

    pub fn progress(&self) -> Option<Progress> {
        self.current.as_ref().map(Job::snapshot)
    }

    pub fn cancel(&self) {
        if let Some(job) = &self.current {
            job.cancel();
        }
    }

    /// Start a job. Returns `false` when one is already running.
    pub fn start<F>(
        &mut self,
        label: impl Into<String>,
        files_total: u64,
        bytes_total: u64,
        task: F,
    ) -> bool
    where
        F: FnOnce(&OpCtx) -> Result<OpStats> + Send + 'static,
    {
        if self.is_busy() {
            return false;
        }
        let id = self.next_id;
        self.next_id += 1;
        let label = label.into();

        let state = Arc::new(Mutex::new(JobState {
            progress: Progress::new(label.clone()).with_totals(files_total, bytes_total),
            stats: OpStats::default(),
        }));
        let cancel = Arc::new(AtomicBool::new(false));

        let ctx = OpCtx::new(state.clone(), cancel.clone());
        let worker_state = state.clone();
        let worker_cancel = cancel.clone();

        let handle = std::thread::spawn(move || {
            let outcome = task(&ctx);
            if let Ok(mut shared) = worker_state.lock() {
                match outcome {
                    Ok(stats) => {
                        shared.stats = stats;
                        shared.progress.files_done = shared.progress.files_total.max(stats.files);
                        shared.progress.bytes_done = shared.progress.bytes_total.max(stats.bytes);
                        shared.progress.phase = if worker_cancel.load(Ordering::Relaxed) {
                            Phase::Cancelled
                        } else {
                            Phase::Done
                        };
                    }
                    Err(err) if err.is_cancelled() => {
                        shared.progress.phase = Phase::Cancelled;
                    }
                    Err(err) => {
                        shared.progress.phase = Phase::Failed;
                        shared.progress.message = Some(err.to_string());
                    }
                }
                shared.progress.current.clear();
            }
        });

        self.current = Some(Job {
            id,
            label,
            state,
            cancel,
            handle: Some(handle),
        });
        true
    }

    /// Returns the finished job exactly once, clearing it.
    pub fn poll(&mut self) -> Option<JobResult> {
        let finished = self
            .current
            .as_ref()
            .map(|job| job.is_finished())
            .unwrap_or(false);
        if !finished {
            return None;
        }
        let mut job = self.current.take()?;
        if let Some(handle) = job.handle.take() {
            let _ = handle.join();
        }
        Some(job.result())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn wait_until(timeout: Duration, mut check: impl FnMut() -> bool) -> bool {
        let start = Instant::now();
        while start.elapsed() < timeout {
            if check() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        check()
    }

    #[test]
    fn job_completes_and_reports_stats() {
        let mut runner = JobRunner::new();
        assert!(runner.start("copy", 2, 4, |ctx| {
            ctx.add_bytes(2);
            ctx.add_file();
            std::thread::sleep(Duration::from_millis(10));
            ctx.add_bytes(2);
            ctx.add_file();
            Ok(OpStats {
                files: 2,
                bytes: 4,
                ..Default::default()
            })
        }));
        assert!(runner.is_busy());
        assert!(wait_until(Duration::from_secs(2), || !runner.is_busy()));

        let result = runner.poll().expect("finished job");
        assert_eq!(result.phase, Phase::Done);
        assert_eq!(result.stats.bytes, 4);
        assert!(result.summary().contains("2 file(s)"));
        assert!(runner.poll().is_none());
    }

    #[test]
    fn cancellation_is_reported() {
        let mut runner = JobRunner::new();
        runner.start("copy", 100, 0, |ctx| {
            for _ in 0..100 {
                ctx.check()?;
                std::thread::sleep(Duration::from_millis(5));
            }
            Ok(OpStats::default())
        });
        std::thread::sleep(Duration::from_millis(12));
        runner.cancel();
        assert!(wait_until(Duration::from_secs(2), || !runner.is_busy()));

        let result = runner.poll().unwrap();
        assert_eq!(result.phase, Phase::Cancelled);
    }

    #[test]
    fn failure_keeps_message() {
        let mut runner = JobRunner::new();
        runner.start("copy", 1, 1, |_| Err(Error::invalid("disk on fire")));
        assert!(wait_until(Duration::from_secs(2), || !runner.is_busy()));
        let result = runner.poll().unwrap();
        assert_eq!(result.phase, Phase::Failed);
        assert!(result.summary().contains("disk on fire"));
    }

    #[test]
    fn only_one_job_at_a_time() {
        let mut runner = JobRunner::new();
        assert!(runner.start("first", 0, 0, |_| {
            std::thread::sleep(Duration::from_millis(50));
            Ok(OpStats::default())
        }));
        assert!(!runner.start("second", 0, 0, |_| Ok(OpStats::default())));
        assert!(wait_until(Duration::from_secs(2), || !runner.is_busy()));
    }

    #[test]
    fn ratio_is_bounded() {
        let mut p = Progress::new("x").with_totals(10, 100);
        assert_eq!(p.ratio(), 0.0);
        p.bytes_done = 50;
        assert!((p.ratio() - 0.5).abs() < f32::EPSILON);
        p.bytes_done = 500;
        assert_eq!(p.ratio(), 1.0);
    }
}
