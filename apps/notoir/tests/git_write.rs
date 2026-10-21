//! Git write side (spec 28): Source Control panel, commit, discard,
//! branches, network operations, clone/init and Sync Notes. Repositories are
//! temporary; "remotes" are local bare repositories reached by the system
//! `git` (no network), or a scripted remote for timing and error cases.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use egui::{Key, Modifiers};
use egui_kittest::kittest::Queryable;
use notoir::files::Prompt;
use notoir::git_write::GitPrompt;
use notoir_core::panels::PanelId;
use notoir_core::settings::SettingValue;
use notoir_git::{
    CancellationToken, FetchOpts, GitError, GitOutput, GitRemote, OutputSink, PullMode, PullResult,
    PushOpts, RepoInfo,
};
use serde_json::{Value, json};

fn ctrl() -> Modifiers {
    Modifiers {
        ctrl: true,
        command: true,
        ..Default::default()
    }
}

fn commit_all(r: &git2::Repository, msg: &str) -> git2::Oid {
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
        .unwrap()
}

fn set_identity(r: &git2::Repository) {
    let mut cfg = r.config().unwrap();
    cfg.set_str("user.name", "Tester").unwrap();
    cfg.set_str("user.email", "t@example.com").unwrap();
    // Checkouts keep LF whatever the global config (Windows: CRLF).
    cfg.set_bool("core.autocrlf", false).unwrap();
}

/// A repo with README.md and notes.md committed.
fn repo() -> (tempfile::TempDir, PathBuf, git2::Repository) {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("notes");
    std::fs::create_dir(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let r = git2::Repository::init(&root).unwrap();
    set_identity(&r);
    std::fs::write(root.join("README.md"), "# Notes\n").unwrap();
    std::fs::write(root.join("notes.md"), "one\n").unwrap();
    commit_all(&r, "Initial");
    (d, root, r)
}

fn branch(r: &git2::Repository) -> String {
    r.head().unwrap().shorthand().unwrap().to_owned()
}

fn head_summary(r: &git2::Repository) -> String {
    r.head()
        .unwrap()
        .peel_to_commit()
        .unwrap()
        .summary()
        .unwrap()
        .unwrap_or_default()
        .to_owned()
}

fn wait_for(app: &mut common::TestApp, f: impl Fn(&common::TestApp) -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        common::settle(app);
        if f(app) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

fn open_with(
    root: &Path,
    files: Vec<PathBuf>,
    configure: impl FnOnce(&mut notoir::core::StartOptions),
) -> common::TestApp {
    let mut paths = vec![(root.to_path_buf(), None)];
    paths.extend(files.into_iter().map(|f| (f, None)));
    let mut app = common::start(move |o| {
        o.open_paths = paths;
        configure(o);
    });
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .git
        .repo
        .is_some()));
    common::run(&mut app, "git.showSourceControl", Value::Null);
    common::settle(&mut app);
    app
}

fn open(root: &Path, files: Vec<PathBuf>) -> common::TestApp {
    open_with(root, files, |_| {})
}

fn front_prompt(app: &common::TestApp) -> Option<Prompt> {
    app.harness.state().core.files.prompts.front().cloned()
}

fn staged(app: &common::TestApp) -> Vec<String> {
    app.harness
        .state()
        .core
        .git
        .status
        .staged
        .keys()
        .cloned()
        .collect()
}

fn notification_texts(app: &common::TestApp) -> Vec<String> {
    app.harness
        .state()
        .core
        .notifications
        .history()
        .map(notoir::shell::overlays::message_text)
        .collect()
}

fn idle(app: &common::TestApp) -> bool {
    !app.harness.state().core.git_busy()
}

// ── system git helpers ──────────────────────────────────────────────────

fn has_git() -> bool {
    let ok = Command::new("git").arg("--version").output().is_ok();
    if !ok {
        eprintln!("system git not found; skipping");
    }
    ok
}

fn git(cwd: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

fn git_identity(dir: &Path) {
    for (k, v) in [
        ("user.name", "Tester"),
        ("user.email", "t@example.com"),
        ("commit.gpgsign", "false"),
        ("pull.rebase", "false"),
        ("push.autoSetupRemote", "false"),
        ("core.autocrlf", "false"),
    ] {
        git(dir, &["config", k, v]);
    }
}

/// A bare remote with `main`, and clones `a` (the workspace) and `b`
/// (someone else) sharing one commit of `n.md`.
fn remote_pair() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().canonicalize().unwrap();
    git(&root, &["init", "-q", "--bare", "-b", "main", "remote.git"]);
    let bare = root.join("remote.git");
    git(
        &root,
        &[
            "clone",
            "-q",
            "-c",
            "core.autocrlf=false",
            "remote.git",
            "a",
        ],
    );
    let a = root.join("a");
    git_identity(&a);
    git(&a, &["symbolic-ref", "HEAD", "refs/heads/main"]);
    std::fs::write(a.join("n.md"), "1\n2\n3\n").unwrap();
    git(&a, &["add", "."]);
    git(&a, &["commit", "-q", "-m", "first"]);
    git(&a, &["push", "-q", "-u", "origin", "main"]);
    git(
        &root,
        &[
            "clone",
            "-q",
            "-c",
            "core.autocrlf=false",
            "remote.git",
            "b",
        ],
    );
    let b = root.join("b");
    git_identity(&b);
    (d, bare, a, b)
}

