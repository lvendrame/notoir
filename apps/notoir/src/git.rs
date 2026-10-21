//! Git in the app (docs/specs/28, read side): repository detection for the
//! workspace, debounced status refresh, Explorer/tab/status-bar decorations,
//! gutter change bars against the index (or HEAD), file history and
//! revision contents for Compare. Every local Git call runs on the Git
//! queue; the write side and network operations live in `git_write`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use notoir_core::ids::DocumentId;
use notoir_editor::Rope;
use notoir_editor::diff::{DiffOptions, LineChange};
use notoir_git::{
    BranchInfo, CommitSummary, DiffBase, FileStatus, GitBackend, GitError, GitOutput, GitRemote,
    RepoInfo, RepoStatus, StatusOpts,
};

use crate::git_write::{LocalOp, NetDone, NetOp, NetRun};

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
/// Status runs allowed at once (a newer one supersedes the older).
pub const STATUS_PARALLEL: usize = 2;
/// Revision name for "the staged version" (`git show :0:path`).
pub const INDEX_REV: &str = ":0";

/// A change opened from a gutter bar: its original lines with Revert
/// Change and Stage Change (spec 28 §Decorations).
#[derive(Clone, Debug)]
pub struct GitPeek {
    pub view: notoir_core::ids::ViewId,
    pub doc: DocumentId,
    pub kind: notoir_editor::diff::LineChangeKind,
    /// Buffer lines of the change (empty for a deletion before `start`).
    pub buf: std::ops::Range<usize>,
    /// Base lines it replaced.
    pub base: std::ops::Range<usize>,
    /// Those base lines (newline-terminated except at the end of the file).
    pub base_text: String,
    /// The bars compare with the index (Stage Change is offered).
    pub stageable: bool,
    /// Change `index` (0-based) of `total`.
    pub index: usize,
    pub total: usize,
    revision: u64,
    base_version: u64,
}

/// Char offset of line `l` (the end of the text past the last line).
fn line_char(r: &Rope, l: usize) -> usize {
    if l >= r.len_lines() {
        r.len_chars()
    } else {
        r.line_to_char(l)
    }
}

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
    Auto(crate::git_auto::AutoResult),
    /// The repository containing the workspace root, then repositories
    /// one level below it (`git.scanNestedRepos`).
    Discovered {
        generation: u64,
        repos: Vec<RepoInfo>,
    },
    Status {
        generation: u64,
        /// Newer runs supersede older ones.
        seq: u64,
        /// The active repository.
        workdir: PathBuf,
        result: Result<RepoStatus, GitError>,
        remotes: Vec<String>,
        /// The other repositories (decorations only).
        others: Vec<(PathBuf, RepoStatus)>,
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
    /// A local write operation finished (short SHA for commits).
    Op {
        generation: u64,
        op: LocalOp,
        result: Result<String, GitError>,
    },
    Branches {
        generation: u64,
        result: Result<Vec<BranchInfo>, GitError>,
    },
    /// Streamed output of a network operation.
    Output(GitOutput),
    Net {
        generation: u64,
        op: NetOp,
        result: Result<NetDone, GitError>,
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
    pub(crate) root: Option<PathBuf>,
    pub status_due: Option<u64>,
    discovery_due: Option<u64>,
    discovery_running: bool,
    pub auto_save: crate::git_auto::AutoSaveState,
    /// Latest status run; older results are dropped.
    status_seq: u64,
    /// Status runs on their threads (at most [`STATUS_PARALLEL`]).
    status_in_flight: usize,
    status_again: bool,
    /// Every repository of the workspace; `repo` is the active one
    /// (Source Control, writes, branch label).
    pub repos: Vec<RepoInfo>,
    /// Status of the other repositories (decorations, gutters).
    pub others: HashMap<PathBuf, Arc<RepoStatus>>,
    pub docs: HashMap<DocumentId, DocGit>,
    pub history: Option<History>,
    /// Network Git operations started (fetch/pull/push/sync/clone); only
    /// user commands start them (spec 28 "No auto fetch by default").
    pub network_runs: u64,
    pub last_error: Option<GitError>,
    /// Network operations (system git unless a test replaces it).
    pub remote: Option<Arc<dyn GitRemote>>,
    /// The Git Output channel, oldest first.
    pub output: std::collections::VecDeque<String>,
    /// Commit message drafts per repository (this session).
    pub drafts: HashMap<PathBuf, String>,
    /// The running network operation.
    pub net: Option<NetRun>,
    /// Local write operations in flight.
    pub busy: usize,
    /// Branch picker rows (loaded before it opens) and what it is for.
    pub branches: Vec<BranchInfo>,
    pub branch_pick: crate::git_write::BranchPick,
    /// Create Branch From…: the chosen start point.
    pub create_from: Option<String>,
    /// Remotes of the repository (from the last status).
    pub remotes: Vec<String>,
    /// Sync confirmed for this workspace (`git.confirmSync`).
    pub sync_confirmed: bool,
    /// Source Control explanation (Sync Notes stopped…).
    pub notice: Option<notoir_core::notifications::Translatable>,
    /// The open gutter peek.
    pub peek: Option<GitPeek>,
    /// Clone Repository: URL waiting for its destination folder.
    pub clone_url: Option<String>,
    pub recent_urls: Vec<String>,
    /// Next `git.autoFetch` run.
    pub next_fetch: Option<u64>,
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
            discovery_due: None,
            discovery_running: false,
            auto_save: Default::default(),
            status_seq: 0,
            status_in_flight: 0,
            status_again: false,
            repos: Vec::new(),
            others: HashMap::new(),
            docs: HashMap::new(),
            history: None,
            network_runs: 0,
            last_error: None,
            remote: None,
            output: std::collections::VecDeque::new(),
            drafts: HashMap::new(),
            net: None,
            busy: 0,
            branches: Vec::new(),
            branch_pick: crate::git_write::BranchPick::Switch,
            create_from: None,
            remotes: Vec::new(),
            sync_confirmed: false,
            notice: None,
            peek: None,
            clone_url: None,
            recent_urls: Vec::new(),
            next_fetch: None,
        }
    }
}

