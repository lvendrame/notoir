//! External changes (docs/specs/10) and autosave (docs/specs/09 §Autosave).
//!
//! Open files are watched through the workspace watcher, or through their
//! parent folder when outside the workspace (watching the folder survives
//! replace-by-rename). A changed path is re-read off the UI thread and
//! classified: same content as last saved → ignore (own saves, touches);
//! missing → deleted; different → reload a clean document (undoable) or show
//! the banner on a dirty one.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use notoir_core::document::{DiskState, DocumentOrigin, ReadOnly};
use notoir_core::ids::DocumentId;
use notoir_workspace::fs_io::{self, LoadOptions, Loaded};
use notoir_workspace::watcher::WorkspaceWatcher;

use crate::core::AppCore;
use crate::files::{LoadPurpose, Prompt};
use crate::io::IoResult;
use crate::recovery::{Banner, BannerKind};

/// A deleted path is checked again after this long (write-temp-rename).
const DELETE_RECHECK: Duration = Duration::from_millis(500);

#[derive(Debug)]
pub enum DiskCheck {
    Missing,
    Present {
        loaded: Box<Loaded>,
        read_only: bool,
    },
    Unreadable(String),
}

#[derive(Debug)]
pub struct DiskCheckResult {
    pub doc: DocumentId,
    pub path: PathBuf,
    pub check: DiskCheck,
}

/// Watches the folders of open files outside the workspace.
#[derive(Default)]
pub struct DocWatch {
    watcher: Option<WorkspaceWatcher>,
    dirs: HashSet<PathBuf>,
    failed: bool,
}

fn read_only_on_disk(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.permissions().readonly())
}

impl AppCore {
    /// Keeps the parent-folder watches in line with the open files.
    pub fn sync_doc_watches(&mut self) {
        let root = self.workspace.as_ref().map(|w| w.root.clone());
        let wanted: HashSet<PathBuf> = self
            .workbench
            .documents
            .values()
            .filter_map(|d| d.path())
            .filter(|p| root.as_ref().is_none_or(|r| !p.starts_with(r)))
            .filter_map(|p| p.parent().map(Path::to_path_buf))
            .chain([self.paths.config.clone(), self.paths.themes.clone()])
            .collect();
        if wanted == self.doc_watch.dirs || self.doc_watch.failed {
            return;
        }
        if self.doc_watch.watcher.is_none() {
            let poster = self.fs.poster();
            match WorkspaceWatcher::new(move |paths| poster.post(IoResult::DocsChanged(paths))) {
                Ok(w) => self.doc_watch.watcher = Some(w),
                Err(e) => {
                    tracing::warn!(error = %e, "file watching unavailable");
                    self.doc_watch.failed = true;
                    return;
                }
            }
        }
        let w = self.doc_watch.watcher.as_mut().expect("created");
        for gone in self.doc_watch.dirs.difference(&wanted) {
            let _ = w.unwatch(gone);
        }
        for new in wanted.difference(&self.doc_watch.dirs) {
            if let Err(e) = w.watch(new, false) {
                tracing::debug!(dir = %new.display(), error = %e, "could not watch folder");
            }
        }
        self.doc_watch.dirs = wanted;
    }

    /// Paths changed on disk (from any watcher): re-check open documents.
    pub fn check_changed_paths(&mut self, paths: &[PathBuf]) {
        if paths.iter().any(|p| self.is_config_file(p)) {
            self.reload_config();
        }
        let themes = &self.paths.themes;
        let canonical = themes.canonicalize().ok();
        if paths
            .iter()
            .any(|p| p.starts_with(themes) || canonical.as_ref().is_some_and(|c| p.starts_with(c)))
        {
            self.reload_custom_themes();
        }
        let changed: HashSet<&Path> = paths.iter().map(PathBuf::as_path).collect();
        let docs: Vec<(DocumentId, PathBuf)> = self
            .workbench
            .documents
            .values()
            .filter_map(|d| d.path().map(|p| (d.id, p.to_path_buf())))
            .filter(|(_, p)| changed.contains(p.as_path()))
            .collect();
        for (doc, path) in docs {
            self.check_disk(doc, path);
        }
    }