// ── scripted remote ─────────────────────────────────────────────────────

/// A remote that records calls, can hold a push open and can fail.
#[derive(Default)]
struct FakeRemote {
    calls: Mutex<Vec<String>>,
    hold: AtomicBool,
    push_error: Mutex<Option<(GitError, String)>>,
    pull_error: Mutex<Option<GitError>>,
}

impl GitRemote for FakeRemote {
    fn fetch(
        &self,
        _: &RepoInfo,
        _: FetchOpts,
        _: OutputSink,
        _: CancellationToken,
    ) -> Result<(), GitError> {
        self.calls.lock().unwrap().push("fetch".into());
        Ok(())
    }

    fn pull(
        &self,
        _: &RepoInfo,
        mode: PullMode,
        _: OutputSink,
        _: CancellationToken,
    ) -> Result<PullResult, GitError> {
        self.calls.lock().unwrap().push(format!("pull {mode:?}"));
        match self.pull_error.lock().unwrap().take() {
            Some(e) => Err(e),
            None => Ok(PullResult::UpToDate),
        }
    }

    fn push(
        &self,
        _: &RepoInfo,
        opts: PushOpts,
        out: OutputSink,
        cancel: CancellationToken,
    ) -> Result<(), GitError> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("push {:?}", opts.set_upstream));
        out(GitOutput::Command("git push --progress".into()));
        out(GitOutput::Progress {
            percent: 45,
            phase: "Writing objects".into(),
        });
        while self.hold.load(Ordering::SeqCst) {
            if cancel.is_cancelled() {
                return Err(GitError::Cancelled);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        if let Some((e, stderr)) = self.push_error.lock().unwrap().take() {
            out(GitOutput::Line(stderr));
            return Err(e);
        }
        Ok(())
    }

    fn clone_repo(
        &self,
        _: &str,
        _: &Path,
        _: OutputSink,
        _: CancellationToken,
    ) -> Result<(), GitError> {
        Ok(())
    }
}

/// `repo()` with an `origin` whose branch tracks `origin/<branch>`.
fn repo_with_upstream() -> (tempfile::TempDir, PathBuf, git2::Repository) {
    let (d, root, r) = repo();
    r.remote("origin", "/nonexistent/remote.git").unwrap();
    let head = r.head().unwrap().target().unwrap();
    let name = branch(&r);
    r.reference(&format!("refs/remotes/origin/{name}"), head, true, "test")
        .unwrap();
    r.find_branch(&name, git2::BranchType::Local)
        .unwrap()
        .set_upstream(Some(&format!("origin/{name}")))
        .unwrap();
    (d, root, r)
}

// ── commit ──────────────────────────────────────────────────────────────

// spec 28: "Commit staged changes"
#[test]
fn mod_enter_commits_staged_changes() {
    let (_d, root, r) = repo();
    std::fs::write(root.join("README.md"), "# Notes\n\nUpdated.\n").unwrap();
    std::fs::write(root.join("notes.md"), "one\ntwo\n").unwrap();
    let mut index = r.index().unwrap();
    index.add_path(Path::new("README.md")).unwrap();
    index.write().unwrap();
    let mut app = open(&root, vec![]);
    assert!(wait_for(&mut app, |a| staged(a) == ["README.md"]));
    assert!(
        app.harness
            .query_by_label("README.md, Modified (Staged Changes)")
            .is_some()
    );
    app.harness.get_by_label("Commit message").click();
    app.harness.run_steps(2);
    app.harness.event(egui::Event::Text("Update readme".into()));
    app.harness.run_steps(2);
    assert_eq!(app.harness.state().core.git_message(), "Update readme");
    app.harness.key_press_modifiers(ctrl(), Key::Enter);
    assert!(wait_for(&mut app, |_| head_summary(&r) == "Update readme"));
    // The commit contains README.md only; notes.md stays a change.
    let head = r.head().unwrap().peel_to_commit().unwrap();
    let parent = head.parent(0).unwrap();
    let diff = r
        .diff_tree_to_tree(
            Some(&parent.tree().unwrap()),
            Some(&head.tree().unwrap()),
            None,
        )
        .unwrap();
    let files: Vec<String> = diff
        .deltas()
        .map(|d| d.new_file().path().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(files, ["README.md"]);
    assert!(wait_for(&mut app, |a| staged(a).is_empty()));
    assert!(
        app.harness.query_by_label("STAGED CHANGES (0)").is_some(),
        "the Staged Changes section is empty"
    );
    assert_eq!(app.harness.state().core.git_message(), "", "draft cleared");
    assert!(
        app.harness
            .query_by_label("notes.md, Modified (Changes)")
            .is_some()
    );
}

// spec 28: "Smart commit prompt"
#[test]
fn commit_with_nothing_staged_asks_to_stage_all() {
    let (_d, root, r) = repo();
    std::fs::write(root.join("README.md"), "# Notes\nmore\n").unwrap();
    std::fs::write(root.join("notes.md"), "one\nmore\n").unwrap();
    let mut app = open(&root, vec![]);
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .git
        .status
        .changes()
        .count()
        == 2));
    *app.harness.state_mut().core.git_message_mut().unwrap() = "Both".into();
    // The Commit button of the panel (the header icon has the same label).
    let buttons = app.harness.get_all_by_label("Commit").collect::<Vec<_>>();
    buttons.last().unwrap().click();
    app.harness.run_steps(3);
    assert!(matches!(
        front_prompt(&app),
        Some(Prompt::Git(GitPrompt::SmartCommit { .. }))
    ));
    assert!(
        app.harness
            .query_by_label("There are no staged changes.")
            .is_some()
    );
    assert!(
        app.harness
            .query_by_label("Stage all changes and commit?")
            .is_some()
    );
    assert_eq!(head_summary(&r), "Initial", "nothing committed yet");
    app.harness.get_by_label("Stage All & Commit").click();
    assert!(wait_for(&mut app, |_| head_summary(&r) == "Both"));
    let s = r.statuses(None).unwrap();
    assert!(s.is_empty(), "both files committed");
}

