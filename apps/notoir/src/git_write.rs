//! Git write side (docs/specs/28): Source Control operations (stage,
//! unstage, discard, commit), branches, network operations through the
//! system `git` (fetch, pull, push, sync, clone) with streamed Git Output and
//! status bar progress, Initialize Repository and Sync Notes.
//!
//! Local operations run on the Git queue, one at a time. A network
//! operation runs on its own thread (one at a time) so the status keeps
//! refreshing while it waits on the remote; it never blocks the UI and only
//! a user command starts one.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use notoir_core::command::{CommandInvocation, InvocationSource};
use notoir_core::ids::DocumentId;
use notoir_core::notifications::{Notification, NotificationSource, Severity, Translatable};
use notoir_core::panels::PanelId;
use notoir_git::{
    BranchInfo, CancellationToken, CommitOpts, FetchOpts, FileStatus, GitBackend, GitError,
    GitOutput, GitRemote, OutputSink, PullMode, PullResult, PushOpts, RepoInfo, StatusOpts,
    SystemGit,
};
use serde_json::json;

use crate::compare::CompareSource;
use crate::core::{AppCore, Effect};
use crate::files::{Answer, Pending, Prompt, Then};
use crate::git::GitResult;
use crate::io::IoResult;
use crate::t;

/// Git Output keeps this many lines.
pub const OUTPUT_MAX_LINES: usize = 10_000;
/// Clone Repository remembers this many URLs.
pub const RECENT_URLS: usize = 10;
/// Default `notes.sync.commitMessage`.
pub const NOTES_MESSAGE: &str = "Notes sync {{date}} {{time}} from {{hostname}}";

/// A local write operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LocalOp {
    Stage(Vec<String>),
    StageAll,
    Unstage(Vec<String>),
    /// Tracked files are restored from the index; untracked ones go to the
    /// trash.
    Discard {
        tracked: Vec<String>,
        untracked: Vec<String>,
    },
    Commit {
        message: String,
        amend: bool,
        /// Stage everything first (Commit All, smart commit).
        stage_all: bool,
        /// Push after committing (Commit & Push).
        push: bool,
    },
    Checkout(String),
    CreateBranch {
        name: String,
        from: Option<String>,
    },
    Init(PathBuf),
}

/// A network operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NetOp {
    Fetch,
    Pull(PullMode),
    /// `publish`: the first push of a branch, which sets its upstream.
    Push {
        publish: Option<(String, String)>,
    },
    /// Pull, then push.
    Sync(PullMode),
    /// Commit everything, fetch, rebase when no conflict is predicted, push.
    SyncNotes {
        template: String,
    },
    Clone {
        url: String,
        dest: PathBuf,
    },
}

impl NetOp {
    /// Locale key suffix (`git:progress.push`, `git:failed.push`).
    pub fn key(&self) -> &'static str {
        match self {
            NetOp::Fetch => "fetch",
            NetOp::Pull(_) => "pull",
            NetOp::Push { .. } => "push",
            NetOp::Sync(_) => "sync",
            NetOp::SyncNotes { .. } => "syncNotes",
            NetOp::Clone { .. } => "clone",
        }
    }

    /// Changes the work tree (open files may need reloading).
    fn updates_files(&self) -> bool {
        matches!(
            self,
            NetOp::Pull(_) | NetOp::Sync(_) | NetOp::SyncNotes { .. }
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NetDone {
    Done,
    Pulled(PullResult),
    /// Sync Notes stopped before pulling: the upstream changed the same
    /// lines as a local commit.
    NotesStopped,
}

/// The running network operation.
pub struct NetRun {
    pub op: NetOp,
    pub cancel: CancellationToken,
    /// Last progress line: percent and phase.
    pub progress: Option<(u8, String)>,
}

/// What the branch picker is choosing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BranchPick {
    Switch,
    /// The start point of Create Branch From…
    CreateFrom,
}

/// Git confirmations (shown by the prompt queue).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitPrompt {
    Discard {
        tracked: Vec<String>,
        untracked: Vec<String>,
    },
    /// Nothing staged: Stage All & Commit / Cancel.
    SmartCommit {
        message: String,
        amend: bool,
        push: bool,
    },
    /// Pull with Rebase / Pull with Merge / Cancel.
    Diverged {
        upstream: String,
    },
    /// First push of a branch: Publish / Cancel.
    Publish {
        branch: String,
        remote: String,
    },
    /// Sync pulls and pushes: OK / Cancel (once per workspace).
    SyncConfirm,
    Init(PathBuf),
    /// Open / Open in New Window / Cancel.
    OpenCloned(PathBuf),
}

