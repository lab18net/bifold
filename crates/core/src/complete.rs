//! Tab completion for text inputs, shared by both front ends.
//!
//! The front ends only hold a [`Completer`] next to their input buffer and
//! call [`Completer::tab`] when Tab (or Shift+Tab) is pressed; everything else —
//! what is completed, shell quoting, the common-prefix step, cycling — lives
//! here, so the terminal and the desktop behave identically.
//!
//! Behaviour, bash-like:
//! * a unique match is filled in (a folder gets `/`; in a command line a file
//!   gets a space, and an open quote is closed);
//! * several matches: the text grows to their common prefix and the candidates
//!   are listed;
//! * Tab again, with nothing left to extend, cycles through the candidates
//!   (Shift+Tab goes back).
//!
//! Hidden entries are offered only when the typed name starts with a dot.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use crate::fsops;

/// What kind of text the input holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputKind {
    /// A folder path (go to, copy/move destination): only folders are offered.
    Dir,
    /// A path or name relative to the active folder: files and folders.
    Path,
    /// A shell command line: the last word is completed as a path (or, for the
    /// first word, as a program on `PATH`), with shell quoting.
    Command,
}

/// How many candidates are listed at once.
pub const LIST_PAGE: usize = 12;
/// Stop reading a huge folder after this many entries.
const MAX_SCAN: usize = 20_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// The text that replaces the word being completed.
    pub token: String,
    /// How the candidate is shown (`name`, or `name/` for a folder).
    pub label: String,
    pub is_dir: bool,
}

/// The candidates for one input state.
#[derive(Debug, Clone, Default)]
pub struct Completion {
    /// Byte offset in the input where the completed word starts.
    pub start: usize,
    pub items: Vec<Item>,
    /// Text for the word right now: the unique match, or the longest common
    /// prefix. `None` when that would not change anything.
    pub insert: Option<String>,
}

// ---------------------------------------------------------------------------
// Parsing the word being completed
// ---------------------------------------------------------------------------

struct Word {
    start: usize,
    /// The word with quoting and escaping removed.
    literal: String,
    /// A quote that was opened and not closed.
    quote: Option<char>,
    first: bool,
}

fn parse(kind: InputKind, text: &str) -> Word {
    if kind != InputKind::Command {
        return Word {
            start: 0,
            literal: text.to_string(),
            quote: None,
            first: false,
        };
    }
    let mut word = Word {
        start: 0,
        literal: String::new(),
        quote: None,
        first: true,
    };
    let mut escaped = false;
    for (at, ch) in text.char_indices() {
        if escaped {
            word.literal.push(ch);
            escaped = false;
            continue;
        }
        match word.quote {
            Some(q) => {
                if ch == q {
                    word.quote = None;
                } else if q == '"' && ch == '\\' {
                    escaped = true;
                } else {
                    word.literal.push(ch);
                }
            }
            None => {
                if ch == '\\' {
                    escaped = true;
                } else if ch == '\'' || ch == '"' {
                    word.quote = Some(ch);
                } else if ch.is_whitespace() {
                    word.start = at + ch.len_utf8();
                    word.literal.clear();
                    word.first = false;
                } else {
                    word.literal.push(ch);
                }
            }
        }
    }
    word
}

fn is_safe(ch: char) -> bool {
    ch.is_ascii_alphanumeric()
        || matches!(
            ch,
            '_' | '@' | '%' | '+' | '=' | ':' | ',' | '.' | '/' | '-'
        )
}

/// Put `literal` back into command-line form.
fn render(kind: InputKind, literal: &str, quote: Option<char>, close: bool) -> String {
    if kind != InputKind::Command {
        return literal.to_string();
    }
    match quote {
        Some(q) => {
            let mut out = String::new();
            out.push(q);
            for ch in literal.chars() {
                match (q, ch) {
                    ('\'', '\'') => out.push_str("'\\''"),
                    ('"', '"' | '\\' | '$' | '`') => {
                        out.push('\\');
                        out.push(ch);
                    }
                    _ => out.push(ch),
                }
            }
            if close {
                out.push(q);
            }
            out
        }
        None => {
            let mut out = String::new();
            for (i, ch) in literal.chars().enumerate() {
                // A leading `~` must stay unescaped to keep meaning "home".
                let tilde = i == 0 && ch == '~' && (literal == "~" || literal.starts_with("~/"));
                if is_safe(ch) || tilde {
                    out.push(ch);
                } else {
                    out.push('\\');
                    out.push(ch);
                }
            }
            out
        }
    }
}

