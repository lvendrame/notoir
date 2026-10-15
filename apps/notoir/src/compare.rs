//! Compare (docs/specs/27): sources, diff tabs and their sessions.
//!
//! A diff tab is a virtual document shown by a `ViewKind::Diff` view; its
//! two panes are detached views of the compared documents, so editing a side
//! edits that document (undo, dirty state, other views) like any editor.
//! Read-only sources (saved version, clipboard, text) become snapshot
//! documents that close with the tab.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};

use notoir_core::document::{DocumentOrigin, LoadedText, ReadOnly};
use notoir_core::events::ChangeOrigin;
use notoir_core::ids::{DocumentId, ViewId};
use notoir_core::notifications::{Notification, Severity, Translatable};
use notoir_core::view::ViewKind;
use notoir_core::workbench::TabPosition;
use notoir_editor::diff::{Diff, DiffAlgorithm, DiffOptions, Side, copy_hunk, rope_lines};
use notoir_editor::{EditKind, LanguageId, Rope, Selection};
use notoir_workspace::fs_io::{LoadError, Loaded};

use crate::core::{AppCore, Effect};
use crate::files::LoadPurpose;
use crate::t;

/// Virtual document title of diff tabs (the shown title is computed).
pub const DIFF_DOC_TITLE: &str = "compare:tab";
/// Both sides together up to this many lines diff on the UI thread at once;
/// larger inputs diff on a worker after a short debounce.
pub const SYNC_LINES: usize = 20_000;
/// Debounce before re-diffing large inputs after an edit.
pub const DEBOUNCE_MS: u64 = 200;

#[derive(Clone, Debug, PartialEq)]
pub enum CompareSource {
    /// An open document (live buffer, editable side).
    Document(DocumentId),
    /// A file read from disk, opened as a document (editable).
    File(PathBuf),
    /// Clipboard text, read-only.
    Clipboard,
    /// The disk content of an open file, read-only.
    SavedVersion(DocumentId),
    /// Internal text (recovery, MCP preview), read-only.
    Text { title: String, text: Arc<str> },
    /// The file as committed in HEAD, read-only (spec 28).
    GitHead(PathBuf),
    /// The file at any commit-ish, read-only.
    GitRevision(PathBuf, String),
}

/// What a read-only snapshot document is called.
#[derive(Clone, Debug)]
enum SnapshotTitle {
    Saved(String),
    Clipboard,
    Text(String),
    /// File name and revision label (`HEAD`, short SHA).
    Revision(String, String),
}

#[derive(Clone, Copy, Debug)]
enum Slot {
    Ready(DocumentId),
    Waiting,
    Failed,
}

#[derive(Debug)]
struct PendingCompare {
    id: u64,
    left: Slot,
    right: Slot,
    current_changes: bool,
}

/// One side of a diff tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pane {
    pub doc: DocumentId,
    pub view: ViewId,
}

type DiffJob = Receiver<(u64, u64, DiffOptions, Diff)>;

pub struct DiffSession {
    /// The tab's virtual document.
    pub doc: DocumentId,
    pub left: Pane,
    pub right: Pane,
    pub opts: DiffOptions,
    pub diff: Arc<Diff>,
    /// Revisions and options `diff` was computed for.
    computed: Option<(u64, u64, DiffOptions)>,
    job: Option<DiffJob>,
    /// When the sides last changed while a diff was out of date.
    stale_since: Option<u64>,
    /// Either side is over `diff.maxSize`.
    pub too_large: bool,
    /// Compare Current Changes: copying left → right is "Revert Hunk".
    pub current_changes: bool,
    /// Hunk last revealed by Next/Previous Change.
    pub current: Option<usize>,
    /// Scroll offsets (vertical, horizontal) both panes were last synced to.
    pub sync: Option<(f32, f32)>,
    /// Offsets to apply to both panes next frame (one pane scrolled).
    pub sync_pending: Option<(f32, f32)>,
    /// The pane centred on a revealed hunk: the other follows it.
    pub driver: Option<Side>,
    /// Pane colouring for the current diff.
    pub decor: Option<(
        Arc<Diff>,
        crate::editor_widget::DiffDecor,
        crate::editor_widget::DiffDecor,
    )>,
}

impl DiffSession {
    pub fn pane(&self, side: Side) -> Pane {
        match side {
            Side::Left => self.left,
            Side::Right => self.right,
        }
    }

    pub fn is_computing(&self) -> bool {
        self.job.is_some() || self.stale_since.is_some()
    }
}

