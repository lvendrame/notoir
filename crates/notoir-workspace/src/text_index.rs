//! Persistent full-text index of a workspace (docs/specs/34 §Indexed Full-Text
//! Search): an SQLite FTS5 trigram table in `<cache>/index/<workspace>/`.
//! It only narrows which files a search reads — the exact matcher still runs
//! on every candidate — so results equal the streaming search. The index is a
//! cache: a missing, corrupt or outdated database is deleted and rebuilt.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::UNIX_EPOCH;

use notoir_editor::find::FindQuery;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};

/// Bumped when the schema or the indexed text changes.
const SCHEMA_VERSION: &str = "1";
/// The database file inside the index folder.
pub const DB_FILE: &str = "text.sqlite";
/// Files are committed in chunks so a long build shows progress and can stop.
const CHUNK: usize = 200;

#[derive(Debug, thiserror::Error)]
pub enum IndexError {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("cancelled")]
    Cancelled,
}

/// What a sync changed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SyncStats {
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
}

/// The index folder for a workspace: `<cache>/index/<hash of its key>`.
pub fn dir_for(cache: &Path, workspace_key: &str) -> PathBuf {
    // FNV-1a: stable across builds (unlike the std hasher).
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in workspace_key.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    cache.join("index").join(format!("{h:016x}"))
}

/// The literal a query can be looked up by, if any: the index finds
/// substrings of at least three characters case-insensitively; regex and
/// shorter queries need the streaming search.
pub fn lookup_text(query: &FindQuery) -> Option<&str> {
    (!query.regex && query.within.is_none() && query.text.chars().count() >= 3)
        .then_some(query.text.as_str())
}

pub struct TextIndex {
    conn: Connection,
}

fn modified(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos() as i64)
}

/// Searchable text of a file, as the streaming search decodes it (`None`
/// for binary files).
fn text_of(bytes: &[u8]) -> Option<String> {
    if notoir_editor::encoding::looks_binary(bytes) {
        return None;
    }
    let d = notoir_editor::encoding::decode(bytes, None);
    Some(notoir_editor::line_ending::normalize(&d.text))
}

fn create_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS files(
             id INTEGER PRIMARY KEY,
             path TEXT NOT NULL UNIQUE,
             mtime INTEGER NOT NULL,
             size INTEGER NOT NULL);
         CREATE VIRTUAL TABLE IF NOT EXISTS body USING fts5(
             text, tokenize = 'trigram', content = '', contentless_delete = 1);",
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO meta(key, value) VALUES('version', ?1)",
        [SCHEMA_VERSION],
    )?;
    Ok(())
}

fn usable(conn: &Connection) -> bool {
    conn.query_row("SELECT value FROM meta WHERE key = 'version'", [], |r| {
        r.get::<_, String>(0)
    })
    .is_ok_and(|v| v == SCHEMA_VERSION)
        && conn
            .query_row("SELECT count(*) FROM files", [], |r| r.get::<_, i64>(0))
            .is_ok()
}

impl TextIndex {
    /// Opens (or creates) the index in `dir`; an unreadable or outdated one
    /// is deleted and started over.
    pub fn open(dir: &Path) -> Result<TextIndex, IndexError> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join(DB_FILE);
        if path.exists() {
            let ok = Connection::open(&path).is_ok_and(|c| usable(&c));
            if !ok {
                tracing::info!(path = %path.display(), "search index unusable; rebuilding");
                Self::delete(dir)?;
            }
        }
        let conn = Connection::open(&path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        create_schema(&conn)?;
        Ok(TextIndex { conn })
    }

