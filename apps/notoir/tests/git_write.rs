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
use egui_kittest::kittest::{NodeT, Queryable};
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

fn edit_open_files(app: &mut common::TestApp, prefix: &str) {
    let core = &mut app.harness.state_mut().core;
    let views: Vec<_> = core
        .workbench
        .documents
        .values()
        .filter(|d| d.path().is_some())
        .filter_map(|d| core.workbench.views.of_document(d.id).next().map(|v| v.id))
        .collect();
    for view in views {
        core.workbench
            .edit(
                view,
                notoir_editor::ChangeSet::insert(0, prefix),
                notoir_editor::Selection::caret(prefix.len()),
                notoir_editor::EditKind::Other,
                0,
            )
            .unwrap();
    }
}

// spec 28: "Detect external repository initialization"
#[test]
fn external_init_is_detected_and_refresh_is_available_without_repository() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().canonicalize().unwrap();
    let mut app = common::start({
        let root = root.clone();
        move |o| o.open_paths = vec![(root, None)]
    });
    common::settle(&mut app);
    common::run(&mut app, "git.showSourceControl", Value::Null);
    assert!(app.harness.query_by_label("Refresh Git Status").is_some());
    git2::Repository::init(&root).unwrap();
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .git
        .repo
        .is_some()));
    assert_eq!(
        app.harness.state().core.git.repo.as_ref().unwrap().workdir,
        root
    );
}

// spec 28: "Save All creates one grouped commit"
#[test]
fn manual_save_all_groups_files_and_preserves_other_staging() {
    let (_d, root, r) = repo();
    let mut app = open(&root, vec![root.join("README.md"), root.join("notes.md")]);
    common::run(&mut app, "git.autoCommit", json!({"mode":"manual"}));
    std::fs::write(root.join("unrelated.md"), "staged separately").unwrap();
    let mut index = r.index().unwrap();
    index.add_path(Path::new("unrelated.md")).unwrap();
    index.write().unwrap();
    let parent = r.head().unwrap().target().unwrap();
    edit_open_files(&mut app, "changed ");
    common::run(&mut app, "file.saveAll", Value::Null);
    assert!(wait_for(&mut app, |_| r.head().unwrap().target().unwrap() != parent));
    let head = r.head().unwrap().peel_to_commit().unwrap();
    assert_eq!(head.parent_id(0).unwrap(), parent);
    let message = head.message().unwrap();
    assert!(message.starts_with("Save files - "));
    assert!(message.ends_with("\n\nREADME.md\nnotes.md"));
    assert!(
        head.tree()
            .unwrap()
            .get_path(Path::new("unrelated.md"))
            .is_err()
    );
    assert!(
        r.index()
            .unwrap()
            .get_path(Path::new("unrelated.md"), 0)
            .is_some()
    );
    assert_eq!(app.harness.state().core.git.network_runs, 0);
}

// spec 28: "Single-file save retains original message"
#[test]
fn manual_single_file_save_and_internal_save_exclusion() {
    let (_d, root, r) = repo();
    let mut app = open(&root, vec![root.join("notes.md")]);
    common::run(&mut app, "git.autoCommit", json!({"mode":"manual"}));
    let parent = r.head().unwrap().target().unwrap();
    edit_open_files(&mut app, "internal ");
    let doc = app
        .harness
        .state()
        .core
        .workbench
        .active_document()
        .unwrap();
    app.harness.state_mut().core.save(doc);
    assert!(wait_for(&mut app, |a| !a
        .harness
        .state()
        .core
        .saving
        .contains(&doc)));
    assert_eq!(r.head().unwrap().target().unwrap(), parent);
    edit_open_files(&mut app, "manual ");
    common::run(&mut app, "file.saveAll", Value::Null);
    assert!(wait_for(&mut app, |_| r.head().unwrap().target().unwrap() != parent));
    let message = head_summary(&r);
    assert!(message.starts_with("File saved - notes.md - Date: "));
    assert!(message.contains(" Time: "));
    assert_eq!(
        r.head()
            .unwrap()
            .peel_to_commit()
            .unwrap()
            .message()
            .unwrap()
            .lines()
            .count(),
        1
    );
}