#[derive(Default)]
pub struct CompareState {
    /// Diff tab view → session.
    pub sessions: HashMap<ViewId, DiffSession>,
    /// "Select for Compare".
    pub selected: Option<CompareSource>,
    /// Compare picker: the left side already chosen.
    pub pick_left: Option<CompareSource>,
    pending: Vec<PendingCompare>,
    next_id: u64,
    titles: HashMap<DocumentId, SnapshotTitle>,
}

fn file_label(p: &std::path::Path) -> String {
    p.file_name().map_or_else(
        || p.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// `HEAD` stays; full SHAs shorten to 7 chars (`<sha>~1` → `abc1234~1`); the
/// index reads "Index".
fn short_rev(rev: &str) -> String {
    if rev == crate::git::INDEX_REV {
        return t!("git:index");
    }
    let hex = rev.chars().take_while(char::is_ascii_hexdigit).count();
    if hex == 40 {
        format!("{}{}", &rev[..7], &rev[40..])
    } else {
        rev.to_owned()
    }
}

impl AppCore {
    fn diff_options(&self) -> DiffOptions {
        DiffOptions {
            ignore_trailing_whitespace: false,
            ignore_whitespace: self.settings.bool("diff.ignoreTrimWhitespace", None),
            ignore_case: false,
            algorithm: if self.settings.str("diff.algorithm", None) == "myers" {
                DiffAlgorithm::Myers
            } else {
                DiffAlgorithm::Patience
            },
        }
    }

    /// Title of a diff tab or snapshot document, if `doc` is one.
    pub fn compare_title(&self, doc: DocumentId) -> Option<String> {
        if let Some(t) = self.compare.titles.get(&doc) {
            return Some(match t {
                SnapshotTitle::Saved(name) => t!("compare:saved", name = name),
                SnapshotTitle::Clipboard => t!("compare:clipboard"),
                SnapshotTitle::Text(title) => title.clone(),
                SnapshotTitle::Revision(name, rev) => format!("{name} ({rev})"),
            });
        }
        let s = self.compare.sessions.values().find(|s| s.doc == doc)?;
        Some(format!(
            "{} \u{2194} {}",
            self.document_title(s.left.doc),
            self.document_title(s.right.doc)
        ))
    }

    /// What a source is called in the UI.
    pub fn compare_source_name(&self, src: &CompareSource) -> String {
        match src {
            CompareSource::Document(d) => self.document_title(*d),
            CompareSource::File(p) => p.file_name().map_or_else(
                || p.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            ),
            CompareSource::Clipboard => t!("compare:clipboard"),
            CompareSource::SavedVersion(d) => t!("compare:saved", name = self.document_title(*d)),
            CompareSource::Text { title, .. } => title.clone(),
            CompareSource::GitHead(p) => format!("{} (HEAD)", file_label(p)),
            CompareSource::GitRevision(p, rev) => format!("{} ({})", file_label(p), short_rev(rev)),
        }
    }

    /// The diff session of the active tab.
    pub fn active_diff(&self) -> Option<ViewId> {
        let v = self.workbench.active_view()?;
        self.compare.sessions.contains_key(&v).then_some(v)
    }

    fn compare_snapshot(
        &mut self,
        title: SnapshotTitle,
        text: &str,
        language: LanguageId,
    ) -> DocumentId {
        let mut loaded = LoadedText::empty(language);
        loaded.text = notoir_editor::line_ending::normalize(text);
        loaded.read_only = ReadOnly::Snapshot;
        let doc = self.workbench.open_document(
            DocumentOrigin::Virtual {
                title: "compare:snapshot".into(),
            },
            loaded,
        );
        self.compare.titles.insert(doc, title);
        doc
    }

    fn resolve(&mut self, src: CompareSource, id: u64, left: bool) -> Slot {
        match src {
            CompareSource::Document(d) => match self.workbench.document(d) {
                Some(_) => Slot::Ready(d),
                None => Slot::Failed,
            },
            CompareSource::File(path) => {
                let (_, key) = notoir_workspace::fs_io::identity(&path);
                if let Some(d) = self.workbench.find_by_key(&key) {
                    return Slot::Ready(d);
                }
                self.load_for(
                    path,
                    LoadPurpose::Compare {
                        id,
                        left,
                        saved_of: None,
                    },
                );
                Slot::Waiting
            }
            CompareSource::SavedVersion(d) => {
                match self.workbench.document(d).and_then(|d| d.path()) {
                    Some(p) => {
                        let p = p.to_path_buf();
                        self.load_for(
                            p,
                            LoadPurpose::Compare {
                                id,
                                left,
                                saved_of: Some(d),
                            },
                        );
                        Slot::Waiting
                    }
                    None => {
                        let now = self.now_ms;
                        self.notifications.notify(
                            Notification::new(
                                Severity::Info,
                                Translatable::new("notifications:compareNoSavedVersion"),
                            ),
                            now,
                        );
                        Slot::Failed
                    }
                }
            }
            CompareSource::Clipboard => {
                match arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
                    Ok(text) => Slot::Ready(self.compare_snapshot(
                        SnapshotTitle::Clipboard,
                        &text,
                        LanguageId::PLAINTEXT,
                    )),
                    Err(e) => {
                        tracing::warn!(error = %e, "clipboard unavailable");
                        let now = self.now_ms;
                        self.notifications.notify(
                            Notification::new(
                                Severity::Info,
                                Translatable::new("notifications:compareNoClipboard"),
                            ),
                            now,
                        );
                        Slot::Failed
                    }
                }
            }
            CompareSource::GitHead(path) => self.resolve_revision(id, left, path, "HEAD".into()),
            CompareSource::GitRevision(path, rev) => self.resolve_revision(id, left, path, rev),
            CompareSource::Text { title, text } => Slot::Ready(self.compare_snapshot(
                SnapshotTitle::Text(title),
                &text,
                LanguageId::PLAINTEXT,
            )),
        }
    }

    fn resolve_revision(&mut self, id: u64, left: bool, path: PathBuf, rev: String) -> Slot {
        let purpose = crate::git::BlobPurpose::Compare {
            id,
            left,
            path,
            rev,
        };
        if self.git_read_blob(purpose) {
            Slot::Waiting
        } else {
            Slot::Failed
        }
    }

    /// A Git revision side finished loading (`None`: the read failed).
    pub fn compare_revision_loaded(
        &mut self,
        id: u64,
        left: bool,
        path: &std::path::Path,
        rev: &str,
        text: Option<String>,
    ) {
        let slot = match text {
            Some(text) => {
                let language =
                    notoir_editor::language::detect(Some(&path.to_string_lossy()), "", &[]);
                Slot::Ready(self.compare_snapshot(
                    SnapshotTitle::Revision(file_label(path), short_rev(rev)),
                    &text,
                    language,
                ))
            }
            None => Slot::Failed,
        };
        self.set_slot(id, left, slot);
    }

    /// Open File at Revision: a read-only tab.
    pub fn open_revision(&mut self, path: &std::path::Path, rev: &str, text: &str) {
        let language = notoir_editor::language::detect(Some(&path.to_string_lossy()), "", &[]);
        let doc = self.compare_snapshot(
            SnapshotTitle::Revision(file_label(path), short_rev(rev)),
            text,
            language,
        );
        let group = self.workbench.layout.active_group;
        if let Ok(v) = self
            .workbench
            .open_view(doc, group, TabPosition::AfterActive)
        {
            self.effects.push(Effect::FocusEditor(v));
        }
    }

    fn set_slot(&mut self, id: u64, left: bool, slot: Slot) {
        if let Some(p) = self.compare.pending.iter_mut().find(|p| p.id == id) {
            if left {
                p.left = slot;
            } else {
                p.right = slot;
            }
        } else if let Slot::Ready(doc) = slot {
            self.drop_compare_doc(doc);
        }
        self.finish_pending(id);
    }

    /// Opens a diff tab of `left` ↔ `right` once both sides are available.
    pub fn compare(&mut self, left: CompareSource, right: CompareSource, current_changes: bool) {
        self.compare.next_id += 1;
        let id = self.compare.next_id;
        let l = self.resolve(left, id, true);
        let r = self.resolve(right, id, false);
        self.compare.pending.push(PendingCompare {
            id,
            left: l,
            right: r,
            current_changes,
        });
        self.finish_pending(id);
    }

    /// Compare Current Changes: saved version (left) ↔ buffer (right).
    pub fn compare_current_changes(&mut self, doc: DocumentId) {
        self.compare(
            CompareSource::SavedVersion(doc),
            CompareSource::Document(doc),
            true,
        );
    }

    /// A Compare source finished loading.
    pub fn compare_loaded(
        &mut self,
        id: u64,
        left: bool,
        saved_of: Option<DocumentId>,
        result: Result<Loaded, LoadError>,
    ) {
        let slot = match result {
            Ok(loaded) => match saved_of {
                Some(of) => {
                    let name = self.document_title(of);
                    let language = self
                        .workbench
                        .document(of)
                        .map_or(loaded.text.language, |d| d.language);
                    Slot::Ready(self.compare_snapshot(
                        SnapshotTitle::Saved(name),
                        &loaded.text.text,
                        language,
                    ))
                }
                None => {
                    let origin = DocumentOrigin::File {
                        path: loaded.path.clone(),
                        key: loaded.key.clone(),
                    };
                    Slot::Ready(self.workbench.open_document(origin, loaded.text))
                }
            },
            Err(e) => {
                tracing::info!(error = %e, "compare source not loaded");
                let now = self.now_ms;
                self.notifications.notify(
                    Notification::new(
                        Severity::Warning,
                        Translatable::new("notifications:compareLoadFailed"),
                    )
                    .detail(e.to_string()),
                    now,
                );
                Slot::Failed
            }
        };
        self.set_slot(id, left, slot);
    }

    fn drop_compare_doc(&mut self, doc: DocumentId) {
        if self.workbench.discard_unused_document(doc) {
            self.compare.titles.remove(&doc);
        }
    }

    fn finish_pending(&mut self, id: u64) {
        let Some(i) = self.compare.pending.iter().position(|p| p.id == id) else {
            return;
        };
        let p = &self.compare.pending[i];
        match (p.left, p.right) {
            (Slot::Ready(l), Slot::Ready(r)) => {
                let cc = p.current_changes;
                self.compare.pending.remove(i);
                self.open_diff_tab(l, r, cc);
            }
            (Slot::Failed, other) | (other, Slot::Failed) => {
                // A side still loading is dropped when it arrives.
                self.compare.pending.remove(i);
                if let Slot::Ready(d) = other {
                    self.drop_compare_doc(d);
                }
            }
            _ => {}
        }
    }

    fn open_diff_tab(&mut self, left: DocumentId, right: DocumentId, current_changes: bool) {
        let doc = self.workbench.open_document(
            DocumentOrigin::Virtual {
                title: DIFF_DOC_TITLE.into(),
            },
            LoadedText::empty(LanguageId::PLAINTEXT),
        );
        let group = self.workbench.layout.active_group;
        let Ok(tab) = self
            .workbench
            .open_view(doc, group, TabPosition::AfterActive)
        else {
            return;
        };
        if let Some(v) = self.workbench.views.get_mut(tab) {
            v.kind = ViewKind::Diff;
        }
        let (Ok(lv), Ok(rv)) = (
            self.workbench.open_detached_view(left),
            self.workbench.open_detached_view(right),
        ) else {
            return;
        };
        self.workbench.pane_focus.insert(tab, rv);
        let mut s = DiffSession {
            doc,
            left: Pane {
                doc: left,
                view: lv,
            },
            right: Pane {
                doc: right,
                view: rv,
            },
            opts: self.diff_options(),
            diff: Arc::new(Diff::default()),
            computed: None,
            job: None,
            stale_since: None,
            too_large: false,
            current_changes,
            current: None,
            sync: None,
            sync_pending: None,
            driver: None,
            decor: None,
        };
        self.recompute(&mut s, true);
        self.compare.sessions.insert(tab, s);
        let _ = self.workbench.activate_view(tab);
        self.effects.push(Effect::FocusEditor(rv));
    }

    /// Brings a session's diff up to date: small inputs now, large ones on a
    /// worker after a debounce. `force` diffs now whatever the size.
    fn recompute(&mut self, s: &mut DiffSession, force: bool) {
        let (Some(l), Some(r)) = (
            self.workbench.document(s.left.doc),
            self.workbench.document(s.right.doc),
        ) else {
            return;
        };
        let key = (l.revision, r.revision, s.opts);
        if s.computed == Some(key) {
            s.stale_since = None;
            return;
        }
        let max = self.settings.int("diff.maxSize", None).max(1) as usize;
        let (lr, rr) = (l.buffer.rope().clone(), r.buffer.rope().clone());
        s.too_large = lr.len_bytes() > max || rr.len_bytes() > max;
        if s.too_large {
            s.diff = Arc::new(Diff::default());
            s.computed = Some(key);
            s.stale_since = None;
            return;
        }
        let small = lr.len_lines() + rr.len_lines() <= SYNC_LINES;
        if small || force {
            s.diff = Arc::new(notoir_editor::diff::diff_ropes(&lr, &rr, &s.opts));
            s.computed = Some(key);
            s.stale_since = None;
            s.job = None;
            return;
        }
        if s.job.is_some() {
            return;
        }
        let now = self.now_ms;
        let since = *s.stale_since.get_or_insert(now);
        if now.saturating_sub(since) < DEBOUNCE_MS {
            return;
        }
        let (tx, rx) = channel();
        let opts = s.opts;
        let poster = self.io.poster();
        let spawned = std::thread::Builder::new()
            .name("notoir-diff".into())
            .spawn(move || {
                let d = notoir_editor::diff::diff_lines(
                    &rope_lines(&lr),
                    &rope_lines(&rr),
                    &opts,
                    Some(std::time::Instant::now() + std::time::Duration::from_secs(5)),
                );
                let _ = tx.send((key.0, key.1, opts, d));
                poster.wake();
            });
        match spawned {
            Ok(_) => s.job = Some(rx),
            Err(e) => tracing::error!(error = %e, "could not start diff thread"),
        }
    }

    /// Per frame: closes sessions whose tab closed, collects worker results
    /// and re-diffs edited sides.
    pub fn compare_tick(&mut self) {
        // Snapshot tabs (Open at Revision) that were closed.
        let wb = &self.workbench;
        self.compare.titles.retain(|d, _| wb.document(*d).is_some());
        let tabs: Vec<ViewId> = self.compare.sessions.keys().copied().collect();
        for tab in tabs {
            if self.workbench.view(tab).is_none() {
                self.close_diff_session(tab);
                continue;
            }
            let Some(mut s) = self.compare.sessions.remove(&tab) else {
                continue;
            };
            if let Some(rx) = &s.job
                && let Ok((lrev, rrev, opts, d)) = rx.try_recv()
            {
                s.job = None;
                s.diff = Arc::new(d);
                s.computed = Some((lrev, rrev, opts));
            }
            self.recompute(&mut s, false);
            if s.current.is_some_and(|i| i >= s.diff.hunks.len()) {
                s.current = None;
            }
            self.compare.sessions.insert(tab, s);
        }
    }

    pub fn compare_needs_repaint(&self) -> bool {
        self.compare
            .sessions
            .values()
            .any(DiffSession::is_computing)
    }

    fn close_diff_session(&mut self, tab: ViewId) {
        let Some(s) = self.compare.sessions.remove(&tab) else {
            return;
        };
        self.workbench.pane_focus.remove(&tab);
        for pane in [s.left, s.right] {
            // A dirty document no other view shows stays open in a tab.
            if let Some(doc) = self.workbench.close_detached_view(pane.view) {
                let group = self.workbench.layout.active_group;
                let _ = self
                    .workbench
                    .open_view(doc, group, TabPosition::AfterActive);
            }
            if self.workbench.document(pane.doc).is_none() {
                self.compare.titles.remove(&pane.doc);
            }
        }
    }

    /// Next / Previous Change: reveals the hunk in both panes.
    pub fn compare_step(&mut self, tab: ViewId, forward: bool) {
        let focus = self.diff_focus_side(tab);
        let Some(s) = self.compare.sessions.get(&tab) else {
            return;
        };
        let n = s.diff.hunks.len();
        if n == 0 {
            return;
        }
        let pane = s.pane(focus);
        let caret_line = self
            .workbench
            .view(pane.view)
            .zip(self.workbench.document(pane.doc))
            .map_or(0, |(v, d)| {
                let rope = d.buffer.rope();
                rope.char_to_line(v.selection.primary().head.min(rope.len_chars()))
            });
        // Keep stepping from the revealed hunk while the caret stays on it.
        let on_current = s
            .current
            .and_then(|i| s.diff.hunks.get(i))
            .is_some_and(|h| h.lines(focus).start == caret_line);
        let next = match (s.current, on_current) {
            (Some(i), true) if forward => (i + 1) % n,
            (Some(i), true) => (i + n - 1) % n,
            _ if forward => s.diff.next_hunk(focus, caret_line, true).unwrap_or(0),
            _ => s.diff.prev_hunk(focus, caret_line).unwrap_or(n - 1),
        };
        self.reveal_hunk(tab, next);
    }

    /// Puts both carets at hunk `i` and centres it.
    pub fn reveal_hunk(&mut self, tab: ViewId, i: usize) {
        let focus = self.diff_focus_side(tab);
        let Some(s) = self.compare.sessions.get_mut(&tab) else {
            return;
        };
        let Some(h) = s.diff.hunks.get(i).cloned() else {
            return;
        };
        s.current = Some(i);
        // The side with lines is centred (the other has fillers there) and
        // the other pane follows its scroll.
        let driver = if !h.lines(focus).is_empty() {
            focus
        } else {
            focus.other()
        };
        s.driver = Some(driver);
        let panes = [(Side::Left, s.left), (Side::Right, s.right)];
        for (side, pane) in panes {
            let Some(rope) = self
                .workbench
                .document(pane.doc)
                .map(|d| d.buffer.rope().clone())
            else {
                continue;
            };
            let line = h.lines(side).start.min(rope.len_lines().saturating_sub(1));
            let _ = self
                .workbench
                .set_selection(pane.view, Selection::caret(rope.line_to_char(line)));
            if side == driver {
                self.find.center.push(pane.view);
            }
        }
    }

    /// Which side of a diff tab has (or last had) focus; right by default.
    pub fn diff_focus_side(&self, tab: ViewId) -> Side {
        match (
            self.compare.sessions.get(&tab),
            self.workbench.pane_focus.get(&tab),
        ) {
            (Some(s), Some(v)) if *v == s.left.view => Side::Left,
            _ => Side::Right,
        }
    }

    /// Whether a side of a diff can be edited.
    pub fn diff_side_editable(&self, tab: ViewId, side: Side) -> bool {
        self.compare
            .sessions
            .get(&tab)
            .and_then(|s| self.workbench.document(s.pane(side).doc))
            .is_some_and(|d| !d.is_read_only())
    }

    /// Copies hunk `i` onto `toward` (from the other side), as one undoable
    /// change on that document.
    pub fn compare_copy_hunk(&mut self, tab: ViewId, i: usize, toward: Side) -> bool {
        if !self.diff_side_editable(tab, toward) {
            return false;
        }
        let Some(mut s) = self.compare.sessions.remove(&tab) else {
            return false;
        };
        self.recompute(&mut s, true);
        let ok = self.copy_hunk_now(&s, i, toward);
        if ok {
            self.recompute(&mut s, true);
        }
        self.compare.sessions.insert(tab, s);
        ok
    }

    fn copy_hunk_now(&mut self, s: &DiffSession, i: usize, toward: Side) -> bool {
        let Some(h) = s.diff.hunks.get(i) else {
            return false;
        };
        let from = s.pane(toward.other());
        let to = s.pane(toward);
        let (Some(fr), Some(tr)) = (
            self.workbench
                .document(from.doc)
                .map(|d| d.buffer.rope().clone()),
            self.workbench
                .document(to.doc)
                .map(|d| d.buffer.rope().clone()),
        ) else {
            return false;
        };
        let cs = copy_hunk(h, &fr, toward.other(), &tr);
        if cs.is_empty() {
            return false;
        }
        let caret = cs.edits().first().map_or(0, |e| e.start);
        let now = self.now_ms;
        self.workbench
            .apply_edit(
                to.doc,
                Some(to.view),
                cs,
                Selection::caret(caret),
                EditKind::Other,
                ChangeOrigin::User { view: to.view },
                now,
            )
            .is_ok()
    }

    /// Swap Sides.
    pub fn compare_swap(&mut self, tab: ViewId) {
        let Some(mut s) = self.compare.sessions.remove(&tab) else {
            return;
        };
        std::mem::swap(&mut s.left, &mut s.right);
        s.current_changes = false;
        s.current = None;
        s.sync = None;
        s.sync_pending = None;
        self.recompute(&mut s, true);
        self.compare.sessions.insert(tab, s);
    }

    /// Ignore Whitespace / Ignore Case toggles.
    pub fn compare_toggle(&mut self, tab: ViewId, f: impl FnOnce(&mut DiffOptions)) {
        let Some(mut s) = self.compare.sessions.remove(&tab) else {
            return;
        };
        f(&mut s.opts);
        s.current = None;
        self.recompute(&mut s, true);
        self.compare.sessions.insert(tab, s);
    }

    /// Ropes of both sides (tests).
    pub fn diff_texts(&self, tab: ViewId) -> Option<(Rope, Rope)> {
        let s = self.compare.sessions.get(&tab)?;
        Some((
            self.workbench.document(s.left.doc)?.buffer.rope().clone(),
            self.workbench.document(s.right.doc)?.buffer.rope().clone(),
        ))
    }
}
