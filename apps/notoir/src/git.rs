//! Git in the app (docs/specs/28, read side): repository detection for the
//! workspace, debounced status refresh, Explorer/tab/status-bar decorations,
//! gutter change bars against the index (or HEAD), file history and
//! revision contents for Compare. Every Git call runs on the Git queue;
//! nothing here touches the network.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use notoir_core::ids::DocumentId;
use notoir_editor::Rope;
use notoir_editor::diff::{DiffOptions, LineChange};
use notoir_git::{
    CommitSummary, DiffBase, FileStatus, GitBackend, GitError, RepoInfo, RepoStatus, StatusOpts,
};

use crate::core::{AppCore, Effect};
use crate::io::IoResult;

/// Status refresh debounce after file-system events, saves and focus.
pub const STATUS_DEBOUNCE_MS: u64 = 500;
/// Gutter bars recompute this long after the last edit.
pub const GUTTER_DEBOUNCE_MS: u64 = 300;
/// Documents with more lines get no gutter bars.
pub const GUTTER_MAX_LINES: usize = 50_000;
/// Commits listed by File History.
pub const HISTORY_LIMIT: usize = 200;

/// What a File History list is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryMode {
    /// Open the diff of a revision against its parent.
    Changes,
    /// Compare a revision with the current file.
    CompareCurrent,
    /// Open the file as it was at a revision (read-only).
    OpenAtRevision,
}

#[derive(Clone, Debug)]
pub struct History {
    pub path: PathBuf,
    pub mode: HistoryMode,
    pub commits: Vec<CommitSummary>,
}

#[derive(Debug)]
pub enum GitResult {
    Discovered {
        generation: u64,
        repo: Option<RepoInfo>,
    },
    Status {
        generation: u64,
        result: Result<RepoStatus, GitError>,
    },
    Base {
        generation: u64,
        doc: DocumentId,
        /// `None`: the file is not in the base (new file).
        text: Option<String>,
    },
    Log {
        generation: u64,
        path: PathBuf,
        mode: HistoryMode,
        result: Result<Vec<CommitSummary>, GitError>,
    },
    /// File content at a revision for Compare / Open at Revision.
    Blob {
        purpose: BlobPurpose,
        result: Result<Option<String>, GitError>,
    },
}

#[derive(Clone, Debug)]
pub enum BlobPurpose {
    Compare {
        id: u64,
        left: bool,
        path: PathBuf,
        rev: String,
    },
    Open {
        path: PathBuf,
        rev: String,
    },
}

/// Gutter state of one document.
#[derive(Default)]
pub struct DocGit {
    /// The base text; `None` while loading or when the file is not in it.
    base: Option<Arc<Rope>>,
    base_loaded: bool,
    base_version: u64,
    /// (document revision, base version) the bars were computed for.
    computed: Option<(u64, u64)>,
    /// Revision seen last tick and when it changed (debounce).
    seen_revision: u64,
    edited_at: u64,
    pub changes: Arc<Vec<LineChange>>,
}

pub struct GitState {
    pub backend: Arc<dyn GitBackend>,
    pub repo: Option<RepoInfo>,
    pub status: Arc<RepoStatus>,
    /// Bumped per workspace; results of older ones are dropped.
    pub generation: u64,
    /// Workspace root Git was detected for.
    root: Option<PathBuf>,
    pub status_due: Option<u64>,
    status_running: bool,
    status_again: bool,
    pub docs: HashMap<DocumentId, DocGit>,
    pub history: Option<History>,
    /// Network Git commands run (fetch/pull/push). None run in M18; the
    /// counter documents "no auto fetch" (spec 28).
    pub network_runs: u64,
    pub last_error: Option<GitError>,
}

impl GitState {
    pub fn new(backend: Arc<dyn GitBackend>) -> Self {
        Self {
            backend,
            repo: None,
            status: Arc::new(RepoStatus::default()),
            generation: 0,
            root: None,
            status_due: None,
            status_running: false,
            status_again: false,
            docs: HashMap::new(),
            history: None,
            network_runs: 0,
            last_error: None,
        }
    }
}

fn utf8(bytes: Vec<u8>) -> String {
    // Line endings normalized like loaded documents so bars line up.
    notoir_editor::line_ending::normalize(&String::from_utf8_lossy(&bytes))
}