fn run_local(
    backend: &dyn GitBackend,
    dialogs: &dyn crate::files::Dialogs,
    repo: Option<&RepoInfo>,
    op: &LocalOp,
) -> Result<String, GitError> {
    if let LocalOp::Init(path) = op {
        return backend.init(path).map(|_| String::new());
    }
    let Some(repo) = repo else {
        return Err(GitError::Git("no repository".into()));
    };
    let none = |r: Result<(), GitError>| r.map(|()| String::new());
    match op {
        LocalOp::Stage(p) => none(backend.stage(repo, p)),
        LocalOp::StageAll => none(backend.stage_all(repo)),
        LocalOp::Unstage(p) => none(backend.unstage(repo, p)),
        LocalOp::Discard { tracked, untracked } => {
            backend.discard(repo, tracked)?;
            if !untracked.is_empty() {
                let paths: Vec<PathBuf> = untracked.iter().map(|r| repo.absolute(r)).collect();
                dialogs.trash(&paths).map_err(GitError::Git)?;
            }
            Ok(String::new())
        }
        LocalOp::Commit {
            message,
            amend,
            stage_all,
            ..
        } => {
            if *stage_all {
                backend.stage_all(repo)?;
            }
            backend.commit(repo, message, CommitOpts { amend: *amend })
        }
        LocalOp::Checkout(b) => none(backend.checkout(repo, b)),
        LocalOp::CreateBranch { name, from } => {
            none(backend.create_branch(repo, name, from.as_deref(), true))
        }
        LocalOp::Init(_) => unreachable!("handled above"),
    }
}

/// `{{date}}`, `{{time}}` (UTC) and `{{hostname}}` in a Sync Notes message.
pub fn render_notes_message(template: &str, now_secs: i64, hostname: &str) -> String {
    let secs = now_secs.rem_euclid(86_400);
    let time = format!("{:02}:{:02}", secs / 3600, secs % 3600 / 60);
    template
        .replace("{{date}}", &crate::shell::overlays::ymd(now_secs))
        .replace("{{time}}", &format!("{time} UTC"))
        .replace("{{hostname}}", hostname)
}

/// The machine name (worker thread: may run `hostname`).
fn hostname() -> String {
    for var in ["HOSTNAME", "COMPUTERNAME"] {
        if let Ok(h) = std::env::var(var)
            && !h.trim().is_empty()
        {
            return h.trim().to_owned();
        }
    }
    std::process::Command::new("hostname")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "unknown".to_owned())
}

fn sync_notes(
    backend: &dyn GitBackend,
    remote: &dyn GitRemote,
    repo: &RepoInfo,
    template: &str,
    out: &OutputSink,
    cancel: &CancellationToken,
) -> Result<NetDone, GitError> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let message = render_notes_message(template, now, &hostname());
    out(GitOutput::Line(t!("git:notes.stepCommit")));
    backend.stage_all(repo)?;
    match backend.commit(repo, &message, CommitOpts::default()) {
        Ok(sha) => out(GitOutput::Line(t!("git:notes.committed", sha = sha))),
        Err(GitError::NothingToCommit) => out(GitOutput::Line(t!("git:notes.nothing"))),
        Err(e) => return Err(e),
    }
    remote.fetch(repo, FetchOpts::default(), out.clone(), cancel.clone())?;
    let head = backend.status(repo, StatusOpts::default())?.head;
    if !head.has_upstream {
        return Err(GitError::NoUpstream);
    }
    if head.behind > 0 {
        if backend.would_conflict(repo)? {
            out(GitOutput::Line(t!("git:notes.stepStopped")));
            return Ok(NetDone::NotesStopped);
        }
        remote.pull(repo, PullMode::Rebase, out.clone(), cancel.clone())?;
    }
    if backend.status(repo, StatusOpts::default())?.head.ahead > 0 {
        remote.push(repo, PushOpts::default(), out.clone(), cancel.clone())?;
    }
    out(GitOutput::Line(t!("git:notes.stepDone")));
    Ok(NetDone::Done)
}