    /// Removes the database files in `dir`.
    pub fn delete(dir: &Path) -> std::io::Result<()> {
        for suffix in ["", "-wal", "-shm"] {
            let p = dir.join(format!("{DB_FILE}{suffix}"));
            match std::fs::remove_file(&p) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
                _ => {}
            }
        }
        Ok(())
    }

    /// Empties the index (Rebuild Index).
    pub fn clear(&mut self) -> Result<(), IndexError> {
        self.conn
            .execute_batch("DELETE FROM body; DELETE FROM files;")?;
        Ok(())
    }

    fn known(&self) -> Result<HashMap<String, (i64, i64, i64)>, IndexError> {
        let mut st = self
            .conn
            .prepare("SELECT path, id, mtime, size FROM files")?;
        let rows = st.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, (r.get(1)?, r.get(2)?, r.get(3)?)))
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Brings the index in line with `files` (root-relative, `/`): new and
    /// changed files (by modification time and size) are read again, files
    /// no longer listed are dropped.
    pub fn sync(
        &mut self,
        root: &Path,
        files: &[String],
        max_size: u64,
        cancel: &AtomicBool,
    ) -> Result<SyncStats, IndexError> {
        let mut known = self.known()?;
        let mut stats = SyncStats::default();
        for chunk in files.chunks(CHUNK) {
            if cancel.load(Ordering::Relaxed) {
                return Err(IndexError::Cancelled);
            }
            let tx = self.conn.transaction()?;
            for rel in chunk {
                let prev = known.remove(rel);
                match index_file(&tx, root, rel, prev, max_size)? {
                    Change::Added => stats.added += 1,
                    Change::Updated => stats.updated += 1,
                    Change::Removed => stats.removed += 1,
                    Change::None => {}
                }
            }
            tx.commit()?;
        }
        if !known.is_empty() {
            let tx = self.conn.transaction()?;
            for (_, (id, _, _)) in known {
                remove_id(&tx, id)?;
                stats.removed += 1;
            }
            tx.commit()?;
        }
        Ok(stats)
    }

    /// Re-reads `rels` (changed on disk); missing ones are dropped. Paths
    /// the index does not know are skipped — new files arrive with the next
    /// [`sync`](Self::sync) of the workspace file list.
    pub fn update(
        &mut self,
        root: &Path,
        rels: &[String],
        max_size: u64,
    ) -> Result<(), IndexError> {
        let tx = self.conn.transaction()?;
        for rel in rels {
            let prev = tx
                .query_row(
                    "SELECT id, mtime, size FROM files WHERE path = ?1",
                    [rel],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?;
            if prev.is_some() {
                index_file(&tx, root, rel, prev, max_size)?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn file_count(&self) -> Result<usize, IndexError> {
        Ok(self
            .conn
            .query_row("SELECT count(*) FROM files", [], |r| r.get::<_, i64>(0))?
            as usize)
    }
}

enum Change {
    None,
    Added,
    Updated,
    Removed,
}

fn remove_id(conn: &Connection, id: i64) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM body WHERE rowid = ?1", [id])?;
    conn.execute("DELETE FROM files WHERE id = ?1", [id])?;
    Ok(())
}

fn index_file(
    conn: &Connection,
    root: &Path,
    rel: &str,
    prev: Option<(i64, i64, i64)>,
    max_size: u64,
) -> Result<Change, IndexError> {
    let path = root.join(rel);
    let meta = match std::fs::metadata(&path) {
        Ok(m) if m.is_file() && m.len() <= max_size => m,
        _ => {
            return Ok(match prev {
                Some((id, _, _)) => {
                    remove_id(conn, id)?;
                    Change::Removed
                }
                None => Change::None,
            });
        }
    };
    let (mtime, size) = (modified(&meta), meta.len() as i64);
    if prev.is_some_and(|(_, m, s)| m == mtime && s == size) {
        return Ok(Change::None);
    }
    // Unreadable or binary files keep a row (so they are not read again
    // until they change) with no text.
    let text = std::fs::read(&path)
        .ok()
        .and_then(|b| text_of(&b))
        .unwrap_or_default();
    let id = match prev {
        Some((id, _, _)) => {
            conn.execute("DELETE FROM body WHERE rowid = ?1", [id])?;
            conn.execute(
                "UPDATE files SET mtime = ?2, size = ?3 WHERE id = ?1",
                params![id, mtime, size],
            )?;
            id
        }
        None => {
            conn.execute(
                "INSERT INTO files(path, mtime, size) VALUES(?1, ?2, ?3)",
                params![rel, mtime, size],
            )?;
            conn.last_insert_rowid()
        }
    };
    if !text.is_empty() {
        conn.execute(
            "INSERT INTO body(rowid, text) VALUES(?1, ?2)",
            params![id, text],
        )?;
    }
    Ok(if prev.is_some() {
        Change::Updated
    } else {
        Change::Added
    })
}

/// Read-only lookups from a search thread (the index may be syncing).
pub struct IndexReader {
    conn: Connection,
}

impl IndexReader {
    pub fn open(dir: &Path) -> Result<IndexReader, IndexError> {
        let conn = Connection::open_with_flags(
            dir.join(DB_FILE),
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.busy_timeout(std::time::Duration::from_secs(2))?;
        Ok(IndexReader { conn })
    }

    /// Files whose text contains `text` (case-insensitive), sorted.
    pub fn candidates(&self, text: &str) -> Result<Vec<String>, IndexError> {
        let phrase = format!("\"{}\"", text.replace('"', "\"\""));
        let mut st = self.conn.prepare(
            "SELECT f.path FROM body JOIN files f ON f.id = body.rowid
             WHERE body MATCH ?1 ORDER BY f.path",
        )?;
        let rows = st.query_map([phrase], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn contains(&self, rel: &str) -> Result<bool, IndexError> {
        Ok(self
            .conn
            .query_row("SELECT 1 FROM files WHERE path = ?1", [rel], |_| Ok(()))
            .optional()?
            .is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, rel: &str, text: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    #[test]
    fn sync_lookup_update_and_rebuild() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("ws");
        let dir = d.path().join("cache/index/x");
        write(&root, "a.md", "The Quick brown fox\n");
        write(&root, "notes/b.txt", "lazy dog; quick thinking\n");
        write(&root, "c.bin", "\0\0\0binary quick");
        let files: Vec<String> = ["a.md", "notes/b.txt", "c.bin"].map(String::from).into();
        let never = AtomicBool::new(false);
        let mut ix = TextIndex::open(&dir).unwrap();
        let s = ix.sync(&root, &files, 1 << 20, &never).unwrap();
        assert_eq!(s.added, 3);
        let r = IndexReader::open(&dir).unwrap();
        assert_eq!(r.candidates("QUICK").unwrap(), ["a.md", "notes/b.txt"]);
        assert_eq!(r.candidates("k br").unwrap(), ["a.md"]);
        assert!(r.candidates("zebra").unwrap().is_empty());
        assert!(r.contains("c.bin").unwrap());

        // Unchanged files are not read again.
        assert_eq!(
            ix.sync(&root, &files, 1 << 20, &never).unwrap(),
            SyncStats::default()
        );
        // Edits are picked up by update, removals by sync.
        write(&root, "a.md", "slow turtle, longer text\n");
        ix.update(&root, &["a.md".into()], 1 << 20).unwrap();
        assert_eq!(r.candidates("quick").unwrap(), ["notes/b.txt"]);
        assert_eq!(r.candidates("turtle").unwrap(), ["a.md"]);
        let s = ix.sync(&root, &files[1..], 1 << 20, &never).unwrap();
        assert_eq!(s.removed, 1);
        assert!(r.candidates("turtle").unwrap().is_empty());
        // Oversized files are left out.
        ix.sync(&root, &files, 4, &never).unwrap();
        assert_eq!(ix.file_count().unwrap(), 0);

        ix.clear().unwrap();
        assert_eq!(ix.file_count().unwrap(), 0);
    }

    #[test]
    fn corrupt_or_outdated_databases_are_rebuilt() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("ix");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(DB_FILE), "not a database").unwrap();
        let ix = TextIndex::open(&dir).unwrap();
        assert_eq!(ix.file_count().unwrap(), 0);
        drop(ix);
        Connection::open(dir.join(DB_FILE))
            .unwrap()
            .execute("UPDATE meta SET value = '0' WHERE key = 'version'", [])
            .unwrap();
        assert_eq!(TextIndex::open(&dir).unwrap().file_count().unwrap(), 0);
    }

    #[test]
    fn lookup_needs_a_literal_of_three_chars() {
        let q = |text: &str, regex: bool| FindQuery {
            text: text.into(),
            regex,
            ..Default::default()
        };
        assert_eq!(lookup_text(&q("abc", false)), Some("abc"));
        assert_eq!(lookup_text(&q("ab", false)), None);
        assert_eq!(lookup_text(&q("abc", true)), None);
        assert_ne!(dir_for(Path::new("/c"), "a"), dir_for(Path::new("/c"), "b"));
    }
}