// ---------------------------------------------------------------------------
// Finding candidates
// ---------------------------------------------------------------------------

struct Found {
    name: String,
    is_dir: bool,
}

fn read_names(base: &Path, dirs_only: bool) -> Vec<Found> {
    let Ok(read) = fs::read_dir(base) else {
        return Vec::new();
    };
    read.flatten()
        .take(MAX_SCAN)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            // Follow links here: a link to a folder should complete like a folder.
            let is_dir = fs::metadata(entry.path())
                .map(|m| m.is_dir())
                .unwrap_or(false);
            (!dirs_only || is_dir).then_some(Found { name, is_dir })
        })
        .collect()
}

fn path_programs(prefix: &str, path_var: &str) -> Vec<Found> {
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for dir in std::env::split_paths(&OsString::from(path_var)) {
        let Ok(read) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in read.flatten().take(MAX_SCAN) {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.starts_with(prefix) || !seen.insert(name.clone()) {
                continue;
            }
            #[cfg(unix)]
            let runnable = {
                use std::os::unix::fs::PermissionsExt;
                fs::metadata(entry.path())
                    .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
                    .unwrap_or(false)
            };
            #[cfg(not(unix))]
            let runnable = fs::metadata(entry.path())
                .map(|m| m.is_file())
                .unwrap_or(false);
            if runnable {
                out.push(Found {
                    name,
                    is_dir: false,
                });
            }
        }
    }
    out
}

fn common_prefix(names: &[&str]) -> String {
    let Some((first, rest)) = names.split_first() else {
        return String::new();
    };
    let mut common: Vec<char> = first.chars().collect();
    for name in rest {
        let keep = common
            .iter()
            .zip(name.chars())
            .take_while(|(a, b)| **a == *b)
            .count();
        common.truncate(keep);
    }
    common.into_iter().collect()
}

/// Candidates for `text`, using `path_var` (a `PATH`-style list) to complete
/// the first word of a command.
pub fn complete_with(kind: InputKind, text: &str, cwd: &Path, path_var: &str) -> Completion {
    let word = parse(kind, text);
    let literal = word.literal.as_str();

    // `~` alone: the only sensible completion is the home folder itself.
    if literal == "~" && fsops::home_dir().is_some() {
        let token = render(kind, "~/", word.quote, false);
        return Completion {
            start: word.start,
            insert: (token != text[word.start..]).then(|| token.clone()),
            items: vec![Item {
                token,
                label: "~/".into(),
                is_dir: true,
            }],
        };
    }

    let program = kind == InputKind::Command
        && word.first
        && !literal.contains('/')
        && !literal.starts_with(['~', '.']);
    let (dir_part, prefix) = match literal.rfind('/') {
        Some(i) if !program => (&literal[..=i], &literal[i + 1..]),
        _ => ("", literal),
    };

    let mut found = if program {
        path_programs(prefix, path_var)
    } else {
        let base = if dir_part.is_empty() {
            cwd.to_path_buf()
        } else if dir_part.starts_with('~') {
            fsops::expand_home(dir_part)
        } else if Path::new(dir_part).is_absolute() {
            PathBuf::from(dir_part)
        } else {
            cwd.join(dir_part)
        };
        let all = read_names(&base, kind == InputKind::Dir);
        let wanted = |name: &str, fold: bool| {
            let hidden_ok = prefix.starts_with('.') || !name.starts_with('.');
            hidden_ok
                && if fold {
                    name.to_lowercase().starts_with(&prefix.to_lowercase())
                } else {
                    name.starts_with(prefix)
                }
        };
        let exact: Vec<Found> = all
            .iter()
            .filter(|f| wanted(&f.name, false))
            .map(|f| Found {
                name: f.name.clone(),
                is_dir: f.is_dir,
            })
            .collect();
        if exact.is_empty() {
            all.into_iter().filter(|f| wanted(&f.name, true)).collect()
        } else {
            exact
        }
    };
    found.sort_by(|a, b| {
        (!a.is_dir, a.name.to_lowercase(), &a.name).cmp(&(
            !b.is_dir,
            b.name.to_lowercase(),
            &b.name,
        ))
    });

    let items: Vec<Item> = found
        .iter()
        .map(|f| {
            let slash = if f.is_dir { "/" } else { "" };
            Item {
                token: render(
                    kind,
                    &format!("{dir_part}{}{slash}", f.name),
                    word.quote,
                    false,
                ),
                label: format!("{}{slash}", f.name),
                is_dir: f.is_dir,
            }
        })
        .collect();

    let typed = &text[word.start..];
    let insert = match found.as_slice() {
        [] => None,
        [only] => {
            let slash = if only.is_dir { "/" } else { "" };
            let lit = format!("{dir_part}{}{slash}", only.name);
            let mut out = render(kind, &lit, word.quote, !only.is_dir);
            // A finished file in a command line: move on to the next word.
            if kind == InputKind::Command && !only.is_dir {
                out.push(' ');
            }
            Some(out)
        }
        many => {
            let names: Vec<&str> = many.iter().map(|f| f.name.as_str()).collect();
            let common = common_prefix(&names);
            Some(render(
                kind,
                &format!("{dir_part}{common}"),
                word.quote,
                false,
            ))
        }
    }
    .filter(|new| new != typed);

    Completion {
        start: word.start,
        items,
        insert,
    }
}

