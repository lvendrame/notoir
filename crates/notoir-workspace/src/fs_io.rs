//! Loading and saving document files (docs/specs/02 §Loading, 09 §Save
//! pipeline). Runs off the UI thread; returns plain data.

use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use encoding_rs::Encoding;
use notoir_core::document::{LargeFileLimits, LoadedText, PathKey, ReadOnly};
use notoir_editor::encoding::{self, TextEncoding};
use notoir_editor::language::{self, Association};
use notoir_editor::line_ending::{self, LineEnding};
use notoir_storage::atomic::{self, WriteOptions, WriteStrategy};

/// Whether paths on this platform's default file system ignore case.
pub fn case_insensitive_fs() -> bool {
    cfg!(any(target_os = "macos", target_os = "windows"))
}

/// Canonical path (symlinks resolved) and its identity key. Falls back to an
/// absolute path when the file does not exist yet.
pub fn identity(path: &Path) -> (PathBuf, PathKey) {
    let canonical = std::fs::canonicalize(path)
        .unwrap_or_else(|_| std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf()));
    let key = PathKey::new(&canonical, case_insensitive_fs());
    (canonical, key)
}

#[derive(Clone, Debug)]
pub struct LoadOptions {
    /// "Reopen with Encoding…".
    pub forced_encoding: Option<&'static Encoding>,
    pub associations: Vec<Association>,
    pub limits: LargeFileLimits,
    /// Refuse files above this size outright (`files.largeFile.maxOpenSize`).
    pub max_open_bytes: u64,
    /// "Open Anyway" for binary files (opens read-only).
    pub allow_binary: bool,
    /// Line ending used for files without line breaks.
    pub default_eol: LineEnding,
}

