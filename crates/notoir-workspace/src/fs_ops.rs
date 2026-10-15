//! Explorer file operations (docs/specs/07 §File Operations): name
//! validation, create, rename (case-only too), duplicate, copy and move.
//! Callers run these off the UI thread. Deleting goes to the OS trash
//! (`notoir-platform`).

use std::io;
use std::path::{Path, PathBuf};

use thiserror::Error;

/// Which OS's naming rules apply (tests check every OS on any host).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetOs {
    Windows,
    MacOs,
    Linux,
}

impl TargetOs {
    pub fn current() -> TargetOs {
        if cfg!(windows) {
            TargetOs::Windows
        } else if cfg!(target_os = "macos") {
            TargetOs::MacOs
        } else {
            TargetOs::Linux
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum NameError {
    #[error("empty name")]
    Empty,
    #[error("invalid character {0:?}")]
    InvalidChar(char),
    #[error("reserved name")]
    Reserved,
    #[error("name ends with a dot or space")]
    TrailingDotOrSpace,
    #[error("name too long")]
    TooLong,
    #[error("already exists")]
    Exists,
}

impl NameError {
    /// i18n key under `explorer:` for the inline editor message.
    pub fn key(&self) -> &'static str {
        match self {
            NameError::Empty => "explorer:nameError.empty",
            NameError::InvalidChar(_) => "explorer:nameError.invalidChar",
            NameError::Reserved => "explorer:nameError.reserved",
            NameError::TrailingDotOrSpace => "explorer:nameError.trailing",
            NameError::TooLong => "explorer:nameError.tooLong",
            NameError::Exists => "explorer:nameError.exists",
        }
    }
}

const WINDOWS_RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Checks a single path component typed in the inline editor.
pub fn validate_name(name: &str, os: TargetOs) -> Result<(), NameError> {
    if name.trim().is_empty() || name == "." || name == ".." {
        return Err(NameError::Empty);
    }
    if name.len() > 255 {
        return Err(NameError::TooLong);
    }
    let invalid: &[char] = match os {
        TargetOs::Windows => &['<', '>', ':', '"', '/', '\\', '|', '?', '*'],
        TargetOs::MacOs => &['/', ':'],
        TargetOs::Linux => &['/'],
    };
    if let Some(c) = name.chars().find(|c| {
        invalid.contains(c) || *c == '\0' || (os == TargetOs::Windows && (*c as u32) < 32)
    }) {
        return Err(NameError::InvalidChar(c));
    }
    if os == TargetOs::Windows {
        if name.ends_with('.') || name.ends_with(' ') {
            return Err(NameError::TrailingDotOrSpace);
        }
        let stem = name.split('.').next().unwrap_or(name).trim_end();
        if WINDOWS_RESERVED
            .iter()
            .any(|r| r.eq_ignore_ascii_case(stem))
        {
            return Err(NameError::Reserved);
        }
    }
    Ok(())
}

/// Validates `name` as a new entry of `dir` (also rejects existing names;
/// `except` is the entry being renamed, so case-only renames pass).
pub fn validate_new_name(
    dir: &Path,
    name: &str,
    except: Option<&Path>,
    os: TargetOs,
) -> Result<PathBuf, NameError> {
    validate_name(name, os)?;
    let target = dir.join(name);
    let same_entry = except.is_some_and(|e| {
        e.parent() == Some(dir)
            && e.file_name()
                .is_some_and(|n| n.to_string_lossy().to_lowercase() == name.to_lowercase())
    });
    if !same_entry && target.symlink_metadata().is_ok() {
        return Err(NameError::Exists);
    }
    if except.is_some_and(|e| e == target) {
        return Err(NameError::Exists);
    }
    Ok(target)
}

pub fn create_file(path: &Path) -> io::Result<()> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map(|_| ())
}

pub fn create_dir(path: &Path) -> io::Result<()> {
    std::fs::create_dir(path)
}

/// Renames `from` to `to`. A case-only rename goes through a temporary name,
/// which case-insensitive file systems need.
pub fn rename(from: &Path, to: &Path) -> io::Result<()> {
    let case_only = from != to
        && from.parent() == to.parent()
        && from.to_string_lossy().to_lowercase() == to.to_string_lossy().to_lowercase();
    if case_only {
        let tmp = from.with_file_name(format!(
            ".{}.notoir-rename",
            from.file_name().unwrap_or_default().to_string_lossy()
        ));
        std::fs::rename(from, &tmp)?;
        return std::fs::rename(&tmp, to);
    }
    if to.symlink_metadata().is_ok() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "target exists",
        ));
    }
    std::fs::rename(from, to)
}

/// A free name in `dir` for a copy of `name`: `name copy.ext`,
/// `name copy 2.ext`, … (folders: `name copy`).
pub fn copy_name(dir: &Path, name: &str, is_dir: bool) -> PathBuf {
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 && !is_dir => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    let mut n = 1;
    loop {
        let candidate = if n == 1 {
            format!("{stem} copy{ext}")
        } else {
            format!("{stem} copy {n}{ext}")
        };
        let p = dir.join(candidate);
        if p.symlink_metadata().is_err() {
            return p;
        }
        n += 1;
    }
}