fn run_net(
    backend: &dyn GitBackend,
    remote: &dyn GitRemote,
    repo: Option<&RepoInfo>,
    op: &NetOp,
    out: OutputSink,
    cancel: CancellationToken,
) -> Result<NetDone, GitError> {
    if let NetOp::Clone { url, dest } = op {
        return remote
            .clone_repo(url, dest, out, cancel)
            .map(|()| NetDone::Done);
    }
    let Some(repo) = repo else {
        return Err(GitError::Git("no repository".into()));
    };
    match op {
        NetOp::Fetch => remote
            .fetch(repo, FetchOpts::default(), out, cancel)
            .map(|()| NetDone::Done),
        NetOp::Pull(mode) => remote.pull(repo, *mode, out, cancel).map(NetDone::Pulled),
        NetOp::Push { publish } => remote
            .push(
                repo,
                PushOpts {
                    set_upstream: publish.clone(),
                },
                out,
                cancel,
            )
            .map(|()| NetDone::Done),
        NetOp::Sync(mode) => {
            let pulled = remote.pull(repo, *mode, out.clone(), cancel.clone())?;
            remote.push(repo, PushOpts::default(), out, cancel)?;
            Ok(NetDone::Pulled(pulled))
        }
        NetOp::SyncNotes { template } => sync_notes(backend, remote, repo, template, &out, &cancel),
        NetOp::Clone { .. } => unreachable!("handled above"),
    }
}

/// Folder name for a clone of `url` (`git@x:a/notes.git` → `notes`).
pub fn clone_dir_name(url: &str) -> String {
    let tail = url
        .trim_end_matches('/')
        .rsplit(['/', ':', '\\'])
        .next()
        .unwrap_or("");
    let name = tail.strip_suffix(".git").unwrap_or(tail);
    if name.is_empty() {
        "repository".to_owned()
    } else {
        name.to_owned()
    }
}

fn pull_mode(s: &str) -> PullMode {
    match s {
        "rebase" => PullMode::Rebase,
        "merge" => PullMode::Merge,
        _ => PullMode::FfOnly,
    }
}

/// Why an operation failed, for notifications.
fn reason_key(e: &GitError) -> &'static str {
    match e {
        GitError::Auth => "git:reason.auth",
        GitError::Locked => "git:reason.locked",
        GitError::NoGit => "git:reason.noGit",
        GitError::Rejected => "git:reason.rejected",
        GitError::NoUpstream => "git:reason.noUpstream",
        GitError::Conflict => "git:reason.conflict",
        GitError::Diverged => "git:reason.diverged",
        GitError::Identity => "git:reason.identity",
        GitError::NothingToCommit => "git:reason.nothingToCommit",
        GitError::InvalidName => "git:reason.invalidName",
        GitError::Exists => "git:reason.exists",
        GitError::CheckoutConflict(_) => "git:reason.checkoutConflict",
        GitError::Cancelled => "git:reason.cancelled",
        GitError::Git(_) => "git:reason.other",
    }
}

impl AppCore {
    fn git_say(&mut self, severity: Severity, message: Translatable) {
        let now = self.now_ms;
        self.notifications.notify(
            Notification::new(severity, message).source(NotificationSource::Git),
            now,
        );
    }

    /// The Source Control message draft of the current repository.
    pub fn git_message(&self) -> String {
        self.git
            .repo
            .as_ref()
            .and_then(|r| self.git.drafts.get(&r.workdir))
            .cloned()
            .unwrap_or_default()
    }

    pub fn git_message_mut(&mut self) -> Option<&mut String> {
        let key = self.git.repo.as_ref()?.workdir.clone();
        Some(self.git.drafts.entry(key).or_default())
    }

    /// Appends to the Git Output channel.
    pub fn git_log_line(&mut self, line: String) {
        self.git.output.push_back(line);
        while self.git.output.len() > OUTPUT_MAX_LINES {
            self.git.output.pop_front();
        }
    }

    pub(crate) fn git_output(&mut self, o: GitOutput) {
        match o {
            GitOutput::Command(c) => self.git_log_line(format!("> {c}")),
            GitOutput::Line(l) => self.git_log_line(l),
            GitOutput::Progress { percent, phase } => {
                if let Some(n) = &mut self.git.net {
                    n.progress = Some((percent, phase));
                }
            }
        }
    }

    /// A local or network operation is running (writes wait).
    pub fn git_busy(&self) -> bool {
        self.git.busy > 0 || self.git.net.is_some()
    }