impl AppCore {
    fn git_enabled(&self) -> bool {
        self.settings.bool("git.enabled", None) && self.settings.bool("git.autoDetect", None)
    }

    /// Work-tree-relative path of `path` in the current repo.
    pub fn git_rel(&self, path: &Path) -> Option<String> {
        self.git.repo.as_ref()?.relative(path)
    }

    /// Git status of a file or folder for decorations.
    pub fn git_status_of(&self, path: &Path, dir: bool) -> Option<FileStatus> {
        let rel = self.git_rel(path)?;
        if dir {
            self.git.status.folder(&rel)
        } else {
            self.git.status.of(&rel)
        }
    }

    /// Schedules a debounced status refresh.
    pub fn git_refresh_soon(&mut self) {
        if self.git.repo.is_some() {
            let due = self.now_ms + STATUS_DEBOUNCE_MS;
            self.git.status_due = Some(self.git.status_due.map_or(due, |d| d.min(due)));
        }
    }

    /// File-system events: work-tree changes and index/HEAD/ref updates
    /// refresh the status (other `.git` churn is ignored).
    pub fn git_paths_changed(&mut self, paths: &[PathBuf]) {
        let Some(repo) = &self.git.repo else {
            return;
        };
        let relevant = paths.iter().any(|p| match p.strip_prefix(&repo.git_dir) {
            Ok(inside) => {
                let s = inside.to_string_lossy();
                s == "index"
                    || s == "HEAD"
                    || s == "packed-refs"
                    || s == "MERGE_HEAD"
                    || s.starts_with("refs")
            }
            Err(_) => !p.starts_with(&repo.git_dir),
        });
        if relevant {
            self.git_refresh_soon();
        }
    }

    /// Per frame: detection on workspace changes, due status refreshes and
    /// gutter bars.
    pub fn git_tick(&mut self) {
        let root = self
            .workspace
            .as_ref()
            .filter(|_| self.git_enabled())
            .map(|w| w.root.clone());
        if root != self.git.root {
            self.git.root = root.clone();
            self.git.generation += 1;
            self.git.repo = None;
            self.git.status = Arc::new(RepoStatus::default());
            self.git.docs.clear();
            self.git.status_due = None;
            if let Some(root) = root {
                let backend = self.git.backend.clone();
                let generation = self.git.generation;
                self.git_queue.submit(move || {
                    let repo = backend.discover(&root).unwrap_or_else(|e| {
                        tracing::warn!(error = %e, "git discovery failed");
                        None
                    });
                    Some(IoResult::Git(Box::new(GitResult::Discovered {
                        generation,
                        repo,
                    })))
                });
            }
        }
        if let Some(due) = self.git.status_due
            && self.now_ms >= due
        {
            self.git.status_due = None;
            self.git_start_status();
        }
        self.git_gutters();
    }

    pub fn git_next_due(&self) -> Option<u64> {
        self.git.status_due
    }

    fn git_start_status(&mut self) {
        let Some(repo) = self.git.repo.clone() else {
            return;
        };
        if self.git.status_running {
            self.git.status_again = true;
            return;
        }
        self.git.status_running = true;
        let backend = self.git.backend.clone();
        let generation = self.git.generation;
        self.git_queue.submit(move || {
            let result = backend.status(&repo, StatusOpts::default());
            Some(IoResult::Git(Box::new(GitResult::Status {
                generation,
                result,
            })))
        });
    }