#[test]
fn empty_message_and_amend() {
    let (_d, root, r) = repo();
    std::fs::write(root.join("README.md"), "# Notes\nx\n").unwrap();
    let mut app = open(&root, vec![]);
    common::run(&mut app, "git.stageAll", Value::Null);
    assert!(wait_for(&mut app, |a| staged(a) == ["README.md"]));
    common::run(&mut app, "git.commit", Value::Null);
    assert!(
        notification_texts(&app)
            .iter()
            .any(|t| t == "Enter a commit message."),
        "{:?}",
        notification_texts(&app)
    );
    common::run(&mut app, "git.commit", json!({ "message": "First try" }));
    assert!(wait_for(&mut app, |_| head_summary(&r) == "First try"));
    assert!(wait_for(&mut app, idle));
    common::run(&mut app, "git.commitAmend", json!({ "message": "Better" }));
    assert!(wait_for(&mut app, |_| head_summary(&r) == "Better"));
    let head = r.head().unwrap().peel_to_commit().unwrap();
    assert_eq!(head.parent(0).unwrap().summary().unwrap(), Some("Initial"));
}

// ── stage / discard ─────────────────────────────────────────────────────

// spec 28: "Discard requires confirmation"
#[test]
fn discard_confirms_with_the_file_name() {
    let (_d, root, _r) = repo();
    std::fs::write(root.join("README.md"), "# Changed\n").unwrap();
    let mut app = open(&root, vec![]);
    let row = "README.md, Modified (Changes)";
    assert!(wait_for(&mut app, |a| a
        .harness
        .query_by_label(row)
        .is_some()));
    app.harness.get_by_label(row).hover();
    app.harness.run_steps(2);
    app.harness.get_by_label("Discard Changes…").click();
    app.harness.run_steps(3);
    assert!(matches!(
        front_prompt(&app),
        Some(Prompt::Git(GitPrompt::Discard { .. }))
    ));
    assert!(
        app.harness
            .query_by_label("Discard changes in README.md?")
            .is_some()
    );
    assert!(
        app.harness
            .query_by_label("This cannot be undone.")
            .is_some()
    );
    assert_eq!(
        std::fs::read_to_string(root.join("README.md")).unwrap(),
        "# Changed\n",
        "nothing discarded before confirming"
    );
    app.harness.get_by_label("Discard Changes").click();
    assert!(wait_for(&mut app, |_| std::fs::read_to_string(
        root.join("README.md")
    )
    .unwrap()
        == "# Notes\n"));
}

#[test]
fn untracked_files_are_trashed_and_rows_stage() {
    let (_d, root, _r) = repo();
    std::fs::write(root.join("idea.md"), "idea\n").unwrap();
    std::fs::write(root.join("notes.md"), "one\ntwo\n").unwrap();
    let mut app = open(&root, vec![]);
    let row = "notes.md, Modified (Changes)";
    assert!(wait_for(&mut app, |a| a
        .harness
        .query_by_label(row)
        .is_some()));
    // Hover action: Stage.
    app.harness.get_by_label(row).hover();
    app.harness.run_steps(2);
    app.harness.get_by_label("Stage Changes").click();
    assert!(wait_for(&mut app, |a| staged(a) == ["notes.md"]));
    // Unstage from the context menu command.
    common::run(&mut app, "git.unstage", json!({ "paths": ["notes.md"] }));
    assert!(wait_for(&mut app, |a| staged(a).is_empty()));
    common::run(&mut app, "git.discard", json!({ "paths": ["idea.md"] }));
    assert!(
        app.harness
            .query_by_label("Delete the untracked file idea.md?")
            .is_some()
    );
    assert!(
        app.harness
            .query_by_label("This cannot be undone. Untracked files are moved to the Trash.")
            .is_some()
    );
    app.harness.get_by_label("Discard Changes").click();
    assert!(wait_for(&mut app, |_| !root.join("idea.md").exists()));
    let trashed = app.harness.state().core.files.dialogs.clone();
    let _ = trashed;
}