    /// Runs a local write operation on the Git queue.
    pub fn git_submit(&mut self, op: LocalOp) {
        let repo = self.git.repo.clone();
        if repo.is_none() && !matches!(op, LocalOp::Init(_)) {
            return;
        }
        let backend = self.git.backend.clone();
        let dialogs = self.files.dialogs.clone();
        let generation = self.git.generation;
        self.git.busy += 1;
        self.git_queue.submit(move || {
            let result = run_local(&*backend, &*dialogs, repo.as_ref(), &op);
            Some(IoResult::Git(Box::new(GitResult::Op {
                generation,
                op,
                result,
            })))
        });
    }

    /// Asks Git to re-read open files of the repository (checkout, pull,
    /// discard): clean ones reload, dirty ones are flagged (spec 10).
    fn git_reload_open_files(&mut self) {
        let Some(repo) = self.git.repo.clone() else {
            return;
        };
        let paths: Vec<PathBuf> = self
            .workbench
            .documents
            .values()
            .filter_map(|d| d.path().filter(|p| p.starts_with(&repo.workdir)))
            .map(Path::to_path_buf)
            .collect();
        self.check_changed_paths(&paths);
    }

    fn retry_invocation(op: &LocalOp) -> CommandInvocation {
        let inv = |id: &str, args: serde_json::Value| {
            CommandInvocation::new(id.to_owned(), InvocationSource::Internal).with_args(args)
        };
        match op {
            LocalOp::Stage(p) => inv("git.stage", json!({ "paths": p })),
            LocalOp::StageAll => inv("git.stageAll", json!({})),
            LocalOp::Unstage(p) => inv("git.unstage", json!({ "paths": p })),
            LocalOp::Discard { tracked, untracked } => {
                let all: Vec<&String> = tracked.iter().chain(untracked).collect();
                inv("git.discard", json!({ "paths": all }))
            }
            LocalOp::Commit { message, amend, .. } => {
                inv("git.commit", json!({ "message": message, "amend": amend }))
            }
            LocalOp::Checkout(b) => inv("git.checkout", json!({ "branch": b })),
            LocalOp::CreateBranch { name, from } => {
                inv("git.createBranch", json!({ "name": name, "from": from }))
            }
            LocalOp::Init(_) => inv("git.init", json!({})),
        }
    }

    pub(crate) fn git_op_done(
        &mut self,
        generation: u64,
        op: LocalOp,
        result: Result<String, GitError>,
    ) {
        self.git.busy = self.git.busy.saturating_sub(1);
        if let LocalOp::Init(path) = &op {
            match &result {
                Ok(_) => {
                    self.git_log_line(t!("git:output.initialized", path = path.display()));
                    // Detect the new repository.
                    self.git.root = None;
                }
                Err(e) => self.git_failed("git:failed.init", e, Some(&op)),
            }
            return;
        }
        if generation != self.git.generation {
            return;
        }
        match result {
            Ok(sha) => match &op {
                LocalOp::Commit { message, push, .. } => {
                    let summary = message.lines().next().unwrap_or_default().to_owned();
                    self.git_log_line(t!("git:output.committed", sha = sha, summary = summary));
                    if let Some(draft) = self.git_message_mut()
                        && draft.trim() == message.trim()
                    {
                        draft.clear();
                    }
                    // `git.autoPush` is restricted: only a trusted workspace
                    // (or the user's own settings) can turn it on.
                    if *push || self.settings.bool("git.autoPush", None) {
                        self.git_push();
                    }
                }
                LocalOp::Checkout(_) | LocalOp::CreateBranch { .. } | LocalOp::Discard { .. } => {
                    self.git_reload_open_files();
                }
                _ => {}
            },
            Err(e) => {
                let key = match &op {
                    LocalOp::Commit { .. } => "git:failed.commit",
                    LocalOp::Checkout(_) | LocalOp::CreateBranch { .. } => "git:failed.checkout",
                    LocalOp::Discard { .. } => "git:failed.discard",
                    _ => "git:failed.stage",
                };
                self.git_failed(key, &e, Some(&op));
            }
        }
        self.git_start_status();
    }

