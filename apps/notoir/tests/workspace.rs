//! Workspaces and the Explorer (specs 07, 08): opening, trust, the tree,
//! file operations and watching.

mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui::{Event, Key, Modifiers};
use egui_kittest::kittest::Queryable;
use notoir::files::ScriptedDialogs;
use notoir_core::document::DiskState;
use serde_json::{Value, json};

fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
    let p = dir.join(name);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&p, text).unwrap();
    p
}

/// A workspace folder (canonical, so paths compare with the app's).
fn folder() -> (tempfile::TempDir, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("notes");
    std::fs::create_dir(&root).unwrap();
    let root = root.canonicalize().unwrap();
    (d, root)
}

fn open_ws(dialogs: &ScriptedDialogs, root: &Path, files: Vec<PathBuf>) -> common::TestApp {
    let d = dialogs.clone();
    let mut paths = vec![(root.to_path_buf(), None)];
    paths.extend(files.into_iter().map(|f| (f, None)));
    let mut app = common::start(move |o| {
        o.dialogs = Some(std::sync::Arc::new(d));
        o.open_paths = paths;
    });
    common::settle(&mut app);
    app
}

fn listed(app: &common::TestApp, name: &str) -> bool {
    app.harness.query_by_label(name).is_some()
}

fn active_text(app: &common::TestApp) -> String {
    let core = &app.harness.state().core;
    let doc = core.workbench.active_document().unwrap();
    core.workbench.document(doc).unwrap().buffer.text()
}

