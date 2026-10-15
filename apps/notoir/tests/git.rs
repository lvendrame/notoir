//! Git read integration (spec 28): detection, decorations, status bar,
//! gutter bars, Compare with HEAD, File History.

mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui_kittest::kittest::Queryable;
use notoir_editor::diff::LineChangeKind;
use notoir_git::FileStatus;
use serde_json::{Value, json};

fn commit_all(r: &git2::Repository, msg: &str) {
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

/// A repo with README.md committed twice.
fn repo() -> (tempfile::TempDir, PathBuf, git2::Repository) {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("notes");
    std::fs::create_dir(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let r = git2::Repository::init(&root).unwrap();
    std::fs::write(root.join("README.md"), "# Notes\n\nfirst\n").unwrap();
    commit_all(&r, "Initial notes");
    std::fs::write(root.join("README.md"), "# Notes\n\nfirst\nsecond\n").unwrap();
    commit_all(&r, "Add second line");
    (d, root, r)
}

fn open(root: &Path, files: Vec<PathBuf>) -> common::TestApp {
    let mut paths = vec![(root.to_path_buf(), None)];
    paths.extend(files.into_iter().map(|f| (f, None)));
    let mut app = common::start(move |o| o.open_paths = paths);
    common::settle(&mut app);
    app
}

fn wait_for(app: &mut common::TestApp, f: impl Fn(&common::TestApp) -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        common::settle(app);
        if f(app) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    false
}

fn status(app: &common::TestApp, rel: &str) -> Option<FileStatus> {
    app.harness.state().core.git.status.of(rel)
}

fn branch(r: &git2::Repository) -> String {
    r.head().unwrap().shorthand().unwrap().to_owned()
}

// spec 28: "Status decorations"
#[test]
fn status_decorations_in_explorer_and_status_bar() {
    let (_d, root, r) = repo();
    std::fs::write(root.join("README.md"), "# Notes\n\nchanged\n").unwrap();
    std::fs::write(root.join("idea.md"), "an idea\n").unwrap();
    let mut app = open(&root, vec![]);
    assert!(wait_for(&mut app, |a| status(a, "idea.md").is_some()));
    assert_eq!(status(&app, "README.md"), Some(FileStatus::Modified));
    assert_eq!(status(&app, "idea.md"), Some(FileStatus::Untracked));
    assert_eq!(FileStatus::Modified.letter(), "M");
    assert_eq!(FileStatus::Untracked.letter(), "U");
    assert!(app.harness.query_by_label("README.md: Modified").is_some());
    assert!(app.harness.query_by_label("idea.md: Untracked").is_some());
    let b = branch(&r);
    assert!(
        app.harness
            .query_by_label(&format!("Branch: {b}*"))
            .is_some(),
        "branch with the dirty mark"
    );
}

// spec 28: "No auto fetch by default"
#[test]
fn no_network_git_by_default() {
    let (_d, root, _r) = repo();
    let mut app = open(&root, vec![]);
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .git
        .repo
        .is_some()));
    let core = &app.harness.state().core;
    assert!(!core.settings.bool("git.autoFetch", None));
    assert!(!core.settings.bool("git.pullOnOpen", None));
    assert!(!core.settings.bool("git.autoPush", None));
    for _ in 0..20 {
        common::settle(&mut app);
    }
    assert_eq!(app.harness.state().core.git.network_runs, 0);
}

#[test]
fn external_changes_refresh_status() {
    let (_d, root, _r) = repo();
    let mut app = open(&root, vec![]);
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .git
        .repo
        .is_some()));
    assert_eq!(status(&app, "later.md"), None);
    std::fs::write(root.join("later.md"), "x\n").unwrap();
    assert!(
        wait_for(&mut app, |a| status(a, "later.md")
            == Some(FileStatus::Untracked)),
        "watcher event refreshes the status"
    );
    // Clean work tree: no dirty mark.
    std::fs::remove_file(root.join("later.md")).unwrap();
    assert!(wait_for(&mut app, |a| !a
        .harness
        .state()
        .core
        .git
        .status
        .dirty()));
}

fn changes(app: &common::TestApp) -> Vec<(LineChangeKind, usize, usize)> {
    let core = &app.harness.state().core;
    let doc = core.workbench.active_document().unwrap();
    core.git_changes(doc)
        .map(|c| c.iter().map(|c| (c.kind, c.start, c.end)).collect())
        .unwrap_or_default()
}

#[test]
fn gutter_bars_follow_edits() {
    let (_d, root, _r) = repo();
    std::fs::write(root.join("README.md"), "# Notes\n\nFIRST\nsecond\n").unwrap();
    let readme = root.join("README.md");
    let mut app = open(&root, vec![readme]);
    assert!(wait_for(&mut app, |a| !changes(a).is_empty()));
    assert_eq!(changes(&app), [(LineChangeKind::Modified, 2, 3)]);
    // Typing a new last line adds an "added" bar after the debounce.
    {
        let core = &mut app.harness.state_mut().core;
        let doc = core.workbench.active_document().unwrap();
        let len = core.workbench.document(doc).unwrap().buffer.len_chars();
        let view = core.workbench.active_view().unwrap();
        core.workbench
            .edit(
                view,
                notoir_editor::ChangeSet::insert(len, "third\n"),
                notoir_editor::Selection::caret(len),
                notoir_editor::EditKind::Other,
                0,
            )
            .unwrap();
    }
    assert!(wait_for(&mut app, |a| changes(a).len() == 2));
    assert_eq!(changes(&app)[1], (LineChangeKind::Added, 4, 5));
    let opts = {
        let core = &app.harness.state().core;
        core.editor_options(core.workbench.active_view().unwrap())
    };
    assert!(opts.git_changes.is_some(), "the editor paints them");
}