    /// "Commit failed. <reason>" with Show Git Output (and Retry when it
    /// may help).
    fn git_failed(&mut self, key: &str, e: &GitError, op: Option<&LocalOp>) {
        let detail = match e {
            GitError::CheckoutConflict(files) => files.join(", "),
            other => other.to_string(),
        };
        self.git_log_line(format!("{}: {detail}", t!(key, reason = "").trim()));
        let reason = match e {
            GitError::CheckoutConflict(files) => {
                t!(reason_key(e), files = files.join(", "))
            }
            _ => t!(reason_key(e)),
        };
        let severity = match e {
            GitError::NothingToCommit => Severity::Info,
            _ => Severity::Error,
        };
        let mut n = Notification::new(severity, Translatable::new(key).arg("reason", &reason))
            .source(NotificationSource::Git)
            .action(
                "git:action.showOutput",
                CommandInvocation::new("git.showOutput", InvocationSource::Internal),
            );
        if matches!(e, GitError::Git(_)) {
            n.detail = Some(detail);
        }
        if let (Some(op), GitError::Locked) = (op, e) {
            n = n.action("common:retry", Self::retry_invocation(op));
        }
        let now = self.now_ms;
        self.notifications.notify(n, now);
    }

    // ── network ──────────────────────────────────────────────────────────

    /// Starts a network operation on its own thread.
    pub fn git_start_net(&mut self, op: NetOp) {
        if self.git.net.is_some() {
            self.git_say(Severity::Info, Translatable::new("git:alreadyRunning"));
            return;
        }
        let repo = self.git.repo.clone();
        if repo.is_none() && !matches!(op, NetOp::Clone { .. }) {
            return;
        }
        let backend = self.git.backend.clone();
        let remote = self.git.remote.clone();
        let git_path = self.settings.str("git.path", None).to_owned();
        let poster = self.git_queue.poster();
        let cancel = CancellationToken::default();
        let generation = self.git.generation;
        self.git.network_runs += 1;
        if matches!(op, NetOp::SyncNotes { .. }) {
            self.git.notice = None;
        }
        self.git.net = Some(NetRun {
            op: op.clone(),
            cancel: cancel.clone(),
            progress: None,
        });
        let spawned = std::thread::Builder::new()
            .name("notoir-git-net".into())
            .spawn(move || {
                let sink: OutputSink = {
                    let poster = poster.clone();
                    Arc::new(move |o| poster.post(IoResult::Git(Box::new(GitResult::Output(o)))))
                };
                let remote: Option<Arc<dyn GitRemote>> = remote.or_else(|| {
                    SystemGit::find(Some(&git_path)).map(|g| Arc::new(g) as Arc<dyn GitRemote>)
                });
                let result = match remote {
                    Some(remote) => run_net(&*backend, &*remote, repo.as_ref(), &op, sink, cancel),
                    None => Err(GitError::NoGit),
                };
                poster.post(IoResult::Git(Box::new(GitResult::Net {
                    generation,
                    op,
                    result,
                })));
            });
        if let Err(e) = spawned {
            tracing::error!(error = %e, "could not start Git thread");
            self.git.net = None;
        }
    }

    pub fn git_cancel(&mut self) {
        if let Some(n) = &self.git.net {
            n.cancel.cancel();
        }
    }

    fn net_invocation(op: &NetOp) -> CommandInvocation {
        let id = match op {
            NetOp::Fetch => "git.fetch",
            NetOp::Pull(PullMode::Rebase) => "git.pullRebase",
            NetOp::Pull(PullMode::Merge) => "git.pullMerge",
            NetOp::Pull(_) => "git.pull",
            NetOp::Push { .. } => "git.push",
            NetOp::Sync(_) => "git.sync",
            NetOp::SyncNotes { .. } => "git.syncNotes",
            NetOp::Clone { url, .. } => {
                return CommandInvocation::new("git.clone", InvocationSource::Internal)
                    .with_args(json!({ "url": url }));
            }
        };
        CommandInvocation::new(id, InvocationSource::Internal)
    }