// spec 28: "Auto Push depends on Auto Commit"
#[test]
fn auto_push_menu_dependency_and_workspace_settings() {
    let (_d, root, _r) = repo();
    let mut app = open(&root, vec![]);
    assert!(
        !app.harness
            .state()
            .core
            .settings
            .bool("git.autoCommit", None)
    );
    common::run(&mut app, "git.autoPush", Value::Null);
    assert!(!app.harness.state().core.settings.bool("git.autoPush", None));
    common::run(&mut app, "git.autoCommit", json!({"mode":"manual"}));
    // Explicit local consent works without trusting unrelated workspace settings.
    common::run(&mut app, "git.autoPush", Value::Null);
    assert!(app.harness.state().core.settings.bool("git.autoPush", None));
    common::run(&mut app, "git.autoCommit", json!({"mode":"off"}));
    assert!(
        !app.harness
            .state()
            .core
            .settings
            .bool("git.autoCommit", None)
    );
    assert!(!app.harness.state().core.settings.bool("git.autoPush", None));
    common::run(&mut app, "git.autoCommit", json!({"mode":"manual"}));
    assert!(!app.harness.state().core.settings.bool("git.autoPush", None));
    assert!(wait_for(&mut app, |_| std::fs::read_to_string(
        root.join(".notoir/settings.toml")
    )
    .is_ok_and(
        |s| s.contains("autoCommit = true") && s.contains("autoPush = false")
    )));
}

// spec 28: "Save All pushes its grouped commit"
#[test]
fn auto_push_sends_grouped_commit_to_local_remote() {
    if !has_git() {
        return;
    }
    let (_d, bare, a, _b) = remote_pair();
    let r = git2::Repository::open(&a).unwrap();
    std::fs::write(a.join("second.md"), "second\n").unwrap();
    git(&a, &["add", "second.md"]);
    git(&a, &["commit", "-q", "-m", "second file"]);
    git(&a, &["push", "-q"]);
    let mut app = open(&a, vec![a.join("n.md"), a.join("second.md")]);
    common::run(&mut app, "workspace.trust", Value::Null);
    common::run(&mut app, "git.autoCommit", json!({"mode":"manual"}));
    common::run(&mut app, "git.autoPush", Value::Null);
    let parent = r.head().unwrap().target().unwrap();
    edit_open_files(&mut app, "saved ");
    common::run(&mut app, "file.saveAll", Value::Null);
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .git
        .network_runs
        == 1
        && a.harness.state().core.git.net.is_none()));
    assert_ne!(r.head().unwrap().target().unwrap(), parent);
    let head = r.head().unwrap().peel_to_commit().unwrap();
    assert_eq!(head.parent_id(0).unwrap(), parent);
    assert!(head.message().unwrap().ends_with("\n\nn.md\nsecond.md"));
    let remote = git2::Repository::open_bare(&bare).unwrap();
    assert_eq!(
        remote.find_reference("refs/heads/main").unwrap().target(),
        r.head().unwrap().target()
    );
}

#[test]
fn save_all_waits_for_conflict_cancellation_then_commits_successful_files() {
    let (_d, root, r) = repo();
    let mut app = open(&root, vec![root.join("README.md"), root.join("notes.md")]);
    common::run(&mut app, "git.autoCommit", json!({"mode":"manual"}));
    let parent = r.head().unwrap().target().unwrap();
    edit_open_files(&mut app, "changed ");
    std::fs::write(root.join("notes.md"), "external change\n").unwrap();
    common::run(&mut app, "file.saveAll", Value::Null);
    assert!(wait_for(&mut app, |a| matches!(
        front_prompt(a),
        Some(Prompt::SaveConflict { .. })
    )));
    assert_eq!(
        r.head().unwrap().target().unwrap(),
        parent,
        "batch waits for the prompt"
    );
    app.harness
        .state_mut()
        .core
        .answer_prompt(notoir::files::Answer::Cancel);
    assert!(wait_for(&mut app, |_| r.head().unwrap().target().unwrap() != parent));
    assert!(head_summary(&r).starts_with("File saved - README.md - Date: "));
    let tree = r.head().unwrap().peel_to_commit().unwrap().tree().unwrap();
    let note = tree.get_path(Path::new("notes.md")).unwrap();
    assert_eq!(r.find_blob(note.id()).unwrap().content(), b"one\n");
    assert_eq!(
        std::fs::read(root.join("notes.md")).unwrap(),
        b"external change\n"
    );
}

