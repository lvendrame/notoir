//! Recovery and hot exit (docs/specs/09 §Recovery): unsaved documents are
//! snapshotted into this window's recovery session (1.5 s after the last edit,
//! at least every 10 s while typing, on focus loss and on quit). At startup,
//! sessions left by crashed processes are offered in the recovery prompt;
//! hot-exit sessions are restored silently into their workspace.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use notoir_core::DocumentOrigin;
use notoir_core::document::{DiskState, LoadedText};
use notoir_core::ids::DocumentId;
use notoir_core::notifications::{Notification, Severity, Translatable};
use notoir_core::workbench::TabPosition;
use notoir_editor::encoding::TextEncoding;
use notoir_editor::language::LanguageId;
use notoir_editor::{ChangeSet, EditKind, LineEnding, Selection};
use notoir_storage::atomic::content_hash;
use notoir_storage::recovery::{
    self, EntryKind, EntryMeta, EntryStatus, OrphanSession, RecoveredEntry, RecoverySession,
    hash_label,
};

use crate::core::AppCore;
use crate::io::IoResult;

/// Snapshot this long after the last edit…
pub const DEBOUNCE_MS: u64 = 1500;
/// …but at least this often while edits keep coming.
pub const MAX_INTERVAL_MS: u64 = 10_000;

/// A document's banner above the editor (external changes, recovered
/// conflicts; docs/specs/10).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BannerKind {
    /// Changed on disk while dirty: Reload / Keep My Version.
    Changed,
    /// Deleted on disk: Save / Close.
    Deleted,
    /// Restored from recovery but the disk file changed after the snapshot:
    /// Keep Recovered / Use Disk Version.
    RecoveredConflict,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Banner {
    pub kind: BannerKind,
    /// Hash of the current disk content (Keep My Version records it).
    pub disk_hash: Option<u64>,
}

pub struct RecoveryState {
    pub root: PathBuf,
    pub session: Option<Arc<Mutex<RecoverySession>>>,
    /// Document → entry id in this session.
    pub entries: HashMap<DocumentId, String>,
    /// Document → (debounced due, latest allowed) snapshot times.
    due: HashMap<DocumentId, (u64, u64)>,
    /// Revision last snapshotted per document.
    snapped: HashMap<DocumentId, u64>,
    /// Crash sessions waiting for Restore All / Review / Discard.
    pub prompt: Vec<OrphanSession>,
    /// The Review list is open.
    pub review: bool,
    /// Entries already restored or discarded from the Review list.
    pub handled: Vec<String>,
    /// Hot-exit sessions of other workspaces (restored when opened).
    pub other_hot: Vec<OrphanSession>,
    /// Entry id of the previous session → restored Untitled document (session
    /// restore finds Untitled tabs by it).
    pub restored_untitled: HashMap<String, DocumentId>,
    write_failed: bool,
}

impl RecoveryState {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            session: None,
            entries: HashMap::new(),
            due: HashMap::new(),
            snapped: HashMap::new(),
            prompt: Vec::new(),
            review: false,
            handled: Vec::new(),
            other_hot: Vec::new(),
            restored_untitled: HashMap::new(),
            write_failed: false,
        }
    }

    /// Entries listed by the prompt (not yet handled in Review).
    pub fn prompt_entries(&self) -> Vec<&RecoveredEntry> {
        self.prompt
            .iter()
            .flat_map(|s| s.entries.iter())
            .filter(|e| !self.handled.contains(&e.id))
            .filter(|e| {
                e.meta
                    .as_ref()
                    .is_none_or(|m| m.kind != EntryKind::Scratchpad)
            })
            .collect()
    }
}

fn epoch_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

fn eol_name(e: LineEnding) -> &'static str {
    match e {
        LineEnding::Lf => "lf",
        LineEnding::CrLf => "crlf",
        LineEnding::Cr => "cr",
    }
}

