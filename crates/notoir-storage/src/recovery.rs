//! Recovery store (docs/specs/09-save-autosave-and-recovery.md §Recovery).
//!
//! ```text
//! <data>/recovery/<session-id>/
//!     session.lock      held (OS file lock) while the window process runs
//!     manifest.json     atomically replaced
//!     docs/<entry>.txt  full content, UTF-8, '\n' line endings (written first)
//!     docs/<entry>.json metadata incl. content hash (written second)
//! ```
//!
//! Plain files, independent of the state DB. A session whose lock can be taken
//! belongs to a process that is gone: crashed (`clean_exit = false`) or hot
//! exit. Damaged entries are reported with whatever content survived — never
//! dropped silently.

use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::atomic::{self, WriteOptions, content_hash};
use crate::state_db::StorageError;

pub const FORMAT: u32 = 1;
const LOCK: &str = "session.lock";
const MANIFEST: &str = "manifest.json";
const DOCS: &str = "docs";
const DISCARDED: &str = ".discarded";
/// How long discarded sessions are kept.
pub const DISCARD_RETENTION_MS: i64 = 7 * 24 * 3600 * 1000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: u32,
    pub session_id: String,
    pub app_version: String,
    pub pid: u32,
    pub workspace: Option<String>,
    pub updated_at_ms: i64,
    pub clean_exit: bool,
    pub entries: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    File,
    Untitled,
    Scratchpad,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntryMeta {
    pub format: u32,
    pub kind: EntryKind,
    pub path: Option<String>,
    pub scratch_id: Option<String>,
    pub title: String,
    pub revision: u64,
    /// Hash of the disk file the edits were based on (`xxh3:<hex>`).
    pub base_disk_hash: Option<String>,
    pub base_disk_mtime_ms: Option<i64>,
    pub encoding: String,
    pub bom: bool,
    /// `lf` | `crlf` | `cr`.
    pub line_ending: String,
    pub language: String,
    /// Filled in by [`RecoverySession::write_entry`].
    #[serde(default)]
    pub content_hash: String,
    pub snapshot_at_ms: i64,
}

pub fn hash_label(hash: u64) -> String {
    format!("xxh3:{hash:016x}")
}

/// The running process's own recovery session.
pub struct RecoverySession {
    dir: PathBuf,
    _lock: File,
    manifest: Manifest,
}

impl RecoverySession {
    pub fn create(
        root: &Path,
        app_version: &str,
        workspace: Option<&str>,
        now_ms: i64,
    ) -> Result<Self, StorageError> {
        let id = uuid::Uuid::new_v4().simple().to_string();
        let dir = root.join(&id);
        fs::create_dir_all(dir.join(DOCS))?;
        let lock = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(dir.join(LOCK))?;
        lock.try_lock()
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        let session = Self {
            dir,
            _lock: lock,
            manifest: Manifest {
                format: FORMAT,
                session_id: id,
                app_version: app_version.to_owned(),
                pid: std::process::id(),
                workspace: workspace.map(str::to_owned),
                updated_at_ms: now_ms,
                clean_exit: false,
                entries: Vec::new(),
            },
        };
        session.write_manifest()?;
        Ok(session)
    }