#[test]
fn auto_push_publish_cancellation_preserves_local_commit() {
    if !has_git() {
        return;
    }
    let (_d, bare, a, _b) = remote_pair();
    git(&a, &["checkout", "-q", "-b", "save-checkpoints"]);
    let r = git2::Repository::open(&a).unwrap();
    let mut app = open(&a, vec![a.join("n.md")]);
    common::run(&mut app, "workspace.trust", Value::Null);
    common::run(&mut app, "git.autoCommit", json!({"mode":"manual"}));
    common::run(&mut app, "git.autoPush", Value::Null);
    let parent = r.head().unwrap().target().unwrap();
    edit_open_files(&mut app, "saved ");
    common::run(&mut app, "file.save", Value::Null);
    assert!(wait_for(&mut app, |a| matches!(
        front_prompt(a),
        Some(Prompt::Git(GitPrompt::AutoPublish { .. }))
    )));
    let saved = r.head().unwrap().target().unwrap();
    assert_ne!(saved, parent);
    app.harness
        .state_mut()
        .core
        .answer_prompt(notoir::files::Answer::Cancel);
    common::settle(&mut app);
    assert_eq!(app.harness.state().core.git.network_runs, 0);
    assert_eq!(r.head().unwrap().target().unwrap(), saved);
    assert!(
        git2::Repository::open_bare(&bare)
            .unwrap()
            .find_reference("refs/heads/save-checkpoints")
            .is_err()
    );
}

#[test]
fn auto_push_rejection_keeps_local_commit_and_offers_targeted_retry() {
    if !has_git() {
        return;
    }
    let (_d, _bare, a, _b) = remote_pair();
    let r = git2::Repository::open(&a).unwrap();
    let remote = Arc::new(FakeRemote::default());
    *remote.push_error.lock().unwrap() = Some((GitError::Rejected, "rejected test push".into()));
    let mut app = open_with(&a, vec![a.join("n.md")], {
        let remote = remote.clone();
        move |o| o.git_remote = Some(remote)
    });
    common::run(&mut app, "workspace.trust", Value::Null);
    common::run(&mut app, "git.autoCommit", json!({"mode":"manual"}));
    common::run(&mut app, "git.autoPush", Value::Null);
    edit_open_files(&mut app, "saved ");
    common::run(&mut app, "file.save", Value::Null);
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .git
        .network_runs
        == 1
        && a.harness.state().core.git.net.is_none()));
    let saved = r.head().unwrap().target().unwrap();
    assert!(head_summary(&r).starts_with("File saved - n.md -"));
    assert!(
        notification_texts(&app)
            .iter()
            .any(|s| s.contains("Push failed"))
    );
    assert_eq!(
        remote.calls.lock().unwrap().len(),
        1,
        "no automatic pull or retry"
    );
    let generation = app.harness.state().core.git.generation;
    common::run(
        &mut app,
        "git.retryAutoPush",
        json!({"workdir":a,"branch":"main","generation":generation}),
    );
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .git
        .network_runs
        == 2
        && a.harness.state().core.git.net.is_none()));
    assert_eq!(r.head().unwrap().target().unwrap(), saved);
}

#[test]
fn save_as_commits_new_file_and_preserves_encoding_and_line_endings() {
    let (_d, root, r) = repo();
    let target = root.join("new.md");
    let dialogs = Arc::new(notoir::files::ScriptedDialogs::default());
    dialogs.answer_save(Some(target.clone()));
    let mut app = open_with(&root, vec![root.join("notes.md")], {
        let dialogs = dialogs.clone();
        move |o| o.dialogs = Some(dialogs)
    });
    common::run(&mut app, "git.autoCommit", json!({"mode":"manual"}));
    let doc = app
        .harness
        .state()
        .core
        .workbench
        .active_document()
        .unwrap();
    app.harness
        .state_mut()
        .core
        .workbench
        .document_mut(doc)
        .unwrap()
        .line_ending = notoir_editor::LineEnding::CrLf;
    edit_open_files(&mut app, "new ");
    let parent = r.head().unwrap().target().unwrap();
    common::run(&mut app, "file.saveAs", Value::Null);
    assert!(wait_for(&mut app, |_| r.head().unwrap().target().unwrap() != parent));
    let tree = r.head().unwrap().peel_to_commit().unwrap().tree().unwrap();
    let entry = tree.get_path(Path::new("new.md")).unwrap();
    assert_eq!(
        r.find_blob(entry.id()).unwrap().content(),
        std::fs::read(target).unwrap()
    );
    assert_eq!(r.find_blob(entry.id()).unwrap().content(), b"new one\r\n");
    assert!(head_summary(&r).starts_with("File saved - new.md - Date: "));
}