#[test]
fn clicking_a_change_opens_index_vs_work_tree() {
    let (_d, root, _r) = repo();
    std::fs::write(root.join("notes.md"), "one\ntwo\n").unwrap();
    let mut app = open(&root, vec![]);
    let row = "notes.md, Modified (Changes)";
    assert!(wait_for(&mut app, |a| a
        .harness
        .query_by_label(row)
        .is_some()));
    app.harness.get_by_label(row).click();
    assert!(wait_for(&mut app, |a| !a
        .harness
        .state()
        .core
        .compare
        .sessions
        .is_empty()));
    assert!(
        app.harness
            .query_by_label("notes.md (Index) \u{2194} notes.md")
            .is_some()
    );
}

// ── branches ────────────────────────────────────────────────────────────

/// `a.md` differs between the current branch and `other`.
fn repo_with_branch() -> (tempfile::TempDir, PathBuf, git2::Repository, String) {
    let (d, root, r) = repo();
    std::fs::write(root.join("a.md"), "main version\n").unwrap();
    commit_all(&r, "a on main");
    let main = branch(&r);
    {
        let head = r.head().unwrap().peel_to_commit().unwrap();
        r.branch("other", &head, false).unwrap();
    }
    r.set_head("refs/heads/other").unwrap();
    std::fs::write(root.join("a.md"), "other version\n").unwrap();
    commit_all(&r, "a on other");
    r.set_head(&format!("refs/heads/{main}")).unwrap();
    r.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(root.join("a.md")).unwrap(),
        "main version\n"
    );
    (d, root, r, main)
}

fn active_text(app: &common::TestApp) -> String {
    let core = &app.harness.state().core;
    let doc = core.workbench.active_document().unwrap();
    core.workbench.document(doc).unwrap().buffer.text()
}

// spec 28: "Git checkout reloads clean open files"
#[test]
fn switching_branches_reloads_clean_files() {
    let (_d, root, r, _main) = repo_with_branch();
    let mut app = open(&root, vec![root.join("a.md")]);
    assert_eq!(active_text(&app), "main version\n");
    // Status bar branch → Switch Branch picker → "other".
    let label = format!("Branch: {}", branch(&r));
    app.harness.get_by_label(&label).click();
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .ui
        .picker
        .is_some()));
    assert!(
        app.harness
            .query_by_label_contains("Create Branch…")
            .is_some()
    );
    app.harness.state_mut().ui.picker = None;
    common::run(&mut app, "git.branchPick", json!({ "value": "other" }));
    assert!(wait_for(&mut app, |a| active_text(a) == "other version\n"));
    assert_eq!(branch(&r), "other");
    let core = &app.harness.state().core;
    let doc = core.workbench.active_document().unwrap();
    assert!(!core.workbench.document(doc).unwrap().is_dirty());
}

#[test]
fn switching_with_unsaved_changes_saves_first_and_refuses_conflicts() {
    let (_d, root, r, main) = repo_with_branch();
    let mut app = open(&root, vec![root.join("a.md")]);
    {
        let core = &mut app.harness.state_mut().core;
        let view = core.workbench.active_view().unwrap();
        core.workbench
            .edit(
                view,
                notoir_editor::ChangeSet::insert(0, "local "),
                notoir_editor::Selection::caret(0),
                notoir_editor::EditKind::Other,
                0,
            )
            .unwrap();
    }
    common::run(&mut app, "git.checkout", json!({ "branch": "other" }));
    assert!(matches!(front_prompt(&app), Some(Prompt::Unsaved { .. })));
    app.harness.get_by_label("Save").click();
    // Saved, then Git refuses to overwrite the local change.
    assert!(wait_for(&mut app, |a| notification_texts(a)
        .iter()
        .any(|t| t.starts_with("Could not switch branches."))));
    assert_eq!(
        std::fs::read_to_string(root.join("a.md")).unwrap(),
        "local main version\n"
    );
    assert_eq!(branch(&r), main);
}

#[test]
fn create_branch_switches_to_it() {
    let (_d, root, r) = repo();
    let mut app = open(&root, vec![]);
    common::run(&mut app, "git.createBranch", Value::Null);
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .ui
        .picker
        .is_some()));
    app.harness.event(egui::Event::Text("topic/one".into()));
    app.harness.run_steps(2);
    assert!(
        app.harness
            .query_by_label_contains("Create branch topic/one")
            .is_some()
    );
    app.harness.key_press(Key::Enter);
    assert!(wait_for(&mut app, |_| branch(&r) == "topic/one"));
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .git_branch_label()
        .is_some_and(|l| l.starts_with("topic/one"))));
}

// ── network ─────────────────────────────────────────────────────────────

