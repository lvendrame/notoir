//! Directory listing and the background index for the Explorer (docs/specs/07):
//! `files.exclude` globs are never returned, dotfiles are flagged hidden and
//! `.gitignore`d entries flagged ignored (the panel decides what to show).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::UNIX_EPOCH;

use globset::{Glob, GlobSet, GlobSetBuilder};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use notoir_core::explorer::{EntryInfo, IndexEntry, NodeKind};

/// Entries the index stops at (a huge tree must not exhaust memory).
pub const INDEX_LIMIT: usize = 200_000;

/// What to leave out of listings, built once per workspace/settings change.
#[derive(Clone)]
pub struct Filters {
    root: PathBuf,
    exclude: GlobSet,
}

impl Filters {
    /// `exclude` are `files.exclude` globs relative to the root
    /// (`**/.git`). Invalid globs are skipped with a warning.
    pub fn new(root: &Path, exclude: &[String]) -> Filters {
        let mut b = GlobSetBuilder::new();
        for g in exclude {
            match Glob::new(g) {
                Ok(g) => {
                    b.add(g);
                }
                Err(e) => tracing::warn!(glob = %g, error = %e, "invalid files.exclude glob"),
            }
        }
        Filters {
            root: root.to_path_buf(),
            exclude: b.build().unwrap_or_else(|_| GlobSet::empty()),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn is_excluded(&self, path: &Path) -> bool {
        match path.strip_prefix(&self.root) {
            Ok(rel) if !rel.as_os_str().is_empty() => self.exclude.is_match(rel),
            _ => false,
        }
    }

    /// The `.gitignore` rules that apply inside `dir`: every `.gitignore`
    /// from the root down to `dir`, plus `.git/info/exclude`.
    fn gitignore_for(&self, dir: &Path) -> Gitignore {
        let mut b = GitignoreBuilder::new(&self.root);
        let info = self.root.join(".git/info/exclude");
        if info.is_file() {
            b.add(info);
        }
        let mut d = self.root.clone();
        let add = |b: &mut GitignoreBuilder, d: &Path| {
            let f = d.join(".gitignore");
            if f.is_file() {
                b.add(f);
            }
        };
        add(&mut b, &d);
        if let Ok(rel) = dir.strip_prefix(&self.root) {
            for c in rel.components() {
                d.push(c);
                add(&mut b, &d);
            }
        }
        b.build().unwrap_or_else(|_| Gitignore::empty())
    }
}

fn is_hidden(name: &str, _meta: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
        if _meta.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0 {
            return true;
        }
    }
    name.starts_with('.')
}

fn modified_ms(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as u64)
}

/// Lists one directory (unsorted; the model sorts).
pub fn list_dir(dir: &Path, filters: &Filters) -> std::io::Result<Vec<EntryInfo>> {
    let ignore = filters.gitignore_for(dir);
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if filters.is_excluded(&path) {
            continue;
        }
        let Ok(link_meta) = entry.metadata() else {
            continue;
        };
        let symlink = entry.file_type().is_ok_and(|t| t.is_symlink());
        // Symlinks show what they point to; broken links show as files.
        let meta = if symlink {
            std::fs::metadata(&path).unwrap_or(link_meta)
        } else {
            link_meta
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_dir = meta.is_dir();
        out.push(EntryInfo {
            hidden: is_hidden(&name, &meta),
            ignored: ignore
                .matched_path_or_any_parents(&path, is_dir)
                .is_ignore(),
            kind: if is_dir {
                NodeKind::Dir
            } else {
                NodeKind::File
            },
            symlink,
            modified: modified_ms(&meta),
            name,
        });
    }
    Ok(out)
}

