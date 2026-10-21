//! Git integration backend (docs/specs/28-git.md, AD-7): a request/response
//! API with no UI state, run by the app on worker threads. Local operations
//! use `git2` (vendored libgit2, no network transports, [`Git2Backend`]);
//! fetch, pull, push and clone run the system `git` ([`SystemGit`]).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

mod local;
mod remote;

pub use local::Git2Backend;
pub use remote::{
    CancellationToken, FetchOpts, GitOutput, GitRemote, OutputSink, PullMode, PullResult, PushOpts,
    SystemGit, classify_stderr, parse_progress,
};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GitError {
    /// `index.lock` exists: another Git process is running.
    #[error("another Git process is running")]
    Locked,
    /// The remote asked for credentials that are not configured.
    #[error("authentication required")]
    Auth,
    /// Pull: local and upstream both have commits (not fast-forwardable).
    #[error("the branch and its upstream have diverged")]
    Diverged,
    /// Push or pull without an upstream branch.
    #[error("the branch has no upstream")]
    NoUpstream,
    /// Push rejected (the remote has commits this branch lacks).
    #[error("push rejected")]
    Rejected,
    /// A pull with rebase hit conflicts; the rebase was aborted.
    #[error("conflict")]
    Conflict,
    /// Checkout would overwrite local changes to these files.
    #[error("local changes would be overwritten")]
    CheckoutConflict(Vec<String>),
    /// `user.name` / `user.email` are not configured.
    #[error("no Git identity configured")]
    Identity,
    #[error("nothing to commit")]
    NothingToCommit,
    #[error("invalid name")]
    InvalidName,
    #[error("already exists")]
    Exists,
    /// No system `git` to run network operations with.
    #[error("Git is not installed")]
    NoGit,
    #[error("cancelled")]
    Cancelled,
    #[error("{0}")]
    Git(String),
}

impl From<git2::Error> for GitError {
    fn from(e: git2::Error) -> Self {
        match e.code() {
            git2::ErrorCode::Locked => GitError::Locked,
            git2::ErrorCode::Exists => GitError::Exists,
            git2::ErrorCode::InvalidSpec => GitError::InvalidName,
            _ => GitError::Git(e.message().to_owned()),
        }
    }
}

/// A discovered repository.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoInfo {
    /// Work tree root (canonical).
    pub workdir: PathBuf,
    pub git_dir: PathBuf,
}

impl RepoInfo {
    /// Path relative to the work tree with `/` separators, if inside it.
    pub fn relative(&self, path: &Path) -> Option<String> {
        let rel = path.strip_prefix(&self.workdir).ok()?;
        Some(rel.to_string_lossy().replace('\\', "/"))
    }