    pub fn handle_git(&mut self, result: GitResult) {
        match result {
            GitResult::Discovered { generation, repo } if generation == self.git.generation => {
                if let Some(r) = &repo {
                    tracing::info!(workdir = %r.workdir.display(), "git repository");
                }
                self.git.repo = repo;
                self.git_start_status();
            }
            GitResult::Status { generation, result } if generation == self.git.generation => {
                self.git.status_running = false;
                match result {
                    Ok(s) => {
                        if s.truncated {
                            tracing::info!("git status truncated");
                        }
                        self.git.status = Arc::new(s);
                        self.git.last_error = None;
                        // The index may have changed: reload every base.
                        for d in self.git.docs.values_mut() {
                            d.base_loaded = false;
                        }
                    }
                    Err(e) => {
                        tracing::info!(error = %e, "git status failed");
                        self.git.last_error = Some(e);
                    }
                }
                if std::mem::take(&mut self.git.status_again) {
                    self.git_start_status();
                }
            }
            GitResult::Base {
                generation,
                doc,
                text,
            } if generation == self.git.generation => {
                if let Some(d) = self.git.docs.get_mut(&doc) {
                    let base = text.map(|t| Arc::new(Rope::from_str(&t)));
                    let same = match (&d.base, &base) {
                        (Some(a), Some(b)) => a.as_ref() == b.as_ref(),
                        (None, None) => true,
                        _ => false,
                    };
                    if !same {
                        d.base = base;
                        d.base_version += 1;
                    }
                }
            }
            GitResult::Log {
                generation,
                path,
                mode,
                result,
            } if generation == self.git.generation => match result {
                Ok(commits) if !commits.is_empty() => {
                    self.git.history = Some(History {
                        path,
                        mode,
                        commits,
                    });
                    self.effects.push(Effect::OpenPicker(
                        crate::shell::overlays::PickerKind::GitHistory,
                    ));
                }
                Ok(_) => self.git_notify("notifications:gitNoHistory", &path),
                Err(e) => {
                    tracing::warn!(error = %e, "git log failed");
                    self.git_notify("notifications:gitFailed", &path);
                }
            },
            GitResult::Blob { purpose, result } => self.git_blob_loaded(purpose, result),
            _ => {}
        }
    }

    fn git_notify(&mut self, key: &str, path: &Path) {
        let name = path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let now = self.now_ms;
        self.notifications.notify(
            notoir_core::notifications::Notification::new(
                notoir_core::notifications::Severity::Info,
                notoir_core::notifications::Translatable::new(key).arg("name", &name),
            )
            .source(notoir_core::notifications::NotificationSource::Git),
            now,
        );
    }

    /// Loads bases and recomputes bars of open files in the repo.
    fn git_gutters(&mut self) {
        let Some(repo) = self.git.repo.clone() else {
            return;
        };
        let base_kind = if self.settings.str("git.gutterBase", None) == "head" {
            DiffBase::Head
        } else {
            DiffBase::Index
        };
        let docs: Vec<(DocumentId, String, u64, bool)> = self
            .workbench
            .documents
            .values()
            .filter_map(|d| {
                let rel = repo.relative(d.path()?)?;
                Some((
                    d.id,
                    rel,
                    d.revision,
                    d.buffer.rope().len_lines() <= GUTTER_MAX_LINES,
                ))
            })
            .collect();
        self.git
            .docs
            .retain(|id, _| docs.iter().any(|(d, ..)| d == id));
        let generation = self.git.generation;
        let now = self.now_ms;
        for (doc, rel, revision, small) in docs {
            let untracked = matches!(
                self.git.status.of(&rel),
                Some(FileStatus::Untracked) | Some(FileStatus::Ignored)
            );
            let entry = self.git.docs.entry(doc).or_default();
            if untracked || !small {
                entry.changes = Arc::new(Vec::new());
                entry.base_loaded = true;
                continue;
            }
            if !entry.base_loaded {
                entry.base_loaded = true;
                let backend = self.git.backend.clone();
                let repo = repo.clone();
                self.git_queue.submit(move || {
                    let text = backend
                        .read_base(&repo, &rel, base_kind)
                        .ok()
                        .flatten()
                        .map(utf8);
                    Some(IoResult::Git(Box::new(GitResult::Base {
                        generation,
                        doc,
                        text,
                    })))
                });
            }
            if entry.seen_revision != revision {
                entry.seen_revision = revision;
                entry.edited_at = now;
            }
            let key = (revision, entry.base_version);
            let fresh_base = entry.computed.is_none_or(|c| c.1 != entry.base_version);
            if entry.computed == Some(key)
                || (!fresh_base && now.saturating_sub(entry.edited_at) < GUTTER_DEBOUNCE_MS)
            {
                continue;
            }
            let Some(base) = entry.base.clone() else {
                // Not in the base yet (staged-less new file or loading).
                entry.changes = Arc::new(Vec::new());
                entry.computed = Some(key);
                continue;
            };
            let Some(buffer) = self
                .workbench
                .document(doc)
                .map(|d| d.buffer.rope().clone())
            else {
                continue;
            };
            let diff = notoir_editor::diff::diff_ropes(&base, &buffer, &DiffOptions::default());
            let entry = self.git.docs.get_mut(&doc).expect("entry");
            entry.changes = Arc::new(diff.line_changes());
            entry.computed = Some(key);
        }
    }

