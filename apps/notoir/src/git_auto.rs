//! Manual-save batches and repository-specific automatic pushes.
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::PathBuf;

use notoir_core::command::{CommandInvocation, InvocationSource};
use notoir_core::ids::DocumentId;
use notoir_core::notifications::{Notification, NotificationSource, Severity, Translatable};
use notoir_git::{GitError, RepoInfo, SavedCommit, SavedFile, StatusOpts};
use serde_json::json;

use crate::core::AppCore;
use crate::git::GitResult;
use crate::git_write::{GitPrompt, NetOp};
use crate::io::IoResult;
use crate::t;

#[derive(Default)]
pub struct AutoSaveState {
    next: u64,
    cancel: notoir_git::CancellationToken,
    pub documents: HashMap<DocumentId, u64>,
    batches: BTreeMap<u64, SaveBatch>,
    pushes: VecDeque<PushTarget>,
}
impl Drop for AutoSaveState {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
impl AutoSaveState {
    pub fn has_ready_work(&self) -> bool {
        !self.pushes.is_empty()
            || self
                .batches
                .first_key_value()
                .is_some_and(|(_, b)| b.waiting.is_empty())
    }
}
struct SaveBatch {
    waiting: HashSet<DocumentId>,
    branches: HashMap<PathBuf, String>,
    files: BTreeMap<PathBuf, (RepoInfo, String, Vec<SavedFile>)>,
    timestamp: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PushTarget {
    pub repo: RepoInfo,
    pub branch: String,
    pub generation: u64,
    pub publish: Option<(String, String)>,
}
#[derive(Debug)]
pub enum AutoResult {
    Commit {
        target: PushTarget,
        result: Result<Option<SavedCommit>, GitError>,
    },
    PushReady {
        target: PushTarget,
        result: Result<Option<String>, GitError>,
    },
}

impl AppCore {
    /// Menu consent is local to this workspace and does not trust other settings.
    pub fn set_workspace_auto_push(&mut self, enabled: bool) {
        let Some(ws) = self.workspace.as_mut() else {
            return;
        };
        ws.auto_push_choice = Some(enabled);
        let key = ws.key.clone();
        self.set_workspace_setting(
            "git.autoPush",
            Some(notoir_core::settings::SettingValue::Bool(enabled)),
        );
        let path = self.paths.data.join("state.db");
        self.io.submit(move || {
            let result = notoir_storage::state_db::StateDb::open(&path)
                .and_then(|(db, _)| db.set_workspace_push_consent(&key, enabled));
            result.err().map(|e| IoResult::SettingsWriteFailed {
                path: path.display().to_string(),
                error: e.to_string(),
            })
        });
    }

    /// Called only by explicit file commands, never by autosave/internal saves.
    pub fn begin_manual_save(&mut self, docs: Vec<DocumentId>, source: &InvocationSource) {
        if *source == InvocationSource::Internal
            || docs.is_empty()
            || !self.settings.bool("git.autoCommit", None)
            || !self.settings.bool("git.enabled", None)
            || self.workspace.is_none()
        {
            return;
        }
        for doc in &docs {
            self.auto_save_cancelled(*doc);
        }
        let branches = self
            .git
            .repos
            .iter()
            .filter_map(|repo| {
                self.git_status_for(&repo.workdir)?
                    .head
                    .branch
                    .clone()
                    .map(|branch| (repo.workdir.clone(), branch))
            })
            .collect();
        let state = &mut self.git.auto_save;
        state.next += 1;
        let id = state.next;
        for doc in &docs {
            state.documents.insert(*doc, id);
        }
        state.batches.insert(
            id,
            SaveBatch {
                waiting: docs.into_iter().collect(),
                branches,
                files: BTreeMap::new(),
                timestamp: String::new(),
            },
        );
    }

    pub fn auto_save_finished(
        &mut self,
        doc: DocumentId,
        batch: Option<(u64, u64)>,
        snapshot: Option<(PathBuf, Vec<u8>)>,
    ) {
        let Some((generation, id)) = batch else {
            return;
        };
        if generation != self.git.generation {
            return;
        }
        let file = snapshot.and_then(|(path, bytes)| {
            self.git_locate(&path)
                .map(|(repo, rel)| (repo, SavedFile { path: rel, bytes }))
        });
        let state = &mut self.git.auto_save;
        if state.documents.get(&doc) == Some(&id) {
            state.documents.remove(&doc);
        }
        if let Some(batch) = state.batches.get_mut(&id) {
            batch.waiting.remove(&doc);
            if let Some((repo, file)) = file
                && let Some(branch) = batch.branches.get(&repo.workdir)
            {
                let entry = batch
                    .files
                    .entry(repo.workdir.clone())
                    .or_insert_with(|| (repo, branch.clone(), Vec::new()));
                entry.2.retain(|f| f.path != file.path);
                entry.2.push(file);
            }
            if batch.waiting.is_empty() {
                batch.timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
            }
        }
    }

    pub fn auto_save_cancelled(&mut self, doc: DocumentId) {
        let batch = self.git.auto_save.documents.get(&doc).copied();
        self.auto_save_finished(doc, batch.map(|id| (self.git.generation, id)), None);
    }