/// Waits (up to `secs`) for a condition that depends on background work.
fn wait_for(app: &mut common::TestApp, secs: u64, f: impl Fn(&common::TestApp) -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        common::settle(app);
        if f(app) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

// spec 07: "Opening a workspace creates no metadata"
#[test]
fn opening_a_workspace_creates_no_metadata() {
    let (_d, root) = folder();
    write(&root, "README.md", "# hi");
    write(&root, "journal/today.md", "x");
    let app = open_ws(&ScriptedDialogs::default(), &root, vec![]);
    assert!(listed(&app, "README.md"));
    assert!(listed(&app, "journal"));
    assert!(!root.join(".notoir").exists());
    assert_eq!(app.harness.state().core.workspace_name(), Some("notes"));
}

// spec 07: "Hidden files toggle"
#[test]
fn hidden_files_toggle() {
    let (_d, root) = folder();
    write(&root, ".env", "SECRET=1");
    write(&root, "README.md", "x");
    let mut app = open_ws(&ScriptedDialogs::default(), &root, vec![]);
    assert!(listed(&app, "README.md"));
    assert!(!listed(&app, ".env"));
    common::run(&mut app, "explorer.toggleHidden", Value::Null);
    common::settle(&mut app);
    assert!(listed(&app, ".env"));
}

// spec 07: "Restricted workspace settings require trust"
#[test]
fn restricted_settings_require_trust() {
    let (_d, root) = folder();
    write(
        &root,
        ".notoir/settings.toml",
        "[mcp]\nautoStart = true\n\n[editor]\ntabSize = 7\n",
    );
    let mut app = open_ws(&ScriptedDialogs::default(), &root, vec![]);
    {
        let core = &app.harness.state().core;
        assert_eq!(
            core.settings.int("editor.tabSize", None),
            7,
            "unrestricted settings apply"
        );
        assert!(
            !core.settings.bool("mcp.autoStart", None),
            "MCP does not start"
        );
        let notice = core
            .notifications
            .history()
            .find(|n| n.message.key == "notifications:workspaceTrust")
            .expect("trust notice");
        assert!(
            notice
                .actions
                .iter()
                .any(|a| a.command.id.as_str() == "workspace.trust")
        );
    }
    common::run(&mut app, "workspace.trust", Value::Null);
    assert!(
        app.harness
            .state()
            .core
            .settings
            .bool("mcp.autoStart", None)
    );
    // Trust is remembered for the next session.
    app.harness.state_mut().core.close_workspace(None);
    common::settle(&mut app);
    common::run(
        &mut app,
        "workspace.open",
        json!({ "path": root.to_string_lossy() }),
    );
    common::settle(&mut app);
    assert!(
        app.harness
            .state()
            .core
            .settings
            .bool("mcp.autoStart", None)
    );
}

// spec 07: "Renaming an open file keeps the editor"
#[test]
fn renaming_an_open_dirty_file_keeps_the_editor() {
    let (_d, root) = folder();
    let draft = write(&root, "draft.md", "text");
    let mut app = open_ws(&ScriptedDialogs::default(), &root, vec![draft.clone()]);
    app.harness.event(Event::Text("unsaved ".into()));
    app.harness.run_steps(2);
    common::run(
        &mut app,
        "explorer.rename",
        json!({ "path": draft.to_string_lossy() }),
    );
    // The stem is selected: typing replaces "draft" and keeps ".md".
    app.harness.event(Event::Text("final".into()));
    app.harness.run_steps(1);
    app.harness.key_press(Key::Enter);
    app.harness.run_steps(2);
    common::settle(&mut app);
    assert!(root.join("final.md").exists());
    assert!(!draft.exists());
    assert!(listed(&app, "final.md ●"), "tab renamed and still dirty");
    assert_eq!(active_text(&app), "unsaved text");
}

#[test]
fn explorer_undo_reverts_a_rename() {
    let (_d, root) = folder();
    let a = write(&root, "a.md", "x");
    let mut app = open_ws(&ScriptedDialogs::default(), &root, vec![]);
    common::run(
        &mut app,
        "explorer.rename",
        json!({ "path": a.to_string_lossy() }),
    );
    app.harness.event(Event::Text("b".into()));
    app.harness.key_press(Key::Enter);
    app.harness.run_steps(2);
    common::settle(&mut app);
    assert!(root.join("b.md").exists());
    common::run(&mut app, "explorer.undo", Value::Null);
    common::settle(&mut app);
    assert!(a.exists() && !root.join("b.md").exists());
}

// spec 07: "Delete moves to trash and keeps open buffer"
#[test]
fn delete_moves_to_trash_and_keeps_buffer() {
    let (_d, root) = folder();
    let old = write(&root, "old.md", "keep me");
    let dialogs = ScriptedDialogs::default();
    let mut app = open_ws(&dialogs, &root, vec![old.clone()]);
    common::run(
        &mut app,
        "explorer.delete",
        json!({ "path": old.to_string_lossy() }),
    );
    assert!(listed(&app, "Move “old.md” to the Trash?"));
    app.harness.key_press(Key::Enter);
    app.harness.run_steps(2);
    common::settle(&mut app);
    assert_eq!(dialogs.trashed(), vec![old.clone()]);
    assert!(!old.exists());
    assert!(
        !listed(&app, "old.md"),
        "gone from the tree; the tab shows the dirty label"
    );
    let core = &app.harness.state().core;
    let doc = core
        .workbench
        .document(core.workbench.active_document().unwrap())
        .unwrap();
    assert_eq!(doc.disk_state, DiskState::DeletedExternally);
    assert!(doc.is_dirty());
    assert_eq!(doc.buffer.text(), "keep me");
}

#[test]
fn failed_trash_offers_permanent_delete() {
    let (_d, root) = folder();
    let f = write(&root, "x.md", "x");
    let dialogs = ScriptedDialogs::default();
    dialogs
        .trash_fails
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let mut app = open_ws(&dialogs, &root, vec![]);
    common::run(
        &mut app,
        "explorer.delete",
        json!({ "path": f.to_string_lossy() }),
    );
    app.harness.key_press(Key::Enter);
    app.harness.run_steps(2);
    common::settle(&mut app);
    assert!(f.exists());
    assert!(listed(&app, "Permanently delete “x.md”?"));
    app.harness.key_press(Key::Enter);
    app.harness.run_steps(2);
    common::settle(&mut app);
    assert!(!f.exists());
}

#[test]
fn new_file_inline_creates_and_opens() {
    let (_d, root) = folder();
    let mut app = open_ws(&ScriptedDialogs::default(), &root, vec![]);
    common::run(&mut app, "explorer.newFile", Value::Null);
    app.harness.event(Event::Text("ideas.md".into()));
    app.harness.key_press(Key::Enter);
    app.harness.run_steps(2);
    common::settle(&mut app);
    assert!(root.join("ideas.md").is_file());
    let core = &app.harness.state().core;
    let doc = core.workbench.active_document().unwrap();
    assert_eq!(core.document_title(doc), "ideas.md");
}

#[test]
fn invalid_new_name_shows_error_and_creates_nothing() {
    let (_d, root) = folder();
    let mut app = open_ws(&ScriptedDialogs::default(), &root, vec![]);
    common::run(&mut app, "explorer.newFile", Value::Null);
    app.harness.event(Event::Text("a/b".into()));
    app.harness.run_steps(1);
    app.harness.key_press(Key::Enter);
    app.harness.run_steps(2);
    common::settle(&mut app);
    assert!(listed(
        &app,
        "The name contains a character that is not allowed."
    ));
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
}

#[test]
fn existing_name_is_rejected() {
    let (_d, root) = folder();
    write(&root, "a.md", "x");
    let mut app = open_ws(&ScriptedDialogs::default(), &root, vec![]);
    common::run(&mut app, "explorer.newFile", Value::Null);
    app.harness.event(Event::Text("a.md".into()));
    app.harness.key_press(Key::Enter);
    app.harness.run_steps(2);
    assert!(listed(
        &app,
        "A file or folder with this name already exists."
    ));
}

// spec 07: "External file creation appears automatically"
#[test]
fn external_creation_appears_automatically() {
    let (_d, root) = folder();
    std::fs::create_dir(root.join("inbox")).unwrap();
    let mut app = open_ws(&ScriptedDialogs::default(), &root, vec![]);
    app.harness.state_mut().core.toggle_dir(&root.join("inbox"));
    common::settle(&mut app);
    // Some watcher backends start delivering shortly after registration.
    std::thread::sleep(Duration::from_millis(200));
    let start = Instant::now();
    write(&root, "inbox/today.md", "x");
    assert!(wait_for(&mut app, 5, |a| listed(a, "today.md")));
    assert!(
        start.elapsed() < Duration::from_secs(1),
        "took {:?}",
        start.elapsed()
    );
}

#[test]
fn drag_and_drop_moves_into_folder_after_confirmation() {
    let (_d, root) = folder();
    write(&root, "a.md", "x");
    std::fs::create_dir(root.join("archive")).unwrap();
    let mut app = open_ws(&ScriptedDialogs::default(), &root, vec![]);
    let from = app.harness.get_by_label("a.md").rect().center();
    let to = app.harness.get_by_label("archive").rect().center();
    let h = &mut app.harness;
    h.hover_at(from);
    h.run_steps(1);
    h.drag_at(from);
    h.run_steps(1);
    for i in 1..=6 {
        h.hover_at(from + (to - from) * (i as f32 / 6.0));
        h.run_steps(1);
    }
    h.event(Event::PointerButton {
        pos: to,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    h.run_steps(3);
    assert!(listed(&app, "Move “a.md” into “archive”?"));
    app.harness.key_press(Key::Enter);
    app.harness.run_steps(2);
    common::settle(&mut app);
    assert!(root.join("archive/a.md").exists());
    assert!(!root.join("a.md").exists());
}

#[test]
fn filter_shows_matches_and_ancestors() {
    let (_d, root) = folder();
    write(&root, "docs/readme.md", "x");
    write(&root, "src/main.rs", "x");
    let mut app = open_ws(&ScriptedDialogs::default(), &root, vec![]);
    common::run(&mut app, "explorer.focusFilter", Value::Null);
    app.harness.event(Event::Text("rdm".into()));
    app.harness.run_steps(3);
    assert!(
        listed(&app, "readme.md"),
        "found although docs/ was never expanded"
    );
    assert!(listed(&app, "docs"));
    assert!(!listed(&app, "src"));
}

#[test]
fn close_workspace_keeps_scratchpads_and_outside_files() {
    let (_d, root) = folder();
    let inside = write(&root, "in.md", "x");
    let other = tempfile::tempdir().unwrap();
    let outside = write(other.path(), "out.md", "y");
    let mut app = open_ws(&ScriptedDialogs::default(), &root, vec![inside, outside]);
    common::run(&mut app, "file.newScratchpad", Value::Null);
    common::run(&mut app, "workspace.close", Value::Null);
    common::settle(&mut app);
    let core = &app.harness.state().core;
    assert!(core.workspace.is_none());
    let titles: Vec<String> = core
        .workbench
        .documents
        .keys()
        .map(|d| core.document_title(*d))
        .collect();
    assert!(titles.contains(&"out.md".to_string()));
    assert!(titles.iter().any(|t| t.starts_with("Scratch")));
    assert!(!titles.contains(&"in.md".to_string()));
    assert!(listed(&app, "No folder opened"));
}

#[test]
fn opening_another_workspace_asks_first() {
    let (_d, root) = folder();
    let other = tempfile::tempdir().unwrap();
    let second = other.path().join("blog");
    std::fs::create_dir(&second).unwrap();
    write(&second, "post.md", "x");
    let mut app = open_ws(&ScriptedDialogs::default(), &root, vec![]);
    common::run(
        &mut app,
        "workspace.open",
        json!({ "path": second.to_string_lossy() }),
    );
    assert!(listed(&app, "Open “blog”?"));
    assert!(listed(&app, "Open in This Window"));
    app.harness.key_press(Key::Enter);
    app.harness.run_steps(2);
    common::settle(&mut app);
    assert_eq!(app.harness.state().core.workspace_name(), Some("blog"));
    assert!(listed(&app, "post.md"));
}

#[test]
fn recent_workspaces_listed_on_empty_state() {
    let (_d, root) = folder();
    let app = open_ws(&ScriptedDialogs::default(), &root, vec![]);
    let core = &app.harness.state().core;
    assert_eq!(core.recent_workspaces()[0].path, root.to_string_lossy());
    // The empty editor area lists it.
    assert!(
        app.harness.query_all_by_label("notes").count() >= 2,
        "tree root and recent entry"
    );
}

#[test]
fn tree_keyboard_navigation_and_f2() {
    let (_d, root) = folder();
    write(&root, "a.md", "x");
    write(&root, "b.md", "y");
    let mut app = open_ws(&ScriptedDialogs::default(), &root, vec![]);
    app.harness.get_by_label("a.md").click();
    app.harness.run_steps(3);
    common::settle(&mut app);
    assert!(app.harness.state().core.explorer_focused);
    app.harness.key_press(Key::ArrowDown);
    app.harness.run_steps(2);
    assert_eq!(
        app.harness
            .state()
            .core
            .workspace
            .as_ref()
            .unwrap()
            .tree
            .selected,
        Some(root.join("b.md"))
    );
    app.harness.key_press(Key::F2);
    app.harness.run_steps(3);
    let edit = app
        .harness
        .state()
        .core
        .workspace
        .as_ref()
        .unwrap()
        .edit
        .clone();
    assert!(matches!(edit, Some(e) if e.text == "b.md"));
    app.harness.key_press(Key::Escape);
    app.harness.run_steps(2);
    assert!(
        app.harness
            .state()
            .core
            .workspace
            .as_ref()
            .unwrap()
            .edit
            .is_none()
    );
}
