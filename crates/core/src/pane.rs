//! One listing: current directory, cursor, multi-selection, sort and filter.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::entry::Entry;
use crate::error::Result;
use crate::filter::matches;
use crate::fsops;
use crate::sort::{SortDir, SortKey, sort_entries};

#[derive(Debug, Clone)]
pub struct Pane {
    pub path: PathBuf,
    /// Rows after filtering and sorting — this is what the UI draws.
    pub entries: Vec<Entry>,
    pub cursor: usize,
    pub offset: usize,
    pub selected: BTreeSet<PathBuf>,
    pub sort_key: SortKey,
    pub sort_dir: SortDir,
    pub show_hidden: bool,
    pub filter: String,
    pub dirs_first: bool,
    pub error: Option<String>,
    /// Number of entries in the directory before filtering.
    pub total: usize,
    history: Vec<PathBuf>,
    history_idx: usize,
}

impl Pane {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        let mut pane = Self {
            path: PathBuf::new(),
            entries: Vec::new(),
            cursor: 0,
            offset: 0,
            selected: BTreeSet::new(),
            sort_key: SortKey::Name,
            sort_dir: SortDir::Asc,
            show_hidden: false,
            filter: String::new(),
            dirs_first: true,
            error: None,
            total: 0,
            history: Vec::new(),
            history_idx: 0,
        };
        let requested = path.into();
        if pane.navigate(&requested).is_err() {
            let fallback = fsops::home_dir().unwrap_or_else(|| PathBuf::from("/"));
            let _ = pane.navigate(&fallback);
        }
        pane
    }

    /// Change directory and remember it in the back/forward history.
    pub fn navigate(&mut self, target: &Path) -> Result<()> {
        let dir = fsops::canonical_dir(target)?;
        self.path = dir.clone();
        self.history.truncate(self.history_idx + 1);
        self.history.push(dir);
        self.history_idx = self.history.len() - 1;
        self.cursor = 0;
        self.offset = 0;
        self.selected.clear();
        self.reload();
        Ok(())
    }

    pub fn back(&mut self) -> bool {
        if self.history_idx == 0 {
            return false;
        }
        self.history_idx -= 1;
        let target = self.history[self.history_idx].clone();
        self.goto_raw(&target)
    }

    pub fn forward(&mut self) -> bool {
        if self.history_idx + 1 >= self.history.len() {
            return false;
        }
        self.history_idx += 1;
        let target = self.history[self.history_idx].clone();
        self.goto_raw(&target)
    }

    fn goto_raw(&mut self, target: &Path) -> bool {
        if fsops::canonical_dir(target).is_err() {
            return false;
        }
        self.path = target.to_path_buf();
        self.cursor = 0;
        self.offset = 0;
        self.selected.clear();
        self.reload();
        true
    }

    /// Re-read the directory, keeping the cursor on the same file when possible.
    pub fn reload(&mut self) {
        let keep = self.entries.get(self.cursor).map(|e| e.path.clone());
        match fsops::list_dir(&self.path, self.show_hidden) {
            Ok(mut list) => {
                self.total = list.len();
                if !self.filter.trim().is_empty() {
                    let pattern = self.filter.clone();
                    list.retain(|entry| matches(&entry.name, &pattern));
                }
                sort_entries(&mut list, self.sort_key, self.sort_dir, self.dirs_first);
                self.entries = list;
                self.error = None;
            }
            Err(err) => {
                self.entries.clear();
                self.total = 0;
                self.error = Some(err.to_string());
            }
        }

        let present: BTreeSet<&PathBuf> = self.entries.iter().map(|e| &e.path).collect();
        let stale: Vec<PathBuf> = self
            .selected
            .iter()
            .filter(|p| !present.contains(p))
            .cloned()
            .collect();
        for path in stale {
            self.selected.remove(&path);
        }

        self.cursor = keep
            .and_then(|path| self.entries.iter().position(|e| e.path == path))
            .unwrap_or_else(|| self.cursor.min(self.entries.len().saturating_sub(1)));
        self.clamp(0);
    }

    pub fn cursor_entry(&self) -> Option<&Entry> {
        self.entries.get(self.cursor)
    }

    /// Paths an operation should act on: the multi-selection when there is one,
    /// otherwise the row under the cursor.
    pub fn targets(&self) -> Vec<PathBuf> {
        if !self.selected.is_empty() {
            self.selected.iter().cloned().collect()
        } else {
            self.cursor_entry()
                .map(|entry| vec![entry.path.clone()])
                .unwrap_or_default()
        }
    }

    pub fn selection_len(&self) -> usize {
        self.selected.len()
    }

    pub fn toggle_select(&mut self) -> bool {
        let Some(entry) = self.cursor_entry().cloned() else {
            return false;
        };
        if !self.selected.remove(&entry.path) {
            self.selected.insert(entry.path);
        }
        true
    }

    pub fn select_all(&mut self) {
        self.selected = self.entries.iter().map(|e| e.path.clone()).collect();
    }

    pub fn clear_selection(&mut self) {
        self.selected.clear();
    }

    pub fn move_cursor(&mut self, delta: isize) {
        if self.entries.is_empty() {
            self.cursor = 0;
            return;
        }
        let max = self.entries.len() - 1;
        let next = self.cursor as isize + delta;
        self.cursor = next.clamp(0, max as isize) as usize;
    }

    pub fn page(&mut self, pages: isize, height: usize) {
        let step = height.max(1) as isize;
        self.move_cursor(pages * step);
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.entries.len().saturating_sub(1);
    }

    /// Enter the directory under the cursor, leaving the cursor on it when
    /// navigating back up.
    pub fn descend(&mut self) -> Result<bool> {
        let Some(entry) = self.cursor_entry() else {
            return Ok(false);
        };
        if !entry.is_dir() {
            return Ok(false);
        }
        let target = entry.path.clone();
        self.navigate(&target)?;
        Ok(true)
    }

    pub fn up(&mut self) -> bool {
        let Some(parent) = self.path.parent().map(Path::to_path_buf) else {
            return false;
        };
        let came_from = self
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned());
        if self.navigate(&parent).is_err() {
            return false;
        }
        if let Some(name) = came_from
            && let Some(index) = self.entries.iter().position(|e| e.name == name)
        {
            self.cursor = index;
        }
        true
    }

    pub fn cycle_sort(&mut self) {
        self.sort_key = self.sort_key.next();
        self.reload();
    }

    pub fn toggle_sort_dir(&mut self) {
        self.sort_dir = self.sort_dir.toggled();
        self.reload();
    }

    pub fn toggle_hidden(&mut self) {
        self.show_hidden = !self.show_hidden;
        self.reload();
    }

    pub fn set_filter(&mut self, filter: impl Into<String>) {
        self.filter = filter.into();
        self.cursor = 0;
        self.offset = 0;
        self.reload();
    }

    /// Keep the cursor inside the viewport of `height` rows.
    pub fn clamp(&mut self, height: usize) {
        if self.entries.is_empty() {
            self.cursor = 0;
            self.offset = 0;
            return;
        }
        self.cursor = self.cursor.min(self.entries.len() - 1);
        if height == 0 {
            return;
        }
        if self.cursor < self.offset {
            self.offset = self.cursor;
        }
        if self.cursor >= self.offset + height {
            self.offset = self.cursor + 1 - height;
        }
        self.offset = self.offset.min(self.entries.len().saturating_sub(height));
    }

    pub fn status(&self) -> String {
        let mut parts = vec![format!("{} item(s)", self.total)];
        if !self.filter.trim().is_empty() {
            parts.push(format!(
                "filter “{}” → {}",
                self.filter.trim(),
                self.entries.len()
            ));
        }
        if let Some(entry) = self.cursor_entry() {
            parts.push(format!("{} {}", entry.kind.label(), entry.name));
            if entry.kind == crate::entry::EntryKind::File {
                parts.push(crate::format::human_size(entry.size));
            }
            if let Some(time) = entry.modified {
                parts.push(crate::format::fmt_time(time));
            }
            if let Some(mode) = entry.mode {
                let _ = mode;
                let perm = entry.mode_string();
                if !perm.is_empty() {
                    parts.push(perm);
                }
            }
        }
        if !self.selected.is_empty() {
            parts.push(format!("{} selected", self.selected.len()));
        }
        parts.join("  ·  ")
    }
}