#[test]
fn save_all_groups_each_nested_repository_separately() {
    let (_d, root, outer) = repo();
    let nested = root.join("nested");
    std::fs::create_dir(&nested).unwrap();
    let inner = git2::Repository::init(&nested).unwrap();
    set_identity(&inner);
    std::fs::write(nested.join("inner.md"), "inside\n").unwrap();
    commit_all(&inner, "Inner initial");
    let mut app = open(&root, vec![root.join("notes.md"), nested.join("inner.md")]);
    app.harness
        .state_mut()
        .core
        .set_user_setting("git.scanNestedRepos", Some(SettingValue::Bool(true)));
    common::run(&mut app, "git.refresh", Value::Null);
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .git
        .repos
        .len()
        == 2
        && a.harness
            .state()
            .core
            .git_status_for(&nested)
            .is_some_and(|s| s.head.branch.is_some())));
    common::run(&mut app, "git.autoCommit", json!({"mode":"manual"}));
    let old_outer = outer.head().unwrap().target().unwrap();
    let old_inner = inner.head().unwrap().target().unwrap();
    edit_open_files(&mut app, "saved ");
    common::run(&mut app, "file.saveAll", Value::Null);
    common::run(&mut app, "git.selectRepository", json!({"path": nested}));
    assert!(wait_for(&mut app, |_| outer
        .head()
        .unwrap()
        .target()
        .unwrap()
        != old_outer
        && inner.head().unwrap().target().unwrap() != old_inner));
    assert!(head_summary(&outer).starts_with("File saved - notes.md - Date: "));
    assert!(head_summary(&inner).starts_with("File saved - inner.md - Date: "));
    assert_eq!(
        outer
            .head()
            .unwrap()
            .peel_to_commit()
            .unwrap()
            .parent_id(0)
            .unwrap(),
        old_outer
    );
    assert_eq!(
        inner
            .head()
            .unwrap()
            .peel_to_commit()
            .unwrap()
            .parent_id(0)
            .unwrap(),
        old_inner
    );
}

