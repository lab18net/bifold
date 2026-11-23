//! Commands typed into the run line, and walking back through them.
//!
//! The list lives in the session (shared by both front ends, in memory for the
//! life of the process). [`Recall`] is the per-prompt cursor: Up goes to older
//! entries, Down comes back toward what was being typed, like a shell.

const MAX_ENTRIES: usize = 500;

#[derive(Debug, Clone, Default)]
pub struct History {
    entries: Vec<String>,
}

impl History {
    /// Remember a command. Blank lines are ignored; a command already in the list
    /// moves to the end instead of appearing twice.
    pub fn push(&mut self, command: &str) {
        let command = command.trim();
        if command.is_empty() {
            return;
        }
        self.entries.retain(|e| e != command);
        self.entries.push(command.to_string());
        if self.entries.len() > MAX_ENTRIES {
            self.entries.remove(0);
        }
    }

    /// Oldest first.
    pub fn entries(&self) -> &[String] {
        &self.entries
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Where one run prompt is in the history. Create a fresh one per prompt and call
/// [`Recall::reset`] whenever the user edits the text.
#[derive(Debug, Clone, Default)]
pub struct Recall {
    /// Index of the entry being shown; `None` while the user's own text is shown.
    at: Option<usize>,
    /// What was typed before the first Up, restored by Down past the newest entry.
    draft: String,
}

impl Recall {
    /// Up: the next older command. `current` is the text in the field. `None`
    /// when there is nothing older (the field is left as it is).
    pub fn older(&mut self, history: &History, current: &str) -> Option<String> {
        let entries = history.entries();
        let at = match self.at {
            None if entries.is_empty() => return None,
            None => {
                self.draft = current.to_string();
                entries.len() - 1
            }
            Some(0) => return None,
            // The list may have shrunk since (a duplicate moved); stay in range.
            Some(i) => (i - 1).min(entries.len() - 1),
        };
        self.at = Some(at);
        Some(entries[at].clone())
    }

    /// Down: the next newer command, and finally the text that was being typed.
    /// `None` when the user's own text is already showing.
    pub fn newer(&mut self, history: &History, _current: &str) -> Option<String> {
        let entries = history.entries();
        match self.at? {
            i if i + 1 < entries.len() => {
                self.at = Some(i + 1);
                Some(entries[i + 1].clone())
            }
            _ => {
                self.at = None;
                Some(std::mem::take(&mut self.draft))
            }
        }
    }

    /// The text was edited: the next Up starts again from the newest entry.
    pub fn reset(&mut self) {
        self.at = None;
        self.draft.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn history(items: &[&str]) -> History {
        let mut h = History::default();
        for item in items {
            h.push(item);
        }
        h
    }

    #[test]
    fn push_trims_ignores_blank_and_moves_duplicates_to_the_end() {
        let mut h = history(&["make", "  git status  ", "", "   ", "make"]);
        assert_eq!(h.entries(), ["git status", "make"]);
        h.push("git status");
        assert_eq!(h.entries(), ["make", "git status"]);
    }

    #[test]
    fn push_is_capped() {
        let mut h = History::default();
        for i in 0..MAX_ENTRIES + 25 {
            h.push(&format!("cmd {i}"));
        }
        assert_eq!(h.entries().len(), MAX_ENTRIES);
        assert_eq!(h.entries()[0], "cmd 25");
    }

    #[test]
    fn up_walks_back_and_stops_at_the_oldest() {
        let h = history(&["one", "two", "three"]);
        let mut r = Recall::default();
        assert_eq!(r.older(&h, "").as_deref(), Some("three"));
        assert_eq!(r.older(&h, "three").as_deref(), Some("two"));
        assert_eq!(r.older(&h, "two").as_deref(), Some("one"));
        assert_eq!(r.older(&h, "one"), None, "nothing older: field unchanged");
        assert_eq!(r.older(&h, "one"), None);
    }

    #[test]
    fn down_comes_back_and_restores_what_was_typed() {
        let h = history(&["one", "two"]);
        let mut r = Recall::default();
        assert_eq!(r.newer(&h, "half-typed"), None, "nothing to come back to");
        assert_eq!(r.older(&h, "half-typed").as_deref(), Some("two"));
        assert_eq!(r.older(&h, "two").as_deref(), Some("one"));
        assert_eq!(r.newer(&h, "one").as_deref(), Some("two"));
        assert_eq!(r.newer(&h, "two").as_deref(), Some("half-typed"));
        assert_eq!(r.newer(&h, "half-typed"), None);
    }

    #[test]
    fn an_empty_history_recalls_nothing() {
        let mut r = Recall::default();
        assert_eq!(r.older(&History::default(), "x"), None);
    }

    #[test]
    fn reset_starts_over_from_the_newest() {
        let h = history(&["one", "two", "three"]);
        let mut r = Recall::default();
        r.older(&h, "");
        r.older(&h, "three");
        r.reset();
        assert_eq!(r.older(&h, "edited").as_deref(), Some("three"));
        assert_eq!(r.newer(&h, "three").as_deref(), Some("edited"));
    }
}
