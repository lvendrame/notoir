//! Workspace boundary (docs/specs/29 §Tool Catalogue, 32 §Evaluation): MCP
//! paths resolve against the workspace root; `..` escapes, absolute paths
//! outside it and symlinks leading out of it are rejected, as are Windows
//! device paths and alternate data streams.

use std::path::{Component, Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathError {
    /// `path_outside_workspace`.
    Outside,
    /// `invalid_path`: empty, NUL, device path or data stream.
    Invalid,
}

impl PathError {
    pub fn code(self) -> &'static str {
        match self {
            PathError::Outside => "path_outside_workspace",
            PathError::Invalid => "invalid_path",
        }
    }
}

/// Resolves `input` (relative to `root`, or absolute) to a path inside the
/// canonical `root`. The file itself need not exist.
pub fn resolve(root: &Path, input: &str) -> Result<PathBuf, PathError> {
    if input.trim().is_empty() || input.contains('\0') {
        return Err(PathError::Invalid);
    }
    if input.starts_with(r"\\?\") || input.starts_with(r"\\.\") || input.starts_with("//./") {
        return Err(PathError::Invalid);
    }
    let root = root.canonicalize().map_err(|_| PathError::Outside)?;
    let raw = Path::new(input);
    let joined = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        root.join(raw)
    };
    // Lexical normalisation (`a/../b`), then symlinks via the deepest
    // existing ancestor.
    let mut normal = PathBuf::new();
    for c in joined.components() {
        match c {
            Component::ParentDir => {
                if !normal.pop() {
                    return Err(PathError::Outside);
                }
            }
            Component::CurDir => {}
            Component::Normal(name) => {
                // `file.txt:stream` (NTFS alternate data stream).
                if name.to_string_lossy().contains(':') {
                    return Err(PathError::Invalid);
                }
                normal.push(name);
            }
            other => normal.push(other.as_os_str()),
        }
    }
    let mut existing = normal.clone();
    let mut rest = Vec::new();
    let real = loop {
        match existing.canonicalize() {
            Ok(p) => break p,
            Err(_) => match existing.file_name() {
                Some(name) => {
                    rest.push(name.to_owned());
                    existing.pop();
                }
                None => return Err(PathError::Outside),
            },
        }
    };
    let mut full = real;
    for name in rest.into_iter().rev() {
        full.push(name);
    }
    if full.starts_with(&root) {
        Ok(full)
    } else {
        Err(PathError::Outside)
    }
}

/// `path` relative to `root` with `/` separators (activity targets).
pub fn display(root: Option<&Path>, path: &Path) -> String {
    root.and_then(|r| path.strip_prefix(r).ok())
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inside_outside_and_invalid() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("ws");
        std::fs::create_dir_all(root.join("docs")).unwrap();
        std::fs::write(root.join("notes.md"), "x").unwrap();
        let canon = root.canonicalize().unwrap();
        assert_eq!(resolve(&root, "notes.md"), Ok(canon.join("notes.md")));
        assert_eq!(
            resolve(&root, "docs/../notes.md"),
            Ok(canon.join("notes.md"))
        );
        assert_eq!(
            resolve(&root, "docs/new/file.md"),
            Ok(canon.join("docs/new/file.md"))
        );
        assert_eq!(
            resolve(&root, canon.join("notes.md").to_str().unwrap()),
            Ok(canon.join("notes.md"))
        );
        assert_eq!(resolve(&root, "../secrets.txt"), Err(PathError::Outside));
        assert_eq!(resolve(&root, "docs/../../x"), Err(PathError::Outside));
        assert_eq!(resolve(&root, "/etc/hosts"), Err(PathError::Outside));
        assert_eq!(resolve(&root, ""), Err(PathError::Invalid));
        assert_eq!(resolve(&root, "notes.md:stream"), Err(PathError::Invalid));
        assert_eq!(resolve(&root, r"\\?\C:\x"), Err(PathError::Invalid));
        assert_eq!(display(Some(&canon), &canon.join("docs/a.md")), "docs/a.md");
    }

    #[cfg(unix)]
    #[test]
    fn symlink_escape() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("ws");
        let outside = d.path().join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("hosts"), "x").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("link")).unwrap();
        assert_eq!(resolve(&root, "link/hosts"), Err(PathError::Outside));
        assert_eq!(resolve(&root, "link/new.txt"), Err(PathError::Outside));
    }
}