/// Opt-in live round trip through the app against the real my_documents
/// origin. Requires an isolated worktree supplied by NOTOIR_LIVE_WORKTREE.
/// The caller owns local worktree/branch cleanup; the test removes its remote
/// branch even on failure. Only generated test files are used.
#[test]
#[ignore = "pushes a temporary branch to the real my_documents origin"]
fn my_documents_auto_save_live_round_trip() {
    let root = PathBuf::from(
        std::env::var("NOTOIR_LIVE_WORKTREE")
            .expect("set NOTOIR_LIVE_WORKTREE to an isolated my_documents worktree"),
    )
    .canonicalize()
    .unwrap();
    let r = git2::Repository::open(&root).unwrap();
    assert_eq!(
        r.find_remote("origin").unwrap().url().ok(),
        Some("git@github.com:lvendrame/my_documents.git")
    );
    let temp_branch = branch(&r);
    assert!(
        temp_branch.starts_with("codex/notoir-live-save-"),
        "must use an isolated test branch"
    );
    // The caller owns the supplied local worktree; this test owns the remote
    // branch it publishes and removes that branch even if an assertion fails.
    struct RemoteCleanup {
        root: PathBuf,
        branch: String,
    }
    impl Drop for RemoteCleanup {
        fn drop(&mut self) {
            let exists = Command::new("git")
                .current_dir(&self.root)
                .args([
                    "ls-remote",
                    "--exit-code",
                    "--heads",
                    "origin",
                    &self.branch,
                ])
                .output();
            if exists.is_ok_and(|out| out.status.success()) {
                match Command::new("git")
                    .current_dir(&self.root)
                    .args(["push", "origin", "--delete", &self.branch])
                    .output()
                {
                    Ok(out) if out.status.success() => {
                        println!("LIVE CLEANUP: temporary remote branch removed")
                    }
                    _ => eprintln!(
                        "Live test cleanup failed: remove remote branch {}",
                        self.branch
                    ),
                }
            }
        }
    }
    let _remote_cleanup = RemoteCleanup {
        root: root.clone(),
        branch: temp_branch.clone(),
    };
    let directory = root.join("__notoir_live__");
    assert!(directory.is_dir());
    let alpha = directory.join("alpha.md");
    let beta = directory.join("beta.md");
    let staged_only = "__notoir_live__/staged-only.md";
    let mut index = r.index().unwrap();
    index.add_path(Path::new(staged_only)).unwrap();
    index.write().unwrap();
    let staged_oid = index.get_path(Path::new(staged_only), 0).unwrap().id;
    let initial = r.head().unwrap().target().unwrap();
    let mut app = open(&root, vec![alpha.clone(), beta.clone()]);
    common::run(&mut app, "git.autoCommit", json!({"mode":"manual"}));
    let beta_doc = app
        .harness
        .state()
        .core
        .workbench
        .active_document()
        .unwrap();
    let beta_view = app.harness.state().core.workbench.active_view().unwrap();
    common::run(
        &mut app,
        "editor.type",
        json!({"text":"First live manual save\n"}),
    );
    common::run(&mut app, "file.save", Value::Null);
    assert!(wait_for(&mut app, |_| r.head().unwrap().target().unwrap() != initial));
    assert!(head_summary(&r).starts_with("File saved - beta.md - Date: "));
    let first = r.head().unwrap().peel_to_commit().unwrap();
    let expected = first
        .tree()
        .unwrap()
        .get_path(Path::new("__notoir_live__/beta.md"))
        .unwrap()
        .id();
    assert_eq!(
        r.find_blob(expected).unwrap().content(),
        std::fs::read(&beta).unwrap()
    );
    assert_eq!(app.harness.state().core.git.network_runs, 0);
    println!(
        "LIVE PASS: manual Save created single-file message and exact saved contents; no push while disabled"
    );

    let before_batch = r.head().unwrap().target().unwrap();
    edit_open_files(&mut app, "Live Save All\n");
    common::run(&mut app, "file.saveAll", Value::Null);
    assert!(wait_for(&mut app, |_| r.head().unwrap().target().unwrap()
        != before_batch));
    let grouped = r.head().unwrap().peel_to_commit().unwrap();
    assert_eq!(grouped.parent_id(0).unwrap(), before_batch);
    assert!(grouped.message().unwrap().starts_with("Save files - "));
    assert!(
        grouped
            .message()
            .unwrap()
            .ends_with("\n\n__notoir_live__/alpha.md\n__notoir_live__/beta.md")
    );
    assert!(
        grouped
            .tree()
            .unwrap()
            .get_path(Path::new(staged_only))
            .is_err()
    );
    assert_eq!(
        r.index()
            .unwrap()
            .get_path(Path::new(staged_only), 0)
            .unwrap()
            .id,
        staged_oid
    );
    println!("LIVE PASS: Save All created one commit with both paths; unrelated staging preserved");

    assert!(!app.harness.state().core.workspace.as_ref().unwrap().trusted);
    app.harness.get_by_label("File").click();
    app.harness.run_steps(2);
    app.harness.get_by_label_contains("Auto Commit").click();
    app.harness.run_steps(2);
    app.harness.get_by_label_contains("Auto Push").click();
    common::settle(&mut app);
    assert!(app.harness.state().core.settings.bool("git.autoPush", None));
    assert!(!app.harness.state().core.workspace.as_ref().unwrap().trusted);
    // Select beta explicitly, then make a real user save through the registry.
    app.harness
        .state_mut()
        .core
        .workbench
        .activate_view(beta_view)
        .unwrap();
    common::run(&mut app, "editor.type", json!({"text":"Live publish\n"}));
    common::run(&mut app, "file.save", json!({"doc":beta_doc.0}));
    assert!(wait_for(&mut app, |a| matches!(
        front_prompt(a),
        Some(Prompt::Git(GitPrompt::AutoPublish { .. }))
    )));
    app.harness.get_by_label("Publish").click();
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .git
        .network_runs
        == 1
        && a.harness.state().core.git.net.is_none()));
    let published = r.head().unwrap().target().unwrap();
    let actual_remote = git(
        &root,
        &["ls-remote", "origin", &format!("refs/heads/{temp_branch}")],
    );
    assert!(
        actual_remote.starts_with(&published.to_string()),
        "remote SHA must match local save commit: {actual_remote}"
    );
    println!(
        "LIVE PASS: Auto Push requested publishing and pushed the save commit to GitHub ({published})"
    );

    let before_push = r.head().unwrap().target().unwrap();
    edit_open_files(&mut app, "Live grouped push\n");
    common::run(&mut app, "file.saveAll", Value::Null);
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .git
        .network_runs
        == 2
        && a.harness.state().core.git.net.is_none()));
    let head = r.head().unwrap().peel_to_commit().unwrap();
    assert_eq!(head.parent_id(0).unwrap(), before_push);
    assert!(
        head.message()
            .unwrap()
            .ends_with("\n\n__notoir_live__/alpha.md\n__notoir_live__/beta.md")
    );
    let actual_remote = git(
        &root,
        &["ls-remote", "origin", &format!("refs/heads/{temp_branch}")],
    );
    assert!(actual_remote.starts_with(&head.id().to_string()));
    assert_eq!(
        r.index()
            .unwrap()
            .get_path(Path::new(staged_only), 0)
            .unwrap()
            .id,
        staged_oid
    );
    assert!(
        head.tree()
            .unwrap()
            .get_path(Path::new(staged_only))
            .is_err()
    );
    println!(
        "LIVE PASS: Save All auto-pushed exactly once; GitHub SHA matches grouped commit; staged-only file excluded"
    );

    common::run(&mut app, "git.autoCommit", json!({"mode":"off"}));
    assert!(
        !app.harness
            .state()
            .core
            .settings
            .bool("git.autoCommit", None)
    );
    assert!(!app.harness.state().core.settings.bool("git.autoPush", None));
    common::run(&mut app, "git.autoCommit", json!({"mode":"manual"}));
    assert!(!app.harness.state().core.settings.bool("git.autoPush", None));
    assert!(wait_for(&mut app, idle));
    println!(
        "LIVE PASS: disabling Auto Commit also disabled Auto Push; re-enabling did not turn Auto Push on"
    );
}