    pub(crate) fn git_net_done(
        &mut self,
        generation: u64,
        op: NetOp,
        result: Result<NetDone, GitError>,
    ) {
        self.git.net = None;
        if let NetOp::Clone { url, dest } = &op {
            match result {
                Ok(_) => {
                    self.git.recent_urls.retain(|u| u != url);
                    self.git.recent_urls.insert(0, url.clone());
                    self.git.recent_urls.truncate(RECENT_URLS);
                    self.git_prompt(GitPrompt::OpenCloned(dest.clone()));
                }
                Err(e) => self.git_net_failed(&op, &e),
            }
            return;
        }
        if generation != self.git.generation {
            return;
        }
        if op.updates_files() && result.is_ok() {
            self.git_reload_open_files();
        }
        self.git_start_status();
        match result {
            Ok(NetDone::NotesStopped) => {
                self.git.notice = Some(Translatable::new("git:notes.stoppedNotice"));
                self.exit_maximize();
                self.panels.show(PanelId::SourceControl);
                self.git_say(
                    Severity::Warning,
                    Translatable::new("git:notes.stoppedNotice"),
                );
            }
            Ok(_) => {
                if matches!(op, NetOp::SyncNotes { .. }) {
                    self.git_say(Severity::Info, Translatable::new("git:notes.done"));
                }
            }
            Err(GitError::Cancelled) => self.git_log_line(t!("git:reason.cancelled")),
            Err(GitError::Diverged) if matches!(op, NetOp::Pull(_) | NetOp::Sync(_)) => {
                let upstream = self
                    .git
                    .status
                    .head
                    .upstream
                    .clone()
                    .unwrap_or_else(|| "upstream".into());
                self.git_prompt(GitPrompt::Diverged { upstream });
            }
            Err(GitError::NoUpstream) if matches!(op, NetOp::Push { .. } | NetOp::Sync(_)) => {
                self.git_push();
            }
            Err(e) => self.git_net_failed(&op, &e),
        }
    }

    fn git_net_failed(&mut self, op: &NetOp, e: &GitError) {
        let key = format!("git:failed.{}", op.key());
        let reason = t!(reason_key(e));
        let mut n = Notification::new(
            Severity::Error,
            Translatable::new(key).arg("reason", &reason),
        )
        .source(NotificationSource::Git)
        .action(
            "git:action.showOutput",
            CommandInvocation::new("git.showOutput", InvocationSource::Internal),
        )
        .action("common:retry", Self::net_invocation(op));
        if let GitError::Git(detail) = e {
            n.detail = Some(detail.clone());
        }
        let now = self.now_ms;
        self.notifications.notify(n, now);
    }

    fn git_prompt(&mut self, p: GitPrompt) {
        self.files.prompts.push_back(Prompt::Git(p));
    }

    /// Push, publishing the branch first when it has no upstream.
    pub fn git_push(&mut self) {
        let head = self.git.status.head.clone();
        let Some(branch) = head.branch.clone() else {
            self.git_say(Severity::Info, Translatable::new("git:detachedPush"));
            return;
        };
        if head.has_upstream {
            self.git_start_net(NetOp::Push { publish: None });
            return;
        }
        let remote = self
            .git
            .remotes
            .iter()
            .find(|r| *r == "origin")
            .or_else(|| self.git.remotes.first())
            .cloned();
        match remote {
            Some(remote) => self.git_prompt(GitPrompt::Publish { branch, remote }),
            None => self.git_say(Severity::Info, Translatable::new("git:noRemote")),
        }
    }

    pub fn git_pull(&mut self, mode: Option<PullMode>) {
        let mode = mode.unwrap_or_else(|| pull_mode(self.settings.str("git.pullMode", None)));
        self.git_start_net(NetOp::Pull(mode));
    }

    /// Sync: confirms once per workspace, publishes a branch without upstream.
    pub fn git_sync(&mut self, confirmed: bool) {
        if !self.git.status.head.has_upstream {
            self.git_push();
            return;
        }
        if !confirmed && self.settings.bool("git.confirmSync", None) && !self.git.sync_confirmed {
            self.git_prompt(GitPrompt::SyncConfirm);
            return;
        }
        let mode = pull_mode(self.settings.str("git.pullMode", None));
        self.git_start_net(NetOp::Sync(mode));
    }

    // ── commit ───────────────────────────────────────────────────────────

    /// Commit (Mod+Enter, the Commit button): validates the message and asks
    /// to stage everything when nothing is staged (`git.smartCommit`).
    pub fn git_commit(
        &mut self,
        message: Option<String>,
        amend: bool,
        stage_all: bool,
        push: bool,
    ) {
        let message = message.unwrap_or_else(|| self.git_message());
        if message.trim().is_empty() {
            self.git_say(Severity::Warning, Translatable::new("git:emptyMessage"));
            return;
        }
        let status = self.git.status.clone();
        if status.conflicts().next().is_some() && !stage_all {
            self.git_say(Severity::Warning, Translatable::new("git:resolveConflicts"));
            return;
        }
        let nothing_staged = status.staged.is_empty();
        let has_changes = status.changes().next().is_some();
        let stage_all = if stage_all || amend || !nothing_staged {
            stage_all
        } else if !has_changes && status.head.state != notoir_git::RepoState::Merging {
            self.git_say(
                Severity::Info,
                Translatable::new("git:reason.nothingToCommit"),
            );
            return;
        } else {
            match self.settings.str("git.smartCommit", None) {
                "always" => true,
                "never" => {
                    self.git_say(Severity::Info, Translatable::new("git:nothingStaged"));
                    return;
                }
                _ => {
                    self.git_prompt(GitPrompt::SmartCommit {
                        message,
                        amend,
                        push,
                    });
                    return;
                }
            }
        };
        self.git_submit(LocalOp::Commit {
            message: message.trim_end().to_owned(),
            amend,
            stage_all,
            push,
        });
    }