fn eol_from(s: &str) -> LineEnding {
    match s {
        "crlf" => LineEnding::CrLf,
        "cr" => LineEnding::Cr,
        _ => LineEnding::Lf,
    }
}

/// The disk file's content hash now (`None` when it is missing).
fn disk_hash(path: &Path) -> Option<u64> {
    std::fs::read(path).ok().map(|b| content_hash(&b))
}

impl AppCore {
    /// Startup: own session, purge old discarded sessions, find orphans.
    pub fn recovery_start(&mut self) {
        let root = self.recovery.root.clone();
        let now = epoch_ms();
        match RecoverySession::create(&root, env!("CARGO_PKG_VERSION"), None, now) {
            Ok(s) => self.recovery.session = Some(Arc::new(Mutex::new(s))),
            Err(e) => {
                tracing::error!(error = %e, "recovery session unavailable");
                let n = self.now_ms;
                self.notifications.notify(
                    Notification::new(
                        Severity::Error,
                        Translatable::new("notifications:recoveryUnavailable"),
                    )
                    .detail(e.to_string()),
                    n,
                );
            }
        }
        if let Err(e) = recovery::purge_discarded(&root, recovery::DISCARD_RETENTION_MS, now) {
            tracing::warn!(error = %e, "could not purge discarded recovery sessions");
        }
        self.scan_orphans();
    }

    fn own_session_id(&self) -> Option<String> {
        self.recovery
            .session
            .as_ref()
            .map(|s| s.lock().unwrap_or_else(|e| e.into_inner()).id().to_owned())
    }

    /// Splits orphan sessions into crashes (prompt) and hot exits.
    pub fn scan_orphans(&mut self) {
        let own = self.own_session_id();
        let found = recovery::scan(&self.recovery.root, own.as_deref()).unwrap_or_default();
        let (hot, crashed): (Vec<_>, Vec<_>) = found.into_iter().partition(|s| s.clean_exit());
        self.recovery.other_hot = hot;
        self.recovery.prompt = crashed;
        self.recovery.handled.clear();
        if !self.recovery.prompt.is_empty() {
            tracing::info!(
                sessions = self.recovery.prompt.len(),
                "unsaved documents from a crashed session"
            );
        }
    }

    /// The workspace key recovery sessions are matched by.
    pub fn recovery_workspace_key(&self) -> Option<String> {
        self.workspace.as_ref().map(|w| w.key.clone())
    }

    /// Restores hot-exit sessions of the current workspace (or of "no
    /// workspace") silently.
    pub fn restore_hot_exit(&mut self) {
        let key = self.recovery_workspace_key();
        let (mine, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut self.recovery.other_hot)
            .into_iter()
            .partition(|s| s.manifest.as_ref().and_then(|m| m.workspace.clone()) == key);
        self.recovery.other_hot = rest;
        for s in mine {
            let entries: Vec<RecoveredEntry> = s.entries.clone();
            self.restore_entries(&entries);
            self.remove_orphan_after_snapshots(s);
        }
        let key = self.recovery_workspace_key();
        if let Some(session) = self.recovery.session.clone() {
            let _ = session
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .set_workspace(key.as_deref(), epoch_ms());
        }
    }

    /// Removes an orphan once the restored documents are in our session (the
    /// IO queue runs the snapshots first).
    fn remove_orphan_after_snapshots(&mut self, s: OrphanSession) {
        self.flush_recovery();
        self.io.submit(move || {
            if let Err(e) = recovery::remove_orphan(&s) {
                tracing::warn!(error = %e, "could not remove restored recovery session");
            }
            None
        });
    }