// spec 28: "Auto Push selection in an untrusted workspace"
#[test]
fn auto_push_is_inside_auto_commit_and_checks_in_an_untrusted_workspace() {
    let (_d, root, _r) = repo();
    let mut app = open(&root, vec![]);
    assert!(!app.harness.state().core.workspace.as_ref().unwrap().trusted);
    let open_submenu = |app: &mut common::TestApp| {
        app.harness.key_press(Key::Escape);
        app.harness.key_press(Key::Escape);
        app.harness.run_steps(2);
        app.harness.get_by_label("File").click();
        app.harness.run_steps(2);
        assert!(
            app.harness.query_by_label_contains("Auto Push").is_none(),
            "Auto Push must not be a top-level File entry"
        );
        app.harness.get_by_label_contains("Auto Commit").click();
        app.harness.run_steps(2);
    };
    open_submenu(&mut app);
    assert!(
        app.harness
            .get_by_label_contains("Auto Push")
            .accesskit_node()
            .is_disabled()
    );
    app.harness.get_by_label_contains("On Manual Save").click();
    common::settle(&mut app);
    open_submenu(&mut app);
    assert!(
        !app.harness
            .get_by_label_contains("Auto Push")
            .accesskit_node()
            .is_disabled()
    );
    app.harness.get_by_label_contains("Auto Push").click();
    common::settle(&mut app);
    assert!(app.harness.state().core.settings.bool("git.autoPush", None));
    assert_eq!(
        app.harness.state().core.context(false).get("autoPush"),
        Some(&notoir_core::when::Value::Bool(true))
    );
    assert!(
        !app.harness.state().core.workspace.as_ref().unwrap().trusted,
        "the checkbox must not trust the whole workspace"
    );
    assert!(
        !app.harness
            .state()
            .core
            .settings
            .bool("mcp.autoStart", None)
    );
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .state_db
        .as_ref()
        .unwrap()
        .workspace_push_consent(&a.harness.state().core.workspace.as_ref().unwrap().key)
        .unwrap()
        == Some(true)));
    let key = app
        .harness
        .state()
        .core
        .workspace
        .as_ref()
        .unwrap()
        .key
        .clone();
    // Reopen the workspace: local consent survives but does not leak to another folder.
    app.harness.key_press(Key::Escape);
    app.harness.key_press(Key::Escape);
    common::run(&mut app, "workspace.close", Value::Null);
    app.harness.state_mut().core.open_workspace(&root);
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .workspace
        .as_ref()
        .is_some_and(|ws| ws.key == key)
        && a.harness.state().core.settings.bool("git.autoPush", None)));
    common::run(&mut app, "git.autoPush", Value::Null);
    assert!(!app.harness.state().core.settings.bool("git.autoPush", None));
}