// spec 28: "Push never freezes the UI"
#[test]
fn push_runs_in_the_background_with_progress() {
    let (_d, root, _r) = repo_with_upstream();
    let fake = Arc::new(FakeRemote::default());
    fake.hold.store(true, Ordering::SeqCst);
    let remote = fake.clone();
    let mut app = open_with(&root, vec![root.join("notes.md")], move |o| {
        o.git_remote = Some(remote);
    });
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .git
        .status
        .head
        .has_upstream));
    common::run(&mut app, "git.push", Value::Null);
    assert!(wait_for(&mut app, |a| a
        .harness
        .query_by_label("Git: Pushing… 45%")
        .is_some()));
    // The editor keeps working while the push waits on the remote.
    let start = Instant::now();
    app.harness.event(egui::Event::Text("typed".into()));
    app.harness.run_steps(10);
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "frames keep flowing"
    );
    assert!(app.harness.state().core.git.net.is_some(), "still pushing");
    assert_eq!(active_text(&app), "typedone\n", "typing reached the editor");
    assert!(app.harness.query_by_label("Cancel Git Operation").is_some());
    fake.hold.store(false, Ordering::SeqCst);
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .git
        .net
        .is_none()));
    assert!(app.harness.query_by_label_contains("Pushing").is_none());
    assert!(
        notification_texts(&app).is_empty(),
        "{:?}",
        notification_texts(&app)
    );
    assert_eq!(fake.calls.lock().unwrap().as_slice(), ["push None"]);
}

#[test]
fn a_push_can_be_cancelled() {
    let (_d, root, _r) = repo_with_upstream();
    let fake = Arc::new(FakeRemote::default());
    fake.hold.store(true, Ordering::SeqCst);
    let remote = fake.clone();
    let mut app = open_with(&root, vec![], move |o| o.git_remote = Some(remote));
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .git
        .status
        .head
        .has_upstream));
    common::run(&mut app, "git.push", Value::Null);
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .git
        .net
        .is_some()));
    app.harness.get_by_label("Cancel Git Operation").click();
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .git
        .net
        .is_none()));
    assert!(notification_texts(&app).is_empty());
}

// spec 28: "Authentication failure"
#[test]
fn authentication_failure_reports_and_logs_stderr() {
    let (_d, root, _r) = repo_with_upstream();
    let fake = Arc::new(FakeRemote::default());
    let stderr =
        "fatal: could not read Username for 'https://example.com': terminal prompts disabled";
    *fake.push_error.lock().unwrap() = Some((GitError::Auth, stderr.into()));
    let remote = fake.clone();
    let mut app = open_with(&root, vec![], move |o| o.git_remote = Some(remote));
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .git
        .status
        .head
        .has_upstream));
    common::run(&mut app, "git.push", Value::Null);
    assert!(wait_for(&mut app, |a| !notification_texts(a).is_empty()));
    let text = notification_texts(&app).remove(0);
    assert!(text.contains("Push failed."), "{text}");
    assert!(text.contains("Authentication required."), "{text}");
    let n = app
        .harness
        .state()
        .core
        .notifications
        .history()
        .next()
        .unwrap()
        .clone();
    let actions: Vec<&str> = n.actions.iter().map(|a| a.command.id.as_str()).collect();
    assert_eq!(
        actions,
        ["git.showOutput", "git.push"],
        "Show Git Output, Retry"
    );
    assert!(
        app.harness
            .state()
            .core
            .git
            .output
            .iter()
            .any(|l| l == stderr),
        "Git Output contains git's stderr"
    );
    common::run(&mut app, "git.showOutput", Value::Null);
    assert!(app.harness.query_by_label(stderr).is_some());
}

#[test]
fn first_push_publishes_the_branch() {
    let (_d, root, r) = repo();
    r.remote("origin", "/nonexistent/remote.git").unwrap();
    let fake = Arc::new(FakeRemote::default());
    let remote = fake.clone();
    let mut app = open_with(&root, vec![], move |o| o.git_remote = Some(remote));
    assert!(wait_for(&mut app, |a| !a
        .harness
        .state()
        .core
        .git
        .remotes
        .is_empty()));
    assert!(app.harness.query_by_label("Publish Branch").is_some());
    common::run(&mut app, "git.push", Value::Null);
    let b = branch(&r);
    assert!(
        app.harness
            .query_by_label(&format!("Publish {b} to origin?"))
            .is_some()
    );
    app.harness.get_by_label("Publish").click();
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .git
        .net
        .is_none()
        && !fake.calls.lock().unwrap().is_empty()));
    assert_eq!(
        fake.calls.lock().unwrap().as_slice(),
        [format!("push Some((\"origin\", \"{b}\"))")]
    );
}

#[test]
fn sync_confirms_once_per_workspace() {
    let (_d, root, _r) = repo_with_upstream();
    let fake = Arc::new(FakeRemote::default());
    let remote = fake.clone();
    let mut app = open_with(&root, vec![], move |o| o.git_remote = Some(remote));
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .git
        .status
        .head
        .has_upstream));
    common::run(&mut app, "git.sync", Value::Null);
    assert!(matches!(
        front_prompt(&app),
        Some(Prompt::Git(GitPrompt::SyncConfirm))
    ));
    app.harness.get_by_label("OK").click();
    assert!(wait_for(&mut app, |_| fake.calls.lock().unwrap().len() == 2));
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .git
        .net
        .is_none()));
    common::run(&mut app, "git.sync", Value::Null);
    assert!(front_prompt(&app).is_none(), "asked once");
    assert!(wait_for(&mut app, |_| fake.calls.lock().unwrap().len() == 4));
    assert_eq!(
        fake.calls.lock().unwrap().as_slice(),
        ["pull FfOnly", "push None", "pull FfOnly", "push None"]
    );
    assert_eq!(app.harness.state().core.git.network_runs, 2);
}

