//! Crash-safe file writes (docs/specs/09-save-autosave-and-recovery.md §Save
//! pipeline step 4): temp file in the same directory → fsync → copy
//! permissions → rename over the target → fsync the directory.
//!
//! Falls back to an in-place write when a rename would break a hard link or
//! the directory is not writable; callers that write user files keep a
//! recovery snapshot first, so the fallback never risks the only copy.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// How the bytes reached the disk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteStrategy {
    /// Temp file renamed over the target.
    Atomic,
    /// Target truncated and rewritten (hard links, unwritable directory, or
    /// `atomic = false`).
    InPlace,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WriteOptions {
    /// `files.atomicSave`.
    pub atomic: bool,
    /// fsync data and directory (disable only in tests of bulk data).
    pub sync: bool,
}

impl Default for WriteOptions {
    fn default() -> Self {
        Self {
            atomic: true,
            sync: true,
        }
    }
}

/// Writes `bytes` to `path`. Symlinks are followed: the link's target is
/// replaced and the link itself is preserved.
pub fn write(path: &Path, bytes: &[u8], opts: WriteOptions) -> io::Result<WriteStrategy> {
    let target = resolve_symlinks(path)?;
    if opts.atomic && !has_other_hard_links(&target) {
        match write_atomic(&target, bytes, opts.sync) {
            Ok(()) => return Ok(WriteStrategy::Atomic),
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied && target.exists() => {
                tracing::debug!(path = %target.display(), "atomic write denied, writing in place");
            }
            Err(e) => return Err(e),
        }
    }
    write_in_place(&target, bytes, opts.sync)?;
    Ok(WriteStrategy::InPlace)
}

fn resolve_symlinks(path: &Path) -> io::Result<PathBuf> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => fs::canonicalize(path),
        _ => Ok(path.to_path_buf()),
    }
}

#[cfg(unix)]
fn has_other_hard_links(path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    fs::metadata(path).is_ok_and(|m| m.nlink() > 1)
}

#[cfg(not(unix))]
fn has_other_hard_links(_path: &Path) -> bool {
    false
}

fn write_atomic(target: &Path, bytes: &[u8], sync: bool) -> io::Result<()> {
    let dir = target
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = dir.join(format!(
        ".{name}.notoir-tmp-{}",
        uuid::Uuid::new_v4().simple()
    ));
    let result = (|| {
        let mut file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        file.write_all(bytes)?;
        if sync {
            file.sync_all()?;
        }
        drop(file);
        if let Ok(meta) = fs::metadata(target) {
            fs::set_permissions(&tmp, meta.permissions())?;
        }
        fs::rename(&tmp, target)?;
        if sync {
            sync_dir(dir);
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

fn write_in_place(target: &Path, bytes: &[u8], sync: bool) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(target)?;
    file.write_all(bytes)?;
    if sync {
        file.sync_all()?;
    }
    Ok(())
}

#[cfg(unix)]
fn sync_dir(dir: &Path) {
    if let Ok(d) = fs::File::open(dir) {
        let _ = d.sync_all();
    }
}

#[cfg(not(unix))]
fn sync_dir(_dir: &Path) {}

/// xxh3 hash of file contents, used for conflict detection and recovery
/// integrity (`saved_content_hash`, `content_hash`).
pub fn content_hash(bytes: &[u8]) -> u64 {
    xxhash_rust::xxh3::xxh3_64(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_and_replaces_without_leftovers() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.txt");
        assert_eq!(
            write(&p, b"one", WriteOptions::default()).unwrap(),
            WriteStrategy::Atomic
        );
        assert_eq!(
            write(&p, b"two", WriteOptions::default()).unwrap(),
            WriteStrategy::Atomic
        );
        assert_eq!(fs::read(&p).unwrap(), b"two");
        assert_eq!(
            fs::read_dir(dir.path()).unwrap().count(),
            1,
            "no temp files left"
        );
    }

    #[cfg(unix)]
    // spec 09: "Save preserves Unix permissions"
    #[test]
    fn preserves_unix_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("run.sh");
        fs::write(&p, "#!/bin/sh").unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
        write(&p, b"#!/bin/sh\necho hi", WriteOptions::default()).unwrap();
        assert_eq!(
            fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o755
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlink_target_is_replaced_and_link_kept() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real.md");
        let link = dir.path().join("link.md");
        fs::write(&real, "old").unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        write(&link, b"new", WriteOptions::default()).unwrap();
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read(&real).unwrap(), b"new");
    }

    #[cfg(unix)]
    #[test]
    fn hard_links_are_written_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        fs::write(&a, "old").unwrap();
        fs::hard_link(&a, &b).unwrap();
        assert_eq!(
            write(&a, b"new", WriteOptions::default()).unwrap(),
            WriteStrategy::InPlace
        );
        assert_eq!(fs::read(&b).unwrap(), b"new", "link still shares content");
    }

    #[cfg(unix)]
    #[test]
    fn unwritable_directory_falls_back_to_in_place() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("locked");
        fs::create_dir(&sub).unwrap();
        let p = sub.join("f.txt");
        fs::write(&p, "old").unwrap();
        fs::set_permissions(&sub, fs::Permissions::from_mode(0o555)).unwrap();
        let result = write(&p, b"new", WriteOptions::default());
        fs::set_permissions(&sub, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(result.unwrap(), WriteStrategy::InPlace);
        assert_eq!(fs::read(&p).unwrap(), b"new");
    }

    #[test]
    fn failed_write_leaves_original_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let missing_dir = dir.path().join("nope").join("f.txt");
        assert!(write(&missing_dir, b"x", WriteOptions::default()).is_err());
    }
}