    /// Opens recovered entries as dirty documents. File entries whose disk
    /// file changed since the snapshot open with the conflict banner.
    pub fn restore_entries(&mut self, entries: &[RecoveredEntry]) -> Vec<DocumentId> {
        let mut out = Vec::new();
        let group = self.workbench.layout.active_group;
        for e in entries {
            let Some(content) = e.content.clone() else {
                continue;
            };
            let meta = e.meta.clone();
            if meta
                .as_ref()
                .is_some_and(|m| m.kind == EntryKind::Scratchpad)
            {
                continue;
            }
            let damaged = matches!(e.status, EntryStatus::Damaged(_));
            let file_path = meta
                .as_ref()
                .filter(|m| m.kind == EntryKind::File && !damaged)
                .and_then(|m| m.path.clone())
                .map(PathBuf::from);
            let encoding = meta
                .as_ref()
                .and_then(|m| encoding_rs::Encoding::for_label(m.encoding.as_bytes()))
                .map(|enc| TextEncoding {
                    encoding: enc,
                    bom: meta.as_ref().is_some_and(|m| m.bom),
                })
                .unwrap_or(TextEncoding::UTF8);
            let language = meta
                .as_ref()
                .and_then(|m| LanguageId::from_id(&m.language))
                .unwrap_or(LanguageId::PLAINTEXT);
            let mut text = LoadedText::empty(language);
            text.text = content.clone();
            text.encoding = encoding;
            text.line_ending = meta
                .as_ref()
                .map_or(LineEnding::platform_default(), |m| eol_from(&m.line_ending));
            let doc = match &file_path {
                Some(path) => {
                    let (canonical, key) = notoir_workspace::fs_io::identity(path);
                    if let Some(existing) = self.workbench.find_by_key(&key) {
                        // Already open (e.g. from the command line): put the
                        // recovered text in as an undoable edit.
                        let len = self
                            .workbench
                            .document(existing)
                            .map_or(0, |d| d.buffer.len_chars());
                        let now = self.now_ms;
                        let _ = self.workbench.apply_edit(
                            existing,
                            None,
                            ChangeSet::replace(0, len, content.as_str()),
                            Selection::caret(0),
                            EditKind::Other,
                            notoir_core::ChangeOrigin::Reload,
                            now,
                        );
                        existing
                    } else {
                        self.workbench.open_document(
                            DocumentOrigin::File {
                                path: canonical,
                                key,
                            },
                            text,
                        )
                    }
                }
                None => {
                    let n = self.workbench.next_untitled_number();
                    self.workbench
                        .open_document(DocumentOrigin::Untitled { n }, text)
                }
            };
            // Dirty against the disk version the edits were based on.
            let base = meta
                .as_ref()
                .and_then(|m| m.base_disk_hash.as_deref())
                .and_then(|h| h.strip_prefix("xxh3:"))
                .and_then(|h| u64::from_str_radix(h, 16).ok());
            if let Some(path) = &file_path {
                let now_hash = disk_hash(path);
                if let Some(d) = self.workbench.document_mut(doc) {
                    d.saved_state = None;
                    d.saved_content_hash = base;
                    d.disk_state = match (now_hash, base) {
                        (None, _) => DiskState::DeletedExternally,
                        (Some(h), Some(b)) if h != b => DiskState::ModifiedExternally,
                        _ => DiskState::InSync,
                    };
                }
                match (now_hash, base) {
                    (None, _) => {
                        self.banners.insert(
                            doc,
                            crate::recovery::Banner {
                                kind: BannerKind::Deleted,
                                disk_hash: None,
                            },
                        );
                    }
                    (Some(h), Some(b)) if h != b => {
                        self.banners.insert(
                            doc,
                            Banner {
                                kind: BannerKind::RecoveredConflict,
                                disk_hash: Some(h),
                            },
                        );
                    }
                    _ => {}
                }
            }
            if meta.as_ref().is_some_and(|m| m.kind == EntryKind::Untitled) || damaged {
                self.recovery.restored_untitled.insert(e.id.clone(), doc);
            }
            if damaged {
                let now = self.now_ms;
                self.notifications.notify(
                    Notification::new(
                        Severity::Warning,
                        Translatable::new("notifications:recoveryDamaged"),
                    )
                    .detail(format!("{}: {:?}", e.id, e.status)),
                    now,
                );
            }
            if self.workbench.views.count_of_document(doc) == 0
                && let Ok(view) = self
                    .workbench
                    .open_view(doc, group, TabPosition::AfterActive)
            {
                let _ = self.workbench.activate_view(view);
            }
            self.recovery.due.insert(doc, (0, 0));
            out.push(doc);
        }
        out
    }