    // ── discard ──────────────────────────────────────────────────────────

    /// Discard Changes…: always confirms.
    pub fn git_discard(&mut self, paths: Vec<String>) {
        let mut tracked = Vec::new();
        let mut untracked = Vec::new();
        for p in paths {
            match self.git.status.unstaged.get(&p) {
                Some(FileStatus::Untracked) => untracked.push(p),
                Some(FileStatus::Conflicted) | None => {}
                Some(_) => tracked.push(p),
            }
        }
        if tracked.is_empty() && untracked.is_empty() {
            return;
        }
        self.git_prompt(GitPrompt::Discard { tracked, untracked });
    }

    // ── branches ─────────────────────────────────────────────────────────

    /// Loads branches, then opens the branch picker.
    pub fn git_pick_branch(&mut self, mode: BranchPick) {
        let Some(repo) = self.git.repo.clone() else {
            return;
        };
        self.git.branch_pick = mode;
        let backend = self.git.backend.clone();
        let generation = self.git.generation;
        self.git_queue.submit(move || {
            let result = backend.branches(&repo);
            Some(IoResult::Git(Box::new(GitResult::Branches {
                generation,
                result,
            })))
        });
    }

    pub(crate) fn git_branches_loaded(&mut self, result: Result<Vec<BranchInfo>, GitError>) {
        match result {
            Ok(b) => {
                self.git.branches = b;
                self.effects.push(Effect::OpenPicker(
                    crate::shell::overlays::PickerKind::GitBranches,
                ));
            }
            Err(e) => self.git_failed("git:failed.checkout", &e, None),
        }
    }

    /// Switch Branch: open documents with unsaved changes are saved first
    /// (Git only sees files).
    pub fn git_checkout(&mut self, branch: String) {
        let dirty = self.git_dirty_docs();
        if dirty.is_empty() {
            self.git_submit(LocalOp::Checkout(branch));
        } else {
            self.files.prompts.push_back(Prompt::Unsaved {
                docs: dirty,
                then: Then::GitCheckout(branch),
            });
        }
    }

    /// Dirty documents whose files are in the repository.
    fn git_dirty_docs(&self) -> Vec<DocumentId> {
        let Some(repo) = &self.git.repo else {
            return Vec::new();
        };
        self.workbench
            .documents
            .values()
            .filter(|d| d.is_dirty() && d.path().is_some_and(|p| p.starts_with(&repo.workdir)))
            .map(|d| d.id)
            .collect()
    }

    // ── Sync Notes ───────────────────────────────────────────────────────

    pub fn git_sync_notes(&mut self) {
        let dirty = self.git_dirty_docs();
        if dirty.is_empty() {
            self.git_sync_notes_now();
            return;
        }
        // Save first; the pipeline continues when every save finished.
        self.files.pending.push(Pending {
            waiting: dirty.iter().copied().collect(),
            then: Then::GitSyncNotes,
        });
        for d in dirty {
            self.save(d);
        }
    }

    pub(crate) fn git_sync_notes_now(&mut self) {
        let template = match self.settings.str("notes.sync.commitMessage", None) {
            "" => NOTES_MESSAGE.to_owned(),
            t => t.to_owned(),
        };
        self.git_log_line(t!("git:notes.stepSave"));
        self.git_start_net(NetOp::SyncNotes { template });
    }

    // ── clone / init ─────────────────────────────────────────────────────

    /// Clone Repository: asks for the destination folder.
    pub fn git_clone(&mut self, url: String) {
        let url = url.trim().to_owned();
        if url.is_empty() {
            return;
        }
        self.git.clone_url = Some(url);
        let opts = notoir_platform::dialogs::FileDialogOptions {
            title: Some(t!("git:clone.destination")),
            ..Default::default()
        };
        self.files.dialogs.pick_folder(opts, self.io.poster());
    }