/// Walks the whole workspace for the filter and Quick Open. Stops early when
/// `cancel` is set or [`INDEX_LIMIT`] is reached (`truncated = true`).
pub fn build_index(filters: &Filters, cancel: &AtomicBool) -> (Vec<IndexEntry>, bool) {
    let root = filters.root.clone();
    let excl = filters.clone();
    // Ignored state comes from a second walker honoring .gitignore: entries
    // it does not yield are ignored.
    let mut not_ignored = std::collections::HashSet::new();
    let walk_git = ignore::WalkBuilder::new(&root)
        .standard_filters(false)
        .git_ignore(true)
        .git_exclude(true)
        .require_git(false)
        .follow_links(false)
        .filter_entry({
            let excl = excl.clone();
            move |e| !excl.is_excluded(e.path())
        })
        .build();
    for e in walk_git.flatten() {
        if cancel.load(Ordering::Relaxed) || not_ignored.len() > INDEX_LIMIT {
            break;
        }
        not_ignored.insert(e.into_path());
    }
    let walk = ignore::WalkBuilder::new(&root)
        .standard_filters(false)
        .follow_links(false)
        .filter_entry(move |e| !excl.is_excluded(e.path()))
        .build();
    let mut out = Vec::new();
    let mut truncated = false;
    for e in walk.flatten() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        if out.len() >= INDEX_LIMIT {
            truncated = true;
            break;
        }
        let Ok(rel) = e.path().strip_prefix(&root) else {
            continue;
        };
        if rel.as_os_str().is_empty() {
            continue;
        }
        let rel_s = rel.to_string_lossy().replace('\\', "/");
        let hidden = rel_s.split('/').any(|c| c.starts_with('.'));
        let is_dir = e.file_type().is_some_and(|t| t.is_dir());
        out.push(IndexEntry {
            ignored: !not_ignored.contains(e.path()),
            rel: rel_s,
            kind: if is_dir {
                NodeKind::Dir
            } else {
                NodeKind::File
            },
            hidden,
        });
    }
    (out, truncated)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults(root: &Path) -> Filters {
        Filters::new(
            root,
            &[
                "**/.git".into(),
                "**/.DS_Store".into(),
                "**/Thumbs.db".into(),
            ],
        )
    }

    fn setup() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        std::fs::create_dir_all(r.join(".git/objects")).unwrap();
        std::fs::create_dir_all(r.join("src/gen")).unwrap();
        std::fs::create_dir_all(r.join("target")).unwrap();
        for f in [
            ".env",
            "README.md",
            ".DS_Store",
            "src/main.rs",
            "src/gen/out.rs",
            "target/bin",
        ] {
            std::fs::write(r.join(f), "x").unwrap();
        }
        std::fs::write(r.join(".gitignore"), "target/\n").unwrap();
        std::fs::write(r.join("src/.gitignore"), "gen/\n").unwrap();
        d
    }

    #[test]
    fn listing_flags_hidden_ignored_and_drops_excluded() {
        let d = setup();
        let f = defaults(d.path());
        let mut root = list_dir(d.path(), &f).unwrap();
        root.sort_by(|a, b| a.name.cmp(&b.name));
        let names: Vec<_> = root.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, [".env", ".gitignore", "README.md", "src", "target"]);
        let get = |n: &str| root.iter().find(|e| e.name == n).unwrap().clone();
        assert!(get(".env").hidden);
        assert!(!get("README.md").hidden);
        assert!(get("target").ignored && get("target").kind == NodeKind::Dir);
        let src = list_dir(&d.path().join("src"), &f).unwrap();
        assert!(
            src.iter().find(|e| e.name == "gen").unwrap().ignored,
            "nested .gitignore"
        );
        assert!(!src.iter().find(|e| e.name == "main.rs").unwrap().ignored);
    }

    #[test]
    fn index_walks_everything_with_flags() {
        let d = setup();
        let (idx, truncated) = build_index(&defaults(d.path()), &AtomicBool::new(false));
        assert!(!truncated);
        let get = |r: &str| idx.iter().find(|e| e.rel == r).cloned();
        assert!(get(".git/objects").is_none(), "excluded");
        assert!(get(".DS_Store").is_none());
        assert!(get("src/gen/out.rs").unwrap().ignored);
        assert!(get("target/bin").unwrap().ignored);
        assert!(!get("src/main.rs").unwrap().ignored);
        assert!(get(".env").unwrap().hidden);
    }
}
