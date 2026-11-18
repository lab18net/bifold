//! Filesystem operations: listing, copy, move, delete (trash first), mkdir,
//! rename, plus the bookkeeping a transfer needs (totals, progress, cancel).
//!
//! All long-running work goes through [`OpCtx`], which both checks the cancel
//! flag and reports byte/file progress. Nothing here touches a UI type.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use crate::entry::Entry;
use crate::error::{Error, Result};
use crate::job::OpCtx;

const BUF_SIZE: usize = 128 * 1024;

/// Environment override for the trash directory (used by tests and by users who
/// want deletions to land somewhere else).
pub const TRASH_ENV: &str = "BIFOLD_TRASH_DIR";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteMode {
    /// Move into the user's trash directory — reversible.
    Trash,
    /// Unlink immediately — not reversible.
    Permanent,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct OpStats {
    pub files: u64,
    pub dirs: u64,
    pub bytes: u64,
    pub skipped: u64,
}

impl OpStats {
    pub fn add(&mut self, other: Self) {
        self.files += other.files;
        self.dirs += other.dirs;
        self.bytes += other.bytes;
        self.skipped += other.skipped;
    }

    pub fn summary(&self) -> String {
        format!(
            "{} file(s), {} dir(s), {}",
            self.files,
            self.dirs,
            crate::format::human_size(self.bytes)
        )
    }
}

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}

pub fn trash_dir() -> Option<PathBuf> {
    if let Some(custom) = std::env::var_os(TRASH_ENV)
        && !custom.is_empty()
    {
        return Some(PathBuf::from(custom));
    }
    home_dir().map(|home| home.join(".Trash"))
}

/// Expand a leading `~` without touching the rest of the string.
pub fn expand_home(raw: &str) -> PathBuf {
    let raw = raw.trim();
    if raw == "~" {
        return home_dir().unwrap_or_else(|| PathBuf::from("/"));
    }
    if let Some(rest) = raw.strip_prefix("~/")
        && let Some(home) = home_dir()
    {
        return home.join(rest);
    }
    PathBuf::from(raw)
}

/// Canonicalize and require a directory.
pub fn canonical_dir(path: &Path) -> Result<PathBuf> {
    let canonical = fs::canonicalize(path).map_err(|e| match e.kind() {
        io::ErrorKind::NotFound => Error::NotFound(path.to_path_buf()),
        _ => Error::io(path, e),
    })?;
    if !canonical.is_dir() {
        return Err(Error::invalid(format!(
            "not a directory: {}",
            canonical.display()
        )));
    }
    Ok(canonical)
}

/// Resolve user input (with `~/` support) into a real directory path.
pub fn resolve_dir(raw: &str) -> Result<PathBuf> {
    let expanded = expand_home(raw);
    let target = if expanded.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        expanded
    };
    canonical_dir(&target)
}

/// `name`, or `name 2.ext`, `name 3.ext`, … — first free sibling.
pub fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() && !is_symlink(&candidate) {
        return candidate;
    }
    let as_path = Path::new(name);
    // Keep compound extensions together: "a.tar.gz" -> "a 2.tar.gz".
    let (stem, ext) = match as_path
        .to_string_lossy()
        .find('.')
        .filter(|index| *index > 0)
    {
        Some(index) => {
            let text = as_path.to_string_lossy();
            (
                text[..index].to_string(),
                Some(text[index + 1..].to_string()),
            )
        }
        None => (name.to_string(), None),
    };
    for n in 2..10_000u32 {
        let file_name = match &ext {
            Some(ext) => format!("{stem} {n}.{ext}"),
            None => format!("{stem} {n}"),
        };
        let candidate = dir.join(file_name);
        if !candidate.exists() && !is_symlink(&candidate) {
            return candidate;
        }
    }
    candidate
}