// spec 28: "Diverged branch"
#[test]
fn diverged_pull_offers_rebase_merge_or_cancel() {
    if !has_git() {
        return;
    }
    let (_d, _bare, a, b) = remote_pair();
    std::fs::write(b.join("b.md"), "from b\n").unwrap();
    git(&b, &["add", "."]);
    git(&b, &["commit", "-q", "-m", "b"]);
    git(&b, &["push", "-q"]);
    std::fs::write(a.join("a.md"), "from a\n").unwrap();
    git(&a, &["add", "."]);
    git(&a, &["commit", "-q", "-m", "a"]);
    let mut app = open(&a, vec![]);
    common::run(&mut app, "git.pull", Value::Null);
    assert!(wait_for(&mut app, |a| front_prompt(a).is_some()));
    assert!(matches!(
        front_prompt(&app),
        Some(Prompt::Git(GitPrompt::Diverged { .. }))
    ));
    assert!(
        app.harness
            .query_by_label("Your branch and origin/main have diverged.")
            .is_some()
    );
    for b in ["Pull with Rebase", "Pull with Merge", "Cancel"] {
        assert!(app.harness.query_by_label(b).is_some(), "{b}");
    }
    app.harness.get_by_label("Pull with Rebase").click();
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .git
        .net
        .is_none()
        && a.files_ok()));
    assert!(wait_for(&mut app, |_| a.join("b.md").exists()));
    assert_eq!(
        git(&a, &["log", "-1", "--format=%s"]),
        "a",
        "rebased on top"
    );
}

trait FilesOk {
    fn files_ok(&self) -> bool;
}

impl FilesOk for common::TestApp {
    fn files_ok(&self) -> bool {
        self.harness.state().core.files.prompts.is_empty()
    }
}

#[test]
fn real_push_and_fetch_against_a_local_remote() {
    if !has_git() {
        return;
    }
    let (_d, bare, a, b) = remote_pair();
    std::fs::write(a.join("n.md"), "1\n2\n3\n4\n").unwrap();
    git(&a, &["commit", "-q", "-am", "four"]);
    let mut app = open(&a, vec![]);
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .git
        .status
        .head
        .ahead
        == 1));
    common::run(&mut app, "git.push", Value::Null);
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .git
        .net
        .is_none()
        && a.harness.state().core.git.status.head.ahead == 0));
    assert_eq!(git(&bare, &["log", "-1", "--format=%s", "main"]), "four");
    assert!(
        app.harness
            .state()
            .core
            .git
            .output
            .iter()
            .any(|l| l.starts_with("> git push")),
        "the command is logged"
    );
    // Someone else pushes; Fetch shows the branch behind.
    git(&b, &["pull", "-q"]);
    std::fs::write(b.join("n.md"), "0\n1\n2\n3\n4\n").unwrap();
    git(&b, &["commit", "-q", "-am", "zero"]);
    git(&b, &["push", "-q"]);
    common::run(&mut app, "git.fetch", Value::Null);
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .git
        .status
        .head
        .behind
        == 1));
    assert!(
        app.harness
            .state()
            .core
            .git_branch_label()
            .unwrap()
            .ends_with("\u{2193}1")
    );
}

// ── Sync Notes ──────────────────────────────────────────────────────────

fn enable_notes_sync(app: &mut common::TestApp) {
    app.harness
        .state_mut()
        .core
        .set_user_setting("notes.sync.enabled", Some(SettingValue::Bool(true)));
    common::run(app, "workspace.trust", Value::Null);
}

// spec 28: "Sync stops on predicted conflict"
#[test]
fn sync_notes_stops_before_a_conflicting_pull() {
    if !has_git() {
        return;
    }
    let (_d, bare, a, b) = remote_pair();
    // The remote changes line 1…
    std::fs::write(b.join("n.md"), "B\n2\n3\n").unwrap();
    git(&b, &["commit", "-q", "-am", "remote edit"]);
    git(&b, &["push", "-q"]);
    let remote_head = git(&bare, &["rev-parse", "main"]);
    let base = git(&a, &["rev-parse", "HEAD"]);
    // …and so does an unsaved local edit.
    let mut app = open(&a, vec![a.join("n.md")]);
    enable_notes_sync(&mut app);
    {
        let core = &mut app.harness.state_mut().core;
        let view = core.workbench.active_view().unwrap();
        core.workbench
            .edit(
                view,
                notoir_editor::ChangeSet::replace(0, 1, "A"),
                notoir_editor::Selection::caret(1),
                notoir_editor::EditKind::Other,
                0,
            )
            .unwrap();
    }
    common::run(&mut app, "git.syncNotes", Value::Null);
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .git
        .notice
        .is_some()));
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .git
        .net
        .is_none()));
    // Local changes are committed (saved first)…
    assert_eq!(
        std::fs::read_to_string(a.join("n.md")).unwrap(),
        "A\n2\n3\n"
    );
    let subject = git(&a, &["log", "-1", "--format=%s"]);
    assert!(subject.starts_with("Notes sync "), "{subject}");
    assert_eq!(git(&a, &["rev-parse", "HEAD~1"]), base, "not rebased");
    // …no pull/rebase or push happened…
    assert_eq!(
        git(&bare, &["rev-parse", "main"]),
        remote_head,
        "not pushed"
    );
    assert_eq!(
        git(&a, &["status", "--porcelain"]),
        "",
        "no rebase in progress"
    );
    // …and Source Control explains.
    let core = &app.harness.state().core;
    assert_eq!(core.panels.primary.active, Some(PanelId::SourceControl));
    assert!(core.panels.primary.visible);
    assert!(
        app.harness
            .query_all_by_label_contains("Sync Notes stopped:")
            .next()
            .is_some(),
        "the panel and a notification explain"
    );
}

