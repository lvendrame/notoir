//! Workspace search and replace (docs/specs/15 §Workspace Search): walks the
//! workspace honouring `.gitignore`, `files.exclude` and `search.exclude`,
//! skips binary and oversized files, searches open documents in memory, and
//! streams results in batches. Replace in closed files goes through the save
//! writer with the conflict check.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use globset::{Glob, GlobSet, GlobSetBuilder};
use notoir_editor::Rope;
use notoir_editor::display_map::line_text;
use notoir_editor::find::{self, FindQuery, Matcher};

use crate::fs_io::{self, LoadOptions, SaveRequest};

/// Preview text around a match is cut to about this many chars.
const PREVIEW_CHARS: usize = 160;
/// Batches are posted at least this often.
pub const BATCH_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Clone, Debug, Default)]
pub struct SearchOptions {
    /// Workspace root; `None` searches only the open documents.
    pub root: Option<PathBuf>,
    pub query: FindQuery,
    /// `files to include` globs (relative to the root).
    pub include: Vec<String>,
    /// `files.exclude` + `search.exclude` + `files to exclude`.
    pub exclude: Vec<String>,
    pub max_file_size: u64,
    pub max_results: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineMatch {
    /// 0-based line and char columns of the match within it.
    pub line: usize,
    pub start_col: usize,
    pub end_col: usize,
    /// The line (shortened) and the match's char range in it.
    pub preview: String,
    pub preview_range: (usize, usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileMatches {
    pub path: PathBuf,
    /// Root-relative, `/`-separated (the absolute path outside a workspace).
    pub rel: String,
    pub matches: Vec<LineMatch>,
}

/// One batch from a running search.
#[derive(Clone, Debug, Default)]
pub struct Batch {
    pub files: Vec<FileMatches>,
    pub done: bool,
    pub truncated: bool,
    /// Invalid regex: the message (the search did not run).
    pub error: Option<String>,
}

/// Globs from user input: a bare name or folder (`docs`, `*.md`) matches
/// anywhere below too.
pub fn glob_set(patterns: &[String]) -> GlobSet {
    let mut b = GlobSetBuilder::new();
    for p in patterns.iter().map(|p| p.trim()).filter(|p| !p.is_empty()) {
        let p = p.trim_start_matches("./").trim_end_matches('/');
        let mut variants = vec![p.to_owned(), format!("{p}/**")];
        if !p.contains('/') && !p.starts_with("**") {
            variants.push(format!("**/{p}"));
            variants.push(format!("**/{p}/**"));
        }
        for v in variants {
            if let Ok(g) = Glob::new(&v) {
                b.add(g);
            }
        }
    }
    b.build().unwrap_or_else(|_| GlobSet::empty())
}

/// Matches in one text.
pub fn search_text(rope: &Rope, matcher_query: &FindQuery, limit: usize) -> Vec<LineMatch> {
    let Ok(ranges) = find::find_all(rope, matcher_query) else {
        return Vec::new();
    };
    ranges
        .into_iter()
        .take(limit)
        .map(|(a, b)| {
            let line = rope.char_to_line(a);
            let start = rope.line_to_char(line);
            let text = line_text(rope, line);
            let len = text.chars().count();
            let sc = a - start;
            let ec = (b - start).min(len.max(sc));
            // Keep some context before long-line matches.
            let from = sc.saturating_sub(40);
            let preview: String = text.chars().skip(from).take(PREVIEW_CHARS).collect();
            let lead = if from > 0 { "…" } else { "" };
            let pr = (
                sc - from + lead.chars().count(),
                (ec - from + lead.chars().count())
                    .min(preview.chars().count() + lead.chars().count()),
            );
            LineMatch {
                line,
                start_col: sc,
                end_col: ec,
                preview: format!("{lead}{}", preview.trim_end()),
                preview_range: pr,
            }
        })
        .collect()
}

fn decode(bytes: &[u8]) -> Option<String> {
    if notoir_editor::encoding::looks_binary(bytes) {
        return None;
    }
    let d = notoir_editor::encoding::decode(bytes, None);
    Some(notoir_editor::line_ending::normalize(&d.text))
}

/// Runs a search, calling `emit` with batches (the last has `done`).
/// `snapshots` are open documents (searched in memory instead of on disk).
pub fn run(
    opts: &SearchOptions,
    snapshots: &HashMap<PathBuf, Rope>,
    unsaved: &[(String, Rope)],
    cancel: &AtomicBool,
    mut emit: impl FnMut(Batch),
) {
    if let Err(e) = Matcher::new(&opts.query) {
        emit(Batch {
            done: true,
            error: Some(e.to_string()),
            ..Default::default()
        });
        return;
    }
    let mut pending: Vec<FileMatches> = Vec::new();
    let mut total = 0usize;
    let mut last = Instant::now();
    let mut truncated = false;
    let include = glob_set(&opts.include);
    let exclude = glob_set(&opts.exclude);
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    let add = |file: FileMatches, pending: &mut Vec<FileMatches>, total: &mut usize| {
        *total += file.matches.len();
        pending.push(file);
    };
    let flush =
        |pending: &mut Vec<FileMatches>, last: &mut Instant, emit: &mut dyn FnMut(Batch)| {
            if !pending.is_empty() && last.elapsed() >= BATCH_INTERVAL {
                emit(Batch {
                    files: std::mem::take(pending),
                    ..Default::default()
                });
                *last = Instant::now();
            }
        };
    if let Some(root) = &opts.root {
        let walk = ignore::WalkBuilder::new(root)
            .standard_filters(false)
            .git_ignore(true)
            .git_exclude(true)
            .require_git(false)
            .follow_links(false)
            .filter_entry({
                let root = root.clone();
                let exclude = exclude.clone();
                move |e| match e.path().strip_prefix(&root) {
                    Ok(rel) if !rel.as_os_str().is_empty() => !exclude.is_match(rel),
                    _ => true,
                }
            })
            .build();
        for entry in walk.flatten() {
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            if !entry.file_type().is_some_and(|t| t.is_file()) {
                continue;
            }
            let path = entry.path();
            let Ok(rel) = path.strip_prefix(root) else {
                continue;
            };
            if !include.is_empty() && !include.is_match(rel) {
                continue;
            }
            let rel = rel.to_string_lossy().replace('\\', "/");
            let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
            let rope = match snapshots.get(&canonical) {
                Some(r) => {
                    seen.insert(canonical.clone());
                    r.clone()
                }
                None => {
                    if entry.metadata().is_ok_and(|m| m.len() > opts.max_file_size) {
                        continue;
                    }
                    let Ok(bytes) = std::fs::read(path) else {
                        continue;
                    };
                    match decode(&bytes) {
                        Some(t) => Rope::from_str(&t),
                        None => continue,
                    }
                }
            };
            let room = opts.max_results.saturating_sub(total);
            let matches = search_text(&rope, &opts.query, room);
            if !matches.is_empty() {
                add(
                    FileMatches {
                        path: canonical,
                        rel,
                        matches,
                    },
                    &mut pending,
                    &mut total,
                );
            }
            if total >= opts.max_results {
                truncated = true;
                break;
            }
            flush(&mut pending, &mut last, &mut emit);
        }
    }
    // Open documents outside the walk (no workspace, or outside the root).
    if !truncated {
        let mut rest: Vec<(&PathBuf, &Rope)> = snapshots
            .iter()
            .filter(|(p, _)| !seen.contains(*p))
            .collect();
        rest.sort_by(|a, b| a.0.cmp(b.0));
        for (path, rope) in rest {
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            let rel = match &opts.root {
                Some(root) => match path.strip_prefix(root) {
                    // Inside the root but not walked: ignored or excluded.
                    Ok(_) => continue,
                    Err(_) => path.to_string_lossy().into_owned(),
                },
                None => path.to_string_lossy().into_owned(),
            };
            let matches = search_text(rope, &opts.query, opts.max_results.saturating_sub(total));
            if !matches.is_empty() {
                add(
                    FileMatches {
                        path: path.clone(),
                        rel,
                        matches,
                    },
                    &mut pending,
                    &mut total,
                );
            }
            if total >= opts.max_results {
                truncated = true;
                break;
            }
        }
        // Untitled documents and Scratchpads (no path): their titles.
        for (title, rope) in unsaved {
            if truncated {
                break;
            }
            let matches = search_text(rope, &opts.query, opts.max_results.saturating_sub(total));
            if !matches.is_empty() {
                add(
                    FileMatches {
                        path: PathBuf::new(),
                        rel: title.clone(),
                        matches,
                    },
                    &mut pending,
                    &mut total,
                );
            }
            truncated = total >= opts.max_results;
        }
    }
    emit(Batch {
        files: pending,
        done: true,
        truncated,
        error: None,
    });
}

/// Replace All in a closed file: load, replace, save with the conflict check.
/// Returns the number of replacements.
pub fn replace_in_file(path: &Path, query: &FindQuery, with: &str) -> Result<usize, String> {
    let loaded = fs_io::load(path, &LoadOptions::default()).map_err(|e| e.to_string())?;
    if loaded.text.read_only != notoir_core::document::ReadOnly::No {
        return Err("read-only".into());
    }
    let rope = Rope::from_str(&loaded.text.text);
    let Some((changes, count)) =
        find::replace_all(&rope, query, with).map_err(|e| e.to_string())?
    else {
        return Ok(0);
    };
    let mut out = rope.clone();
    changes.apply(&mut out);
    fs_io::save(&SaveRequest {
        path: loaded.path.clone(),
        text: out.to_string(),
        encoding: loaded.text.encoding,
        line_ending: loaded.text.line_ending,
        expected_hash: loaded.text.content_hash,
        overwrite: false,
        atomic: true,
    })
    .map_err(|e| e.to_string())?;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(root: &Path, text: &str) -> SearchOptions {
        SearchOptions {
            root: Some(root.to_path_buf()),
            query: FindQuery {
                text: text.into(),
                ..Default::default()
            },
            include: Vec::new(),
            exclude: vec!["**/.git".into()],
            max_file_size: 5 * 1024 * 1024,
            max_results: 20_000,
        }
    }

    fn collect(o: &SearchOptions, snaps: &HashMap<PathBuf, Rope>) -> Vec<FileMatches> {
        let mut out = Vec::new();
        run(o, snaps, &[], &AtomicBool::new(false), |b| {
            out.extend(b.files)
        });
        out.sort_by(|a, b| a.rel.cmp(&b.rel));
        out
    }

    fn tree() -> (tempfile::TempDir, PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap();
        std::fs::create_dir_all(root.join("docs")).unwrap();
        std::fs::create_dir_all(root.join("build")).unwrap();
        std::fs::write(root.join(".gitignore"), "build/\n").unwrap();
        std::fs::write(
            root.join("docs/mcp.md"),
            "intro\nNotoir exposes an MCP server\n",
        )
        .unwrap();
        std::fs::write(root.join("README.md"), "MCP integration\nmcp again\n").unwrap();
        std::fs::write(root.join("build/out.txt"), "MCP\n").unwrap();
        std::fs::write(root.join("bin.dat"), b"MCP\0\0\0").unwrap();
        (d, root)
    }

    #[test]
    fn finds_and_respects_gitignore_and_binary() {
        let (_d, root) = tree();
        let r = collect(&opts(&root, "MCP"), &HashMap::new());
        let files: Vec<_> = r.iter().map(|f| f.rel.as_str()).collect();
        assert_eq!(files, ["README.md", "docs/mcp.md"]);
        assert_eq!(r[0].matches.len(), 2, "case-insensitive by default");
        let m = &r[1].matches[0];
        assert_eq!((m.line, m.start_col, m.end_col), (1, 18, 21));
        assert_eq!(&m.preview[m.preview_range.0..m.preview_range.1], "MCP");
    }

    #[test]
    fn include_exclude_case_and_snapshots() {
        let (_d, root) = tree();
        let mut o = opts(&root, "MCP");
        o.include = vec!["docs".into()];
        assert_eq!(collect(&o, &HashMap::new()).len(), 1);
        let mut o = opts(&root, "MCP");
        o.exclude.push("*.md".into());
        assert!(collect(&o, &HashMap::new()).is_empty());
        let mut o = opts(&root, "MCP");
        o.query.case_sensitive = true;
        assert_eq!(collect(&o, &HashMap::new())[0].matches.len(), 1);
        // In-memory content wins over disk.
        let mut snaps = HashMap::new();
        snaps.insert(root.join("README.md"), Rope::from_str("zebra\n"));
        let r = collect(&opts(&root, "zebra"), &snaps);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].rel, "README.md");
    }

    #[test]
    fn invalid_regex_reports_error() {
        let (_d, root) = tree();
        let mut o = opts(&root, "(");
        o.query.regex = true;
        let mut err = None;
        run(&o, &HashMap::new(), &[], &AtomicBool::new(false), |b| {
            err = b.error
        });
        assert!(err.is_some());
    }

    #[test]
    fn replace_closed_file() {
        let (_d, root) = tree();
        let q = FindQuery {
            text: "mcp".into(),
            ..Default::default()
        };
        assert_eq!(
            replace_in_file(&root.join("README.md"), &q, "X").unwrap(),
            2
        );
        assert_eq!(
            std::fs::read_to_string(root.join("README.md")).unwrap(),
            "X integration\nX again\n"
        );
    }
}
