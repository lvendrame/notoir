//! `Document`: an open file, Untitled buffer, Scratchpad or virtual document,
//! independent of any tab (docs/specs/02-document-model.md).

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use notoir_editor::encoding::TextEncoding;
use notoir_editor::{ChangeSet, LanguageId, LineEnding, TextBuffer};

use crate::ids::DocumentId;

/// Identity key for a file path. Built by the caller from a canonical path
/// (symlinks resolved); case is folded on case-insensitive file systems.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PathKey(String);

impl PathKey {
    pub fn new(canonical: &Path, case_insensitive: bool) -> Self {
        let s = canonical.to_string_lossy().replace('\\', "/");
        Self(if case_insensitive {
            s.to_lowercase()
        } else {
            s
        })
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DocumentOrigin {
    File { path: PathBuf, key: PathKey },
    Scratchpad { scratch_id: String, title: String },
    Untitled { n: u32 },
    Virtual { title: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiskState {
    InSync,
    ModifiedExternally,
    DeletedExternally,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadOnly {
    No,
    FileSystem,
    User,
    LargeFile,
    /// Opened with replacement characters (spec 02 §Loading step 3).
    LossyDecode,
    /// A read-only snapshot (saved version, clipboard) shown by Compare.
    Snapshot,
}

/// Large-file thresholds (`files.largeFile.*`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LargeFileLimits {
    pub size_bytes: u64,
    pub lines: usize,
    pub longest_line: usize,
    pub max_editable_bytes: u64,
}

impl Default for LargeFileLimits {
    fn default() -> Self {
        Self {
            size_bytes: 20 * 1024 * 1024,
            lines: 300_000,
            longest_line: 20_000,
            max_editable_bytes: 512 * 1024 * 1024,
        }
    }
}

impl LargeFileLimits {
    pub fn is_large(&self, size_bytes: u64, text: &str) -> bool {
        if size_bytes > self.size_bytes {
            return true;
        }
        let mut lines = 0;
        for line in text.split('\n') {
            lines += 1;
            if lines > self.lines || line.chars().count() > self.longest_line {
                return true;
            }
        }
        false
    }
}

/// Text and metadata produced by loading (decoding/normalizing happens in the
/// IO layer using `notoir_editor::{encoding, line_ending}`).
#[derive(Clone, Debug)]
pub struct LoadedText {
    /// `\n`-normalized text.
    pub text: String,
    pub encoding: TextEncoding,
    pub line_ending: LineEnding,
    pub mixed_line_endings: bool,
    pub language: LanguageId,
    pub read_only: ReadOnly,
    pub large_file: bool,
    pub content_hash: Option<u64>,
    /// Detected indentation (`editor.detectIndentation`).
    pub indentation: Option<notoir_editor::edit_ops::IndentStyle>,
}

impl LoadedText {
    /// An empty UTF-8 document with platform line endings.
    pub fn empty(language: LanguageId) -> Self {
        Self {
            text: String::new(),
            encoding: TextEncoding::UTF8,
            line_ending: LineEnding::platform_default(),
            mixed_line_endings: false,
            language,
            read_only: ReadOnly::No,
            large_file: false,
            content_hash: None,
            indentation: None,
        }
    }
}

#[derive(Debug)]
pub struct Document {
    pub id: DocumentId,
    pub origin: DocumentOrigin,
    pub buffer: TextBuffer,
    /// +1 on every applied change, including undo/redo.
    pub revision: u64,
    /// History state matching the disk content; `None` = never saved.
    pub saved_state: Option<u64>,
    pub saved_content_hash: Option<u64>,
    pub disk_state: DiskState,
    pub encoding: TextEncoding,
    pub line_ending: LineEnding,
    pub mixed_line_endings: bool,
    pub language: LanguageId,
    pub language_overridden: bool,
    pub read_only: ReadOnly,
    pub large_file: bool,
    pub indentation: Option<notoir_editor::edit_ops::IndentStyle>,
    /// Chosen from the status bar indentation picker; wins over settings and
    /// detection.
    pub indent_override: Option<notoir_editor::edit_ops::IndentStyle>,
    /// Bookmarks (spec 12), anchored at their line's start; sorted, one per line.
    pub bookmarks: Vec<Bookmark>,
    /// Recent change sets by resulting revision, so caches (display maps,
    /// highlighting) can catch up incrementally.
    recent_changes: VecDeque<(u64, Arc<ChangeSet>)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bookmark {
    /// Char offset of the bookmarked line's start.
    pub pos: usize,
    pub label: Option<String>,
}

/// Change sets kept per document for incremental cache updates.
const RECENT_CHANGES: usize = 64;

impl Document {
    pub fn new(id: DocumentId, origin: DocumentOrigin, loaded: LoadedText) -> Self {
        let buffer = TextBuffer::new(&loaded.text);
        let saved_state = matches!(origin, DocumentOrigin::File { .. }).then(|| buffer.state());
        Self {
            id,
            origin,
            buffer,
            revision: 0,
            saved_state,
            saved_content_hash: loaded.content_hash,
            disk_state: DiskState::InSync,
            encoding: loaded.encoding,
            line_ending: loaded.line_ending,
            mixed_line_endings: loaded.mixed_line_endings,
            language: loaded.language,
            language_overridden: false,
            read_only: loaded.read_only,
            large_file: loaded.large_file,
            indentation: loaded.indentation,
            indent_override: None,
            bookmarks: Vec::new(),
            recent_changes: VecDeque::new(),
        }
    }

    /// Records a change that produced `revision` (called by the workbench).
    pub fn record_change(&mut self, revision: u64, changes: Arc<ChangeSet>) {
        if self.recent_changes.len() == RECENT_CHANGES {
            self.recent_changes.pop_front();
        }
        self.recent_changes.push_back((revision, changes));
    }

    /// Change sets that turn revision `since` into the current revision, in
    /// order; `None` when the history no longer reaches back that far.
    pub fn changes_since(&self, since: u64) -> Option<Vec<Arc<ChangeSet>>> {
        if since == self.revision {
            return Some(Vec::new());
        }
        let first = self
            .recent_changes
            .iter()
            .position(|(r, _)| *r == since + 1)?;
        let out: Vec<_> = self
            .recent_changes
            .iter()
            .skip(first)
            .map(|(_, c)| c.clone())
            .collect();
        (out.len() as u64 == self.revision - since).then_some(out)
    }

    /// Unsaved relative to disk. Scratchpads persist themselves and are never
    /// dirty in the save-prompt sense; Untitled documents are dirty when non-empty.
    pub fn is_dirty(&self) -> bool {
        match &self.origin {
            DocumentOrigin::File { .. } => {
                self.saved_state != Some(self.buffer.state())
                    || self.disk_state == DiskState::DeletedExternally
            }
            DocumentOrigin::Untitled { .. } => self.buffer.len_chars() > 0,
            DocumentOrigin::Scratchpad { .. } | DocumentOrigin::Virtual { .. } => false,
        }
    }

    pub fn is_read_only(&self) -> bool {
        self.read_only != ReadOnly::No
    }

    pub fn path(&self) -> Option<&Path> {
        match &self.origin {
            DocumentOrigin::File { path, .. } => Some(path),
            _ => None,
        }
    }

    pub fn path_key(&self) -> Option<&PathKey> {
        match &self.origin {
            DocumentOrigin::File { key, .. } => Some(key),
            _ => None,
        }
    }

    /// Records that the content of history state `state` is now on disk.
    /// Bookmarked lines (0-based), ascending.
    pub fn bookmark_lines(&self) -> Vec<usize> {
        let rope = self.buffer.rope();
        self.bookmarks
            .iter()
            .map(|b| rope.char_to_line(b.pos.min(rope.len_chars())))
            .collect()
    }

    /// Adds or removes the bookmark on `line`; returns whether it is now set.
    pub fn toggle_bookmark(&mut self, line: usize) -> bool {
        let rope = self.buffer.rope();
        let line = line.min(rope.len_lines().saturating_sub(1));
        let pos = rope.line_to_char(line);
        if let Some(i) = self.bookmarks.iter().position(|b| b.pos == pos) {
            self.bookmarks.remove(i);
            return false;
        }
        self.bookmarks.push(Bookmark { pos, label: None });
        self.bookmarks.sort_by_key(|b| b.pos);
        true
    }

    /// Sets bookmarks from line numbers (session restore; reconciled by line).
    pub fn set_bookmark_lines(&mut self, lines: &[usize]) {
        let rope = self.buffer.rope();
        let last = rope.len_lines().saturating_sub(1);
        self.bookmarks = lines
            .iter()
            .map(|&l| Bookmark {
                pos: rope.line_to_char(l.min(last)),
                label: None,
            })
            .collect();
        self.normalize_bookmarks();
    }

    /// Drops bookmarks whose whole line `changes` deletes (pure deletions;
    /// needs the text `before`) and maps the rest through it. Call
    /// [`Self::normalize_bookmarks`] once the buffer holds the result.
    pub fn map_bookmarks(&mut self, before: Option<&notoir_editor::Rope>, changes: &ChangeSet) {
        if self.bookmarks.is_empty() {
            return;
        }
        if let Some(rope) = before {
            self.bookmarks.retain(|b| {
                let line = rope.char_to_line(b.pos.min(rope.len_chars()));
                let end = if line + 1 < rope.len_lines() {
                    rope.line_to_char(line + 1)
                } else {
                    rope.len_chars()
                };
                // A bookmark on the empty last line survives.
                end <= b.pos
                    || !changes
                        .edits()
                        .iter()
                        .any(|e| e.inserted_len() == 0 && e.start <= b.pos && e.end >= end)
            });
        }
        for b in &mut self.bookmarks {
            b.pos = changes.map_pos(b.pos, notoir_editor::Assoc::Before);
        }
    }

    /// Snaps bookmarks to their line's start, sorted, one per line.
    pub fn normalize_bookmarks(&mut self) {
        let rope = self.buffer.rope();
        for b in &mut self.bookmarks {
            b.pos = rope.line_to_char(rope.char_to_line(b.pos.min(rope.len_chars())));
        }
        self.bookmarks.sort_by_key(|b| b.pos);
        self.bookmarks.dedup_by_key(|b| b.pos);
    }

    pub fn mark_saved(&mut self, state: u64, content_hash: u64) {
        self.saved_state = Some(state);
        self.saved_content_hash = Some(content_hash);
        self.disk_state = DiskState::InSync;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_file_thresholds() {
        let limits = LargeFileLimits {
            size_bytes: 100,
            lines: 3,
            longest_line: 5,
            max_editable_bytes: 1000,
        };
        assert!(!limits.is_large(10, "a\nb"));
        assert!(limits.is_large(101, ""));
        assert!(limits.is_large(10, "1\n2\n3\n4"));
        assert!(limits.is_large(10, "123456"));
    }
}