/// Copies a file or folder (recursively) to `to`, which must not exist.
pub fn copy_recursive(from: &Path, to: &Path) -> io::Result<()> {
    if to.symlink_metadata().is_ok() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "target exists",
        ));
    }
    if from.is_dir() {
        if to.starts_with(from) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "cannot copy a folder into itself",
            ));
        }
        std::fs::create_dir(to)?;
        for e in std::fs::read_dir(from)? {
            let e = e?;
            copy_recursive(&e.path(), &to.join(e.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(from, to).map(|_| ())
    }
}

/// Duplicate: a copy next to the original. Returns the new path.
pub fn duplicate(path: &Path) -> io::Result<PathBuf> {
    let dir = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no parent"))?;
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let target = copy_name(dir, &name, path.is_dir());
    copy_recursive(path, &target)?;
    Ok(target)
}

/// Copies `src` into folder `dir` (a copy name is chosen on conflict).
pub fn copy_into(src: &Path, dir: &Path) -> io::Result<PathBuf> {
    let name = src.file_name().unwrap_or_default();
    let mut target = dir.join(name);
    if target.symlink_metadata().is_ok() {
        target = copy_name(dir, &name.to_string_lossy(), src.is_dir());
    }
    copy_recursive(src, &target)?;
    Ok(target)
}

/// Moves `src` into folder `dir`. Fails if the name is taken there or if a
/// folder would move into itself.
pub fn move_into(src: &Path, dir: &Path) -> io::Result<PathBuf> {
    if dir.starts_with(src) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "cannot move a folder into itself",
        ));
    }
    let target = dir.join(src.file_name().unwrap_or_default());
    if target == src {
        return Ok(target);
    }
    if target.symlink_metadata().is_ok() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "target exists",
        ));
    }
    match std::fs::rename(src, &target) {
        Ok(()) => Ok(target),
        // Different volume: copy then remove.
        Err(_) => {
            copy_recursive(src, &target)?;
            if src.is_dir() {
                std::fs::remove_dir_all(src)?;
            } else {
                std::fs::remove_file(src)?;
            }
            Ok(target)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // spec 07: "Invalid file name"
    #[test]
    fn windows_reserved_and_invalid_names() {
        assert_eq!(
            validate_name("CON", TargetOs::Windows),
            Err(NameError::Reserved)
        );
        assert_eq!(
            validate_name("nul.txt", TargetOs::Windows),
            Err(NameError::Reserved)
        );
        assert_eq!(
            validate_name("a:b", TargetOs::Windows),
            Err(NameError::InvalidChar(':'))
        );
        assert_eq!(
            validate_name("notes.", TargetOs::Windows),
            Err(NameError::TrailingDotOrSpace)
        );
        assert_eq!(validate_name("CON", TargetOs::Linux), Ok(()));
        assert_eq!(
            validate_name("a/b", TargetOs::Linux),
            Err(NameError::InvalidChar('/'))
        );
        assert_eq!(
            validate_name("a:b", TargetOs::MacOs),
            Err(NameError::InvalidChar(':'))
        );
        assert_eq!(validate_name("  ", TargetOs::MacOs), Err(NameError::Empty));
        // No file is created when validation fails: the caller only creates
        // after `validate_new_name` succeeds.
        let d = tempfile::tempdir().unwrap();
        assert!(validate_new_name(d.path(), "CON", None, TargetOs::Windows).is_err());
        assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 0);
    }

    #[test]
    fn existing_names_rejected_except_case_only_rename() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("Notes.md");
        std::fs::write(&f, "x").unwrap();
        assert_eq!(
            validate_new_name(d.path(), "Notes.md", None, TargetOs::Linux),
            Err(NameError::Exists)
        );
        assert!(validate_new_name(d.path(), "notes.md", Some(&f), TargetOs::Linux).is_ok());
        rename(&f, &d.path().join("notes.md")).unwrap();
        let names: Vec<_> = std::fs::read_dir(d.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["notes.md"]);
    }

    #[test]
    fn copies_get_numbered_names() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("a.md");
        std::fs::write(&f, "x").unwrap();
        assert_eq!(duplicate(&f).unwrap(), d.path().join("a copy.md"));
        assert_eq!(duplicate(&f).unwrap(), d.path().join("a copy 2.md"));
        let dir = d.path().join("docs");
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub/n.md"), "n").unwrap();
        let copy = duplicate(&dir).unwrap();
        assert_eq!(copy, d.path().join("docs copy"));
        assert_eq!(std::fs::read_to_string(copy.join("sub/n.md")).unwrap(), "n");
    }

    #[test]
    fn move_and_copy_into() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("a.md");
        std::fs::write(&f, "x").unwrap();
        let sub = d.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        let moved = move_into(&f, &sub).unwrap();
        assert_eq!(moved, sub.join("a.md"));
        assert!(!f.exists());
        let c = copy_into(&moved, &sub).unwrap();
        assert_eq!(c, sub.join("a copy.md"));
        assert!(move_into(&sub, &sub).is_err());
    }
}