    fn check_disk(&mut self, doc: DocumentId, path: PathBuf) {
        let forced = self.workbench.document(doc).map(|d| d.encoding.encoding);
        self.fs.submit(move || {
            let read = |path: &Path| {
                if !path.exists() {
                    return DiskCheck::Missing;
                }
                let opts = LoadOptions {
                    forced_encoding: forced,
                    allow_binary: true,
                    ..Default::default()
                };
                match fs_io::load(path, &opts) {
                    Ok(l) => DiskCheck::Present {
                        read_only: read_only_on_disk(path),
                        loaded: Box::new(l),
                    },
                    Err(fs_io::LoadError::NotFound) => DiskCheck::Missing,
                    Err(e) => DiskCheck::Unreadable(e.to_string()),
                }
            };
            let mut check = read(&path);
            if matches!(check, DiskCheck::Missing) {
                std::thread::sleep(DELETE_RECHECK);
                check = read(&path);
            }
            Some(IoResult::DiskChecked(Box::new(DiskCheckResult {
                doc,
                path,
                check,
            })))
        });
    }

    pub fn handle_disk_check(&mut self, r: DiskCheckResult) {
        let DiskCheckResult { doc, path, check } = r;
        let Some(d) = self.workbench.document(doc) else {
            return;
        };
        if d.path() != Some(path.as_path()) || self.saving.contains(&doc) {
            // Renamed meanwhile, or our own save is in flight (its result
            // records the new disk hash).
            return;
        }
        let dirty = d.is_dirty();
        match check {
            DiskCheck::Unreadable(e) => {
                tracing::debug!(path = %path.display(), error = %e, "changed file unreadable")
            }
            DiskCheck::Missing => {
                if d.disk_state != DiskState::DeletedExternally {
                    tracing::info!(path = %path.display(), "open file deleted on disk");
                    self.workbench.mark_deleted(&path);
                    self.banners.insert(
                        doc,
                        Banner {
                            kind: BannerKind::Deleted,
                            disk_hash: None,
                        },
                    );
                }
            }
            DiskCheck::Present { loaded, read_only } => {
                self.apply_read_only(doc, read_only);
                let hash = loaded.text.content_hash;
                let d = self.workbench.document(doc).expect("checked");
                let was_deleted = d.disk_state == DiskState::DeletedExternally;
                if hash.is_some() && hash == d.saved_content_hash {
                    // Our own save, a touch, or the file came back unchanged.
                    if was_deleted || d.disk_state == DiskState::ModifiedExternally {
                        if let Some(d) = self.workbench.document_mut(doc) {
                            d.disk_state = DiskState::InSync;
                        }
                        self.banners.remove(&doc);
                    }
                    return;
                }
                let reload_clean = self
                    .settings
                    .bool("files.reloadCleanOnExternalChange", None);
                let clean = !dirty || (was_deleted && d.saved_state == Some(d.buffer.state()));
                if clean && reload_clean {
                    tracing::info!(path = %path.display(), "reloading file changed on disk");
                    self.reload_from_loaded(doc, *loaded);
                } else {
                    if let Some(d) = self.workbench.document_mut(doc) {
                        d.disk_state = DiskState::ModifiedExternally;
                    }
                    self.banners.insert(
                        doc,
                        Banner {
                            kind: BannerKind::Changed,
                            disk_hash: hash,
                        },
                    );
                }
            }
        }
    }