    // ── prompt answers ───────────────────────────────────────────────────

    pub fn recovery_restore_all(&mut self) {
        let sessions = std::mem::take(&mut self.recovery.prompt);
        let handled = std::mem::take(&mut self.recovery.handled);
        for s in sessions {
            let entries: Vec<RecoveredEntry> = s
                .entries
                .iter()
                .filter(|e| !handled.contains(&e.id))
                .cloned()
                .collect();
            self.restore_entries(&entries);
            self.remove_orphan_after_snapshots(s);
        }
        self.recovery.review = false;
    }

    /// Discard (after confirmation): sessions move to `.discarded/` (7 days).
    pub fn recovery_discard_all(&mut self) {
        let root = self.recovery.root.clone();
        for s in std::mem::take(&mut self.recovery.prompt) {
            if let Err(e) = recovery::discard(&root, &s.dir, epoch_ms()) {
                tracing::warn!(error = %e, "could not discard recovery session");
            }
        }
        self.recovery.handled.clear();
        self.recovery.review = false;
    }

    /// Review list: restore or discard one entry.
    pub fn recovery_entry(&mut self, id: &str, restore: bool) {
        let entry = self
            .recovery
            .prompt
            .iter()
            .flat_map(|s| s.entries.iter())
            .find(|e| e.id == id)
            .cloned();
        if let Some(e) = entry {
            if restore {
                self.restore_entries(std::slice::from_ref(&e));
                self.flush_recovery();
            }
            self.recovery.handled.push(e.id);
        }
        if self.recovery.prompt_entries().is_empty() {
            // Everything decided: keep what is left for 7 days, like Discard.
            self.recovery_discard_all();
        }
    }

    // ── snapshots ────────────────────────────────────────────────────────

    /// Documents that belong in the recovery store: dirty files and
    /// non-empty Untitled documents.
    fn snapshottable(&self, doc: DocumentId) -> bool {
        self.workbench
            .document(doc)
            .is_some_and(|d| match d.origin {
                DocumentOrigin::File { .. } => d.is_dirty(),
                DocumentOrigin::Untitled { .. } => d.buffer.len_chars() > 0 || d.is_dirty(),
                _ => false,
            })
    }

    /// A document changed: schedule its snapshot (or drop its entry when it
    /// became clean).
    pub fn recovery_doc_changed(&mut self, doc: DocumentId) {
        if self.snapshottable(doc) {
            let now = self.now_ms;
            let hard = self
                .recovery
                .due
                .get(&doc)
                .map_or(now + MAX_INTERVAL_MS, |d| d.1);
            self.recovery.due.insert(doc, (now + DEBOUNCE_MS, hard));
        } else {
            self.recovery_forget(doc);
        }
    }

    /// Saved, reverted, closed with Don't Save, or closed: no entry any more.
    pub fn recovery_forget(&mut self, doc: DocumentId) {
        self.recovery.due.remove(&doc);
        self.recovery.snapped.remove(&doc);
        let Some(entry) = self.recovery.entries.remove(&doc) else {
            return;
        };
        let Some(session) = self.recovery.session.clone() else {
            return;
        };
        self.io.submit(move || {
            let r = session
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove_entry(&entry, epoch_ms());
            if let Err(e) = r {
                tracing::warn!(error = %e, "could not remove recovery entry");
            }
            None
        });
    }

    pub fn recovery_tick(&mut self) {
        let now = self.now_ms;
        let ready: Vec<DocumentId> = self
            .recovery
            .due
            .iter()
            .filter(|(_, (soft, hard))| now >= *soft.min(hard))
            .map(|(d, _)| *d)
            .collect();
        for doc in ready {
            self.recovery.due.remove(&doc);
            self.snapshot(doc);
        }
    }

