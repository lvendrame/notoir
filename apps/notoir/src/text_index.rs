//! The workspace's persistent full-text index (docs/specs/34 §Indexed
//! Full-Text Search): a worker thread per workspace keeps
//! `<cache>/index/<workspace>/` in line with the workspace file list (which
//! the watcher refreshes). Searches use it only while it is current; any
//! other time they stream (spec 15), so results never depend on it.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};

use notoir_core::explorer::NodeKind;
use notoir_workspace::text_index::{self, IndexError, SyncStats, TextIndex};

use crate::core::AppCore;
use crate::io::{IoResult, Poster};

enum Job {
    Sync {
        root: PathBuf,
        files: Vec<String>,
        max_size: u64,
    },
    /// Rebuild Index: drop everything, then sync.
    Rebuild {
        root: PathBuf,
        files: Vec<String>,
        max_size: u64,
    },
}

struct Worker {
    generation: u64,
    dir: PathBuf,
    tx: Sender<Job>,
    cancel: Arc<AtomicBool>,
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// A finished batch of index jobs.
#[derive(Debug)]
pub struct IndexDone {
    pub generation: u64,
    /// Jobs this result accounts for (queued syncs are merged).
    pub jobs: usize,
    pub result: Result<SyncStats, String>,
}

#[derive(Default)]
pub struct TextIndexState {
    worker: Option<Worker>,
    /// The workspace file list (`Arc` address) last sent to the worker.
    sent_from: Option<usize>,
    in_flight: usize,
    /// A sync finished since the worker started (or since Rebuild).
    synced: bool,
    failed: bool,
    pub last: Option<SyncStats>,
}

fn run_worker(
    dir: PathBuf,
    generation: u64,
    rx: Receiver<Job>,
    cancel: Arc<AtomicBool>,
    poster: Poster,
) {
    let mut index: Option<TextIndex> = None;
    while let Ok(first) = rx.recv() {
        // Only the newest file list matters; older queued ones are merged.
        let mut job = first;
        let mut jobs = 1;
        let mut rebuild = matches!(job, Job::Rebuild { .. });
        while let Ok(next) = rx.try_recv() {
            rebuild |= matches!(next, Job::Rebuild { .. });
            job = next;
            jobs += 1;
        }
        let (root, files, max_size) = match job {
            Job::Sync {
                root,
                files,
                max_size,
            }
            | Job::Rebuild {
                root,
                files,
                max_size,
            } => (root, files, max_size),
        };
        let result = (|| -> Result<SyncStats, IndexError> {
            if index.is_none() {
                index = Some(TextIndex::open(&dir)?);
            }
            let ix = index.as_mut().expect("opened");
            if rebuild {
                ix.clear()?;
            }
            ix.sync(&root, &files, max_size, &cancel)
        })();
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        if let Err(e) = &result {
            tracing::warn!(error = %e, dir = %dir.display(), "search index update failed");
            // Start from a fresh database next time.
            index = None;
        }
        poster.post(IoResult::TextIndex(Box::new(IndexDone {
            generation,
            jobs,
            result: result.map_err(|e| e.to_string()),
        })));
    }
}

impl AppCore {
    fn text_index_enabled(&self) -> bool {
        self.workspace.is_some() && self.settings.bool("search.useIndex", None)
    }

    /// Per frame: start the worker for the open workspace and send it each
    /// new file list.
    pub fn text_index_tick(&mut self) {
        if !self.text_index_enabled() {
            if self.text_index.worker.is_some() {
                self.text_index = TextIndexState::default();
            }
            return;
        }
        let ws = self.workspace.as_ref().expect("enabled");
        if self
            .text_index
            .worker
            .as_ref()
            .is_none_or(|w| w.generation != ws.generation)
        {
            let dir = text_index::dir_for(&self.paths.cache, &ws.key);
            let (tx, rx) = channel();
            let cancel = Arc::new(AtomicBool::new(false));
            let generation = ws.generation;
            let spawned = std::thread::Builder::new()
                .name("notoir-index".into())
                .spawn({
                    let dir = dir.clone();
                    let cancel = cancel.clone();
                    let poster = self.fs.poster();
                    move || run_worker(dir, generation, rx, cancel, poster)
                });
            if let Err(e) = spawned {
                tracing::error!(error = %e, "could not start the search index worker");
                return;
            }
            self.text_index = TextIndexState {
                worker: Some(Worker {
                    generation,
                    dir,
                    tx,
                    cancel,
                }),
                ..Default::default()
            };
        }
        let ws = self.workspace.as_ref().expect("enabled");
        let ptr = Arc::as_ptr(&ws.index) as usize;
        if ws.index_ready && self.text_index.sent_from != Some(ptr) {
            self.text_index.sent_from = Some(ptr);
            self.text_index_send(false);
        }
    }

    fn text_index_send(&mut self, rebuild: bool) {
        let Some(ws) = self.workspace.as_ref() else {
            return;
        };
        let files: Vec<String> = ws
            .index
            .iter()
            .filter(|e| e.kind == NodeKind::File && !e.ignored)
            .map(|e| e.rel.clone())
            .collect();
        let root = ws.root.clone();
        let max_size = self.settings.int("search.maxFileSize", None).max(0) as u64;
        let job = if rebuild {
            Job::Rebuild {
                root,
                files,
                max_size,
            }
        } else {
            Job::Sync {
                root,
                files,
                max_size,
            }
        };
        if let Some(w) = &self.text_index.worker
            && w.tx.send(job).is_ok()
        {
            self.text_index.in_flight += 1;
        }
    }

    pub fn text_index_done(&mut self, done: IndexDone) {
        if self.text_index.worker.as_ref().map(|w| w.generation) != Some(done.generation) {
            return;
        }
        let st = &mut self.text_index;
        st.in_flight = st.in_flight.saturating_sub(done.jobs);
        match done.result {
            Ok(stats) => {
                st.synced = true;
                st.failed = false;
                st.last = Some(stats);
                tracing::debug!(?stats, "search index synced");
            }
            Err(_) => st.failed = true,
        }
    }

    /// Search: Rebuild Index — empties the cache and indexes every file again
    /// (searches stream meanwhile).
    pub fn text_index_rebuild(&mut self) {
        if self.text_index.worker.is_none() {
            return;
        }
        self.text_index.synced = false;
        self.text_index_send(true);
    }

    /// The index folder while it matches the workspace on disk.
    pub fn text_index_dir(&self) -> Option<PathBuf> {
        let st = &self.text_index;
        let w = st.worker.as_ref()?;
        let current = st.synced
            && !st.failed
            && st.in_flight == 0
            && self
                .workspace
                .as_ref()
                .is_some_and(|ws| ws.generation == w.generation && ws.index_due.is_none());
        current.then(|| w.dir.clone())
    }

    /// The index is enabled but not usable yet (search panel hint).
    pub fn text_index_building(&self) -> bool {
        self.text_index.worker.is_some() && self.text_index_dir().is_none()
    }
}