pub fn is_symlink(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Listing
// ---------------------------------------------------------------------------

pub fn list_dir(path: &Path, show_hidden: bool) -> Result<Vec<Entry>> {
    let read = fs::read_dir(path).map_err(|e| Error::io(path, e))?;
    let mut entries = Vec::new();
    for item in read.flatten() {
        if let Some(entry) = Entry::from_dir_entry(&item) {
            if !show_hidden && entry.hidden() {
                continue;
            }
            entries.push(entry);
        }
    }
    Ok(entries)
}

// ---------------------------------------------------------------------------
// Measurement
// ---------------------------------------------------------------------------

/// Walk a tree without following symlinks, counting files and bytes.
pub fn measure(path: &Path) -> Result<OpStats> {
    let mut stats = OpStats::default();
    measure_inner(path, &mut stats)?;
    Ok(stats)
}

pub fn measure_many(paths: &[PathBuf]) -> Result<OpStats> {
    let mut stats = OpStats::default();
    for path in paths {
        stats.add(measure(path)?);
    }
    Ok(stats)
}

fn measure_inner(path: &Path, stats: &mut OpStats) -> Result<()> {
    let meta = fs::symlink_metadata(path).map_err(|e| Error::io(path, e))?;
    if meta.file_type().is_symlink() {
        stats.files += 1;
        return Ok(());
    }
    if meta.is_dir() {
        stats.dirs += 1;
        let read = fs::read_dir(path).map_err(|e| Error::io(path, e))?;
        for item in read.flatten() {
            measure_inner(&item.path(), stats)?;
        }
    } else if meta.is_file() {
        stats.files += 1;
        stats.bytes += meta.len();
    } else {
        stats.skipped += 1;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Copy
// ---------------------------------------------------------------------------

/// Copy one entry into `dest_dir`, keeping its name and avoiding collisions.
pub fn copy_entry(src: &Path, dest_dir: &Path, ctx: &OpCtx) -> Result<OpStats> {
    let name = src
        .file_name()
        .ok_or_else(|| Error::invalid("cannot copy a filesystem root"))?
        .to_string_lossy()
        .into_owned();
    let target = unique_path(dest_dir, &name);
    copy_tree(src, &target, ctx)
}

pub fn copy_tree(src: &Path, dst: &Path, ctx: &OpCtx) -> Result<OpStats> {
    let mut stats = OpStats::default();
    copy_tree_inner(src, dst, ctx, &mut stats)?;
    Ok(stats)
}

fn copy_tree_inner(src: &Path, dst: &Path, ctx: &OpCtx, stats: &mut OpStats) -> Result<()> {
    ctx.check()?;
    let meta = fs::symlink_metadata(src).map_err(|e| Error::io(src, e))?;

    if meta.file_type().is_symlink() {
        let link_target = fs::read_link(src).map_err(|e| Error::io(src, e))?;
        create_symlink(&link_target, dst)?;
        stats.files += 1;
        ctx.add_file();
        return Ok(());
    }

    if meta.is_dir() {
        ctx.set_current(src);
        fs::create_dir_all(dst).map_err(|e| Error::io(dst, e))?;
        stats.dirs += 1;

        let mut children: Vec<PathBuf> = fs::read_dir(src)
            .map_err(|e| Error::io(src, e))?
            .flatten()
            .map(|d| d.path())
            .collect();
        children.sort();
        for child in children {
            let name = child
                .file_name()
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("unnamed"));
            copy_tree_inner(&child, &dst.join(name), ctx, stats)?;
        }
        copy_metadata(src, &meta, dst);
        return Ok(());
    }

    if meta.is_file() {
        let bytes = copy_file(src, dst, ctx)?;
        stats.files += 1;
        stats.bytes += bytes;
        ctx.add_file();
        return Ok(());
    }

    stats.skipped += 1;
    Ok(())
}

fn copy_file(src: &Path, dst: &Path, ctx: &OpCtx) -> Result<u64> {
    let meta = fs::metadata(src).map_err(|e| Error::io(src, e))?;
    let mut reader = File::open(src).map_err(|e| Error::io(src, e))?;
    let mut writer = File::create(dst).map_err(|e| Error::io(dst, e))?;
    let mut buf = vec![0u8; BUF_SIZE];
    let mut total: u64 = 0;

    loop {
        ctx.check()?;
        let read = reader.read(&mut buf).map_err(|e| Error::io(src, e))?;
        if read == 0 {
            break;
        }
        writer
            .write_all(&buf[..read])
            .map_err(|e| Error::io(dst, e))?;
        total += read as u64;
        ctx.add_bytes(read as u64);
    }
    writer.flush().map_err(|e| Error::io(dst, e))?;
    copy_metadata(src, &meta, dst);
    Ok(total)
}

#[cfg(unix)]
fn create_symlink(target: &Path, link: &Path) -> Result<()> {
    let _ = fs::remove_file(link);
    std::os::unix::fs::symlink(target, link).map_err(|e| Error::io(link, e))
}

#[cfg(not(unix))]
fn create_symlink(_target: &Path, _link: &Path) -> Result<()> {
    Err(Error::invalid(
        "symlinks are not supported on this platform",
    ))
}

/// Best-effort permission and mtime preservation; a failure here must not fail
/// the transfer, only lose metadata.
fn copy_metadata(src: &Path, meta: &fs::Metadata, dst: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(dst, fs::Permissions::from_mode(meta.permissions().mode()));
    }
    let _ = src;
    if let Ok(modified) = meta.modified()
        && let Ok(file) = OpenOptions::new().write(true).open(dst)
    {
        let times = fs::FileTimes::new().set_modified(modified);
        let _ = file.set_times(times);
    }
}

// ---------------------------------------------------------------------------
// Move / delete
// ---------------------------------------------------------------------------

/// Move one entry into `dest_dir`: one `rename` when possible, copy+remove
/// across devices.
pub fn move_entry(src: &Path, dest_dir: &Path, ctx: &OpCtx) -> Result<OpStats> {
    ctx.check()?;
    let name = src
        .file_name()
        .ok_or_else(|| Error::invalid("cannot move a filesystem root"))?
        .to_string_lossy()
        .into_owned();
    let target = unique_path(dest_dir, &name);
    let stats = measure(src)?;

    match fs::rename(src, &target) {
        Ok(()) => {
            ctx.add_bytes(stats.bytes);
            for _ in 0..stats.files {
                ctx.add_file();
            }
            Ok(stats)
        }
        Err(_) => {
            let copied = copy_tree(src, &target, ctx)?;
            delete_entry(src, DeleteMode::Permanent, None, ctx)?;
            Ok(copied)
        }
    }
}

/// Delete (or trash) one entry. `trash_root` overrides the trash directory;
/// `None` means "resolve from `BIFOLD_TRASH_DIR` / `~/.Trash`".
pub fn delete_entry(
    path: &Path,
    mode: DeleteMode,
    trash_root: Option<&Path>,
    ctx: &OpCtx,
) -> Result<OpStats> {
    ctx.check()?;
    let stats = measure(path)?;
    match mode {
        DeleteMode::Permanent => remove_tree(path, ctx)?,
        DeleteMode::Trash => {
            let root = match trash_root {
                Some(root) => root.to_path_buf(),
                None => trash_dir().ok_or_else(|| {
                    Error::invalid("no trash directory (set HOME or BIFOLD_TRASH_DIR)")
                })?,
            };
            fs::create_dir_all(&root).map_err(|e| Error::io(&root, e))?;
            let name = path
                .file_name()
                .ok_or_else(|| Error::invalid("cannot trash a filesystem root"))?
                .to_string_lossy()
                .into_owned();
            let target = unique_path(&root, &name);
            match fs::rename(path, &target) {
                Ok(()) => {}
                Err(_) => {
                    copy_tree(path, &target, ctx)?;
                    remove_tree(path, ctx)?;
                }
            }
        }
    }
    Ok(stats)
}

fn remove_tree(path: &Path, ctx: &OpCtx) -> Result<()> {
    ctx.check()?;
    let meta = fs::symlink_metadata(path).map_err(|e| Error::io(path, e))?;
    if meta.is_dir() && !meta.file_type().is_symlink() {
        let read = fs::read_dir(path).map_err(|e| Error::io(path, e))?;
        for item in read.flatten() {
            remove_tree(&item.path(), ctx)?;
        }
        fs::remove_dir(path).map_err(|e| Error::io(path, e))?;
    } else {
        fs::remove_file(path).map_err(|e| Error::io(path, e))?;
        ctx.add_file();
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Create / rename
// ---------------------------------------------------------------------------

fn validate_name(name: &str) -> Result<&str> {
    let name = name.trim();
    if name.is_empty() {
        return Err(Error::invalid("name cannot be empty"));
    }
    if name.contains('/') {
        return Err(Error::invalid("name cannot contain '/'"));
    }
    if name == "." || name == ".." {
        return Err(Error::invalid("reserved name"));
    }
    Ok(name)
}

pub fn create_dir(parent: &Path, name: &str) -> Result<PathBuf> {
    let name = validate_name(name)?;
    let path = parent.join(name);
    fs::create_dir(&path).map_err(|e| Error::io(&path, e))?;
    Ok(path)
}

pub fn create_file(parent: &Path, name: &str) -> Result<PathBuf> {
    let name = validate_name(name)?;
    let path = parent.join(name);
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| Error::io(&path, e))?;
    Ok(path)
}

/// Rename `old` to `new_name` (a bare name) or to an absolute path.
pub fn rename(old: &Path, target: &str) -> Result<PathBuf> {
    let target = target.trim();
    if target.is_empty() {
        return Err(Error::invalid("new name cannot be empty"));
    }
    let as_path = PathBuf::from(target);
    let destination = if as_path.is_absolute() {
        as_path
    } else {
        let parent = old
            .parent()
            .ok_or_else(|| Error::invalid("cannot rename a filesystem root"))?;
        parent.join(validate_name(target)?)
    };
    if destination.exists() && destination != old {
        return Err(Error::invalid(format!(
            "already exists: {}",
            destination.display()
        )));
    }
    fs::rename(old, &destination).map_err(|e| Error::io(old, e))?;
    Ok(destination)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::job::{JobState, OpCtx};

    fn ctx() -> (OpCtx, Arc<AtomicBool>) {
        let state = Arc::new(Mutex::new(JobState::default()));
        let cancel = Arc::new(AtomicBool::new(false));
        (OpCtx::new(state, cancel.clone()), cancel)
    }

    fn touch(path: &Path, bytes: usize) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, vec![b'x'; bytes]).unwrap();
    }

    #[test]
    fn list_skips_hidden_unless_asked() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("a.txt"), 1);
        touch(&dir.path().join(".hidden"), 1);
        fs::create_dir(dir.path().join("sub")).unwrap();

        let visible = list_dir(dir.path(), false).unwrap();
        assert_eq!(visible.len(), 2);
        let all = list_dir(dir.path(), true).unwrap();
        assert_eq!(all.len(), 3);
        assert!(all.iter().any(|e| e.name == "sub" && e.is_dir()));
    }

    #[test]
    fn copy_tree_preserves_layout_and_counts_bytes() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        let tree = src.path().join("project");
        touch(&tree.join("readme.md"), 10);
        touch(&tree.join("nested/deep/file.bin"), 2048);

        let (op, _cancel) = ctx();
        let stats = copy_tree(&tree, &dst.path().join("project"), &op).unwrap();
        assert_eq!(stats.files, 2);
        assert!(stats.dirs >= 2, "directories counted");
        assert_eq!(stats.bytes, 2058);
        assert_eq!(
            fs::read(dst.path().join("project/nested/deep/file.bin"))
                .unwrap()
                .len(),
            2048
        );
    }

    #[test]
    fn copy_entry_avoids_name_collisions() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        touch(&src.path().join("report.pdf"), 4);
        touch(&dst.path().join("report.pdf"), 4);

        let (op, _cancel) = ctx();
        copy_entry(&src.path().join("report.pdf"), dst.path(), &op).unwrap();
        assert!(dst.path().join("report 2.pdf").exists());
    }

    #[test]
    fn move_entry_moves_directory() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        let tree = src.path().join("box");
        touch(&tree.join("inner/f.txt"), 8);

        let (op, _cancel) = ctx();
        move_entry(&tree, dst.path(), &op).unwrap();
        assert!(!tree.exists());
        assert!(dst.path().join("box/inner/f.txt").exists());
    }

    #[test]
    fn delete_to_trash_is_reversible() {
        let src = tempfile::tempdir().unwrap();
        let trash = tempfile::tempdir().unwrap();
        touch(&src.path().join("old.txt"), 3);

        let (op, _cancel) = ctx();
        delete_entry(
            &src.path().join("old.txt"),
            DeleteMode::Trash,
            Some(trash.path()),
            &op,
        )
        .unwrap();
        assert!(!src.path().join("old.txt").exists());
        assert!(trash.path().join("old.txt").exists());

        touch(&src.path().join("old.txt"), 5);
        delete_entry(
            &src.path().join("old.txt"),
            DeleteMode::Trash,
            Some(trash.path()),
            &op,
        )
        .unwrap();
        assert!(trash.path().join("old 2.txt").exists());
    }

    #[test]
    fn permanent_delete_removes_tree() {
        let src = tempfile::tempdir().unwrap();
        let tree = src.path().join("doomed");
        touch(&tree.join("a/b/c.txt"), 1);
        let (op, _cancel) = ctx();
        delete_entry(&tree, DeleteMode::Permanent, None, &op).unwrap();
        assert!(!tree.exists());
    }

    #[test]
    fn cancel_stops_a_copy() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        touch(&src.path().join("big.bin"), 4 * BUF_SIZE);
        let state = Arc::new(Mutex::new(JobState::default()));
        let cancel = Arc::new(AtomicBool::new(true));
        let op = OpCtx::new(state, cancel);

        let err = copy_tree(
            &src.path().join("big.bin"),
            &dst.path().join("big.bin"),
            &op,
        )
        .unwrap_err();
        assert!(err.is_cancelled());
    }

    #[test]
    fn create_and_rename_guard_bad_names() {
        let dir = tempfile::tempdir().unwrap();
        assert!(create_dir(dir.path(), "").is_err());
        assert!(create_dir(dir.path(), "a/b").is_err());
        let made = create_dir(dir.path(), "new dir").unwrap();
        assert!(made.is_dir());
        let file = create_file(dir.path(), "notes.txt").unwrap();
        assert!(file.is_file());
        assert!(create_file(dir.path(), "notes.txt").is_err());

        let renamed = rename(&file, "renamed.txt").unwrap();
        assert!(renamed.exists());
        assert!(!file.exists());
        assert!(
            rename(&renamed, "new dir").is_err(),
            "existing target rejected"
        );
    }

    #[test]
    fn unique_path_keeps_extension() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("a.tar.gz"), 1);
        let next = unique_path(dir.path(), "a.tar.gz");
        assert_eq!(next.file_name().unwrap(), "a 2.tar.gz");
    }

    #[test]
    fn home_expansion() {
        let home = home_dir().unwrap();
        assert_eq!(expand_home("~"), home);
        assert_eq!(expand_home("~/docs"), home.join("docs"));
        assert_eq!(expand_home("relative/dir"), PathBuf::from("relative/dir"));
    }
}