    /// Absolute path of a work-tree-relative path.
    pub fn absolute(&self, rel: &str) -> PathBuf {
        self.workdir.join(rel)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FileStatus {
    // Ordered weakest → strongest for folder roll-ups.
    Ignored,
    Deleted,
    Renamed,
    Untracked,
    Added,
    Modified,
    Conflicted,
}

impl FileStatus {
    /// Explorer badge letter (spec 07).
    pub fn letter(self) -> &'static str {
        match self {
            FileStatus::Modified => "M",
            FileStatus::Added => "A",
            FileStatus::Untracked => "U",
            FileStatus::Deleted => "D",
            FileStatus::Conflicted => "!",
            FileStatus::Renamed => "R",
            FileStatus::Ignored => "",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RepoState {
    #[default]
    Clean,
    Merging,
    Rebasing,
    /// Cherry-pick, revert, bisect…
    Other,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HeadInfo {
    /// Local branch name; `None` when detached (or unborn without a name).
    pub branch: Option<String>,
    /// Short SHA of HEAD (`None` on an unborn branch).
    pub short_sha: Option<String>,
    /// Ahead / behind the upstream as of the last fetch.
    pub ahead: usize,
    pub behind: usize,
    pub has_upstream: bool,
    /// Upstream as `remote/branch`.
    pub upstream: Option<String>,
    pub state: RepoState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StatusOpts {
    pub include_untracked: bool,
    pub recurse_untracked_dirs: bool,
    pub exclude_submodules: bool,
    /// More changes than this are truncated.
    pub limit: usize,
}

impl Default for StatusOpts {
    fn default() -> Self {
        Self {
            include_untracked: true,
            recurse_untracked_dirs: true,
            exclude_submodules: true,
            limit: 10_000,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RepoStatus {
    /// Work-tree-relative path (`/` separators) → combined status
    /// (decorations).
    pub files: BTreeMap<String, FileStatus>,
    /// Changes in the index against HEAD (Staged Changes).
    pub staged: BTreeMap<String, FileStatus>,
    /// Work-tree changes against the index, untracked files and conflicts
    /// (Changes / Merge Conflicts).
    pub unstaged: BTreeMap<String, FileStatus>,
    pub head: HeadInfo,
    pub truncated: bool,
}

impl RepoStatus {
    pub fn of(&self, rel: &str) -> Option<FileStatus> {
        self.files.get(rel).copied()
    }

    /// Strongest status below folder `rel` ("" = root): conflict > modified
    /// > added > untracked (deleted children roll up as modified).
    pub fn folder(&self, rel: &str) -> Option<FileStatus> {
        let prefix = if rel.is_empty() {
            String::new()
        } else {
            format!("{rel}/")
        };
        self.files
            .range(prefix.clone()..)
            .take_while(|(k, _)| k.starts_with(&prefix))
            .map(|(_, s)| match s {
                FileStatus::Deleted | FileStatus::Renamed => FileStatus::Modified,
                s => *s,
            })
            .filter(|s| *s != FileStatus::Ignored)
            .max()
    }

    /// Deleted files directly or deeply inside folder `rel`.
    pub fn deleted_in(&self, rel: &str) -> usize {
        let prefix = format!("{rel}/");
        self.files
            .iter()
            .filter(|(k, s)| {
                **s == FileStatus::Deleted && (rel.is_empty() || k.starts_with(&prefix))
            })
            .count()
    }

    /// The work tree has changes (`⎇ main*`).
    pub fn dirty(&self) -> bool {
        self.files.values().any(|s| *s != FileStatus::Ignored)
    }

    /// Unmerged paths.
    pub fn conflicts(&self) -> impl Iterator<Item = &String> {
        self.unstaged
            .iter()
            .filter(|(_, s)| **s == FileStatus::Conflicted)
            .map(|(k, _)| k)
    }

    /// Work-tree changes that are not conflicts (the Changes section).
    pub fn changes(&self) -> impl Iterator<Item = (&String, FileStatus)> {
        self.unstaged
            .iter()
            .filter(|(_, s)| **s != FileStatus::Conflicted)
            .map(|(k, s)| (k, *s))
    }
}

/// What a file's buffer is compared with for gutter bars.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffBase {
    Index,
    Head,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitSummary {
    pub id: String,
    pub short: String,
    pub author: String,
    /// Seconds since the Unix epoch.
    pub time: i64,
    pub summary: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CommitOpts {
    /// Replace the last commit (message and tree).
    pub amend: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchInfo {
    /// `main`, or `origin/main` for remote branches.
    pub name: String,
    pub remote: bool,
    pub current: bool,
    /// Upstream of a local branch (`origin/main`).
    pub upstream: Option<String>,
    /// Tip commit time (seconds since the Unix epoch).
    pub time: i64,
}

/// Exact bytes successfully written by one save operation.
#[derive(Clone, Debug)]
pub struct SavedFile {
    pub path: String,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct SavedCommit {
    pub sha: String,
    pub message: String,
}

/// Local Git operations.
pub trait GitBackend: Send + Sync {
    /// The repository containing `path`, if any.
    fn discover(&self, path: &Path) -> Result<Option<RepoInfo>, GitError>;
    fn status(&self, repo: &RepoInfo, opts: StatusOpts) -> Result<RepoStatus, GitError>;
    /// Content of `rel` in the index or HEAD; `None` when it is not there
    /// (new file).
    fn read_base(
        &self,
        repo: &RepoInfo,
        rel: &str,
        base: DiffBase,
    ) -> Result<Option<Vec<u8>>, GitError>;
    /// Content of `rel` at a commit-ish; `None` when the file did not exist.
    fn read_blob(&self, repo: &RepoInfo, rel: &str, rev: &str)
    -> Result<Option<Vec<u8>>, GitError>;
    /// Commits touching `rel`, newest first.
    fn log_file(
        &self,
        repo: &RepoInfo,
        rel: &str,
        limit: usize,
    ) -> Result<Vec<CommitSummary>, GitError>;
    /// Adds work-tree paths to the index (deleted files are removed).
    fn stage(&self, repo: &RepoInfo, paths: &[String]) -> Result<(), GitError>;
    /// Replaces the index content of `rel` (Stage Change from the gutter
    /// peek); the work tree is untouched.
    fn stage_content(&self, repo: &RepoInfo, rel: &str, content: &[u8]) -> Result<(), GitError>;
    /// Every change, respecting `.gitignore`.
    fn stage_all(&self, repo: &RepoInfo) -> Result<(), GitError>;
    /// Resets paths in the index to HEAD.
    fn unstage(&self, repo: &RepoInfo, paths: &[String]) -> Result<(), GitError>;
    /// Restores tracked paths in the work tree from the index. Untracked
    /// files are the caller's (they go to the trash).
    fn discard(&self, repo: &RepoInfo, paths: &[String]) -> Result<(), GitError>;
    /// Commits the index; returns the short SHA.
    fn commit(&self, repo: &RepoInfo, msg: &str, opts: CommitOpts) -> Result<String, GitError>;
    /// Commit saved snapshots without including unrelated index entries.
    /// `branch` is checked before writing; an unchanged batch returns None.
    fn commit_saved(
        &self,
        repo: &RepoInfo,
        files: &[SavedFile],
        branch: &str,
        timestamp: &str,
    ) -> Result<Option<SavedCommit>, GitError>;
    /// Local branches (newest first), then remote branches.
    fn branches(&self, repo: &RepoInfo) -> Result<Vec<BranchInfo>, GitError>;
    /// Switches to a local branch, or creates a tracking branch for a remote
    /// one (`origin/x` → `x`). Refuses when local changes would be lost.
    fn checkout(&self, repo: &RepoInfo, branch: &str) -> Result<(), GitError>;
    fn create_branch(
        &self,
        repo: &RepoInfo,
        name: &str,
        from: Option<&str>,
        checkout: bool,
    ) -> Result<(), GitError>;
    fn init(&self, path: &Path) -> Result<RepoInfo, GitError>;
    /// Whether merging the upstream into HEAD would conflict (Sync Notes
    /// dry run, computed in memory).
    fn would_conflict(&self, repo: &RepoInfo) -> Result<bool, GitError>;
    /// Configured remotes.
    fn remotes(&self, repo: &RepoInfo) -> Result<Vec<String>, GitError>;
}
