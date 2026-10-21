//! `git2` implementation of the local operations.

use std::path::{Path, PathBuf};

use git2::{Repository, Status, StatusOptions};

use crate::{
    BranchInfo, CommitOpts, CommitSummary, DiffBase, FileStatus, GitBackend, GitError, HeadInfo,
    RepoInfo, RepoState, RepoStatus, StatusOpts,
};

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

/// The index side of a status (Staged Changes).
fn staged_status(s: Status) -> Option<FileStatus> {
    if s.is_conflicted() {
        None
    } else if s.is_index_new() {
        Some(FileStatus::Added)
    } else if s.is_index_renamed() {
        Some(FileStatus::Renamed)
    } else if s.is_index_deleted() {
        Some(FileStatus::Deleted)
    } else if s.is_index_modified() || s.is_index_typechange() {
        Some(FileStatus::Modified)
    } else {
        None
    }
}

/// The work-tree side of a status (Changes / Merge Conflicts).
fn unstaged_status(s: Status) -> Option<FileStatus> {
    if s.is_conflicted() {
        Some(FileStatus::Conflicted)
    } else if s.is_wt_new() {
        Some(FileStatus::Untracked)
    } else if s.is_wt_renamed() {
        Some(FileStatus::Renamed)
    } else if s.is_wt_deleted() {
        Some(FileStatus::Deleted)
    } else if s.is_wt_modified() || s.is_wt_typechange() {
        Some(FileStatus::Modified)
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
                    h.upstream = up.name().ok().flatten().map(str::to_owned);
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

fn slash(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// Checks out `target` (safe: refuses to overwrite local changes) and
/// points HEAD at `refname`.
fn switch_to(r: &Repository, refname: &str) -> Result<(), GitError> {
    let obj = r.revparse_single(refname)?;
    let mut conflicts = Vec::new();
    let result = {
        let mut cb = git2::build::CheckoutBuilder::new();
        cb.safe()
            .notify_on(git2::CheckoutNotificationType::CONFLICT)
            .notify(|_, path, _, _, _| {
                if let Some(p) = path {
                    conflicts.push(slash(p));
                }
                true
            });
        r.checkout_tree(&obj, Some(&mut cb))
    };
    match result {
        Ok(()) => {}
        Err(_) if !conflicts.is_empty() => return Err(GitError::CheckoutConflict(conflicts)),
        Err(e) => return Err(e.into()),
    }
    r.set_head(refname)?;
    Ok(())
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
            let s = e.status();
            let Some(st) = map_status(s) else {
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
            let rel = slash(&path);
            if let Some(x) = staged_status(s) {
                out.staged.insert(rel.clone(), x);
            }
            if let Some(x) = unstaged_status(s) {
                let wt = e
                    .index_to_workdir()
                    .and_then(|d| d.new_file().path().map(slash))
                    .unwrap_or_else(|| rel.clone());
                out.unstaged.insert(wt, x);
            }
            out.files.insert(rel, st);
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

    fn stage(&self, repo: &RepoInfo, paths: &[String]) -> Result<(), GitError> {
        let r = open(repo)?;
        let mut index = r.index()?;
        for rel in paths {
            let p = Path::new(rel);
            if repo.absolute(rel).symlink_metadata().is_ok() {
                index.add_path(p)?;
            } else {
                index.remove_path(p)?;
            }
        }
        index.write()?;
        Ok(())
    }

    fn stage_content(&self, repo: &RepoInfo, rel: &str, content: &[u8]) -> Result<(), GitError> {
        let r = open(repo)?;
        let mut index = r.index()?;
        let id = r.blob(content)?;
        let entry = match index.get_path(Path::new(rel), 0) {
            Some(mut e) => {
                e.id = id;
                e.file_size = content.len() as u32;
                e
            }
            None => git2::IndexEntry {
                ctime: git2::IndexTime::new(0, 0),
                mtime: git2::IndexTime::new(0, 0),
                dev: 0,
                ino: 0,
                mode: 0o100_644,
                uid: 0,
                gid: 0,
                file_size: content.len() as u32,
                id,
                flags: 0,
                flags_extended: 0,
                path: rel.as_bytes().to_vec(),
            },
        };
        index.add(&entry)?;
        index.write()?;
        Ok(())
    }

    fn stage_all(&self, repo: &RepoInfo) -> Result<(), GitError> {
        let r = open(repo)?;
        let mut index = r.index()?;
        index.add_all(["*"], git2::IndexAddOption::DEFAULT, None)?;
        index.update_all(["*"], None)?;
        index.write()?;
        Ok(())
    }

    fn unstage(&self, repo: &RepoInfo, paths: &[String]) -> Result<(), GitError> {
        let r = open(repo)?;
        match r.head().and_then(|h| h.peel_to_commit()) {
            Ok(head) => {
                r.reset_default(Some(head.as_object()), paths.iter().map(String::as_str))?;
            }
            Err(_) => {
                // Unborn: nothing in HEAD, unstaging removes the entry.
                let mut index = r.index()?;
                for rel in paths {
                    index.remove_path(Path::new(rel))?;
                }
                index.write()?;
            }
        }
        Ok(())
    }

    fn discard(&self, repo: &RepoInfo, paths: &[String]) -> Result<(), GitError> {
        if paths.is_empty() {
            return Ok(());
        }
        let r = open(repo)?;
        let mut cb = git2::build::CheckoutBuilder::new();
        cb.force().disable_pathspec_match(true);
        for p in paths {
            cb.path(p.as_str());
        }
        r.checkout_index(None, Some(&mut cb))?;
        Ok(())
    }

    fn commit(&self, repo: &RepoInfo, msg: &str, opts: CommitOpts) -> Result<String, GitError> {
        let mut r = open(repo)?;
        let mut merge_heads = Vec::new();
        if !opts.amend && r.state() == git2::RepositoryState::Merge {
            r.mergehead_foreach(|oid| {
                merge_heads.push(*oid);
                true
            })?;
        }
        let r = r;
        let sig = r.signature().map_err(|_| GitError::Identity)?;
        let mut index = r.index()?;
        if index.has_conflicts() {
            return Err(GitError::Git("unmerged files".into()));
        }
        let tree = r.find_tree(index.write_tree()?)?;
        let head = r.head().ok().and_then(|h| h.peel_to_commit().ok());
        let id = if opts.amend {
            let Some(head) = head else {
                return Err(GitError::NothingToCommit);
            };
            head.amend(Some("HEAD"), None, Some(&sig), None, Some(msg), Some(&tree))?
        } else {
            let unchanged = match &head {
                Some(h) => h.tree_id() == tree.id(),
                None => index.is_empty(),
            };
            if unchanged && merge_heads.is_empty() {
                return Err(GitError::NothingToCommit);
            }
            let mut parents: Vec<git2::Commit> = head.into_iter().collect();
            for oid in merge_heads {
                parents.push(r.find_commit(oid)?);
            }
            let refs: Vec<&git2::Commit> = parents.iter().collect();
            let id = r.commit(Some("HEAD"), &sig, &sig, msg, &tree, &refs)?;
            if !refs.is_empty() && r.state() == git2::RepositoryState::Merge {
                r.cleanup_state()?;
            }
            id
        };
        Ok(id.to_string()[..7].to_owned())
    }

    fn commit_saved(
        &self,
        repo: &RepoInfo,
        files: &[crate::SavedFile],
        branch: &str,
        timestamp: &str,
    ) -> Result<Option<crate::SavedCommit>, GitError> {
        let r = open(repo)?;
        if r.state() != git2::RepositoryState::Clean || r.index()?.has_conflicts() {
            return Err(GitError::Git(
                "Repository has an unfinished merge, rebase, or conflicts".into(),
            ));
        }
        let reference = format!("refs/heads/{branch}");
        if r.find_reference("HEAD")?.symbolic_target()? != Some(reference.as_str()) {
            return Err(GitError::Git("Branch changed since saving".into()));
        }
        // Hold both the branch and index locks throughout the update.
        let mut transaction = r.transaction()?;
        transaction.lock_ref(&reference)?;
        let lock_path = repo.git_dir.join("index.lock");
        let lock = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::AlreadyExists {
                    GitError::Locked
                } else {
                    GitError::Git(e.to_string())
                }
            })?;
        struct LockGuard(Option<PathBuf>);
        impl Drop for LockGuard {
            fn drop(&mut self) {
                if let Some(path) = self.0.take() {
                    let _ = std::fs::remove_file(path);
                }
            }
        }
        let mut guard = LockGuard(Some(lock_path.clone()));
        let head = r.head().ok().and_then(|h| h.peel_to_commit().ok());
        let base = head.as_ref().map(|h| h.tree()).transpose()?;
        let mut tree_index = git2::Index::new()?;
        if let Some(tree) = &base {
            tree_index.read_tree(tree)?;
        }
        let index = r.index()?;
        let temp = tempfile::tempdir_in(&repo.git_dir).map_err(|e| GitError::Git(e.to_string()))?;
        let mut updated = git2::Index::open(&temp.path().join("index"))?;
        for entry in index.iter() {
            updated.add(&entry)?;
        }
        let mut changed = std::collections::BTreeSet::new();
        for file in files {
            let path = Path::new(&file.path);
            if path.is_absolute()
                || path
                    .components()
                    .any(|c| !matches!(c, std::path::Component::Normal(_)))
            {
                return Err(GitError::Git("Invalid repository-relative path".into()));
            }
            let original = tree_index.get_path(path, 0);
            if original.is_none()
                && index.get_path(path, 0).is_none()
                && r.status_should_ignore(path)?
            {
                continue;
            }
            let oid = r.blob(&file.bytes)?;
            if original.as_ref().is_some_and(|e| e.id == oid) {
                continue;
            }
            let mut entry =
                original
                    .or_else(|| index.get_path(path, 0))
                    .unwrap_or(git2::IndexEntry {
                        ctime: git2::IndexTime::new(0, 0),
                        mtime: git2::IndexTime::new(0, 0),
                        dev: 0,
                        ino: 0,
                        mode: 0o100644,
                        uid: 0,
                        gid: 0,
                        file_size: 0,
                        id: oid,
                        flags: 0,
                        flags_extended: 0,
                        path: file.path.as_bytes().to_vec(),
                    });
            entry.id = oid;
            entry.file_size = file.bytes.len() as u32;
            entry.mtime = git2::IndexTime::new(0, 0);
            entry.ctime = git2::IndexTime::new(0, 0);
            tree_index.add(&entry)?;
            updated.add(&entry)?;
            changed.insert(file.path.clone());
        }
        if changed.is_empty() {
            return Ok(None);
        }
        let message = if changed.len() == 1 {
            let path = changed.first().expect("one path");
            let name = Path::new(path)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy();
            let (date, time) = timestamp.split_once(' ').unwrap_or((timestamp, "00:00:00"));
            format!(
                "File saved - {name} - Date: {date} Time: {}",
                time.replace(':', "-")
            )
        } else {
            format!(
                "Save files - {timestamp}\n\n{}",
                changed.into_iter().collect::<Vec<_>>().join("\n")
            )
        };
        let sig = r.signature().map_err(|_| GitError::Identity)?;
        let tree = r.find_tree(tree_index.write_tree_to(&r)?)?;
        let parents: Vec<_> = head.iter().collect();
        let oid = r.commit(None, &sig, &sig, &message, &tree, &parents)?;
        updated.write()?;
        let bytes =
            std::fs::read(temp.path().join("index")).map_err(|e| GitError::Git(e.to_string()))?;
        use std::io::Write;
        let mut lock = lock;
        lock.write_all(&bytes)
            .and_then(|()| lock.sync_all())
            .map_err(|e| GitError::Git(e.to_string()))?;
        drop(lock);
        // Recheck HEAD: external checkouts must not redirect this operation.
        if r.find_reference("HEAD")?.symbolic_target()? != Some(reference.as_str()) {
            return Err(GitError::Git("Branch changed since saving".into()));
        }
        transaction.set_target(&reference, oid, Some(&sig), &message)?;
        transaction.commit()?;
        // Disarm before releasing the filename: another process may immediately
        // acquire index.lock after the rename, and its lock must remain intact.
        let owned_lock = guard.0.take().expect("owned index lock");
        if let Err(e) = std::fs::rename(&owned_lock, repo.git_dir.join("index")) {
            let _ = std::fs::remove_file(&owned_lock);
            return Err(GitError::Git(e.to_string()));
        }
        Ok(Some(crate::SavedCommit {
            sha: oid.to_string()[..7].to_owned(),
            message,
        }))
    }

    fn branches(&self, repo: &RepoInfo) -> Result<Vec<BranchInfo>, GitError> {
        let r = open(repo)?;
        let mut local = Vec::new();
        let mut remote = Vec::new();
        for b in r.branches(None)? {
            let (b, kind) = b?;
            let Some(name) = b.name().ok().flatten().map(str::to_owned) else {
                continue;
            };
            if name.ends_with("/HEAD") {
                continue;
            }
            let time = b
                .get()
                .peel_to_commit()
                .map(|c| c.time().seconds())
                .unwrap_or(0);
            let info = BranchInfo {
                remote: kind == git2::BranchType::Remote,
                current: b.is_head(),
                upstream: b
                    .upstream()
                    .ok()
                    .and_then(|u| u.name().ok().flatten().map(str::to_owned)),
                time,
                name,
            };
            if info.remote {
                remote.push(info);
            } else {
                local.push(info);
            }
        }
        local.sort_by(|a, b| b.time.cmp(&a.time).then_with(|| a.name.cmp(&b.name)));
        remote.sort_by(|a, b| a.name.cmp(&b.name));
        local.extend(remote);
        Ok(local)
    }

    fn checkout(&self, repo: &RepoInfo, branch: &str) -> Result<(), GitError> {
        let r = open(repo)?;
        if r.find_branch(branch, git2::BranchType::Local).is_ok() {
            return switch_to(&r, &format!("refs/heads/{branch}"));
        }
        let remote = r.find_branch(branch, git2::BranchType::Remote)?;
        let local = branch.split_once('/').map_or(branch, |(_, b)| b);
        if r.find_branch(local, git2::BranchType::Local).is_err() {
            let commit = remote.get().peel_to_commit()?;
            let mut created = r.branch(local, &commit, false)?;
            created.set_upstream(Some(branch))?;
        }
        switch_to(&r, &format!("refs/heads/{local}"))
    }

    fn create_branch(
        &self,
        repo: &RepoInfo,
        name: &str,
        from: Option<&str>,
        checkout: bool,
    ) -> Result<(), GitError> {
        if !git2::Branch::name_is_valid(name).unwrap_or(false) {
            return Err(GitError::InvalidName);
        }
        let r = open(repo)?;
        let commit = r
            .revparse_single(from.unwrap_or("HEAD"))?
            .peel_to_commit()?;
        r.branch(name, &commit, false)?;
        if checkout {
            switch_to(&r, &format!("refs/heads/{name}"))?;
        }
        Ok(())
    }

    fn init(&self, path: &Path) -> Result<RepoInfo, GitError> {
        Repository::init(path)?;
        self.discover(path)?
            .ok_or_else(|| GitError::Git("repository not found after init".into()))
    }

    fn would_conflict(&self, repo: &RepoInfo) -> Result<bool, GitError> {
        let r = open(repo)?;
        let head = r.head()?;
        let name = head.shorthand().map_err(GitError::from)?.to_owned();
        let ours = head.peel_to_commit()?;
        let branch = r.find_branch(&name, git2::BranchType::Local)?;
        let theirs = branch
            .upstream()
            .map_err(|_| GitError::NoUpstream)?
            .get()
            .peel_to_commit()?;
        if ours.id() == theirs.id() || r.graph_descendant_of(ours.id(), theirs.id())? {
            return Ok(false);
        }
        Ok(r.merge_commits(&ours, &theirs, None)?.has_conflicts())
    }

    fn remotes(&self, repo: &RepoInfo) -> Result<Vec<String>, GitError> {
        let r = open(repo)?;
        Ok(r.remotes()?
            .iter()
            .filter_map(|n| n.ok().flatten().map(str::to_owned))
            .collect())
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
        let mut cfg = r.config().unwrap();
        cfg.set_str("user.name", "Tester").unwrap();
        cfg.set_str("user.email", "t@example.com").unwrap();
        // Checkouts keep LF whatever the global config (Windows: CRLF).
        cfg.set_bool("core.autocrlf", false).unwrap();
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
    fn saved_commit_preserves_unrelated_staging_and_uses_saved_bytes() {
        let (_d, repo) = fixture();
        let r = Repository::open(&repo.git_dir).unwrap();
        std::fs::write(repo.absolute("docs/b.md"), "staged elsewhere\n").unwrap();
        Git2Backend.stage(&repo, &["docs/b.md".into()]).unwrap();
        std::fs::write(repo.absolute("a.md"), "later disk content\n").unwrap();
        let branch = r.head().unwrap().shorthand().unwrap().to_owned();
        let result = Git2Backend
            .commit_saved(
                &repo,
                &[crate::SavedFile {
                    path: "a.md".into(),
                    bytes: b"saved snapshot\n".to_vec(),
                }],
                &branch,
                "2026-10-21 14:32:08",
            )
            .unwrap()
            .unwrap();
        assert_eq!(
            result.message,
            "File saved - a.md - Date: 2026-10-21 Time: 14-32-08"
        );
        assert_eq!(
            Git2Backend
                .read_base(&repo, "a.md", DiffBase::Head)
                .unwrap()
                .unwrap(),
            b"saved snapshot\n"
        );
        assert_eq!(
            Git2Backend
                .read_base(&repo, "a.md", DiffBase::Index)
                .unwrap()
                .unwrap(),
            b"saved snapshot\n"
        );
        assert_eq!(
            Git2Backend
                .read_base(&repo, "docs/b.md", DiffBase::Head)
                .unwrap()
                .unwrap(),
            b"b\n"
        );
        assert_eq!(
            Git2Backend
                .read_base(&repo, "docs/b.md", DiffBase::Index)
                .unwrap()
                .unwrap(),
            b"staged elsewhere\n"
        );
        assert_eq!(
            std::fs::read(repo.absolute("a.md")).unwrap(),
            b"later disk content\n"
        );
    }

    #[test]
    fn saved_batch_filters_sorts_and_avoids_empty_commits() {
        let (_d, repo) = fixture();
        let r = Repository::open(&repo.git_dir).unwrap();
        let branch = r.head().unwrap().shorthand().unwrap().to_owned();
        std::fs::write(repo.absolute(".gitignore"), "ignored.md\n").unwrap();
        let file = |path: &str, bytes: &[u8]| crate::SavedFile {
            path: path.into(),
            bytes: bytes.to_vec(),
        };
        let result = Git2Backend
            .commit_saved(
                &repo,
                &[
                    file("z.md", b"new"),
                    file("a.md", b"changed"),
                    file("ignored.md", b"ignored"),
                    file("docs/b.md", b"b\n"),
                ],
                &branch,
                "2026-10-21 14:32:08",
            )
            .unwrap()
            .unwrap();
        assert_eq!(
            result.message,
            "Save files - 2026-10-21 14:32:08\n\na.md\nz.md"
        );
        assert!(
            Git2Backend
                .commit_saved(
                    &repo,
                    &[file("a.md", b"changed")],
                    &branch,
                    "2026-10-21 14:32:09"
                )
                .unwrap()
                .is_none()
        );
        assert!(!repo.git_dir.join("index.lock").exists());
    }

    #[test]
    fn saved_commit_rejects_locks_branch_changes_and_conflicts_without_staging() {
        let (_d, repo) = fixture();
        let r = Repository::open(&repo.git_dir).unwrap();
        let branch = r.head().unwrap().shorthand().unwrap().to_owned();
        let head = r.head().unwrap().target().unwrap();
        let files = [crate::SavedFile {
            path: "a.md".into(),
            bytes: b"new".to_vec(),
        }];
        std::fs::write(repo.git_dir.join("index.lock"), "external lock").unwrap();
        assert!(matches!(
            Git2Backend.commit_saved(&repo, &files, &branch, "2026-10-21 14:32:08"),
            Err(GitError::Locked)
        ));
        assert_eq!(
            std::fs::read(repo.git_dir.join("index.lock")).unwrap(),
            b"external lock"
        );
        std::fs::remove_file(repo.git_dir.join("index.lock")).unwrap();
        assert!(
            Git2Backend
                .commit_saved(&repo, &files, "another", "2026-10-21 14:32:08")
                .is_err()
        );
        std::fs::write(repo.git_dir.join("MERGE_HEAD"), head.to_string()).unwrap();
        assert!(
            Git2Backend
                .commit_saved(&repo, &files, &branch, "2026-10-21 14:32:08")
                .is_err()
        );
        assert_eq!(r.head().unwrap().target().unwrap(), head);
        assert_eq!(
            Git2Backend
                .read_base(&repo, "a.md", DiffBase::Index)
                .unwrap()
                .unwrap(),
            b"one\ntwo\n"
        );
    }

    #[test]
    fn saved_commit_supports_unborn_repository() {
        let dir = tempfile::tempdir().unwrap();
        let r = Repository::init(dir.path()).unwrap();
        r.config().unwrap().set_str("user.name", "Tester").unwrap();
        r.config()
            .unwrap()
            .set_str("user.email", "t@example.com")
            .unwrap();
        let branch = r
            .find_reference("HEAD")
            .unwrap()
            .symbolic_target()
            .unwrap()
            .unwrap()
            .strip_prefix("refs/heads/")
            .unwrap()
            .to_owned();
        let repo = Git2Backend.discover(dir.path()).unwrap().unwrap();
        let result = Git2Backend
            .commit_saved(
                &repo,
                &[crate::SavedFile {
                    path: "new.md".into(),
                    bytes: b"first".to_vec(),
                }],
                &branch,
                "2026-10-21 14:32:08",
            )
            .unwrap()
            .unwrap();
        assert!(result.message.starts_with("File saved - new.md -"));
        assert_eq!(
            r.head().unwrap().peel_to_commit().unwrap().parent_count(),
            0
        );
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
        assert!(s.staged.is_empty());
        assert_eq!(s.unstaged.len(), 3);
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
        // Staged and unstaged sides both show.
        let s = Git2Backend.status(&repo, StatusOpts::default()).unwrap();
        assert_eq!(s.staged.get("a.md"), Some(&FileStatus::Modified));
        assert_eq!(s.unstaged.get("a.md"), Some(&FileStatus::Modified));
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
        // Stage and unstage before the first commit.
        Git2Backend.stage(&repo, &["x.md".into()]).unwrap();
        let s = Git2Backend.status(&repo, StatusOpts::default()).unwrap();
        assert_eq!(s.staged.get("x.md"), Some(&FileStatus::Added));
        Git2Backend.unstage(&repo, &["x.md".into()]).unwrap();
        let s = Git2Backend.status(&repo, StatusOpts::default()).unwrap();
        assert!(s.staged.is_empty());
    }

    #[test]
    fn stage_commit_unstage_discard() {
        let (_d, repo) = fixture();
        let w = &repo.workdir;
        std::fs::write(w.join("a.md"), "changed\n").unwrap();
        std::fs::write(w.join("docs/b.md"), "b2\n").unwrap();
        std::fs::write(w.join("new.md"), "n\n").unwrap();
        assert_eq!(
            Git2Backend.commit(&repo, "empty", CommitOpts::default()),
            Err(GitError::NothingToCommit)
        );
        Git2Backend.stage(&repo, &["a.md".into()]).unwrap();
        let s = Git2Backend.status(&repo, StatusOpts::default()).unwrap();
        assert_eq!(s.staged.keys().collect::<Vec<_>>(), ["a.md"]);
        assert!(s.unstaged.contains_key("docs/b.md"));
        let short = Git2Backend
            .commit(&repo, "Update a", CommitOpts::default())
            .unwrap();
        assert_eq!(short.len(), 7);
        let log = Git2Backend.log_file(&repo, "a.md", 5).unwrap();
        assert_eq!(log[0].summary, "Update a");
        let s = Git2Backend.status(&repo, StatusOpts::default()).unwrap();
        assert!(s.staged.is_empty());
        // Amend keeps one commit.
        Git2Backend.stage(&repo, &["docs/b.md".into()]).unwrap();
        Git2Backend
            .commit(&repo, "Update a and b", CommitOpts { amend: true })
            .unwrap();
        let log = Git2Backend.log_file(&repo, "docs/b.md", 5).unwrap();
        assert_eq!(log[0].summary, "Update a and b");
        assert_eq!(Git2Backend.log_file(&repo, "a.md", 5).unwrap().len(), 2);
        // Unstage then discard.
        std::fs::write(w.join("a.md"), "again\n").unwrap();
        Git2Backend.stage(&repo, &["a.md".into()]).unwrap();
        Git2Backend.unstage(&repo, &["a.md".into()]).unwrap();
        let s = Git2Backend.status(&repo, StatusOpts::default()).unwrap();
        assert!(s.staged.is_empty());
        assert_eq!(s.unstaged.get("a.md"), Some(&FileStatus::Modified));
        Git2Backend.discard(&repo, &["a.md".into()]).unwrap();
        assert_eq!(
            std::fs::read_to_string(w.join("a.md")).unwrap(),
            "changed\n"
        );
        // Deletions stage as removals; stage all picks up untracked files.
        std::fs::remove_file(w.join("docs/b.md")).unwrap();
        Git2Backend.stage_all(&repo).unwrap();
        let s = Git2Backend.status(&repo, StatusOpts::default()).unwrap();
        assert_eq!(s.staged.get("docs/b.md"), Some(&FileStatus::Deleted));
        assert_eq!(s.staged.get("new.md"), Some(&FileStatus::Added));
        assert!(s.unstaged.is_empty());
    }

    #[test]
    fn stage_content_changes_only_the_index() {
        let (_d, repo) = fixture();
        std::fs::write(repo.workdir.join("a.md"), "ONE\ntwo\nTHREE\n").unwrap();
        Git2Backend
            .stage_content(&repo, "a.md", b"ONE\ntwo\n")
            .unwrap();
        let idx = Git2Backend
            .read_base(&repo, "a.md", DiffBase::Index)
            .unwrap();
        assert_eq!(idx.as_deref(), Some(&b"ONE\ntwo\n"[..]));
        assert_eq!(
            std::fs::read_to_string(repo.workdir.join("a.md")).unwrap(),
            "ONE\ntwo\nTHREE\n"
        );
        let s = Git2Backend.status(&repo, StatusOpts::default()).unwrap();
        assert_eq!(s.staged.get("a.md"), Some(&FileStatus::Modified));
        assert_eq!(s.unstaged.get("a.md"), Some(&FileStatus::Modified));
    }

    #[test]
    fn missing_identity() {
        let dir = tempfile::tempdir().unwrap();
        let r = Repository::init(dir.path()).unwrap();
        // An empty identity in the repo config overrides any global one.
        let mut cfg = r.config().unwrap();
        cfg.set_str("user.name", "").unwrap();
        cfg.set_str("user.email", "").unwrap();
        std::fs::write(dir.path().join("x"), "x").unwrap();
        let repo = Git2Backend.discover(dir.path()).unwrap().unwrap();
        Git2Backend.stage_all(&repo).unwrap();
        assert_eq!(
            Git2Backend.commit(&repo, "m", CommitOpts::default()),
            Err(GitError::Identity)
        );
    }

    #[test]
    fn branches_checkout_and_conflicts() {
        let (_d, repo) = fixture();
        let w = &repo.workdir;
        let main = Git2Backend
            .status(&repo, StatusOpts::default())
            .unwrap()
            .head
            .branch
            .unwrap();
        assert_eq!(
            Git2Backend.create_branch(&repo, "bad name..", None, false),
            Err(GitError::InvalidName)
        );
        Git2Backend
            .create_branch(&repo, "feature", None, true)
            .unwrap();
        std::fs::write(w.join("a.md"), "feature\n").unwrap();
        let r = Repository::open(&repo.git_dir).unwrap();
        commit_all(&r, "feature work");
        let b = Git2Backend.branches(&repo).unwrap();
        assert!(b.iter().any(|b| b.name == "feature" && b.current));
        assert!(b.iter().any(|b| b.name == main && !b.current));
        // Switching back changes the file.
        Git2Backend.checkout(&repo, &main).unwrap();
        assert_eq!(
            std::fs::read_to_string(w.join("a.md")).unwrap(),
            "one\ntwo\n"
        );
        // A local edit that the switch would overwrite is refused.
        std::fs::write(w.join("a.md"), "local\n").unwrap();
        match Git2Backend.checkout(&repo, "feature") {
            Err(GitError::CheckoutConflict(files)) => assert_eq!(files, ["a.md"]),
            other => panic!("expected conflict, got {other:?}"),
        }
        assert_eq!(std::fs::read_to_string(w.join("a.md")).unwrap(), "local\n");
        let s = Git2Backend.status(&repo, StatusOpts::default()).unwrap();
        assert_eq!(s.head.branch.as_deref(), Some(main.as_str()));
        assert_eq!(
            Git2Backend.create_branch(&repo, "feature", None, false),
            Err(GitError::Exists)
        );
    }

    #[test]
    fn predicted_conflict_against_upstream() {
        let (_d, repo) = fixture();
        let r = Repository::open(&repo.git_dir).unwrap();
        let main = r.head().unwrap().shorthand().unwrap().to_owned();
        // A fake upstream: a branch with a conflicting edit.
        Git2Backend
            .create_branch(&repo, "upstream", None, false)
            .unwrap();
        r.find_branch(&main, git2::BranchType::Local)
            .unwrap()
            .set_upstream(Some("upstream"))
            .unwrap();
        assert!(!Git2Backend.would_conflict(&repo).unwrap(), "same commit");
        Git2Backend.checkout(&repo, "upstream").unwrap();
        std::fs::write(repo.workdir.join("a.md"), "theirs\ntwo\n").unwrap();
        commit_all(&r, "theirs");
        Git2Backend.checkout(&repo, &main).unwrap();
        std::fs::write(repo.workdir.join("docs/b.md"), "other file\n").unwrap();
        commit_all(&r, "unrelated");
        assert!(!Git2Backend.would_conflict(&repo).unwrap());
        std::fs::write(repo.workdir.join("a.md"), "ours\ntwo\n").unwrap();
        commit_all(&r, "ours");
        assert!(Git2Backend.would_conflict(&repo).unwrap());
    }

    #[test]
    fn init_creates_a_repository() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Git2Backend.init(dir.path()).unwrap();
        assert!(repo.git_dir.ends_with(".git/") || repo.git_dir.ends_with(".git"));
        assert!(Git2Backend.remotes(&repo).unwrap().is_empty());
    }
}
