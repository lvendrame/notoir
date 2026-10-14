//! Scratchpad store (docs/specs/08-opening-files-scratchpads-and-recent.md):
//! content in `<data>/scratchpads/<id>.txt`, metadata in the state DB.
//! Independent of autosave; the app writes content debounced after edits.

use std::path::{Path, PathBuf};

use crate::atomic::{self, WriteOptions};
use crate::state_db::{ScratchpadRow, StateDb, StorageError};

pub struct ScratchpadStore {
    dir: PathBuf,
}

impl ScratchpadStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    fn path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.txt"))
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Lowest positive N such that "<prefix> N" is unused.
    pub fn next_number(db: &StateDb, prefix: &str) -> Result<u32, StorageError> {
        let used: Vec<u32> = db
            .scratchpads()?
            .iter()
            .filter_map(|r| r.title.strip_prefix(prefix)?.trim().parse().ok())
            .collect();
        Ok((1..).find(|n| !used.contains(n)).unwrap_or(1))
    }

    /// Creates an empty Scratchpad titled `title` (the app passes the
    /// translated "Scratch N").
    pub fn create(
        &self,
        db: &StateDb,
        title: &str,
        language: &str,
        now_ms: i64,
    ) -> Result<ScratchpadRow, StorageError> {
        std::fs::create_dir_all(&self.dir)?;
        let row = ScratchpadRow {
            scratch_id: uuid::Uuid::new_v4().simple().to_string(),
            title: title.to_owned(),
            language: language.to_owned(),
            created_at: now_ms,
            updated_at: now_ms,
            open: true,
        };
        atomic::write(&self.path(&row.scratch_id), b"", WriteOptions::default())?;
        db.upsert_scratchpad(&row)?;
        Ok(row)
    }

    pub fn write_content(&self, id: &str, text: &str) -> Result<(), StorageError> {
        std::fs::create_dir_all(&self.dir)?;
        atomic::write(&self.path(id), text.as_bytes(), WriteOptions::default())?;
        Ok(())
    }

    /// Missing content reads as empty (a Scratchpad created right before a crash).
    pub fn read_content(&self, id: &str) -> Result<String, StorageError> {
        match std::fs::read(self.path(id)) {
            Ok(bytes) => Ok(String::from_utf8_lossy(&bytes).into_owned()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn delete(&self, db: &StateDb, id: &str) -> Result<(), StorageError> {
        match std::fs::remove_file(self.path(id)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        db.delete_scratchpad(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // spec 08: "New scratchpad numbering"
    #[test]
    fn numbering_reuses_lowest_gap() {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open_in_memory().unwrap();
        let store = ScratchpadStore::new(dir.path());
        store.create(&db, "Scratch 1", "markdown", 1).unwrap();
        let s2 = store.create(&db, "Scratch 2", "markdown", 2).unwrap();
        store.create(&db, "Scratch 3", "markdown", 3).unwrap();
        store.delete(&db, &s2.scratch_id).unwrap();
        assert_eq!(ScratchpadStore::next_number(&db, "Scratch").unwrap(), 2);
    }

    #[test]
    fn content_round_trips_and_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("state.db");
        let store = ScratchpadStore::new(dir.path().join("scratchpads"));
        let id = {
            let (db, _) = StateDb::open(&db_path).unwrap();
            let row = store.create(&db, "Scratch 1", "markdown", 1).unwrap();
            store.write_content(&row.scratch_id, "buy milk").unwrap();
            row.scratch_id
        };
        let (db, _) = StateDb::open(&db_path).unwrap();
        assert_eq!(db.scratchpads().unwrap()[0].scratch_id, id);
        assert_eq!(store.read_content(&id).unwrap(), "buy milk");
        store.delete(&db, &id).unwrap();
        assert_eq!(store.read_content(&id).unwrap(), "");
        assert!(db.scratchpads().unwrap().is_empty());
    }
}