#[test]
fn sync_notes_commits_rebases_and_pushes() {
    if !has_git() {
        return;
    }
    let (_d, bare, a, b) = remote_pair();
    std::fs::write(b.join("other.md"), "remote note\n").unwrap();
    git(&b, &["add", "."]);
    git(&b, &["commit", "-q", "-m", "remote note"]);
    git(&b, &["push", "-q"]);
    std::fs::write(a.join("local.md"), "local note\n").unwrap();
    let mut app = open(&a, vec![]);
    enable_notes_sync(&mut app);
    common::run(&mut app, "git.syncNotes", Value::Null);
    assert!(wait_for(&mut app, |a| notification_texts(a)
        .iter()
        .any(|t| t == "Notes synced.")));
    assert!(a.join("other.md").exists(), "pulled with rebase");
    let pushed = git(&bare, &["log", "-1", "--format=%s", "main"]);
    assert!(pushed.starts_with("Notes sync "), "{pushed}");
    assert!(app.harness.state().core.git.notice.is_none());
}

#[test]
fn sync_notes_needs_the_setting_and_trust() {
    let (_d, root, _r) = repo_with_upstream();
    let mut app = open(&root, vec![]);
    let core = &app.harness.state().core;
    assert!(!core.settings.bool("notes.sync.enabled", None));
    let notes_sync = |a: &common::TestApp| {
        a.harness.state().core.context(false).get("notesSync")
            == Some(&notoir_core::when::Value::Bool(true))
    };
    assert!(!notes_sync(&app));
    enable_notes_sync(&mut app);
    assert!(notes_sync(&app));
}

// ── clone / init ────────────────────────────────────────────────────────

#[test]
fn clone_asks_for_a_folder_then_offers_to_open() {
    if !has_git() {
        return;
    }
    let (_d, bare, _a, _b) = remote_pair();
    let dest = tempfile::tempdir().unwrap();
    let dest_root = dest.path().canonicalize().unwrap();
    let dialogs = notoir::files::ScriptedDialogs::default();
    dialogs.answer_folder(Some(dest_root.clone()));
    let mut app = common::start(move |o| o.dialogs = Some(Arc::new(dialogs)));
    common::run(&mut app, "git.clone", Value::Null);
    assert!(app.harness.state().ui.picker.is_some());
    app.harness
        .event(egui::Event::Text(bare.to_string_lossy().into_owned()));
    app.harness.run_steps(2);
    app.harness.key_press(Key::Enter);
    // Observe the first frame of the asynchronously opened prompt, including
    // when cloning finishes at the end of a UI-test settling loop (Windows CI).
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        app.harness.step();
        if front_prompt(&app).is_some() {
            break;
        }
        assert!(Instant::now() < deadline, "clone did not finish");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(matches!(
        front_prompt(&app),
        Some(Prompt::Git(GitPrompt::OpenCloned(_)))
    ));
    // A centered modal needs another frame to position itself using its actual
    // size. Clicking its first-frame coordinates can miss the Open button.
    app.harness.run_steps(3);
    assert!(dest_root.join("remote/n.md").exists());
    assert!(
        app.harness
            .query_by_label("Open the cloned repository remote?")
            .is_some()
    );
    app.harness.get_by_label("Open").click();
    app.harness.run_steps(3);
    let cloned = dest_root.join("remote").canonicalize().unwrap();
    assert_eq!(
        app.harness.state().core.workspace.as_ref().map(|w| &w.root),
        Some(&cloned),
        "Open must activate the cloned workspace before repository detection"
    );
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .git
        .repo
        .is_some()));
    assert_eq!(
        app.harness.state().core.git.repo.as_ref().unwrap().workdir,
        cloned
    );
    assert_eq!(
        app.harness.state().core.git.recent_urls,
        [bare.to_string_lossy().into_owned()]
    );
}

#[test]
fn initialize_repository_confirms_and_detects_it() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().canonicalize().unwrap();
    std::fs::write(root.join("a.md"), "x\n").unwrap();
    let mut app = common::start({
        let root = root.clone();
        move |o| o.open_paths = vec![(root, None)]
    });
    common::settle(&mut app);
    common::run(&mut app, "git.showSourceControl", Value::Null);
    assert!(
        app.harness
            .query_by_label("This workspace is not a Git repository.")
            .is_some()
    );
    app.harness.get_by_label("Initialize Repository").click();
    app.harness.run_steps(4);
    let name = root.file_name().unwrap().to_string_lossy().into_owned();
    assert!(
        app.harness
            .query_by_label(&format!("Initialize a Git repository in {name}?"))
            .is_some()
    );
    app.harness.get_by_label("Initialize").click();
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .git
        .repo
        .is_some()));
    assert!(root.join(".git").is_dir());
    assert!(wait_for(&mut app, |a| a
        .harness
        .query_by_label("a.md, Untracked (Changes)")
        .is_some()));
}

