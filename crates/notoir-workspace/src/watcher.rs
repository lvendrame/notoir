//! Recursive file watching for a workspace (docs/specs/07, 10). Raw `notify`
//! events are coalesced for a short window on a helper thread and delivered
//! as the set of changed paths.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{RecvTimeoutError, channel};
use std::time::{Duration, Instant};

use notify::{RecursiveMode, Watcher};

/// How long events are gathered before delivery.
pub const COALESCE: Duration = Duration::from_millis(100);

/// Stops watching when dropped.
pub struct WorkspaceWatcher {
    _watcher: notify::RecommendedWatcher,
}

impl WorkspaceWatcher {
    /// Watches `root` recursively; `on_change` receives every changed path
    /// (created, removed, renamed, modified) in batches, on a helper thread.
    pub fn start(
        root: &Path,
        on_change: impl Fn(Vec<PathBuf>) + Send + 'static,
    ) -> notify::Result<WorkspaceWatcher> {
        let mut w = Self::new(on_change)?;
        w.watch(root, true)?;
        Ok(w)
    }

    /// Adds a path (a folder, recursively or not).
    pub fn watch(&mut self, path: &Path, recursive: bool) -> notify::Result<()> {
        let mode = if recursive {
            RecursiveMode::Recursive
        } else {
            RecursiveMode::NonRecursive
        };
        self._watcher.watch(path, mode)
    }

    pub fn unwatch(&mut self, path: &Path) -> notify::Result<()> {
        self._watcher.unwatch(path)
    }

    /// A watcher with nothing watched yet.
    pub fn new(
        on_change: impl Fn(Vec<PathBuf>) + Send + 'static,
    ) -> notify::Result<WorkspaceWatcher> {
        let (tx, rx) = channel::<notify::Result<notify::Event>>();
        let watcher = notify::recommended_watcher(tx)?;
        std::thread::Builder::new()
            .name("notoir-watch".into())
            .spawn(move || {
                let mut pending: BTreeSet<PathBuf> = BTreeSet::new();
                let mut deadline: Option<Instant> = None;
                loop {
                    let wait = deadline.map_or(Duration::from_secs(3600), |d| {
                        d.saturating_duration_since(Instant::now())
                    });
                    match rx.recv_timeout(wait) {
                        Ok(Ok(ev)) => {
                            if matches!(ev.kind, notify::EventKind::Access(_)) {
                                continue;
                            }
                            pending.extend(ev.paths);
                            deadline.get_or_insert_with(|| Instant::now() + COALESCE);
                        }
                        Ok(Err(e)) => {
                            tracing::warn!(error = %e, "file watcher error");
                        }
                        Err(RecvTimeoutError::Timeout) => {
                            if !pending.is_empty() {
                                on_change(std::mem::take(&mut pending).into_iter().collect());
                            }
                            deadline = None;
                        }
                        // The watcher was dropped.
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                }
            })
            .map_err(notify::Error::io)?;
        Ok(WorkspaceWatcher { _watcher: watcher })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_created_files() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap();
        std::fs::create_dir(root.join("inbox")).unwrap();
        let (tx, rx) = channel();
        let _w = WorkspaceWatcher::start(&root, move |paths| {
            let _ = tx.send(paths);
        })
        .unwrap();
        // Some backends need a moment before the first event is delivered.
        std::thread::sleep(Duration::from_millis(150));
        std::fs::write(root.join("inbox/today.md"), "x").unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut seen = false;
        while Instant::now() < deadline && !seen {
            if let Ok(paths) = rx.recv_timeout(Duration::from_millis(200)) {
                seen = paths.iter().any(|p| p.ends_with("inbox/today.md"));
            }
        }
        assert!(seen);
    }
}
