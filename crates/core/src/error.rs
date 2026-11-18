use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

/// Errors surfaced by core operations. Every variant carries enough context to
/// be shown directly in a status line or dialog.
#[derive(Debug)]
pub enum Error {
    Io {
        path: Option<PathBuf>,
        source: io::Error,
    },
    NotFound(PathBuf),
    Cancelled,
    Invalid(String),
}

impl Error {
    pub fn io(path: &Path, source: io::Error) -> Self {
        Self::Io {
            path: Some(path.to_path_buf()),
            source,
        }
    }

    pub fn invalid(msg: impl Into<String>) -> Self {
        Self::Invalid(msg.into())
    }

    pub fn is_cancelled(&self) -> bool {
        matches!(self, Self::Cancelled)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io {
                path: Some(p),
                source,
            } => write!(f, "{}: {}", p.display(), source),
            Self::Io { path: None, source } => write!(f, "{source}"),
            Self::NotFound(p) => write!(f, "not found: {}", p.display()),
            Self::Cancelled => write!(f, "cancelled"),
            Self::Invalid(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(source: io::Error) -> Self {
        Self::Io { path: None, source }
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