/// The repository containing `root`, then (with `nested`) repositories whose
/// work tree is a direct child folder of `root` (monorepo-of-notes setups).
fn discover_repos(backend: &dyn GitBackend, root: &Path, nested: bool) -> Vec<RepoInfo> {
    let mut repos: Vec<RepoInfo> = Vec::new();
    match backend.discover(root) {
        Ok(Some(r)) => repos.push(r),
        Ok(None) => {}
        Err(e) => tracing::warn!(error = %e, "git discovery failed"),
    }
    if nested && let Ok(entries) = std::fs::read_dir(root) {
        let mut dirs: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.is_dir() && p.join(".git").exists())
            .collect();
        dirs.sort();
        for dir in dirs {
            let canonical = dir.canonicalize().unwrap_or(dir.clone());
            if let Ok(Some(r)) = backend.discover(&dir)
                && r.workdir == canonical
                && !repos.contains(&r)
            {
                repos.push(r);
            }
        }
    }
    repos
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

    /// The innermost repository containing `path`, and the path in it.
    pub fn git_locate(&self, path: &Path) -> Option<(RepoInfo, String)> {
        self.git
            .repos
            .iter()
            .filter_map(|r| r.relative(path).map(|rel| (r, rel)))
            .max_by_key(|(r, _)| r.workdir.as_os_str().len())
            .map(|(r, rel)| (r.clone(), rel))
    }

    /// Status of the repository at `workdir`.
    pub fn git_status_for(&self, workdir: &Path) -> Option<Arc<RepoStatus>> {
        if self.git.repo.as_ref().is_some_and(|r| r.workdir == workdir) {
            Some(self.git.status.clone())
        } else {
            self.git.others.get(workdir).cloned()
        }
    }

    /// Git status of a file or folder for decorations.
    pub fn git_status_of(&self, path: &Path, dir: bool) -> Option<FileStatus> {
        let (repo, rel) = self.git_locate(path)?;
        let status = self.git_status_for(&repo.workdir)?;
        if dir {
            status.folder(&rel)
        } else {
            status.of(&rel)
        }
    }

    /// Source Control works on `workdir` (one of the workspace's
    /// repositories).
    pub fn git_select_repository(&mut self, workdir: &Path) -> bool {
        let Some(next) = self
            .git
            .repos
            .iter()
            .find(|r| r.workdir == workdir)
            .cloned()
        else {
            return false;
        };
        if self.git.repo.as_ref() == Some(&next) {
            return true;
        }
        if let Some(prev) = self.git.repo.take() {
            let status = std::mem::take(&mut self.git.status);
            self.git.others.insert(prev.workdir, status);
        }
        self.git.status = self.git.others.remove(&next.workdir).unwrap_or_default();
        self.git.repo = Some(next);
        self.git.remotes.clear();
        self.git.notice = None;
        self.git_start_status();
        true
    }

    /// Rediscover even when the workspace has no known repository.
    pub fn git_rediscover_soon(&mut self) {
        if self.git_enabled() && self.workspace.is_some() {
            self.git
                .discovery_due
                .get_or_insert(self.now_ms + STATUS_DEBOUNCE_MS);
        }
        self.git_refresh_soon();
    }

    /// Schedules a debounced status refresh.
    pub fn git_refresh_soon(&mut self) {
        if !self.git.repos.is_empty() {
            let due = self.now_ms + STATUS_DEBOUNCE_MS;
            self.git.status_due = Some(self.git.status_due.map_or(due, |d| d.min(due)));
        }
    }

    /// File-system events: work-tree changes and index/HEAD/ref updates
    /// refresh the status (other `.git` churn is ignored).
    pub fn git_paths_changed(&mut self, paths: &[PathBuf]) {
        if self.git.repos.is_empty()
            || paths.iter().any(|p| {
                p.file_name().is_some_and(|n| n == ".git")
                    || (p.file_name().is_some_and(|n| n == "HEAD")
                        && p.parent()
                            .and_then(|p| p.file_name())
                            .is_some_and(|n| n == ".git"))
            })
        {
            self.git_rediscover_soon();
        }
        let relevant = self.git.repos.iter().any(|repo| {
            paths.iter().any(|p| match p.strip_prefix(&repo.git_dir) {
                Ok(inside) => {
                    let s = inside.to_string_lossy();
                    s == "index"
                        || s == "HEAD"
                        || s == "packed-refs"
                        || s == "MERGE_HEAD"
                        || s.starts_with("refs")
                }
                Err(_) => p.starts_with(&repo.workdir),
            })
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
            self.git.discovery_running = false;
            self.git.discovery_due = None;
            self.git.auto_save = Default::default();
            self.git.generation += 1;
            self.git.repo = None;
            self.git.repos.clear();
            self.git.others.clear();
            self.git.status = Arc::new(RepoStatus::default());
            self.git.docs.clear();
            self.git.status_due = None;
            self.git.sync_confirmed = false;
            self.git.next_fetch = None;
            self.git.notice = None;
            self.git.remotes.clear();
            if root.is_some() {
                self.git.discovery_due = Some(self.now_ms);
            }
        }
        if self.git.discovery_due.is_some_and(|due| self.now_ms >= due)
            && !self.git.discovery_running
        {
            self.git.discovery_due = None;
            if let Some(root) = root {
                self.git.discovery_running = true;
                let backend = self.git.backend.clone();
                let generation = self.git.generation;
                let nested = self.settings.bool("git.scanNestedRepos", None);
                self.git_queue.submit(move || {
                    let repos = discover_repos(&*backend, &root, nested);
                    Some(IoResult::Git(Box::new(GitResult::Discovered {
                        generation,
                        repos,
                    })))
                });
            }
        }
        self.git_auto_tick();
        if let Some(due) = self.git.status_due
            && self.now_ms >= due
        {
            self.git.status_due = None;
            self.git_start_status();
        }
        self.git_auto_fetch();
        self.git_gutters();
        // An edit or a new base closes the peek (its lines moved).
        if let Some(p) = &self.git.peek {
            let stale = self
                .workbench
                .document(p.doc)
                .is_none_or(|d| d.revision != p.revision)
                || self.workbench.view(p.view).is_none()
                || self
                    .git
                    .docs
                    .get(&p.doc)
                    .is_none_or(|d| d.base_version != p.base_version);
            if stale {
                self.git.peek = None;
            }
        }
    }

    pub fn git_next_due(&self) -> Option<u64> {
        [
            self.git.status_due,
            self.git.next_fetch,
            self.git.discovery_due,
            (!self.git_busy() && self.git.auto_save.has_ready_work()).then_some(self.now_ms),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// `git.autoFetch` (off by default): fetch every `git.autoFetchPeriod`
    /// seconds; never pulls.
    fn git_auto_fetch(&mut self) {
        if self.git.repo.is_none() || !self.settings.bool("git.autoFetch", None) {
            self.git.next_fetch = None;
            return;
        }
        let period = self.settings.int("git.autoFetchPeriod", None).max(30) as u64 * 1000;
        match self.git.next_fetch {
            None => self.git.next_fetch = Some(self.now_ms + period),
            Some(due) if self.now_ms >= due => {
                self.git.next_fetch = Some(self.now_ms + period);
                if !self.git_busy() {
                    self.git_start_net(crate::git_write::NetOp::Fetch);
                }
            }
            Some(_) => {}
        }
    }

    /// Starts a status run on its own thread; a newer run supersedes the
    /// running one (its result is dropped). Writes stay on the Git queue.
    pub fn git_start_status(&mut self) {
        let Some(repo) = self.git.repo.clone() else {
            return;
        };
        if self.git.status_in_flight >= STATUS_PARALLEL {
            self.git.status_again = true;
            return;
        }
        self.git.status_seq += 1;
        self.git.status_in_flight += 1;
        let seq = self.git.status_seq;
        let others: Vec<RepoInfo> = self
            .git
            .repos
            .iter()
            .filter(|r| **r != repo)
            .cloned()
            .collect();
        let backend = self.git.backend.clone();
        let generation = self.git.generation;
        let poster = self.git_queue.poster();
        let spawned = std::thread::Builder::new()
            .name("notoir-git-status".into())
            .spawn(move || {
                let result = backend.status(&repo, StatusOpts::default());
                let remotes = backend.remotes(&repo).unwrap_or_default();
                let others = others
                    .into_iter()
                    .filter_map(|r| {
                        let s = backend.status(&r, StatusOpts::default()).ok()?;
                        Some((r.workdir, s))
                    })
                    .collect();
                poster.post(IoResult::Git(Box::new(GitResult::Status {
                    generation,
                    seq,
                    workdir: repo.workdir,
                    result,
                    remotes,
                    others,
                })));
            });
        if let Err(e) = spawned {
            tracing::error!(error = %e, "could not start Git status");
            self.git.status_in_flight -= 1;
        }
    }

    /// Status runs not finished yet (tests).
    pub fn git_status_pending(&self) -> bool {
        self.git.status_in_flight > 0 || self.git.status_due.is_some()
    }

    pub fn handle_git(&mut self, result: GitResult) {
        match result {
            GitResult::Auto(result) => self.git_auto_result(result),
            GitResult::Discovered { generation, repos } if generation == self.git.generation => {
                for r in &repos {
                    tracing::info!(workdir = %r.workdir.display(), "git repository");
                }
                self.git.discovery_running = false;
                let found = !repos.is_empty() && self.git.repos.is_empty();
                self.git.repo = self
                    .git
                    .repo
                    .as_ref()
                    .and_then(|prev| repos.iter().find(|r| r.workdir == prev.workdir))
                    .cloned()
                    .or_else(|| repos.first().cloned());
                self.git.repos = repos;
                self.git_start_status();
                // `git.pullOnOpen` (off by default).
                if found && self.settings.bool("git.pullOnOpen", None) {
                    self.git_pull(None);
                }
            }
            GitResult::Status { generation, .. } if generation != self.git.generation => {
                self.git.status_in_flight = self.git.status_in_flight.saturating_sub(1);
            }
            GitResult::Status {
                seq,
                workdir,
                result,
                remotes,
                others,
                ..
            } => {
                self.git.status_in_flight = self.git.status_in_flight.saturating_sub(1);
                let current = seq == self.git.status_seq
                    && self.git.repo.as_ref().is_some_and(|r| r.workdir == workdir);
                if !current {
                    // Superseded by a newer run (or another repository).
                    if std::mem::take(&mut self.git.status_again) {
                        self.git_start_status();
                    }
                    return;
                }
                self.git.remotes = remotes;
                self.git.others = others.into_iter().map(|(w, s)| (w, Arc::new(s))).collect();
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
            GitResult::Op {
                generation,
                op,
                result,
            } => self.git_op_done(generation, op, result),
            GitResult::Branches { generation, result } if generation == self.git.generation => {
                self.git_branches_loaded(result)
            }
            GitResult::Output(o) => self.git_output(o),
            GitResult::Net {
                generation,
                op,
                result,
            } => self.git_net_done(generation, op, result),
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
        if self.git.repos.is_empty() {
            return;
        }
        let base_kind = if self.settings.str("git.gutterBase", None) == "head" {
            DiffBase::Head
        } else {
            DiffBase::Index
        };
        let docs: Vec<(DocumentId, RepoInfo, String, u64, bool)> = self
            .workbench
            .documents
            .values()
            .filter_map(|d| {
                let (repo, rel) = self.git_locate(d.path()?)?;
                Some((
                    d.id,
                    repo,
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
        for (doc, repo, rel, revision, small) in docs {
            let untracked = matches!(
                self.git_status_for(&repo.workdir).and_then(|s| s.of(&rel)),
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

    /// Opens the peek for the change at `line` of `view`'s document.
    pub fn git_peek(&mut self, view: notoir_core::ids::ViewId, line: usize) -> bool {
        let Some(doc) = self.workbench.view(view).map(|v| v.document_id) else {
            return false;
        };
        let Some(d) = self.workbench.document(doc) else {
            return false;
        };
        let (buffer, revision) = (d.buffer.rope().clone(), d.revision);
        let Some(g) = self.git.docs.get(&doc) else {
            return false;
        };
        let Some(base) = g.base.clone() else {
            return false;
        };
        let base_version = g.base_version;
        let diff = notoir_editor::diff::diff_ropes(&base, &buffer, &DiffOptions::default());
        let total = diff.hunks.len();
        let Some((index, h)) = diff.hunks.iter().enumerate().find(|(_, h)| {
            h.right.contains(&line) || (h.right.is_empty() && h.right.start == line)
        }) else {
            return false;
        };
        let kind = match (h.left.is_empty(), h.right.is_empty()) {
            (true, _) => notoir_editor::diff::LineChangeKind::Added,
            (_, true) => notoir_editor::diff::LineChangeKind::Deleted,
            _ => notoir_editor::diff::LineChangeKind::Modified,
        };
        let base_text = base
            .slice(line_char(&base, h.left.start)..line_char(&base, h.left.end))
            .to_string();
        self.git.peek = Some(GitPeek {
            view,
            doc,
            kind,
            buf: h.right.clone(),
            base: h.left.clone(),
            base_text,
            // Stage Change writes the active repository's index.
            stageable: self.settings.str("git.gutterBase", None) != "head"
                && d.path()
                    .and_then(|p| self.git_locate(p))
                    .is_some_and(|(r, _)| self.git.repo.as_ref() == Some(&r)),
            index,
            total,
            revision,
            base_version,
        });
        true
    }

    /// Revert Change: the peeked lines go back to the base (undoable).
    pub fn git_revert_change(&mut self) {
        let Some(p) = self.git.peek.take() else {
            return;
        };
        let Some(rope) = self
            .workbench
            .document(p.doc)
            .map(|d| d.buffer.rope().clone())
        else {
            return;
        };
        let (start, end) = (line_char(&rope, p.buf.start), line_char(&rope, p.buf.end));
        let now = self.now_ms;
        let result = self.workbench.edit(
            p.view,
            notoir_editor::ChangeSet::replace(start, end, p.base_text.as_str()),
            notoir_editor::Selection::caret(start),
            notoir_editor::EditKind::Other,
            now,
        );
        if let Err(e) = result {
            tracing::info!(error = ?e, "revert change refused");
        }
    }

    /// Stage Change: the index takes the peeked lines from the buffer; the
    /// file and the rest of the index are untouched.
    pub fn git_stage_change(&mut self) {
        let Some(p) = self.git.peek.take().filter(|p| p.stageable) else {
            return;
        };
        let Some(d) = self.workbench.document(p.doc) else {
            return;
        };
        let rope = d.buffer.rope().clone();
        let Some(rel) = d.path().and_then(|p| self.git_rel(p)) else {
            return;
        };
        let Some(base) = self.git.docs.get(&p.doc).and_then(|g| g.base.clone()) else {
            return;
        };
        let new_lines = rope
            .slice(line_char(&rope, p.buf.start)..line_char(&rope, p.buf.end))
            .to_string();
        let mut index = base.as_ref().clone();
        let (a, b) = (
            line_char(&index, p.base.start),
            line_char(&index, p.base.end),
        );
        index.remove(a..b);
        index.insert(a, &new_lines);
        self.git_submit(crate::git_write::LocalOp::StageContent {
            rel,
            text: index.to_string(),
        });
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
        let Some((repo, rel)) = self.git_locate(&path) else {
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
        let Some((repo, rel)) = self.git_locate(&path) else {
            return false;
        };
        let backend = self.git.backend.clone();
        self.git_queue.submit(move || {
            let result = if rev == INDEX_REV {
                backend.read_base(&repo, &rel, DiffBase::Index)
            } else {
                backend.read_blob(&repo, &rel, &rev)
            }
            .map(|b| b.map(utf8));
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