/// [`complete_with`] using the process's `PATH`.
pub fn complete(kind: InputKind, text: &str, cwd: &Path) -> Completion {
    let path = std::env::var("PATH").unwrap_or_default();
    complete_with(kind, text, cwd, &path)
}

// ---------------------------------------------------------------------------
// The Tab key
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Cycle {
    start: usize,
    items: Vec<Item>,
    index: usize,
    /// The input as this cycle last left it; an edit invalidates the cycle.
    text: String,
}

/// What the front end shows after a Tab press.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tab {
    /// The new input text; `None` = leave it unchanged.
    pub text: Option<String>,
    /// Candidates to list (one page of them).
    pub listing: Vec<String>,
    /// Which of `listing` is the one just inserted while cycling.
    pub current: Option<usize>,
    /// Candidates not on this page.
    pub more: usize,
    /// "no match" and the like.
    pub message: Option<String>,
}

/// Per-input Tab state. Keep one next to each input buffer.
#[derive(Debug, Clone, Default)]
pub struct Completer {
    cycle: Option<Cycle>,
}

fn page(items: &[Item], current: Option<usize>) -> (Vec<String>, Option<usize>, usize) {
    let first = current.map_or(0, |c| c / LIST_PAGE * LIST_PAGE);
    let shown: Vec<String> = items
        .iter()
        .skip(first)
        .take(LIST_PAGE)
        .map(|i| i.label.clone())
        .collect();
    let more = items.len().saturating_sub(first + shown.len()) + first;
    (shown, current.map(|c| c - first), more)
}

impl Completer {
    pub fn tab(&mut self, kind: InputKind, text: &str, cwd: &Path, backwards: bool) -> Tab {
        let path = std::env::var("PATH").unwrap_or_default();
        self.tab_with(kind, text, cwd, &path, backwards)
    }