impl Default for Pane {
    fn default() -> Self {
        Self::new(".")
    }
}

/// Convenience for callers that have a string path.
pub fn pane_at(path: &str) -> Result<Pane> {
    let dir = fsops::resolve_dir(path)?;
    Ok(Pane::new(dir))
}

impl std::fmt::Display for Pane {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.path.display())
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("alpha.txt"), b"aaa").unwrap();
        fs::write(dir.path().join("beta.txt"), b"bb").unwrap();
        fs::write(dir.path().join(".hidden"), b"h").unwrap();
        fs::create_dir(dir.path().join("zeta")).unwrap();
        dir
    }

    #[test]
    fn sorts_and_filters() {
        let dir = fixture();
        let mut pane = Pane::new(dir.path());
        let names: Vec<_> = pane.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["zeta", "alpha.txt", "beta.txt"]);
        assert_eq!(pane.total, 3);

        pane.toggle_hidden();
        assert_eq!(pane.total, 4);

        pane.set_filter("*.txt");
        assert_eq!(pane.entries.len(), 2);
        pane.set_filter("zzz");
        assert!(pane.entries.is_empty());
        assert_eq!(pane.cursor, 0);
        pane.set_filter("");
        assert_eq!(pane.entries.len(), 4);
    }

    #[test]
    fn cursor_moves_and_clamps() {
        let dir = fixture();
        let mut pane = Pane::new(dir.path());
        pane.move_cursor(1);
        assert_eq!(pane.cursor, 1);
        pane.move_cursor(99);
        assert_eq!(pane.cursor, 2);
        pane.move_cursor(-99);
        assert_eq!(pane.cursor, 0);
        pane.end();
        assert_eq!(pane.cursor, 2);
        pane.home();
        assert_eq!(pane.cursor, 0);
    }

    #[test]
    fn reload_keeps_cursor_on_same_file() {
        let dir = fixture();
        let mut pane = Pane::new(dir.path());
        assert_eq!(pane.cursor_entry().unwrap().name, "zeta");
        pane.move_cursor(2);
        let kept = pane.cursor_entry().unwrap().name.clone();
        fs::write(dir.path().join("0-first.txt"), b"x").unwrap();
        pane.reload();
        assert_eq!(pane.cursor_entry().unwrap().name, kept);
    }

    #[test]
    fn descend_and_up_restores_cursor() {
        let dir = fixture();
        let mut pane = Pane::new(dir.path());
        assert!(pane.descend().unwrap(), "zeta is a directory");
        assert_eq!(
            pane.path,
            fs::canonicalize(dir.path().join("zeta")).unwrap()
        );
        assert!(pane.up());
        assert_eq!(pane.path, fs::canonicalize(dir.path()).unwrap());
        assert_eq!(pane.cursor_entry().unwrap().name, "zeta");
    }

    #[test]
    fn selection_and_targets() {
        let dir = fixture();
        let mut pane = Pane::new(dir.path());
        assert_eq!(pane.targets().len(), 1);
        assert!(pane.toggle_select());
        pane.move_cursor(1);
        assert!(pane.toggle_select());
        assert_eq!(pane.targets().len(), 2);
        pane.clear_selection();
        assert_eq!(pane.targets().len(), 1);

        pane.select_all();
        assert_eq!(pane.selection_len(), 3);
        fs::remove_file(dir.path().join("beta.txt")).unwrap();
        pane.reload();
        assert_eq!(pane.selection_len(), 2, "stale selection dropped");
    }

    #[test]
    fn hidden_toggle_and_errors() {
        let dir = fixture();
        let mut pane = Pane::new(dir.path());
        pane.toggle_hidden();
        assert!(pane.entries.iter().any(|e| e.name == ".hidden"));

        let missing = dir.path().join("nope");
        assert!(pane.navigate(&missing).is_err());
        assert_eq!(pane.path, fs::canonicalize(dir.path()).unwrap());
    }

    #[test]
    fn history_back_forward() {
        let dir = fixture();
        let start = fs::canonicalize(dir.path()).unwrap();
        let mut pane = Pane::new(&start);
        pane.navigate(&start.join("zeta")).unwrap();
        assert!(pane.back());
        assert_eq!(pane.path, start);
        assert!(pane.forward());
        assert_eq!(pane.path, start.join("zeta"));
    }

    #[test]
    fn status_mentions_current_entry() {
        let dir = fixture();
        let mut pane = Pane::new(dir.path());
        pane.move_cursor(1);
        let status = pane.status();
        assert!(status.contains("alpha.txt"));
        assert!(status.contains("3B"));
    }

    #[test]
    fn sort_by_size_descending_puts_biggest_file_first() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("small"), vec![0u8; 1]).unwrap();
        fs::write(dir.path().join("large"), vec![0u8; 4096]).unwrap();
        let mut pane = Pane::new(dir.path());
        pane.sort_key = SortKey::Size;
        pane.sort_dir = SortDir::Desc;
        pane.reload();
        assert_eq!(pane.entries[0].name, "large");
    }

    #[test]
    fn pane_at_rejects_files() {
        let dir = fixture();
        let file = dir.path().join("alpha.txt");
        assert!(pane_at(&file.to_string_lossy()).is_err());
        assert!(pane_at(&dir.path().to_string_lossy()).is_ok());
    }
}