impl Default for LoadOptions {
    fn default() -> Self {
        Self {
            forced_encoding: None,
            associations: Vec::new(),
            limits: LargeFileLimits::default(),
            max_open_bytes: 2 * 1024 * 1024 * 1024,
            allow_binary: false,
            default_eol: LineEnding::platform_default(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("file not found")]
    NotFound,
    #[error("permission denied")]
    PermissionDenied,
    #[error("is a directory")]
    IsDirectory,
    #[error("file is too large to open ({size} bytes, limit {max})")]
    TooLarge { size: u64, max: u64 },
    #[error("file appears to be binary")]
    Binary,
    #[error(transparent)]
    Io(io::Error),
}

impl From<io::Error> for LoadError {
    fn from(e: io::Error) -> Self {
        match e.kind() {
            io::ErrorKind::NotFound => LoadError::NotFound,
            io::ErrorKind::PermissionDenied => LoadError::PermissionDenied,
            io::ErrorKind::IsADirectory => LoadError::IsDirectory,
            _ => LoadError::Io(e),
        }
    }
}

#[derive(Debug)]
pub struct Loaded {
    pub path: PathBuf,
    pub key: PathKey,
    pub text: LoadedText,
    pub size: u64,
    pub modified: Option<SystemTime>,
}

pub fn load(path: &Path, opts: &LoadOptions) -> Result<Loaded, LoadError> {
    let meta = std::fs::metadata(path)?;
    if meta.is_dir() {
        return Err(LoadError::IsDirectory);
    }
    let size = meta.len();
    if size > opts.max_open_bytes {
        return Err(LoadError::TooLarge {
            size,
            max: opts.max_open_bytes,
        });
    }
    let bytes = std::fs::read(path)?;
    let binary = encoding::looks_binary(&bytes);
    if binary && !opts.allow_binary {
        return Err(LoadError::Binary);
    }
    let decoded = encoding::decode(&bytes, opts.forced_encoding);
    let eol = line_ending::detect(&decoded.text, opts.default_eol);
    let text = line_ending::normalize(&decoded.text);
    let (canonical, key) = identity(path);
    let first_line = text.lines().next().unwrap_or("");
    let lang = language::detect(
        Some(&canonical.to_string_lossy()),
        first_line,
        &opts.associations,
    );
    let large = opts.limits.is_large(size, &text);
    let read_only = if decoded.lossy {
        ReadOnly::LossyDecode
    } else if binary {
        ReadOnly::User
    } else if size > opts.limits.max_editable_bytes {
        ReadOnly::LargeFile
    } else if meta.permissions().readonly() {
        ReadOnly::FileSystem
    } else {
        ReadOnly::No
    };
    let indentation = if large {
        None
    } else {
        notoir_editor::indent_detect::detect(&text)
    };
    Ok(Loaded {
        path: canonical,
        key,
        text: LoadedText {
            text,
            encoding: decoded.encoding,
            line_ending: eol.ending,
            mixed_line_endings: eol.mixed,
            language: lang,
            read_only,
            large_file: large,
            content_hash: Some(atomic::content_hash(&bytes)),
            indentation,
        },
        size,
        modified: meta.modified().ok(),
    })
}

#[derive(Clone, Debug)]
pub struct SaveRequest {
    pub path: PathBuf,
    /// `\n`-normalized text.
    pub text: String,
    pub encoding: TextEncoding,
    pub line_ending: LineEnding,
    /// Hash of the disk content the document was based on; a different disk
    /// content is a conflict (spec 09 step 3). `None` for new files / Save As.
    pub expected_hash: Option<u64>,
    /// The user chose "Overwrite" after a conflict.
    pub overwrite: bool,
    pub atomic: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    #[error("file changed on disk")]
    Conflict,
    #[error("text cannot be encoded as {0}")]
    Unmappable(String),
    #[error("permission denied")]
    PermissionDenied,
    #[error("disk is full")]
    DiskFull,
    #[error("folder not found")]
    NotFound,
    #[error(transparent)]
    Io(io::Error),
}

impl From<io::Error> for SaveError {
    fn from(e: io::Error) -> Self {
        match e.kind() {
            io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem => {
                SaveError::PermissionDenied
            }
            io::ErrorKind::StorageFull | io::ErrorKind::QuotaExceeded => SaveError::DiskFull,
            io::ErrorKind::NotFound => SaveError::NotFound,
            _ => SaveError::Io(e),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Saved {
    pub hash: u64,
    pub strategy: WriteStrategy,
}

/// Encodes and writes atomically after the conflict check.
pub fn save(req: &SaveRequest) -> Result<Saved, SaveError> {
    let text = line_ending::denormalize(&req.text, req.line_ending);
    let bytes = encoding::encode(&text, req.encoding)
        .map_err(|e| SaveError::Unmappable(e.encoding.to_owned()))?;
    if let (Some(expected), false) = (req.expected_hash, req.overwrite) {
        match std::fs::read(&req.path) {
            Ok(disk) if atomic::content_hash(&disk) != expected => return Err(SaveError::Conflict),
            // Deleted on disk: saving recreates it.
            Ok(_) | Err(_) => {}
        }
    }
    let strategy = atomic::write(
        &req.path,
        &bytes,
        WriteOptions {
            atomic: req.atomic,
            sync: true,
        },
    )?;
    Ok(Saved {
        hash: atomic::content_hash(&bytes),
        strategy,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, bytes).unwrap();
        p
    }

    #[test]
    fn loads_and_round_trips_crlf_utf8() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(dir.path(), "notes.md", b"# T\r\n\r\n  - a\r\n");
        let l = load(&p, &LoadOptions::default()).unwrap();
        assert_eq!(l.text.text, "# T\n\n  - a\n");
        assert_eq!(l.text.line_ending, LineEnding::CrLf);
        assert_eq!(l.text.language, notoir_editor::LanguageId::MARKDOWN);
        let req = SaveRequest {
            path: p.clone(),
            text: format!("{}x\n", l.text.text),
            encoding: l.text.encoding,
            line_ending: l.text.line_ending,
            expected_hash: l.text.content_hash,
            overwrite: false,
            atomic: true,
        };
        save(&req).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"# T\r\n\r\n  - a\r\nx\r\n");
    }

    #[test]
    fn binary_and_lossy_files() {
        let dir = tempfile::tempdir().unwrap();
        let bin = write(dir.path(), "a.bin", b"\x00\x01\x02");
        assert!(matches!(
            load(&bin, &LoadOptions::default()),
            Err(LoadError::Binary)
        ));
        let opened = load(
            &bin,
            &LoadOptions {
                allow_binary: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_ne!(opened.text.read_only, ReadOnly::No);
        let lossy = write(dir.path(), "bad.txt", b"ok \xff\xfe\xfd");
        let opts = LoadOptions {
            forced_encoding: Some(encoding_rs::UTF_8),
            ..Default::default()
        };
        assert_eq!(
            load(&lossy, &opts).unwrap().text.read_only,
            ReadOnly::LossyDecode
        );
    }

    #[test]
    fn too_large_files_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(dir.path(), "big.txt", &[b'a'; 100]);
        let opts = LoadOptions {
            max_open_bytes: 10,
            ..Default::default()
        };
        assert!(matches!(
            load(&p, &opts),
            Err(LoadError::TooLarge { size: 100, .. })
        ));
    }

    #[test]
    fn save_detects_conflicts_and_allows_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(dir.path(), "a.txt", b"one");
        let base = load(&p, &LoadOptions::default()).unwrap().text.content_hash;
        std::fs::write(&p, "changed elsewhere").unwrap();
        let mut req = SaveRequest {
            path: p.clone(),
            text: "mine".into(),
            encoding: TextEncoding::UTF8,
            line_ending: LineEnding::Lf,
            expected_hash: base,
            overwrite: false,
            atomic: true,
        };
        assert!(matches!(save(&req), Err(SaveError::Conflict)));
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "changed elsewhere");
        req.overwrite = true;
        save(&req).unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "mine");
    }

    #[test]
    fn saving_a_deleted_file_recreates_it() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(dir.path(), "gone.txt", b"x");
        let base = load(&p, &LoadOptions::default()).unwrap().text.content_hash;
        std::fs::remove_file(&p).unwrap();
        let req = SaveRequest {
            path: p.clone(),
            text: "back".into(),
            encoding: TextEncoding::UTF8,
            line_ending: LineEnding::Lf,
            expected_hash: base,
            overwrite: false,
            atomic: true,
        };
        save(&req).unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "back");
    }

    #[test]
    fn unmappable_text_is_not_written() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(dir.path(), "l1.txt", b"abc");
        let req = SaveRequest {
            path: p.clone(),
            text: "日本".into(),
            encoding: TextEncoding {
                encoding: encoding_rs::WINDOWS_1252,
                bom: false,
            },
            line_ending: LineEnding::Lf,
            expected_hash: None,
            overwrite: false,
            atomic: true,
        };
        assert!(matches!(save(&req), Err(SaveError::Unmappable(_))));
        assert_eq!(std::fs::read(&p).unwrap(), b"abc");
    }

    // spec 02: "Two paths to the same file share a document"
    #[cfg(unix)]
    #[test]
    fn symlinks_share_identity() {
        let dir = tempfile::tempdir().unwrap();
        let real = write(dir.path(), "b.txt", b"x");
        let link = dir.path().join("a.txt");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert_eq!(identity(&link).1, identity(&real).1);
    }

    #[cfg(unix)]
    #[test]
    fn write_failure_keeps_original() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("locked");
        std::fs::create_dir(&sub).unwrap();
        let p = write(&sub, "f.txt", b"original");
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o444)).unwrap();
        std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o555)).unwrap();
        let req = SaveRequest {
            path: p.clone(),
            text: "new".into(),
            encoding: TextEncoding::UTF8,
            line_ending: LineEnding::Lf,
            expected_hash: None,
            overwrite: false,
            atomic: true,
        };
        let result = save(&req);
        std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(
            matches!(result, Err(SaveError::PermissionDenied)),
            "{result:?}"
        );
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "original");
    }
}