    pub fn tab_with(
        &mut self,
        kind: InputKind,
        text: &str,
        cwd: &Path,
        path_var: &str,
        backwards: bool,
    ) -> Tab {
        // Already cycling and the text is as we left it: next candidate.
        if let Some(cycle) = &mut self.cycle
            && cycle.text == text
            && !cycle.items.is_empty()
        {
            let n = cycle.items.len();
            cycle.index = if backwards {
                (cycle.index + n - 1) % n
            } else {
                (cycle.index + 1) % n
            };
            let new_text = format!("{}{}", &text[..cycle.start], cycle.items[cycle.index].token);
            cycle.text = new_text.clone();
            let (listing, current, more) = page(&cycle.items, Some(cycle.index));
            return Tab {
                text: Some(new_text),
                listing,
                current,
                more,
                message: None,
            };
        }
        self.cycle = None;

        let completion = complete_with(kind, text, cwd, path_var);
        if completion.items.is_empty() {
            return Tab {
                message: Some("no match".into()),
                ..Tab::default()
            };
        }
        if let Some(insert) = &completion.insert {
            let new_text = format!("{}{}", &text[..completion.start], insert);
            let (listing, _, more) = if completion.items.len() > 1 {
                page(&completion.items, None)
            } else {
                (Vec::new(), None, 0)
            };
            return Tab {
                text: Some(new_text),
                listing,
                current: None,
                more,
                message: None,
            };
        }
        if completion.items.len() == 1 {
            return Tab::default(); // already complete
        }
        // Several candidates and nothing common left to add: start cycling.
        let index = if backwards {
            completion.items.len() - 1
        } else {
            0
        };
        let new_text = format!(
            "{}{}",
            &text[..completion.start],
            completion.items[index].token
        );
        let (listing, current, more) = page(&completion.items, Some(index));
        self.cycle = Some(Cycle {
            start: completion.start,
            items: completion.items,
            index,
            text: new_text.clone(),
        });
        Tab {
            text: Some(new_text),
            listing,
            current,
            more,
            message: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for d in ["docs", "downloads", "src", "my dir", ".config"] {
            fs::create_dir(root.join(d)).unwrap();
        }
        for f in [
            "notes.txt",
            "notes.md",
            "readme",
            "my file.txt",
            "Zebra.txt",
            ".hidden",
        ] {
            fs::write(root.join(f), "x").unwrap();
        }
        fs::write(root.join("docs/guide.md"), "x").unwrap();
        fs::write(root.join("docs/gloss.md"), "x").unwrap();
        dir
    }

    /// The input after applying the immediate completion (unchanged if none).
    fn insert(kind: InputKind, text: &str, cwd: &Path) -> Option<String> {
        let c = complete_with(kind, text, cwd, "");
        c.insert.map(|i| format!("{}{}", &text[..c.start], i))
    }

    #[test]
    fn a_unique_folder_gets_a_slash() {
        let dir = tree();
        assert_eq!(
            insert(InputKind::Dir, "sr", dir.path()).as_deref(),
            Some("src/")
        );
        assert_eq!(
            insert(InputKind::Path, "sr", dir.path()).as_deref(),
            Some("src/")
        );
    }

    #[test]
    fn several_matches_extend_to_the_common_prefix_and_list_candidates() {
        let dir = tree();
        let c = complete_with(InputKind::Path, "do", dir.path(), "");
        assert_eq!(
            c.insert, None,
            "docs/downloads share only `do`, which is already typed"
        );
        assert_eq!(
            c.items.iter().map(|i| i.label.as_str()).collect::<Vec<_>>(),
            ["docs/", "downloads/"]
        );
        let c = complete_with(InputKind::Path, "n", dir.path(), "");
        assert_eq!(c.insert.as_deref(), Some("notes."));
        let c = complete_with(InputKind::Path, "docs/g", dir.path(), "");
        assert_eq!(c.insert, None, "guide.md / gloss.md share only `g`");
        assert_eq!(c.items.len(), 2);
        assert_eq!(
            insert(InputKind::Path, "docs/gu", dir.path()).as_deref(),
            Some("docs/guide.md")
        );
    }

    #[test]
    fn dir_inputs_offer_only_folders_and_path_inputs_offer_both() {
        let dir = tree();
        let dirs = complete_with(InputKind::Dir, "", dir.path(), "");
        assert!(dirs.items.iter().all(|i| i.is_dir), "{:?}", dirs.items);
        assert!(!dirs.items.iter().any(|i| i.label == "readme"));
        let all = complete_with(InputKind::Path, "", dir.path(), "");
        assert!(all.items.iter().any(|i| i.label == "readme"));
        assert!(all.items.iter().any(|i| i.label == "src/"));
        assert!(all.items[0].is_dir, "folders first");
    }

    #[test]
    fn hidden_entries_only_when_the_name_starts_with_a_dot() {
        let dir = tree();
        let plain = complete_with(InputKind::Path, "", dir.path(), "");
        assert!(!plain.items.iter().any(|i| i.label.starts_with('.')));
        let dotted = complete_with(InputKind::Path, ".", dir.path(), "");
        let labels: Vec<_> = dotted.items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(labels, [".config/", ".hidden"]);
    }

    #[test]
    fn case_is_respected_and_only_ignored_when_nothing_else_matches() {
        let dir = tree();
        assert_eq!(
            complete_with(InputKind::Path, "zeb", dir.path(), "")
                .insert
                .as_deref(),
            Some("Zebra.txt"),
            "falls back to case-insensitive"
        );
        // exact-case matches win over case-insensitive ones
        assert_eq!(
            complete_with(InputKind::Path, "Z", dir.path(), "")
                .items
                .len(),
            1
        );
        assert!(
            complete_with(InputKind::Path, "nothing-like-this", dir.path(), "")
                .items
                .is_empty()
        );
    }

    #[test]
    fn absolute_and_relative_prefixes_resolve_against_the_right_folder() {
        let dir = tree();
        let abs = format!("{}/do", dir.path().display());
        assert_eq!(
            complete_with(InputKind::Dir, &abs, Path::new("/"), "")
                .items
                .len(),
            2
        );

        // relative paths are relative to the folder given, `..` included
        let other = tempfile::tempdir().unwrap();
        fs::create_dir(other.path().join("sibling")).unwrap();
        fs::create_dir(other.path().join("work")).unwrap();
        let cwd = other.path().join("work");
        assert_eq!(
            insert(InputKind::Dir, "../sib", &cwd).as_deref(),
            Some("../sibling/")
        );
        assert_eq!(
            insert(InputKind::Dir, "./", &cwd),
            None,
            "empty work folder: nothing to add"
        );

        let nested = format!("{}/docs/", dir.path().display());
        assert_eq!(
            complete_with(InputKind::Path, &nested, other.path(), "")
                .items
                .len(),
            2
        );
    }

    #[test]
    fn command_lines_escape_spaces_and_complete_the_last_word() {
        let dir = tree();
        // `cp my f` -> the word is `my f` once escaped
        let c = complete_with(InputKind::Command, "cp my\\ f", dir.path(), "");
        assert_eq!(c.start, 3);
        assert_eq!(
            c.insert.as_deref(),
            Some("my\\ file.txt "),
            "escaped, and a space for the next word"
        );
        // folder: slash, no trailing space
        let c = complete_with(InputKind::Command, "ls my\\ d", dir.path(), "");
        assert_eq!(c.insert.as_deref(), Some("my\\ dir/"));
        // listing shows the real names
        let c = complete_with(InputKind::Command, "ls my", dir.path(), "");
        assert_eq!(
            c.insert.as_deref(),
            Some("my\\ "),
            "common prefix `my ` is escaped"
        );
        assert_eq!(
            c.items.iter().map(|i| i.label.as_str()).collect::<Vec<_>>(),
            ["my dir/", "my file.txt"]
        );
        assert_eq!(c.items[1].token, "my\\ file.txt");
    }

    #[test]
    fn quotes_are_continued_and_closed() {
        let dir = tree();
        let c = complete_with(InputKind::Command, "cat 'my f", dir.path(), "");
        assert_eq!(c.insert.as_deref(), Some("'my file.txt' "));
        let c = complete_with(InputKind::Command, "cat \"my d", dir.path(), "");
        assert_eq!(
            c.insert.as_deref(),
            Some("\"my dir/"),
            "a folder keeps the quote open for the next level"
        );
    }

    #[test]
    fn the_first_word_completes_programs_from_path() {
        let dir = tree();
        let bin = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for (name, mode) in [
                ("frobnicate", 0o755),
                ("frobulate", 0o755),
                ("frob-data", 0o644),
            ] {
                let p = bin.path().join(name);
                fs::write(&p, "#!/bin/sh\n").unwrap();
                fs::set_permissions(&p, fs::Permissions::from_mode(mode)).unwrap();
            }
            let path = bin.path().display().to_string();
            let c = complete_with(InputKind::Command, "frob", dir.path(), &path);
            assert_eq!(
                c.items.iter().map(|i| i.label.as_str()).collect::<Vec<_>>(),
                ["frobnicate", "frobulate"],
                "not the non-executable file"
            );
            assert_eq!(
                c.insert, None,
                "the shared prefix is just `frob`, already typed"
            );
            assert_eq!(
                complete_with(InputKind::Command, "frobn", dir.path(), &path)
                    .insert
                    .as_deref(),
                Some("frobnicate ")
            );
            // `./x` and later words are paths, not programs
            assert!(
                complete_with(InputKind::Command, "./fro", dir.path(), &path)
                    .items
                    .is_empty()
            );
            assert!(
                complete_with(InputKind::Command, "echo fro", dir.path(), &path)
                    .items
                    .is_empty()
            );
        }
    }