    pub fn recovery_next_due(&self) -> Option<u64> {
        self.recovery.due.values().map(|(s, h)| *s.min(h)).min()
    }

    /// Snapshots every unsaved document now (focus loss, quit, tests).
    pub fn flush_recovery(&mut self) {
        // Scratchpads have their own store; write pending edits now too.
        self.flush_scratchpads(true);
        let docs: Vec<DocumentId> = self.workbench.documents.keys().copied().collect();
        for doc in docs {
            self.recovery.due.remove(&doc);
            if self.snapshottable(doc) {
                self.snapshot(doc);
            } else {
                self.recovery_forget(doc);
            }
        }
    }

    fn snapshot(&mut self, doc: DocumentId) {
        let Some(session) = self.recovery.session.clone() else {
            return;
        };
        let Some(d) = self.workbench.document(doc) else {
            return;
        };
        if !self.snapshottable(doc) {
            return;
        }
        if self.recovery.snapped.get(&doc) == Some(&d.revision)
            && self.recovery.entries.contains_key(&doc)
        {
            return;
        }
        let (kind, path) = match &d.origin {
            DocumentOrigin::File { path, .. } => {
                (EntryKind::File, Some(path.to_string_lossy().into_owned()))
            }
            _ => (EntryKind::Untitled, None),
        };
        let meta = EntryMeta {
            format: recovery::FORMAT,
            kind,
            path,
            scratch_id: None,
            title: self.document_title(doc),
            revision: d.revision,
            base_disk_hash: d.saved_content_hash.map(hash_label),
            base_disk_mtime_ms: None,
            encoding: d.encoding.encoding.name().to_owned(),
            bom: d.encoding.bom,
            line_ending: eol_name(d.line_ending).to_owned(),
            language: d.language.as_str().to_owned(),
            content_hash: String::new(),
            snapshot_at_ms: epoch_ms(),
        };
        let rope = d.buffer.rope().clone();
        let revision = d.revision;
        let entry = self
            .recovery
            .entries
            .entry(doc)
            .or_insert_with(|| uuid::Uuid::new_v4().simple().to_string())
            .clone();
        self.recovery.snapped.insert(doc, revision);
        self.io.submit(move || {
            let text = rope.to_string();
            let r = session
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .write_entry(&entry, meta, &text, epoch_ms());
            r.err().map(|e| IoResult::RecoveryWriteFailed {
                error: e.to_string(),
            })
        });
    }

    pub fn recovery_write_failed(&mut self, error: String) {
        tracing::error!(%error, "recovery snapshot failed");
        if !self.recovery.write_failed {
            self.recovery.write_failed = true;
            let now = self.now_ms;
            self.notifications.notify(
                Notification::new(
                    Severity::Error,
                    Translatable::new("notifications:recoveryWriteFailed"),
                )
                .detail(error)
                .dedupe("recovery-write"),
                now,
            );
        }
    }

    /// Process exit: with hot exit, unsaved work stays in the session for the
    /// next launch; otherwise the session is removed.
    pub fn recovery_shutdown(&mut self) {
        let hot = self.settings.bool("app.hotExit", None);
        if hot {
            self.flush_recovery();
        }
        self.flush_io();
        let Some(session) = self.recovery.session.take() else {
            return;
        };
        let Ok(session) =
            Arc::try_unwrap(session).map(|m| m.into_inner().unwrap_or_else(|e| e.into_inner()))
        else {
            return;
        };
        if hot && !session.manifest().entries.is_empty() {
            let mut session = session;
            if let Err(e) = session.mark_clean_exit(epoch_ms()) {
                tracing::error!(error = %e, "could not mark hot exit");
            }
        } else if let Err(e) = session.remove() {
            tracing::warn!(error = %e, "could not remove recovery session");
        }
    }
}