    pub fn git_auto_tick(&mut self) {
        if !self.settings.bool("git.autoCommit", None) {
            self.git.auto_save.cancel.cancel();
            self.git.auto_save.cancel = Default::default();
            self.git.auto_save.batches.clear();
            self.git.auto_save.documents.clear();
        }
        if !self.settings.bool("git.autoPush", None) {
            self.git.auto_save.pushes.clear();
            self.files
                .prompts
                .retain(|p| !matches!(p, crate::files::Prompt::Git(GitPrompt::AutoPublish { .. })));
        }
        if self.git_busy() {
            return;
        }
        if self
            .git
            .auto_save
            .batches
            .first_key_value()
            .is_some_and(|(_, b)| b.waiting.is_empty())
        {
            let (_, mut batch) = self.git.auto_save.batches.pop_first().expect("ready batch");
            if let Some((_, (repo, branch, files))) = batch.files.pop_first() {
                let target = PushTarget {
                    repo,
                    branch,
                    generation: self.git.generation,
                    publish: None,
                };
                let backend = self.git.backend.clone();
                self.git.busy += 1;
                let timestamp = batch.timestamp.clone();
                let cancel = self.git.auto_save.cancel.clone();
                self.git_queue.submit(move || {
                    let result = if cancel.is_cancelled() {
                        Err(GitError::Cancelled)
                    } else {
                        backend.commit_saved(&target.repo, &files, &target.branch, &timestamp)
                    };
                    Some(IoResult::Git(Box::new(GitResult::Auto(
                        AutoResult::Commit { target, result },
                    ))))
                });
                if !batch.files.is_empty() {
                    self.git.auto_save.batches.insert(0, batch);
                }
                return;
            }
        }
        if let Some(target) = self.git.auto_save.pushes.pop_front() {
            let backend = self.git.backend.clone();
            self.git.busy += 1;
            self.git_queue.submit(move || {
                let result = (|| {
                    let status = backend.status(&target.repo, StatusOpts::default())?;
                    if status.head.branch.as_deref() != Some(&target.branch) {
                        return Err(GitError::Git("Branch changed since saving".into()));
                    }
                    if status.head.has_upstream || target.publish.is_some() {
                        return Ok(None);
                    }
                    let remotes = backend.remotes(&target.repo)?;
                    remotes
                        .iter()
                        .find(|r| *r == "origin")
                        .or_else(|| remotes.first())
                        .cloned()
                        .map(Some)
                        .ok_or(GitError::NoUpstream)
                })();
                Some(IoResult::Git(Box::new(GitResult::Auto(
                    AutoResult::PushReady { target, result },
                ))))
            });
        }
    }

    pub fn git_queue_auto_push(&mut self, target: PushTarget) {
        if target.generation == self.git.generation && self.settings.bool("git.autoPush", None) {
            if self.files.prompts.iter().any(|p| matches!(p, crate::files::Prompt::Git(GitPrompt::AutoPublish { target: waiting, .. }) if waiting.repo == target.repo && waiting.branch == target.branch)) { return; }
            self.git
                .auto_save
                .pushes
                .retain(|t| t.repo != target.repo || t.branch != target.branch);
            self.git.auto_save.pushes.push_back(target);
        }
    }

    pub fn git_auto_result(&mut self, result: AutoResult) {
        self.git.busy = self.git.busy.saturating_sub(1);
        match result {
            AutoResult::Commit { target, result } if target.generation == self.git.generation => {
                match result {
                    Ok(Some(commit)) => {
                        self.git_log_line(t!(
                            "git:output.committed",
                            sha = commit.sha,
                            summary = commit.message.lines().next().unwrap_or_default()
                        ));
                        self.git_queue_auto_push(target);
                    }
                    Ok(None) => {}
                    Err(e) => self.git_auto_failed("git:failed.commit", &target, &e, false),
                }
                self.git_refresh_soon();
            }
            AutoResult::PushReady { target, result }
                if target.generation == self.git.generation
                    && self.settings.bool("git.autoPush", None) =>
            {
                match result {
                    Ok(Some(remote)) => self.files.prompts.push_back(crate::files::Prompt::Git(
                        GitPrompt::AutoPublish { target, remote },
                    )),
                    Ok(None) => self.git_start_auto_net(target),
                    Err(e) => self.git_auto_failed("git:failed.push", &target, &e, true),
                }
            }
            _ => {}
        }
    }

    fn git_start_auto_net(&mut self, target: PushTarget) {
        let op = NetOp::Push {
            publish: target.publish.clone(),
        };
        self.git_start_net_for(op, Some(target.repo.clone()), Some(target));
    }

    pub fn git_auto_failed(&mut self, key: &str, target: &PushTarget, e: &GitError, retry: bool) {
        let detail = e.to_string();
        self.git_log_line(detail.clone());
        let mut n = Notification::new(
            Severity::Error,
            Translatable::new(key).arg("reason", t!(crate::git_write::reason_key(e))),
        )
        .source(NotificationSource::Git)
        .detail(detail)
        .dedupe(format!("auto-git:{}:{key}", target.repo.workdir.display()))
        .action(
            "git:action.showOutput",
            CommandInvocation::new("git.showOutput", InvocationSource::Internal),
        );
        if retry {
            n = n.action("common:retry", CommandInvocation::new("git.retryAutoPush", InvocationSource::Internal).with_args(json!({"workdir": target.repo.workdir, "branch": target.branch, "generation": target.generation})));
        }
        self.notify(n);
    }
}
