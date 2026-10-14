//! SQLite state store (docs/specs/23-session-persistence.md, AD-6): sessions,
//! window geometry, recent files/workspaces, Scratchpad metadata, palette
//! history, dismissed notices. Never holds unsaved document text (recovery is
//! file-based) or settings (TOML).
//!
//! Robustness: WAL + busy timeout for concurrent window processes, forward-only
//! embedded migrations with a backup before migrating, a corrupt file is set
//! aside and replaced, and a database from a newer Notoir opens read-only.

use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, params};

/// Embedded migrations; index + 1 = resulting schema version.
const MIGRATIONS: &[&str] = &[include_str!("../migrations/0001_initial.sql")];
pub const SCHEMA_VERSION: u32 = MIGRATIONS.len() as u32;
/// Unpinned recent entries kept per kind.
pub const RECENT_LIMIT: usize = 50;

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// What happened while opening (shown as notices by the app).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OpenReport {
    /// The previous file was unreadable and was moved here.
    pub corrupt_backup: Option<PathBuf>,
    /// Migrated from this version (a backup `state.db.bak-v<N>` was made).
    pub migrated_from: Option<u32>,
    /// Written by a newer Notoir: opened read-only.
    pub newer_version: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecentKind {
    File,
    Workspace,
}

impl RecentKind {
    fn as_str(self) -> &'static str {
        match self {
            RecentKind::File => "file",
            RecentKind::Workspace => "workspace",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecentEntry {
    pub kind: RecentKind,
    pub path: String,
    pub display_name: Option<String>,
    pub last_opened: i64,
    pub pinned: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScratchpadRow {
    pub scratch_id: String,
    pub title: String,
    pub language: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub open: bool,
}

pub struct StateDb {
    conn: Connection,
    read_only: bool,
}

impl StateDb {
    /// Opens (or creates) the database at `path`, migrating or recovering as needed.
    pub fn open(path: &Path) -> Result<(StateDb, OpenReport), StorageError> {
        let mut report = OpenReport::default();
        let conn = match Self::connect_checked(path) {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(error = %e, path = %path.display(), "state database unreadable; starting fresh");
                let backup = set_aside_corrupt(path)?;
                report.corrupt_backup = Some(backup);
                Self::connect_checked(path)?
            }
        };
        let mut db = StateDb {
            conn,
            read_only: false,
        };
        let version = db.user_version()?;
        if version > SCHEMA_VERSION {
            tracing::warn!(
                version,
                known = SCHEMA_VERSION,
                "state database is newer; opening read-only"
            );
            db.read_only = true;
            report.newer_version = Some(version);
        } else if version < SCHEMA_VERSION {
            if version > 0 {
                let backup = path.with_extension(format!("db.bak-v{version}"));
                let _ = std::fs::remove_file(&backup);
                db.conn
                    .execute("VACUUM INTO ?1", [backup.to_string_lossy()])?;
                report.migrated_from = Some(version);
            }
            db.migrate(version)?;
        }
        Ok((db, report))
    }

    pub fn open_in_memory() -> Result<StateDb, StorageError> {
        let mut db = StateDb {
            conn: Connection::open_in_memory()?,
            read_only: false,
        };
        db.migrate(0)?;
        Ok(db)
    }

    fn connect_checked(path: &Path) -> Result<Connection, StorageError> {
        let conn = Connection::open(path)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        let check: String = conn.query_row("PRAGMA quick_check", [], |r| r.get(0))?;
        if check != "ok" {
            return Err(StorageError::Sqlite(rusqlite::Error::InvalidQuery));
        }
        Ok(conn)
    }

    fn user_version(&self) -> Result<u32, StorageError> {
        Ok(self
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))?)
    }