    pub fn id(&self) -> &str {
        &self.manifest.session_id
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn write_manifest(&self) -> Result<(), StorageError> {
        let json = serde_json::to_vec_pretty(&self.manifest).map_err(std::io::Error::other)?;
        atomic::write(&self.dir.join(MANIFEST), &json, WriteOptions::default())?;
        Ok(())
    }

    pub fn set_workspace(
        &mut self,
        workspace: Option<&str>,
        now_ms: i64,
    ) -> Result<(), StorageError> {
        self.manifest.workspace = workspace.map(str::to_owned);
        self.manifest.updated_at_ms = now_ms;
        self.write_manifest()
    }

    /// Snapshots one document: content first, then metadata, then manifest.
    pub fn write_entry(
        &mut self,
        entry_id: &str,
        mut meta: EntryMeta,
        content: &str,
        now_ms: i64,
    ) -> Result<(), StorageError> {
        let docs = self.dir.join(DOCS);
        meta.format = FORMAT;
        meta.content_hash = hash_label(content_hash(content.as_bytes()));
        atomic::write(
            &docs.join(format!("{entry_id}.txt")),
            content.as_bytes(),
            WriteOptions::default(),
        )?;
        let json = serde_json::to_vec_pretty(&meta).map_err(std::io::Error::other)?;
        atomic::write(
            &docs.join(format!("{entry_id}.json")),
            &json,
            WriteOptions::default(),
        )?;
        if !self.manifest.entries.iter().any(|e| e == entry_id) {
            self.manifest.entries.push(entry_id.to_owned());
        }
        self.manifest.updated_at_ms = now_ms;
        self.write_manifest()
    }

    /// Drops an entry (document saved, reverted, or closed with Don't Save).
    pub fn remove_entry(&mut self, entry_id: &str, now_ms: i64) -> Result<(), StorageError> {
        let docs = self.dir.join(DOCS);
        for ext in ["json", "txt"] {
            match fs::remove_file(docs.join(format!("{entry_id}.{ext}"))) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        self.manifest.entries.retain(|e| e != entry_id);
        self.manifest.updated_at_ms = now_ms;
        self.write_manifest()
    }

    /// Hot exit: entries stay and are restored silently next launch.
    pub fn mark_clean_exit(&mut self, now_ms: i64) -> Result<(), StorageError> {
        self.manifest.clean_exit = true;
        self.manifest.updated_at_ms = now_ms;
        self.write_manifest()
    }

    /// Normal exit with nothing to keep: removes the session directory.
    pub fn remove(self) -> Result<(), StorageError> {
        let dir = self.dir.clone();
        drop(self);
        fs::remove_dir_all(dir)?;
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EntryStatus {
    Ok,
    /// Metadata missing/unreadable or content hash mismatch; `content` holds
    /// whatever text survived ("Open Raw Content").
    Damaged(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveredEntry {
    pub id: String,
    pub meta: Option<EntryMeta>,
    pub content: Option<String>,
    pub status: EntryStatus,
}

/// A session left by a process that is no longer running.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrphanSession {
    pub dir: PathBuf,
    /// `None` when the manifest is unreadable (entries are still scanned).
    pub manifest: Option<Manifest>,
    pub entries: Vec<RecoveredEntry>,
}

impl OrphanSession {
    /// Hot exit (restore silently) vs crash (prompt).
    pub fn clean_exit(&self) -> bool {
        self.manifest.as_ref().is_some_and(|m| m.clean_exit)
    }
}

/// Finds sessions whose owner is gone. Live sessions (lock held), the caller's
/// own session and sessions from a newer format are skipped.
pub fn scan(root: &Path, own_id: Option<&str>) -> Result<Vec<OrphanSession>, StorageError> {
    let mut out = Vec::new();
    let Ok(read) = fs::read_dir(root) else {
        return Ok(out);
    };
    for entry in read.flatten() {
        let dir = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if !dir.is_dir() || name == DISCARDED || Some(name.as_str()) == own_id {
            continue;
        }
        if is_locked(&dir) {
            continue;
        }
        let manifest = fs::read(dir.join(MANIFEST))
            .ok()
            .and_then(|b| serde_json::from_slice::<Manifest>(&b).ok());
        if let Some(m) = &manifest
            && m.format > FORMAT
        {
            tracing::warn!(dir = %dir.display(), format = m.format, "skipping recovery session from a newer Notoir");
            continue;
        }
        let entries = scan_entries(&dir.join(DOCS), manifest.as_ref());
        if entries.is_empty() && manifest.as_ref().is_none_or(|m| m.entries.is_empty()) {
            // Nothing to recover (e.g. a clean session whose removal was interrupted).
            let _ = fs::remove_dir_all(&dir);
            continue;
        }
        out.push(OrphanSession {
            dir,
            manifest,
            entries,
        });
    }
    out.sort_by_key(|s| std::cmp::Reverse(s.manifest.as_ref().map_or(0, |m| m.updated_at_ms)));
    Ok(out)
}

fn is_locked(dir: &Path) -> bool {
    match OpenOptions::new().write(true).open(dir.join(LOCK)) {
        Ok(f) => f.try_lock().is_err(),
        Err(_) => false,
    }
}

fn scan_entries(docs: &Path, manifest: Option<&Manifest>) -> Vec<RecoveredEntry> {
    let mut ids: Vec<String> = manifest.map(|m| m.entries.clone()).unwrap_or_default();
    if let Ok(read) = fs::read_dir(docs) {
        for f in read.flatten() {
            let p = f.path();
            if let (Some(stem), Some(ext)) = (p.file_stem(), p.extension())
                && (ext == "json" || ext == "txt")
            {
                let stem = stem.to_string_lossy().into_owned();
                if !ids.contains(&stem) {
                    ids.push(stem);
                }
            }
        }
    }
    ids.into_iter()
        .filter_map(|id| {
            let content = fs::read(docs.join(format!("{id}.txt")))
                .ok()
                .map(|b| String::from_utf8_lossy(&b).into_owned());
            let meta = fs::read(docs.join(format!("{id}.json")))
                .ok()
                .and_then(|b| serde_json::from_slice::<EntryMeta>(&b).ok());
            if content.is_none() && meta.is_none() {
                return None;
            }
            let status = match (&meta, &content) {
                (Some(m), Some(c)) if m.content_hash == hash_label(content_hash(c.as_bytes())) => {
                    EntryStatus::Ok
                }
                (Some(_), Some(_)) => EntryStatus::Damaged("content hash mismatch".into()),
                (None, Some(_)) => EntryStatus::Damaged("metadata missing or unreadable".into()),
                (_, None) => EntryStatus::Damaged("content missing".into()),
            };
            Some(RecoveredEntry {
                id,
                meta,
                content,
                status,
            })
        })
        .collect()
}

/// Discard: moves the session to `.discarded/` (kept 7 days, spec 09).
pub fn discard(root: &Path, session_dir: &Path, now_ms: i64) -> Result<PathBuf, StorageError> {
    let trash = root.join(DISCARDED);
    fs::create_dir_all(&trash)?;
    let name = session_dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let target = trash.join(format!("{name}-{now_ms}"));
    fs::rename(session_dir, &target)?;
    Ok(target)
}

/// Deletes discarded sessions older than `max_age_ms`.
pub fn purge_discarded(root: &Path, max_age_ms: i64, now_ms: i64) -> Result<usize, StorageError> {
    let mut removed = 0;
    let Ok(read) = fs::read_dir(root.join(DISCARDED)) else {
        return Ok(0);
    };
    for entry in read.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let stamp = name
            .rsplit_once('-')
            .and_then(|(_, t)| t.parse::<i64>().ok());
        if stamp.is_some_and(|t| now_ms - t > max_age_ms) {
            fs::remove_dir_all(entry.path())?;
            removed += 1;
        }
    }
    Ok(removed)
}

/// Removes an orphan session after its entries were restored into the new one.
pub fn remove_orphan(session: &OrphanSession) -> Result<(), StorageError> {
    fs::remove_dir_all(&session.dir)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(kind: EntryKind, title: &str) -> EntryMeta {
        EntryMeta {
            format: FORMAT,
            kind,
            path: (kind == EntryKind::File).then(|| format!("/notes/{title}")),
            scratch_id: None,
            title: title.into(),
            revision: 7,
            base_disk_hash: Some(hash_label(42)),
            base_disk_mtime_ms: Some(1),
            encoding: "UTF-8".into(),
            bom: false,
            line_ending: "lf".into(),
            language: "markdown".into(),
            content_hash: String::new(),
            snapshot_at_ms: 5,
        }
    }

    #[test]
    fn live_session_is_not_an_orphan() {
        let root = tempfile::tempdir().unwrap();
        let mut s = RecoverySession::create(root.path(), "0.1.0", None, 1).unwrap();
        s.write_entry("e1", meta(EntryKind::File, "a.md"), "unsaved", 2)
            .unwrap();
        assert!(
            scan(root.path(), None).unwrap().is_empty(),
            "lock held → alive"
        );
    }

    #[test]
    fn crashed_session_is_recovered_with_content() {
        let root = tempfile::tempdir().unwrap();
        {
            let mut s = RecoverySession::create(root.path(), "0.1.0", Some("/notes"), 1).unwrap();
            s.write_entry("e1", meta(EntryKind::File, "a.md"), "edits to a", 2)
                .unwrap();
            s.write_entry("e2", meta(EntryKind::Untitled, "Untitled-1"), "14 lines", 3)
                .unwrap();
            s.write_entry("e2", meta(EntryKind::Untitled, "Untitled-1"), "15 lines", 4)
                .unwrap();
            // Dropped without remove(): simulates the process dying.
        }
        let orphans = scan(root.path(), None).unwrap();
        assert_eq!(orphans.len(), 1);
        let o = &orphans[0];
        assert!(!o.clean_exit());
        assert_eq!(
            o.manifest.as_ref().unwrap().workspace.as_deref(),
            Some("/notes")
        );
        assert_eq!(o.entries.len(), 2);
        assert!(o.entries.iter().all(|e| e.status == EntryStatus::Ok));
        assert_eq!(o.entries[1].content.as_deref(), Some("15 lines"));
        assert_eq!(
            o.entries[0].meta.as_ref().unwrap().path.as_deref(),
            Some("/notes/a.md")
        );
    }

    #[test]
    fn hot_exit_is_marked_clean() {
        let root = tempfile::tempdir().unwrap();
        {
            let mut s = RecoverySession::create(root.path(), "0.1.0", None, 1).unwrap();
            s.write_entry("e1", meta(EntryKind::File, "a.md"), "x", 2)
                .unwrap();
            s.mark_clean_exit(3).unwrap();
        }
        assert!(scan(root.path(), None).unwrap()[0].clean_exit());
    }

    #[test]
    fn removed_entries_and_normal_exit_leave_nothing() {
        let root = tempfile::tempdir().unwrap();
        let mut s = RecoverySession::create(root.path(), "0.1.0", None, 1).unwrap();
        s.write_entry("e1", meta(EntryKind::File, "a.md"), "x", 2)
            .unwrap();
        s.remove_entry("e1", 3).unwrap();
        assert!(s.manifest().entries.is_empty());
        s.remove().unwrap();
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    }

    #[test]
    fn damaged_entries_are_reported_not_dropped() {
        let root = tempfile::tempdir().unwrap();
        let dir = {
            let mut s = RecoverySession::create(root.path(), "0.1.0", None, 1).unwrap();
            s.write_entry("e1", meta(EntryKind::File, "a.md"), "original", 2)
                .unwrap();
            s.write_entry("e2", meta(EntryKind::File, "b.md"), "second", 2)
                .unwrap();
            s.dir().to_path_buf()
        };
        fs::write(dir.join("docs/e1.txt"), "torn wri").unwrap();
        fs::write(dir.join("docs/e2.json"), "{ not json").unwrap();
        fs::write(dir.join("manifest.json"), "garbage").unwrap();
        let o = &scan(root.path(), None).unwrap()[0];
        assert!(o.manifest.is_none());
        let by_id = |id: &str| o.entries.iter().find(|e| e.id == id).unwrap();
        assert!(matches!(by_id("e1").status, EntryStatus::Damaged(_)));
        assert_eq!(by_id("e1").content.as_deref(), Some("torn wri"));
        assert!(matches!(by_id("e2").status, EntryStatus::Damaged(_)));
        assert_eq!(by_id("e2").content.as_deref(), Some("second"));
    }

    #[test]
    fn newer_format_sessions_are_left_alone() {
        let root = tempfile::tempdir().unwrap();
        let dir = {
            let mut s = RecoverySession::create(root.path(), "0.1.0", None, 1).unwrap();
            s.write_entry("e1", meta(EntryKind::File, "a.md"), "x", 2)
                .unwrap();
            s.dir().to_path_buf()
        };
        let mut m: Manifest =
            serde_json::from_slice(&fs::read(dir.join("manifest.json")).unwrap()).unwrap();
        m.format = 99;
        fs::write(dir.join("manifest.json"), serde_json::to_vec(&m).unwrap()).unwrap();
        assert!(scan(root.path(), None).unwrap().is_empty());
        assert!(dir.exists());
    }

    #[test]
    fn discard_keeps_data_for_seven_days() {
        let root = tempfile::tempdir().unwrap();
        {
            let mut s = RecoverySession::create(root.path(), "0.1.0", None, 1).unwrap();
            s.write_entry(
                "e1",
                meta(EntryKind::Scratchpad, "Scratch 2"),
                "phone 555",
                2,
            )
            .unwrap();
        }
        let o = scan(root.path(), None).unwrap().remove(0);
        let moved = discard(root.path(), &o.dir, 1_000).unwrap();
        assert!(moved.join("docs/e1.txt").exists());
        assert!(scan(root.path(), None).unwrap().is_empty());
        assert_eq!(
            purge_discarded(
                root.path(),
                DISCARD_RETENTION_MS,
                1_000 + DISCARD_RETENTION_MS
            )
            .unwrap(),
            0
        );
        assert_eq!(
            purge_discarded(
                root.path(),
                DISCARD_RETENTION_MS,
                2_000 + DISCARD_RETENTION_MS
            )
            .unwrap(),
            1
        );
    }

    #[test]
    fn own_session_is_excluded() {
        let root = tempfile::tempdir().unwrap();
        {
            let mut s = RecoverySession::create(root.path(), "0.1.0", None, 1).unwrap();
            s.write_entry("e1", meta(EntryKind::File, "a.md"), "x", 2)
                .unwrap();
        }
        let id = scan(root.path(), None).unwrap()[0]
            .manifest
            .clone()
            .unwrap()
            .session_id;
        assert!(scan(root.path(), Some(&id)).unwrap().is_empty());
    }
}
