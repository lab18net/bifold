use std::fs::{DirEntry, Metadata};
use std::path::PathBuf;
use std::time::SystemTime;

/// What a directory entry *is*, after following symlinks once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EntryKind {
    Dir,
    File,
    Symlink,
    Other,
}

impl EntryKind {
    pub fn is_dir(self) -> bool {
        matches!(self, Self::Dir)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Dir => "dir",
            Self::File => "file",
            Self::Symlink => "link",
            Self::Other => "other",
        }
    }
}

/// A single row in a pane.
#[derive(Debug, Clone)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub kind: EntryKind,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub mode: Option<u32>,
    /// True when the on-disk object is a symlink; `kind` then describes the target.
    pub symlink: bool,
}

impl Entry {
    pub fn from_dir_entry(de: &DirEntry) -> Option<Self> {
        let path = de.path();
        let name = de.file_name().to_string_lossy().into_owned();
        let link_meta = de.metadata().ok()?;
        let symlink = link_meta.file_type().is_symlink();

        let (kind, size, modified, mode) = if symlink {
            match std::fs::metadata(&path) {
                Ok(target) => meta_parts(&target),
                Err(_) => (
                    EntryKind::Symlink,
                    link_meta.len(),
                    link_meta.modified().ok(),
                    mode_of(&link_meta),
                ),
            }
        } else {
            meta_parts(&link_meta)
        };

        Some(Self {
            name,
            path,
            kind,
            size,
            modified,
            mode,
            symlink,
        })
    }

    /// Build an entry for a known path (used for parent rows and tests).
    pub fn from_path(path: &std::path::Path) -> Option<Self> {
        let meta = std::fs::symlink_metadata(path).ok()?;
        let symlink = meta.file_type().is_symlink();
        let (kind, size, modified, mode) = if symlink {
            match std::fs::metadata(path) {
                Ok(target) => meta_parts(&target),
                Err(_) => (
                    EntryKind::Symlink,
                    meta.len(),
                    meta.modified().ok(),
                    mode_of(&meta),
                ),
            }
        } else {
            meta_parts(&meta)
        };
        Some(Self {
            name: path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string()),
            path: path.to_path_buf(),
            kind,
            size,
            modified,
            mode,
            symlink,
        })
    }

    pub fn is_dir(&self) -> bool {
        self.kind == EntryKind::Dir
    }

    pub fn hidden(&self) -> bool {
        self.name.starts_with('.')
    }

    /// Human readable permission bits, e.g. `drwxr-xr-x`.
    pub fn mode_string(&self) -> String {
        let Some(mode) = self.mode else {
            return String::new();
        };
        let mut out = String::with_capacity(10);
        out.push(match self.kind {
            EntryKind::Dir => 'd',
            EntryKind::Symlink => 'l',
            EntryKind::File => '-',
            EntryKind::Other => '?',
        });
        for shift in [6u32, 3, 0] {
            out.push(if mode >> (shift + 2) & 1 == 1 {
                'r'
            } else {
                '-'
            });
            out.push(if mode >> (shift + 1) & 1 == 1 {
                'w'
            } else {
                '-'
            });
            out.push(if mode >> shift & 1 == 1 { 'x' } else { '-' });
        }
        out
    }
}

fn meta_parts(meta: &Metadata) -> (EntryKind, u64, Option<SystemTime>, Option<u32>) {
    let kind = if meta.is_dir() {
        EntryKind::Dir
    } else if meta.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    };
    (kind, meta.len(), meta.modified().ok(), mode_of(meta))
}

fn mode_of(meta: &Metadata) -> Option<u32> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        Some(meta.permissions().mode())
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_string_shape() {
        let mut e = Entry {
            name: "x".into(),
            path: PathBuf::from("/x"),
            kind: EntryKind::Dir,
            size: 0,
            modified: None,
            mode: Some(0o40755),
            symlink: false,
        };
        assert_eq!(e.mode_string(), "drwxr-xr-x");
        e.kind = EntryKind::File;
        e.mode = Some(0o100644);
        assert_eq!(e.mode_string(), "-rw-r--r--");
        e.mode = None;
        assert_eq!(e.mode_string(), "");
    }

    #[test]
    fn hidden_names() {
        let e = Entry {
            name: ".gitignore".into(),
            path: PathBuf::from("/x/.gitignore"),
            kind: EntryKind::File,
            size: 1,
            modified: None,
            mode: None,
            symlink: false,
        };
        assert!(e.hidden());
        assert!(!e.is_dir());
    }
}