    fn migrate(&mut self, from: u32) -> Result<(), StorageError> {
        let tx = self.conn.transaction()?;
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(from as usize) {
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", (i + 1) as u32)?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn schema_version(&self) -> Result<u32, StorageError> {
        self.user_version()
    }

    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// Runs a write unless the database is read-only (newer schema).
    fn write(
        &self,
        f: impl FnOnce(&Connection) -> rusqlite::Result<()>,
    ) -> Result<(), StorageError> {
        if self.read_only {
            return Ok(());
        }
        f(&self.conn).map_err(Into::into)
    }

    // ── sessions ─────────────────────────────────────────────────────────

    pub fn save_session(
        &self,
        workspace_key: &str,
        session_json: &str,
        now_ms: i64,
    ) -> Result<(), StorageError> {
        self.write(|c| {
            c.execute(
                "INSERT INTO workspace_session (workspace_key, updated_at, session_json) VALUES (?1, ?2, ?3)
                 ON CONFLICT(workspace_key) DO UPDATE SET updated_at = ?2, session_json = ?3",
                params![workspace_key, now_ms, session_json],
            )
            .map(drop)
        })
    }

    pub fn load_session(&self, workspace_key: &str) -> Result<Option<String>, StorageError> {
        Ok(self
            .conn
            .query_row(
                "SELECT session_json FROM workspace_session WHERE workspace_key = ?1",
                [workspace_key],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn save_bookmarks(
        &self,
        workspace_key: &str,
        bookmarks_json: &str,
        now_ms: i64,
    ) -> Result<(), StorageError> {
        self.write(|c| {
            c.execute(
                "INSERT INTO workspace_session (workspace_key, updated_at, session_json, bookmarks_json) VALUES (?1, ?2, '{}', ?3)
                 ON CONFLICT(workspace_key) DO UPDATE SET updated_at = ?2, bookmarks_json = ?3",
                params![workspace_key, now_ms, bookmarks_json],
            )
            .map(drop)
        })
    }

    pub fn load_bookmarks(&self, workspace_key: &str) -> Result<Option<String>, StorageError> {
        Ok(self
            .conn
            .query_row(
                "SELECT bookmarks_json FROM workspace_session WHERE workspace_key = ?1",
                [workspace_key],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn save_window(
        &self,
        slot: i64,
        workspace_key: Option<&str>,
        geometry_json: &str,
        now_ms: i64,
    ) -> Result<(), StorageError> {
        self.write(|c| {
            c.execute(
                "INSERT INTO window_state (slot, workspace_key, geometry_json, updated_at) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(slot) DO UPDATE SET workspace_key = ?2, geometry_json = ?3, updated_at = ?4",
                params![slot, workspace_key, geometry_json, now_ms],
            )
            .map(drop)
        })
    }

    /// (workspace key, geometry JSON) of a window slot.
    pub fn load_window(&self, slot: i64) -> Result<Option<(Option<String>, String)>, StorageError> {
        Ok(self
            .conn
            .query_row(
                "SELECT workspace_key, geometry_json FROM window_state WHERE slot = ?1",
                [slot],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    }

    // ── recent ───────────────────────────────────────────────────────────

    /// Records an open (moves the entry to the top) and prunes old unpinned ones.
    pub fn record_recent(
        &self,
        kind: RecentKind,
        path: &str,
        display_name: Option<&str>,
        now_ms: i64,
    ) -> Result<(), StorageError> {
        self.write(|c| {
            c.execute(
                "INSERT INTO recent (kind, path, display_name, last_opened) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(kind, path) DO UPDATE SET last_opened = ?4, display_name = COALESCE(?3, display_name)",
                params![kind.as_str(), path, display_name, now_ms],
            )?;
            c.execute(
                "DELETE FROM recent WHERE kind = ?1 AND pinned = 0 AND path NOT IN (
                   SELECT path FROM recent WHERE kind = ?1 AND pinned = 0 ORDER BY last_opened DESC LIMIT ?2)",
                params![kind.as_str(), RECENT_LIMIT as i64],
            )
            .map(drop)
        })
    }

    /// Pinned entries first, then most recently opened.
    pub fn recent(&self, kind: RecentKind) -> Result<Vec<RecentEntry>, StorageError> {
        let mut stmt = self.conn.prepare(
            "SELECT path, display_name, last_opened, pinned FROM recent WHERE kind = ?1
             ORDER BY pinned DESC, last_opened DESC",
        )?;
        let rows = stmt.query_map([kind.as_str()], |r| {
            Ok(RecentEntry {
                kind,
                path: r.get(0)?,
                display_name: r.get(1)?,
                last_opened: r.get(2)?,
                pinned: r.get::<_, i64>(3)? != 0,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn set_recent_pinned(
        &self,
        kind: RecentKind,
        path: &str,
        pinned: bool,
    ) -> Result<(), StorageError> {
        self.write(|c| {
            c.execute(
                "UPDATE recent SET pinned = ?3 WHERE kind = ?1 AND path = ?2",
                params![kind.as_str(), path, pinned as i64],
            )
            .map(drop)
        })
    }

    pub fn remove_recent(&self, kind: RecentKind, path: &str) -> Result<(), StorageError> {
        self.write(|c| {
            c.execute(
                "DELETE FROM recent WHERE kind = ?1 AND path = ?2",
                params![kind.as_str(), path],
            )
            .map(drop)
        })
    }

    /// Clear Recently Opened: unpinned entries of both kinds.
    pub fn clear_recent(&self) -> Result<(), StorageError> {
        self.write(|c| {
            c.execute("DELETE FROM recent WHERE pinned = 0", [])
                .map(drop)
        })
    }

    // ── scratchpads ──────────────────────────────────────────────────────

    pub fn upsert_scratchpad(&self, row: &ScratchpadRow) -> Result<(), StorageError> {
        self.write(|c| {
            c.execute(
                "INSERT INTO scratchpad (scratch_id, title, language, created_at, updated_at, open)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(scratch_id) DO UPDATE SET title = ?2, language = ?3, updated_at = ?5, open = ?6",
                params![row.scratch_id, row.title, row.language, row.created_at, row.updated_at, row.open as i64],
            )
            .map(drop)
        })
    }

    pub fn scratchpads(&self) -> Result<Vec<ScratchpadRow>, StorageError> {
        let mut stmt = self.conn.prepare(
            "SELECT scratch_id, title, language, created_at, updated_at, open FROM scratchpad ORDER BY created_at",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(ScratchpadRow {
                scratch_id: r.get(0)?,
                title: r.get(1)?,
                language: r.get(2)?,
                created_at: r.get(3)?,
                updated_at: r.get(4)?,
                open: r.get::<_, i64>(5)? != 0,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn delete_scratchpad(&self, scratch_id: &str) -> Result<(), StorageError> {
        self.write(|c| {
            c.execute("DELETE FROM scratchpad WHERE scratch_id = ?1", [scratch_id])
                .map(drop)
        })
    }

    // ── palette & notices ────────────────────────────────────────────────

    pub fn touch_palette(&self, command_id: &str, now_ms: i64) -> Result<(), StorageError> {
        self.write(|c| {
            c.execute(
                "INSERT INTO palette_recent (command_id, used_at) VALUES (?1, ?2)
                 ON CONFLICT(command_id) DO UPDATE SET used_at = ?2",
                params![command_id, now_ms],
            )
            .map(drop)
        })
    }

    pub fn palette_recent(&self, limit: usize) -> Result<Vec<String>, StorageError> {
        let mut stmt = self
            .conn
            .prepare("SELECT command_id FROM palette_recent ORDER BY used_at DESC LIMIT ?1")?;
        let rows = stmt.query_map([limit as i64], |r| r.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn dismiss_notice(&self, key: &str, now_ms: i64) -> Result<(), StorageError> {
        self.write(|c| {
            c.execute(
                "INSERT OR REPLACE INTO notice_dismissed (key, at) VALUES (?1, ?2)",
                params![key, now_ms],
            )
            .map(drop)
        })
    }

    pub fn is_notice_dismissed(&self, key: &str) -> Result<bool, StorageError> {
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM notice_dismissed WHERE key = ?1",
                [key],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }
}

/// Moves `path` (and its WAL/SHM files) to `state.db.corrupt-<unix-ms>`.
fn set_aside_corrupt(path: &Path) -> Result<PathBuf, StorageError> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let backup = path.with_file_name(format!("{name}.corrupt-{stamp}"));
    if path.exists() {
        std::fs::rename(path, &backup)?;
    }
    for suffix in ["-wal", "-shm"] {
        let side = path.with_file_name(format!("{name}{suffix}"));
        if side.exists() {
            let _ = std::fs::rename(
                &side,
                backup.with_file_name(format!("{name}{suffix}.corrupt-{stamp}")),
            );
        }
    }
    Ok(backup)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_schema_and_reopens() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let (db, report) = StateDb::open(&path).unwrap();
        assert_eq!(report, OpenReport::default());
        assert_eq!(db.schema_version().unwrap(), SCHEMA_VERSION);
        db.save_session("/w", r#"{"v":1}"#, 1).unwrap();
        drop(db);
        let (db, _) = StateDb::open(&path).unwrap();
        assert_eq!(
            db.load_session("/w").unwrap().as_deref(),
            Some(r#"{"v":1}"#)
        );
        assert_eq!(db.load_session("/other").unwrap(), None);
    }

    #[test]
    fn corrupt_database_is_set_aside() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        std::fs::write(
            &path,
            b"this is definitely not sqlite, just garbage bytes!!",
        )
        .unwrap();
        let (db, report) = StateDb::open(&path).unwrap();
        let backup = report.corrupt_backup.expect("backup made");
        assert!(backup.exists());
        assert!(
            backup
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("state.db.corrupt-")
        );
        db.record_recent(RecentKind::File, "/a.md", None, 1)
            .unwrap();
        assert_eq!(db.recent(RecentKind::File).unwrap().len(), 1);
    }

    #[test]
    fn newer_database_opens_read_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        {
            let (db, _) = StateDb::open(&path).unwrap();
            db.record_recent(RecentKind::Workspace, "/notes", None, 1)
                .unwrap();
            db.conn.pragma_update(None, "user_version", 99).unwrap();
        }
        let (db, report) = StateDb::open(&path).unwrap();
        assert_eq!(report.newer_version, Some(99));
        assert!(db.is_read_only());
        db.record_recent(RecentKind::Workspace, "/other", None, 2)
            .unwrap();
        let recent = db.recent(RecentKind::Workspace).unwrap();
        assert_eq!(recent.len(), 1, "reads work, writes are skipped");
    }

    #[test]
    // The range is empty until schema v2 exists; the test then covers every step.
    #[allow(clippy::reversed_empty_ranges)]
    fn migration_from_every_previous_version_keeps_data() {
        // Each historical version is created by applying its migration prefix.
        for from in 1..SCHEMA_VERSION {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("state.db");
            {
                let conn = Connection::open(&path).unwrap();
                for (i, sql) in MIGRATIONS.iter().enumerate().take(from as usize) {
                    conn.execute_batch(sql).unwrap();
                    conn.pragma_update(None, "user_version", (i + 1) as u32)
                        .unwrap();
                }
                conn.execute(
                    "INSERT INTO recent (kind, path, last_opened) VALUES ('file', '/keep.md', 1)",
                    [],
                )
                .unwrap();
            }
            let (db, report) = StateDb::open(&path).unwrap();
            assert_eq!(report.migrated_from, Some(from));
            assert!(path.with_extension(format!("db.bak-v{from}")).exists());
            assert_eq!(db.recent(RecentKind::File).unwrap()[0].path, "/keep.md");
        }
    }

    #[test]
    fn recent_ordering_pins_and_clear() {
        let db = StateDb::open_in_memory().unwrap();
        let k = RecentKind::Workspace;
        db.record_recent(k, "/personal", Some("Personal Notes"), 1)
            .unwrap();
        db.record_recent(k, "/stringart", Some("StringArtIt"), 2)
            .unwrap();
        db.record_recent(k, "/docs", None, 3).unwrap();
        db.set_recent_pinned(k, "/personal", true).unwrap();
        let paths: Vec<_> = db.recent(k).unwrap().into_iter().map(|e| e.path).collect();
        assert_eq!(paths, ["/personal", "/docs", "/stringart"]);
        db.record_recent(k, "/stringart", None, 4).unwrap();
        let e = &db.recent(k).unwrap()[1];
        assert_eq!(
            (e.path.as_str(), e.display_name.as_deref()),
            ("/stringart", Some("StringArtIt"))
        );
        db.clear_recent().unwrap();
        let left: Vec<_> = db.recent(k).unwrap().into_iter().map(|e| e.path).collect();
        assert_eq!(left, ["/personal"]);
    }

    #[test]
    fn recent_is_pruned_but_pins_survive() {
        let db = StateDb::open_in_memory().unwrap();
        let k = RecentKind::File;
        db.record_recent(k, "/pinned", None, 0).unwrap();
        db.set_recent_pinned(k, "/pinned", true).unwrap();
        for i in 1..=(RECENT_LIMIT as i64 + 10) {
            db.record_recent(k, &format!("/f{i}"), None, i).unwrap();
        }
        let all = db.recent(k).unwrap();
        assert_eq!(all.len(), RECENT_LIMIT + 1);
        assert_eq!(all[0].path, "/pinned");
        assert!(all.iter().all(|e| e.path != "/f1"));
    }

    #[test]
    fn two_connections_write_concurrently() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let (a, _) = StateDb::open(&path).unwrap();
        let (b, _) = StateDb::open(&path).unwrap();
        let p2 = path.clone();
        let t = std::thread::spawn(move || {
            let (c, _) = StateDb::open(&p2).unwrap();
            for i in 0..50 {
                c.save_session("/B", &format!("{{\"n\":{i}}}"), i).unwrap();
            }
        });
        for i in 0..50 {
            a.save_session("/A", &format!("{{\"n\":{i}}}"), i).unwrap();
        }
        t.join().unwrap();
        assert_eq!(b.load_session("/A").unwrap().as_deref(), Some("{\"n\":49}"));
        assert_eq!(b.load_session("/B").unwrap().as_deref(), Some("{\"n\":49}"));
    }

    #[test]
    fn scratchpads_palette_notices() {
        let db = StateDb::open_in_memory().unwrap();
        let row = ScratchpadRow {
            scratch_id: "s1".into(),
            title: "Scratch 1".into(),
            language: "markdown".into(),
            created_at: 1,
            updated_at: 1,
            open: true,
        };
        db.upsert_scratchpad(&row).unwrap();
        db.upsert_scratchpad(&ScratchpadRow {
            title: "Ideas".into(),
            ..row.clone()
        })
        .unwrap();
        assert_eq!(db.scratchpads().unwrap()[0].title, "Ideas");
        db.delete_scratchpad("s1").unwrap();
        assert!(db.scratchpads().unwrap().is_empty());

        db.touch_palette("view.toggleWordWrap", 1).unwrap();
        db.touch_palette("file.save", 2).unwrap();
        assert_eq!(
            db.palette_recent(10).unwrap(),
            ["file.save", "view.toggleWordWrap"]
        );

        assert!(!db.is_notice_dismissed("x").unwrap());
        db.dismiss_notice("x", 1).unwrap();
        assert!(db.is_notice_dismissed("x").unwrap());
    }
}