    #[test]
    fn tab_fills_in_extends_lists_then_cycles_and_goes_back() {
        let dir = tree();
        let mut completer = Completer::default();
        let none = "";

        // unique
        let t = completer.tab_with(InputKind::Path, "sr", dir.path(), none, false);
        assert_eq!(t.text.as_deref(), Some("src/"));
        assert!(t.listing.is_empty());

        // ambiguous, can extend: `n` -> `notes.`, candidates listed, nothing cycling yet
        let t = completer.tab_with(InputKind::Path, "n", dir.path(), none, false);
        assert_eq!(t.text.as_deref(), Some("notes."));
        assert_eq!(t.listing, ["notes.md", "notes.txt"]);
        assert_eq!(t.current, None);

        // nothing left to extend: Tab starts cycling at the first candidate …
        let t = completer.tab_with(InputKind::Path, "notes.", dir.path(), none, false);
        assert_eq!(t.text.as_deref(), Some("notes.md"));
        assert_eq!(t.current, Some(0));
        // … and the next ones, wrapping
        let t = completer.tab_with(InputKind::Path, "notes.md", dir.path(), none, false);
        assert_eq!(t.text.as_deref(), Some("notes.txt"));
        assert_eq!(t.current, Some(1));
        let t = completer.tab_with(InputKind::Path, "notes.txt", dir.path(), none, false);
        assert_eq!(t.text.as_deref(), Some("notes.md"), "wraps");
        // Shift+Tab goes back
        let t = completer.tab_with(InputKind::Path, "notes.md", dir.path(), none, true);
        assert_eq!(t.text.as_deref(), Some("notes.txt"));
    }

