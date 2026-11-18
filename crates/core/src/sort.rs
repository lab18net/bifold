use std::cmp::Ordering;

use crate::entry::{Entry, EntryKind};

/// Column the listing is ordered by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Name,
    Size,
    Modified,
    Kind,
}

impl SortKey {
    pub fn label(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Size => "size",
            Self::Modified => "time",
            Self::Kind => "kind",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Name => Self::Size,
            Self::Size => Self::Modified,
            Self::Modified => Self::Kind,
            Self::Kind => Self::Name,
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "name" => Some(Self::Name),
            "size" => Some(Self::Size),
            "time" | "modified" | "mtime" => Some(Self::Modified),
            "kind" | "type" => Some(Self::Kind),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDir {
    Asc,
    Desc,
}

impl SortDir {
    pub fn toggled(self) -> Self {
        match self {
            Self::Asc => Self::Desc,
            Self::Desc => Self::Asc,
        }
    }

    pub fn arrow(self) -> &'static str {
        match self {
            Self::Asc => "↑",
            Self::Desc => "↓",
        }
    }
}

/// Sort in place. Directories first is handled outside the key ordering so that
/// "descending by size" still keeps folders on top, like a commander should.
pub fn sort_entries(entries: &mut [Entry], key: SortKey, dir: SortDir, dirs_first: bool) {
    entries.sort_by(|a, b| {
        if dirs_first && a.is_dir() != b.is_dir() {
            return if a.is_dir() {
                Ordering::Less
            } else {
                Ordering::Greater
            };
        }
        let ord = compare(a, b, key);
        match dir {
            SortDir::Asc => ord,
            SortDir::Desc => ord.reverse(),
        }
    });
}

fn compare(a: &Entry, b: &Entry, key: SortKey) -> Ordering {
    let primary = match key {
        SortKey::Name => Ordering::Equal,
        SortKey::Size => a.size.cmp(&b.size),
        SortKey::Modified => a.modified.cmp(&b.modified),
        SortKey::Kind => rank(a.kind).cmp(&rank(b.kind)),
    };
    primary
        .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        .then_with(|| a.name.cmp(&b.name))
}

fn rank(kind: EntryKind) -> u8 {
    match kind {
        EntryKind::Dir => 0,
        EntryKind::File => 1,
        EntryKind::Symlink => 2,
        EntryKind::Other => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn entry(name: &str, kind: EntryKind, size: u64) -> Entry {
        Entry {
            name: name.to_string(),
            path: PathBuf::from("/t").join(name),
            kind,
            size,
            modified: None,
            mode: None,
            symlink: false,
        }
    }

    #[test]
    fn dirs_stay_first_while_descending() {
        let mut v = vec![
            entry("b.txt", EntryKind::File, 10),
            entry("a", EntryKind::Dir, 0),
            entry("a.txt", EntryKind::File, 400),
        ];
        sort_entries(&mut v, SortKey::Size, SortDir::Desc, true);
        assert_eq!(v[0].name, "a");
        assert_eq!(v[1].name, "a.txt");
        assert_eq!(v[2].name, "b.txt");
    }

    #[test]
    fn name_sort_is_case_insensitive() {
        let mut v = vec![
            entry("Zed", EntryKind::File, 0),
            entry("apple", EntryKind::File, 0),
            entry("Banana", EntryKind::File, 0),
        ];
        sort_entries(&mut v, SortKey::Name, SortDir::Asc, true);
        let names: Vec<_> = v.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["apple", "Banana", "Zed"]);
    }

    #[test]
    fn sort_key_roundtrip() {
        for key in [
            SortKey::Name,
            SortKey::Size,
            SortKey::Modified,
            SortKey::Kind,
        ] {
            assert_eq!(SortKey::parse(key.label()), Some(key));
            assert_eq!(key.next().next().next().next(), key);
        }
        assert_eq!(SortKey::parse("nope"), None);
    }
}