/// The spec's test repository (spec 28 §Test Repository): a temporary branch
/// with a generated file is committed and pushed to origin, then the remote
/// branch is deleted and the original branch restored. Needs network access
/// and SSH credentials: `cargo test -p notoir --test git_write -- --ignored`.
#[test]
#[ignore]
fn my_documents_push_round_trip() {
    let root = std::env::var("HOME")
        .map(|h| PathBuf::from(h).join("projects/my_documents"))
        .unwrap()
        .canonicalize()
        .expect("~/projects/my_documents exists");
    let r = git2::Repository::open(&root).unwrap();
    assert_eq!(
        r.find_remote("origin").unwrap().url().ok(),
        Some("git@github.com:lvendrame/my_documents.git")
    );
    let original = branch(&r);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let temp = format!("notoir-test-{stamp}");
    let file = format!("notoir-test-{stamp}.md");
    struct Cleanup {
        root: PathBuf,
        original: String,
        temp: String,
        file: String,
    }
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let run = |args: &[&str]| {
                let _ = Command::new("git")
                    .current_dir(&self.root)
                    .args(args)
                    .output();
            };
            // Only the test's own file, branch and remote branch.
            run(&["checkout", "-q", &self.original]);
            let _ = std::fs::remove_file(self.root.join(&self.file));
            run(&["branch", "-D", &self.temp]);
            run(&["push", "-q", "origin", "--delete", &self.temp]);
        }
    }
    let _cleanup = Cleanup {
        root: root.clone(),
        original: original.clone(),
        temp: temp.clone(),
        file: file.clone(),
    };
    let mut app = open(&root, vec![]);
    common::run(&mut app, "git.createBranch", json!({ "name": temp }));
    assert!(wait_for(&mut app, |_| branch(&r) == temp));
    std::fs::write(root.join(&file), "generated by the Notoir test suite\n").unwrap();
    common::run(&mut app, "git.stage", json!({ "paths": [file] }));
    assert!(wait_for(&mut app, |a| staged(a) == [file.clone()]));
    common::run(
        &mut app,
        "git.commit",
        json!({ "message": "Notoir test commit" }),
    );
    assert!(wait_for(&mut app, |_| head_summary(&r) == "Notoir test commit"));
    assert!(wait_for(&mut app, idle));
    common::run(&mut app, "git.push", Value::Null);
    assert!(
        app.harness
            .query_by_label(&format!("Publish {temp} to origin?"))
            .is_some()
    );
    app.harness.get_by_label("Publish").click();
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .git
        .net
        .is_none()
        && a.harness.state().core.git.status.head.has_upstream));
    let remote = git(&root, &["ls-remote", "origin", &temp]);
    assert!(!remote.is_empty(), "branch pushed");
    // Back to the original branch through the app.
    common::run(&mut app, "git.checkout", json!({ "branch": original }));
    assert!(wait_for(&mut app, |_| branch(&r) == original));
}

#[test]
fn auto_fetch_runs_only_when_enabled() {
    let (_d, root, _r) = repo_with_upstream();
    let fake = Arc::new(FakeRemote::default());
    let remote = fake.clone();
    let mut app = open_with(&root, vec![], move |o| o.git_remote = Some(remote));
    for _ in 0..5 {
        common::settle(&mut app);
    }
    assert!(fake.calls.lock().unwrap().is_empty(), "off by default");
    assert_eq!(app.harness.state().core.git.next_fetch, None);
    app.harness
        .state_mut()
        .core
        .set_user_setting("git.autoFetch", Some(SettingValue::Bool(true)));
    common::settle(&mut app);
    let due = app.harness.state().core.git.next_fetch.unwrap();
    let now = app.harness.state().core.now_ms;
    assert!(due >= now + 299_000, "every git.autoFetchPeriod (300 s)");
    // Time passes: the fetch runs, and never a pull.
    app.harness.state_mut().core.git.next_fetch = Some(0);
    assert!(wait_for(&mut app, |_| !fake
        .calls
        .lock()
        .unwrap()
        .is_empty()));
    assert_eq!(fake.calls.lock().unwrap().as_slice(), ["fetch"]);
}

#[test]
fn source_control_panel_keeps_its_width() {
    let (_d, root, _r) = repo();
    std::fs::write(root.join("notes.md"), "one\ntwo\n").unwrap();
    let mut app = open(&root, vec![]);
    common::settle(&mut app);
    let before = app.harness.state().core.panels.primary.size;
    assert_eq!(
        before,
        notoir_core::panels::PanelArea::Primary.default_size()
    );
    // Repaints driven by the pointer moving over the window.
    for i in 0..40 {
        app.harness.hover_at(egui::Pos2::new(
            600.0 + (i % 7) as f32,
            300.0 + (i % 5) as f32,
        ));
        app.harness.run_steps(1);
    }
    common::settle(&mut app);
    let after = app.harness.state().core.panels.primary.size;
    assert!(
        (after - before).abs() < 0.5,
        "the sidebar grew from {before} to {after}"
    );
}