// spec 27: "No copy toward read-only side"
#[test]
fn compare_with_head_shows_buffer_changes() {
    let (_d, root, _r) = repo();
    let readme = root.join("README.md");
    let mut app = open(&root, vec![readme]);
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .git
        .repo
        .is_some()));
    {
        let core = &mut app.harness.state_mut().core;
        let doc = core.workbench.active_document().unwrap();
        let view = core.workbench.active_view().unwrap();
        core.workbench
            .edit(
                view,
                notoir_editor::ChangeSet::insert(0, "Intro\n"),
                notoir_editor::Selection::caret(0),
                notoir_editor::EditKind::Other,
                0,
            )
            .unwrap();
        let _ = doc;
    }
    common::run(&mut app, "git.compareWithHead", Value::Null);
    assert!(wait_for(&mut app, |a| !a
        .harness
        .state()
        .core
        .compare
        .sessions
        .is_empty()));
    assert!(
        app.harness
            .query_by_label("README.md (HEAD) \u{2194} README.md")
            .is_some()
    );
    let core = &app.harness.state().core;
    let s = core.compare.sessions.values().next().unwrap();
    assert_eq!(s.diff.hunks.len(), 1);
    let left = core.workbench.document(s.left.doc).unwrap();
    assert!(left.is_read_only());
    assert_eq!(left.buffer.text(), "# Notes\n\nfirst\nsecond\n");
    // HEAD is read-only: only the copy toward the buffer is offered.
    assert!(app.harness.query_by_label("Copy Change to Right").is_some());
    assert!(app.harness.query_by_label("Copy Change to Left").is_none());
}

#[test]
fn file_history_lists_commits_and_opens_changes() {
    let (_d, root, _r) = repo();
    let readme = root.join("README.md");
    let mut app = open(&root, vec![readme.clone()]);
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .git
        .repo
        .is_some()));
    common::run(&mut app, "git.fileHistory", Value::Null);
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .ui
        .picker
        .is_some()));
    assert!(
        app.harness
            .query_by_label_contains("Add second line")
            .is_some()
    );
    assert!(
        app.harness
            .query_by_label_contains("Initial notes")
            .is_some()
    );
    let newest = app
        .harness
        .state()
        .core
        .git
        .history
        .as_ref()
        .unwrap()
        .commits[0]
        .id
        .clone();
    app.harness.state_mut().ui.picker = None;
    common::run(&mut app, "git.historyPick", json!({ "rev": newest }));
    assert!(wait_for(&mut app, |a| !a
        .harness
        .state()
        .core
        .compare
        .sessions
        .is_empty()));
    let core = &app.harness.state().core;
    let s = core.compare.sessions.values().next().unwrap();
    let (l, r) = core
        .diff_texts(*core.compare.sessions.keys().next().unwrap())
        .unwrap();
    assert_eq!(l.to_string(), "# Notes\n\nfirst\n", "parent");
    assert_eq!(r.to_string(), "# Notes\n\nfirst\nsecond\n", "revision");
    assert_eq!(s.diff.hunks.len(), 1);
    let short = &newest[..7];
    assert!(
        app.harness
            .query_by_label(&format!(
                "README.md ({short}~1) \u{2194} README.md ({short})"
            ))
            .is_some()
    );
}

#[test]
fn open_file_at_revision_is_read_only() {
    let (_d, root, _r) = repo();
    let readme = root.join("README.md");
    let mut app = open(&root, vec![readme]);
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .git
        .repo
        .is_some()));
    common::run(&mut app, "git.openFileAtRevision", Value::Null);
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .git
        .history
        .is_some()));
    let oldest = app
        .harness
        .state()
        .core
        .git
        .history
        .as_ref()
        .unwrap()
        .commits[1]
        .id
        .clone();
    app.harness.state_mut().ui.picker = None;
    common::run(&mut app, "git.historyPick", json!({ "rev": oldest }));
    let label = format!("README.md ({})", &oldest[..7]);
    assert!(wait_for(&mut app, |a| a
        .harness
        .query_by_label(&label)
        .is_some()));
    let core = &app.harness.state().core;
    let doc = core
        .workbench
        .document(core.workbench.active_document().unwrap())
        .unwrap();
    assert!(doc.is_read_only());
    assert_eq!(doc.buffer.text(), "# Notes\n\nfirst\n");
}

#[test]
fn no_git_outside_repositories() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().canonicalize().unwrap();
    std::fs::write(root.join("a.md"), "x").unwrap();
    let mut app = open(&root, vec![]);
    for _ in 0..5 {
        common::settle(&mut app);
    }
    let core = &app.harness.state().core;
    assert!(core.git.repo.is_none());
    assert!(core.git_branch_label().is_none());
    assert!(app.harness.query_by_label_contains("Branch:").is_none());
}

/// The spec's test repository (spec 28 §Test Repository), read-only checks.
/// Machine-specific: `cargo test -p notoir --test git -- --ignored`.
#[test]
#[ignore]
fn my_documents_repository_is_detected() {
    let root = std::env::var("HOME")
        .map(|h| PathBuf::from(h).join("projects/my_documents"))
        .unwrap();
    let root = root.canonicalize().expect("~/projects/my_documents exists");
    let r = git2::Repository::open(&root).unwrap();
    let origin = r.find_remote("origin").unwrap();
    assert_eq!(
        origin.url().ok(),
        Some("git@github.com:lvendrame/my_documents.git")
    );
    let mut app = open(&root, vec![]);
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .git_branch_label()
        .is_some()));
    let label = app.harness.state().core.git_branch_label().unwrap();
    assert!(label.starts_with(&branch(&r)), "{label}");
    assert_eq!(app.harness.state().core.git.network_runs, 0);
}