    #[test]
    fn editing_the_text_ends_a_cycle() {
        let dir = tree();
        let mut completer = Completer::default();
        let t = completer.tab_with(InputKind::Path, "notes.", dir.path(), "", false);
        assert_eq!(t.text.as_deref(), Some("notes.md"));
        // the user types something else: a fresh completion, not the next candidate
        let t = completer.tab_with(InputKind::Path, "notes.t", dir.path(), "", false);
        assert_eq!(t.text.as_deref(), Some("notes.txt"));
    }

    #[test]
    fn no_match_and_already_complete_leave_the_text_alone() {
        let dir = tree();
        let mut completer = Completer::default();
        let t = completer.tab_with(InputKind::Path, "zzz", dir.path(), "", false);
        assert_eq!((t.text, t.message.as_deref()), (None, Some("no match")));
        let t = completer.tab_with(InputKind::Path, "readme", dir.path(), "", false);
        assert_eq!(
            (t.text, t.message),
            (None, None),
            "exactly one match and it is typed out"
        );
    }

    #[test]
    fn long_candidate_lists_are_paged() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..30 {
            fs::write(dir.path().join(format!("f{i:02}")), "x").unwrap();
        }
        let mut completer = Completer::default();
        // Nothing common to add after `f`, so the first Tab already cycles.
        let t = completer.tab_with(InputKind::Path, "f", dir.path(), "", false);
        assert_eq!(t.text.as_deref(), Some("f00"));
        assert_eq!(
            (t.listing.len(), t.more, t.current),
            (LIST_PAGE, 30 - LIST_PAGE, Some(0))
        );
        // Cycling past the first page shows the next page with the cursor on it.
        let mut text = "f00".to_string();
        for _ in 0..LIST_PAGE {
            text = completer
                .tab_with(InputKind::Path, &text, dir.path(), "", false)
                .text
                .unwrap();
        }
        assert_eq!(text, "f12");
        let t = completer.tab_with(InputKind::Path, &text, dir.path(), "", false);
        assert_eq!(t.text.as_deref(), Some("f13"));
        assert_eq!(
            t.listing.first().map(String::as_str),
            Some("f12"),
            "second page"
        );
        assert_eq!(t.current, Some(1));
    }

    #[test]
    fn tilde_expands_to_the_home_folder() {
        if fsops::home_dir().is_some() {
            let c = complete_with(InputKind::Dir, "~", Path::new("/"), "");
            assert_eq!(c.insert.as_deref(), Some("~/"));
        }
    }
}