    /// Permission changes: the lock icon follows the file (spec 10
    /// §Read-Only Transitions); Save then goes through Save As.
    fn apply_read_only(&mut self, doc: DocumentId, read_only: bool) {
        if let Some(d) = self.workbench.document_mut(doc) {
            match (d.read_only, read_only) {
                (ReadOnly::No, true) => d.read_only = ReadOnly::FileSystem,
                (ReadOnly::FileSystem, false) => d.read_only = ReadOnly::No,
                _ => {}
            }
        }
    }

    fn reload_from_loaded(&mut self, doc: DocumentId, loaded: Loaded) {
        self.replace_with_disk(doc, loaded);
        self.banners.remove(&doc);
    }

    // ── banner actions ───────────────────────────────────────────────────

    /// Reload: dirty documents confirm first; the reload is undoable.
    pub fn reload_from_disk(&mut self, doc: DocumentId, confirm: bool) {
        let Some(d) = self.workbench.document(doc) else {
            return;
        };
        let Some(path) = d.path().map(Path::to_path_buf) else {
            return;
        };
        if confirm && d.is_dirty() {
            self.files.prompts.push_back(Prompt::Revert { doc });
        } else {
            self.load_for(path, LoadPurpose::Revert(doc));
        }
    }

    /// Keep My Version / Keep Recovered: stay dirty, and the next save
    /// overwrites the disk version without another conflict prompt.
    pub fn keep_my_version(&mut self, doc: DocumentId) {
        let banner = self.banners.remove(&doc);
        if let Some(d) = self.workbench.document_mut(doc) {
            if let Some(h) = banner.and_then(|b| b.disk_hash) {
                d.saved_content_hash = Some(h);
            }
            if d.disk_state == DiskState::ModifiedExternally {
                d.disk_state = DiskState::InSync;
            }
        }
    }

    // ── autosave ─────────────────────────────────────────────────────────

    fn autosave_mode(&self) -> String {
        self.settings.str("files.autoSave", None).to_owned()
    }

    /// Autosave only touches file documents that can be saved in place
    /// without a prompt.
    fn autosave_eligible(&self, doc: DocumentId) -> bool {
        self.workbench.document(doc).is_some_and(|d| {
            matches!(d.origin, DocumentOrigin::File { .. })
                && d.is_dirty()
                && !d.is_read_only()
                && d.disk_state == DiskState::InSync
                && !self.saving.contains(&doc)
                && !self.banners.contains_key(&doc)
        })
    }

    pub fn autosave_doc_changed(&mut self, doc: DocumentId) {
        if self.autosave_mode() == "afterDelay" && self.autosave_eligible(doc) {
            let delay = self.settings.int("files.autoSaveDelay", None).max(0) as u64;
            self.autosave_due.insert(doc, self.now_ms + delay);
        }
    }

    pub fn autosave_tick(&mut self) {
        let now = self.now_ms;
        let ready: Vec<DocumentId> = self
            .autosave_due
            .iter()
            .filter(|(_, t)| now >= **t)
            .map(|(d, _)| *d)
            .collect();
        for doc in ready {
            self.autosave_due.remove(&doc);
            if self.autosave_eligible(doc) {
                self.save(doc);
            }
        }
    }

    pub fn autosave_next_due(&self) -> Option<u64> {
        self.autosave_due.values().min().copied()
    }

    /// The active document changed (`onFocusChange`).
    pub fn autosave_focus_change(&mut self, previous: Option<DocumentId>) {
        if self.autosave_mode() == "onFocusChange"
            && let Some(doc) = previous
            && self.autosave_eligible(doc)
        {
            self.save(doc);
        }
    }

    /// The window lost focus: autosave (`onWindowFocusLoss`), recovery and
    /// session flush.
    pub fn window_focus_lost(&mut self) {
        if self.autosave_mode() == "onWindowFocusLoss" {
            let docs: Vec<DocumentId> = self.workbench.documents.keys().copied().collect();
            for doc in docs {
                if self.autosave_eligible(doc) {
                    self.save(doc);
                }
            }
        }
        self.flush_recovery();
        self.write_session(false);
    }
}