    /// Whether bars are waiting for the edit debounce.
    pub fn git_gutter_pending(&self) -> bool {
        self.git
            .docs
            .values()
            .any(|d| d.computed.is_none_or(|c| c.0 != d.seen_revision))
    }

    /// Gutter bars for a document (empty outside repos).
    pub fn git_changes(&self, doc: DocumentId) -> Option<Arc<Vec<LineChange>>> {
        self.git
            .docs
            .get(&doc)
            .map(|d| d.changes.clone())
            .filter(|c| !c.is_empty())
    }

    /// File History / Compare with Revision / Open File at Revision: lists
    /// the file's commits, then a picker.
    pub fn git_history(&mut self, path: PathBuf, mode: HistoryMode) {
        let (Some(repo), Some(rel)) = (self.git.repo.clone(), self.git_rel(&path)) else {
            self.git_notify("notifications:gitNotInRepo", &path);
            return;
        };
        let backend = self.git.backend.clone();
        let generation = self.git.generation;
        self.git_queue.submit(move || {
            let result = backend.log_file(&repo, &rel, HISTORY_LIMIT);
            Some(IoResult::Git(Box::new(GitResult::Log {
                generation,
                path,
                mode,
                result,
            })))
        });
    }

    /// Reads `path` at `rev` on the Git queue.
    pub fn git_read_blob(&mut self, purpose: BlobPurpose) -> bool {
        let (path, rev) = match &purpose {
            BlobPurpose::Compare { path, rev, .. } | BlobPurpose::Open { path, rev } => {
                (path.clone(), rev.clone())
            }
        };
        let (Some(repo), Some(rel)) = (self.git.repo.clone(), self.git_rel(&path)) else {
            return false;
        };
        let backend = self.git.backend.clone();
        self.git_queue.submit(move || {
            let result = backend.read_blob(&repo, &rel, &rev).map(|b| b.map(utf8));
            Some(IoResult::Git(Box::new(GitResult::Blob { purpose, result })))
        });
        true
    }

    fn git_blob_loaded(&mut self, purpose: BlobPurpose, result: Result<Option<String>, GitError>) {
        match purpose {
            BlobPurpose::Compare {
                id,
                left,
                path,
                rev,
            } => {
                // A file missing at that revision compares as empty.
                let text = result.as_ref().ok().map(|t| t.clone().unwrap_or_default());
                if let Err(e) = &result {
                    tracing::warn!(error = %e, "git blob failed");
                }
                self.compare_revision_loaded(id, left, &path, &rev, text);
            }
            BlobPurpose::Open { path, rev } => match result {
                Ok(Some(text)) => self.open_revision(&path, &rev, &text),
                Ok(None) => self.git_notify("notifications:gitNotAtRevision", &path),
                Err(e) => {
                    tracing::warn!(error = %e, "git blob failed");
                    self.git_notify("notifications:gitFailed", &path);
                }
            },
        }
    }

    /// Branch label for the status bar: `main`, `main*`, `a1b2c3d` (detached),
    /// with `↑n ↓m` and MERGING/REBASING.
    pub fn git_branch_label(&self) -> Option<String> {
        self.git.repo.as_ref()?;
        let h = &self.git.status.head;
        let mut s = h
            .branch
            .clone()
            .or_else(|| h.short_sha.clone())
            .unwrap_or_default();
        if self.git.status.dirty() {
            s.push('*');
        }
        if h.ahead > 0 {
            s.push_str(&format!(" \u{2191}{}", h.ahead));
        }
        if h.behind > 0 {
            s.push_str(&format!(" \u{2193}{}", h.behind));
        }
        match h.state {
            notoir_git::RepoState::Merging => s.push_str(" MERGING"),
            notoir_git::RepoState::Rebasing => s.push_str(" REBASING"),
            _ => {}
        }
        Some(s)
    }
}
