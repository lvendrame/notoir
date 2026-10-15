//! Git integration backend (docs/specs/28-git.md, AD-7): a request/response
//! API with no UI state, run by the app on worker threads. Local reads use
//! `git2` (vendored libgit2, no network transports); network operations
//! arrive with the system `git` adapter in M19.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use git2::{Repository, Status, StatusOptions};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GitError {
    /// `index.lock` exists: another Git process is running.
    #[error("another Git process is running")]
    Locked,
    #[error("{0}")]
    Git(String),
}

impl From<git2::Error> for GitError {
    fn from(e: git2::Error) -> Self {
        if e.code() == git2::ErrorCode::Locked {
            GitError::Locked
        } else {
            GitError::Git(e.message().to_owned())
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
    /// Work-tree-relative path (`/` separators) → status.
    pub files: BTreeMap<String, FileStatus>,
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

/// Local Git operations (read side for M18).
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
}

/// `git2`-based backend.
pub struct Git2Backend;

fn open(repo: &RepoInfo) -> Result<Repository, GitError> {
    Ok(Repository::open(&repo.git_dir)?)
}

fn map_status(s: Status) -> Option<FileStatus> {
    if s.is_conflicted() {
        Some(FileStatus::Conflicted)
    } else if s.is_index_new() {
        Some(FileStatus::Added)
    } else if s.is_wt_new() {
        Some(FileStatus::Untracked)
    } else if s.is_index_renamed() || s.is_wt_renamed() {
        Some(FileStatus::Renamed)
    } else if s.is_index_deleted() || s.is_wt_deleted() {
        Some(FileStatus::Deleted)
    } else if s.is_index_modified()
        || s.is_wt_modified()
        || s.is_index_typechange()
        || s.is_wt_typechange()
    {
        Some(FileStatus::Modified)
    } else if s.is_ignored() {
        Some(FileStatus::Ignored)
    } else {
        None
    }
}

fn head_info(r: &Repository) -> HeadInfo {
    let mut h = HeadInfo {
        state: match r.state() {
            git2::RepositoryState::Clean => RepoState::Clean,
            git2::RepositoryState::Merge => RepoState::Merging,
            git2::RepositoryState::Rebase
            | git2::RepositoryState::RebaseInteractive
            | git2::RepositoryState::RebaseMerge => RepoState::Rebasing,
            _ => RepoState::Other,
        },
        ..Default::default()
    };
    match r.head() {
        Ok(head) => {
            if head.is_branch() {
                h.branch = head.shorthand().ok().map(str::to_owned);
            }
            if let Some(oid) = head.target() {
                h.short_sha = Some(oid.to_string()[..7].to_owned());
                if let Some(name) = h.branch.clone()
                    && let Ok(branch) = r.find_branch(&name, git2::BranchType::Local)
                    && let Ok(up) = branch.upstream()
                    && let Some(up_oid) = up.get().target()
                {
                    h.has_upstream = true;
                    if let Ok((a, b)) = r.graph_ahead_behind(oid, up_oid) {
                        h.ahead = a;
                        h.behind = b;
                    }
                }
            }
        }
        Err(_) => {
            // Unborn branch: HEAD names it symbolically.
            if let Ok(reference) = r.find_reference("HEAD")
                && let Ok(Some(target)) = reference.symbolic_target()
            {
                h.branch = target.strip_prefix("refs/heads/").map(str::to_owned);
            }
        }
    }
    h
}

impl GitBackend for Git2Backend {
    fn discover(&self, path: &Path) -> Result<Option<RepoInfo>, GitError> {
        match Repository::discover(path) {
            Ok(r) => {
                let Some(workdir) = r.workdir() else {
                    return Ok(None); // bare
                };
                let workdir = workdir
                    .canonicalize()
                    .unwrap_or_else(|_| workdir.to_path_buf());
                Ok(Some(RepoInfo {
                    workdir,
                    git_dir: r.path().to_path_buf(),
                }))
            }
            Err(e) if e.code() == git2::ErrorCode::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn status(&self, repo: &RepoInfo, opts: StatusOpts) -> Result<RepoStatus, GitError> {
        let r = open(repo)?;
        let mut so = StatusOptions::new();
        so.include_untracked(opts.include_untracked)
            .recurse_untracked_dirs(opts.recurse_untracked_dirs)
            .exclude_submodules(opts.exclude_submodules)
            .include_ignored(false)
            .renames_head_to_index(true);
        let statuses = r.statuses(Some(&mut so))?;
        let mut out = RepoStatus {
            head: head_info(&r),
            ..Default::default()
        };
        for e in statuses.iter() {
            let Some(st) = map_status(e.status()) else {
                continue;
            };
            let path = e
                .head_to_index()
                .and_then(|d| d.new_file().path().map(Path::to_path_buf))
                .or_else(|| e.path().ok().map(PathBuf::from));
            let Some(path) = path else { continue };
            if out.files.len() >= opts.limit {
                out.truncated = true;
                break;
            }
            out.files
                .insert(path.to_string_lossy().replace('\\', "/"), st);
        }
        Ok(out)
    }

    fn read_base(
        &self,
        repo: &RepoInfo,
        rel: &str,
        base: DiffBase,
    ) -> Result<Option<Vec<u8>>, GitError> {
        match base {
            DiffBase::Head => self.read_blob(repo, rel, "HEAD"),
            DiffBase::Index => {
                let r = open(repo)?;
                let index = r.index()?;
                let Some(entry) = index.get_path(Path::new(rel), 0) else {
                    return Ok(None);
                };
                Ok(Some(r.find_blob(entry.id)?.content().to_vec()))
            }
        }
    }

    fn read_blob(
        &self,
        repo: &RepoInfo,
        rel: &str,
        rev: &str,
    ) -> Result<Option<Vec<u8>>, GitError> {
        let r = open(repo)?;
        let commit = match r.revparse_single(rev).and_then(|o| o.peel_to_commit()) {
            Ok(c) => c,
            // Unborn HEAD: nothing committed yet.
            Err(e)
                if e.code() == git2::ErrorCode::NotFound
                    || e.code() == git2::ErrorCode::UnbornBranch =>
            {
                return Ok(None);
            }
            Err(e) => return Err(e.into()),
        };
        let tree = commit.tree()?;
        let entry = match tree.get_path(Path::new(rel)) {
            Ok(e) => e,
            Err(e) if e.code() == git2::ErrorCode::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let blob = r.find_blob(entry.id())?;
        Ok(Some(blob.content().to_vec()))
    }

    fn log_file(
        &self,
        repo: &RepoInfo,
        rel: &str,
        limit: usize,
    ) -> Result<Vec<CommitSummary>, GitError> {
        let r = open(repo)?;
        let mut walk = r.revwalk()?;
        if walk.push_head().is_err() {
            return Ok(Vec::new());
        }
        walk.set_sorting(git2::Sort::TIME)?;
        let path = Path::new(rel);
        let id_at = |c: &git2::Commit| c.tree().ok()?.get_path(path).ok().map(|e| e.id());
        let mut out = Vec::new();
        for oid in walk {
            let c = r.find_commit(oid?)?;
            let mine = id_at(&c);
            let changed = if c.parent_count() == 0 {
                mine.is_some()
            } else {
                // Changed against every parent (merges that only take one
                // side's version are skipped, like `git log` simplification).
                c.parents().all(|p| id_at(&p) != mine)
            };
            if !changed {
                continue;
            }
            let id = c.id().to_string();
            out.push(CommitSummary {
                short: id[..7].to_owned(),
                id,
                author: c.author().name().unwrap_or_default().to_owned(),
                time: c.time().seconds(),
                summary: c.summary().ok().flatten().unwrap_or_default().to_owned(),
            });
            if out.len() >= limit {
                break;
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A repo with one commit of `a.md` and `docs/b.md`.
    pub(crate) fn fixture() -> (tempfile::TempDir, RepoInfo) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let r = Repository::init(&root).unwrap();
        std::fs::write(root.join("a.md"), "one\ntwo\n").unwrap();
        std::fs::create_dir(root.join("docs")).unwrap();
        std::fs::write(root.join("docs/b.md"), "b\n").unwrap();
        commit_all(&r, "init");
        let info = Git2Backend.discover(&root).unwrap().unwrap();
        (dir, info)
    }

    pub(crate) fn commit_all(r: &Repository, msg: &str) {
        let mut index = r.index().unwrap();
        index
            .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree = r.find_tree(index.write_tree().unwrap()).unwrap();
        let sig = git2::Signature::now("Tester", "t@example.com").unwrap();
        let parent = r.head().ok().and_then(|h| h.peel_to_commit().ok());
        let parents: Vec<&git2::Commit> = parent.iter().collect();
        r.commit(Some("HEAD"), &sig, &sig, msg, &tree, &parents)
            .unwrap();
    }

    #[test]
    fn discover_status_and_folders() {
        let (_d, repo) = fixture();
        assert!(
            Git2Backend
                .discover(&repo.workdir.join("docs"))
                .unwrap()
                .is_some()
        );
        let outside = tempfile::tempdir().unwrap();
        assert!(Git2Backend.discover(outside.path()).unwrap().is_none());
        std::fs::write(repo.workdir.join("a.md"), "one\nTWO\n").unwrap();
        std::fs::write(repo.workdir.join("docs/new.md"), "n\n").unwrap();
        std::fs::remove_file(repo.workdir.join("docs/b.md")).unwrap();
        let s = Git2Backend.status(&repo, StatusOpts::default()).unwrap();
        assert_eq!(s.of("a.md"), Some(FileStatus::Modified));
        assert_eq!(s.of("docs/new.md"), Some(FileStatus::Untracked));
        assert_eq!(s.of("docs/b.md"), Some(FileStatus::Deleted));
        assert_eq!(
            s.folder("docs"),
            Some(FileStatus::Modified),
            "deletion rolls up"
        );
        assert_eq!(s.deleted_in("docs"), 1);
        assert_eq!(s.folder(""), Some(FileStatus::Modified));
        assert!(s.dirty());
        let h = &s.head;
        assert!(h.branch.is_some());
        assert_eq!(h.short_sha.as_ref().map(String::len), Some(7));
        assert!(!h.has_upstream);
        assert_eq!(h.state, RepoState::Clean);
        // Truncation.
        let s = Git2Backend
            .status(
                &repo,
                StatusOpts {
                    limit: 1,
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(s.truncated);
    }

    #[test]
    fn blobs_index_and_history() {
        let (_d, repo) = fixture();
        let r = Repository::open(&repo.git_dir).unwrap();
        std::fs::write(repo.workdir.join("a.md"), "one\ntwo\nthree\n").unwrap();
        commit_all(&r, "add three");
        std::fs::write(repo.workdir.join("a.md"), "staged\n").unwrap();
        let mut index = r.index().unwrap();
        index.add_path(Path::new("a.md")).unwrap();
        index.write().unwrap();
        std::fs::write(repo.workdir.join("a.md"), "work tree\n").unwrap();
        let head = Git2Backend
            .read_base(&repo, "a.md", DiffBase::Head)
            .unwrap();
        assert_eq!(head.as_deref(), Some(&b"one\ntwo\nthree\n"[..]));
        let idx = Git2Backend
            .read_base(&repo, "a.md", DiffBase::Index)
            .unwrap();
        assert_eq!(idx.as_deref(), Some(&b"staged\n"[..]));
        assert_eq!(
            Git2Backend
                .read_base(&repo, "nope.md", DiffBase::Head)
                .unwrap(),
            None
        );
        let log = Git2Backend.log_file(&repo, "a.md", 10).unwrap();
        let msgs: Vec<&str> = log.iter().map(|c| c.summary.as_str()).collect();
        assert_eq!(msgs, ["add three", "init"]);
        assert_eq!(
            Git2Backend.log_file(&repo, "docs/b.md", 10).unwrap().len(),
            1
        );
        let old = Git2Backend
            .read_blob(&repo, "a.md", &format!("{}~1", log[0].id))
            .unwrap();
        assert_eq!(old.as_deref(), Some(&b"one\ntwo\n"[..]));
    }

    #[test]
    fn unborn_repo() {
        let dir = tempfile::tempdir().unwrap();
        Repository::init(dir.path()).unwrap();
        std::fs::write(dir.path().join("x.md"), "x").unwrap();
        let repo = Git2Backend.discover(dir.path()).unwrap().unwrap();
        let s = Git2Backend.status(&repo, StatusOpts::default()).unwrap();
        assert_eq!(s.of("x.md"), Some(FileStatus::Untracked));
        assert!(s.head.branch.is_some(), "unborn branch named");
        assert_eq!(s.head.short_sha, None);
        assert_eq!(
            Git2Backend
                .read_base(&repo, "x.md", DiffBase::Head)
                .unwrap(),
            None
        );
        assert!(Git2Backend.log_file(&repo, "x.md", 5).unwrap().is_empty());
    }
}