    /// A folder was picked for Clone Repository; false when no clone waits.
    pub fn git_clone_folder(&mut self, folder: Option<&Path>) -> bool {
        let Some(url) = self.git.clone_url.take() else {
            return false;
        };
        if let Some(folder) = folder {
            let dest = folder.join(clone_dir_name(&url));
            self.git_start_net(NetOp::Clone { url, dest });
        }
        true
    }

    // ── changes ──────────────────────────────────────────────────────────

    /// The current side for `path`: its open document, else the file.
    pub fn git_current_source(&self, path: &Path) -> CompareSource {
        let (_, key) = notoir_workspace::fs_io::identity(path);
        match self.workbench.find_by_key(&key) {
            Some(doc) => CompareSource::Document(doc),
            None => CompareSource::File(path.to_path_buf()),
        }
    }

    /// Open Changes: index vs work tree (or HEAD vs index when staged).
    pub fn git_open_change(&mut self, rel: &str, staged: bool) {
        let Some(repo) = self.git.repo.clone() else {
            return;
        };
        let path = repo.absolute(rel);
        let index = || CompareSource::GitRevision(path.clone(), crate::git::INDEX_REV.into());
        if staged {
            let right = index();
            self.compare(CompareSource::GitHead(path.clone()), right, false);
            return;
        }
        match self.git.status.unstaged.get(rel) {
            Some(FileStatus::Untracked) | Some(FileStatus::Conflicted) => {
                self.open_path(&path, crate::files::OpenAt::default());
            }
            Some(FileStatus::Deleted) => {
                let right = CompareSource::Text {
                    title: t!("git:deletedSide", name = rel),
                    text: Arc::from(""),
                };
                self.compare(index(), right, false);
            }
            _ => {
                let right = self.git_current_source(&path);
                self.compare(index(), right, false);
            }
        }
    }

    // ── prompts ──────────────────────────────────────────────────────────

    pub fn git_answer(&mut self, p: GitPrompt, answer: Answer) {
        match (p, answer) {
            (_, Answer::Cancel) => {}
            (GitPrompt::Discard { tracked, untracked }, Answer::Primary) => {
                self.git_submit(LocalOp::Discard { tracked, untracked })
            }
            (
                GitPrompt::SmartCommit {
                    message,
                    amend,
                    push,
                },
                Answer::Primary,
            ) => self.git_submit(LocalOp::Commit {
                message: message.trim_end().to_owned(),
                amend,
                stage_all: true,
                push,
            }),
            (GitPrompt::Diverged { .. }, Answer::Primary) => {
                self.git_start_net(NetOp::Pull(PullMode::Rebase))
            }
            (GitPrompt::Diverged { .. }, Answer::Secondary) => {
                self.git_start_net(NetOp::Pull(PullMode::Merge))
            }
            (GitPrompt::Publish { branch, remote }, Answer::Primary) => {
                self.git_start_net(NetOp::Push {
                    publish: Some((remote, branch)),
                })
            }
            (GitPrompt::SyncConfirm, Answer::Primary) => {
                self.git.sync_confirmed = true;
                self.git_sync(true);
            }
            (GitPrompt::Init(path), Answer::Primary) => self.git_submit(LocalOp::Init(path)),
            (GitPrompt::OpenCloned(path), Answer::Primary) => self.open_workspace(&path),
            (GitPrompt::OpenCloned(path), Answer::Secondary) => {
                self.open_workspace_in_new_window(&path)
            }
            _ => {}
        }
    }

    /// Status bar progress: "Pushing… 45%".
    pub fn git_progress_label(&self) -> Option<String> {
        let n = self.git.net.as_ref()?;
        let mut s = t!(&format!("git:progress.{}", n.op.key()));
        if let Some((pct, _)) = &n.progress {
            s.push_str(&format!(" {pct}%"));
        }
        Some(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_message_and_clone_names() {
        assert_eq!(
            render_notes_message(NOTES_MESSAGE, 1_760_486_400 + 3_600 * 13 + 60 * 5, "box"),
            "Notes sync 2025-10-15 13:05 UTC from box"
        );
        assert_eq!(
            clone_dir_name("git@github.com:lvendrame/notes.git"),
            "notes"
        );
        assert_eq!(clone_dir_name("https://example.com/a/b/"), "b");
        assert_eq!(clone_dir_name("/srv/repos/x.git"), "x");
        assert_eq!(clone_dir_name(""), "repository");
    }
}
