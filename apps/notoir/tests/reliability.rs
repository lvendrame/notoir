//! Reliability (specs 09, 10, 23): recovery after crashes, hot exit,
//! autosave, external changes and session restore.

mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui::Event;
use egui_kittest::kittest::Queryable;
use notoir_core::document::DiskState;
use notoir_core::settings::SettingValue;
use serde_json::{Value, json};

fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
    let p = dir.join(name);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&p, text).unwrap();
    p.canonicalize().unwrap()
}

fn files(paths: Vec<PathBuf>) -> impl FnOnce(&mut notoir::core::StartOptions) {
    move |o| o.open_paths = paths.into_iter().map(|p| (p, None)).collect()
}

fn type_text(app: &mut common::TestApp, s: &str) {
    app.harness.event(Event::Text(s.into()));
    app.harness.run_steps(2);
}

fn doc_text(app: &common::TestApp, title: &str) -> Option<String> {
    let core = &app.harness.state().core;
    core.workbench
        .documents
        .values()
        .find(|d| core.document_title(d.id) == title)
        .map(|d| d.buffer.text())
}

fn doc_state(app: &common::TestApp, title: &str) -> (bool, DiskState) {
    let core = &app.harness.state().core;
    let d = core
        .workbench
        .documents
        .values()
        .find(|d| core.document_title(d.id) == title)
        .unwrap();
    (d.is_dirty(), d.disk_state)
}

fn wait_for(app: &mut common::TestApp, secs: u64, f: impl Fn(&common::TestApp) -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        common::settle(app);
        if f(app) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(40));
    }
    false
}

fn set(app: &mut common::TestApp, key: &str, v: SettingValue) {
    app.harness.state_mut().core.set_user_setting(key, Some(v));
    common::settle(app);
}

fn recovery_files(app: &common::TestApp) -> usize {
    let root = &app.harness.state().core.recovery.root;
    walk(root)
        .into_iter()
        .filter(|p| p.extension().is_some_and(|e| e == "txt"))
        .count()
}

fn walk(p: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(p) {
        for e in rd.flatten() {
            let path = e.path();
            if path.is_dir() {
                out.extend(walk(&path));
            } else {
                out.push(path);
            }
        }
    }
    out
}

// spec 09: "Crash with unsaved files"
#[test]
fn crash_with_unsaved_files_offers_recovery() {
    let dir = tempfile::tempdir().unwrap();
    let a = write(dir.path(), "a.md", "original");
    let mut app = common::start(files(vec![a.clone()]));
    common::settle(&mut app);
    type_text(&mut app, "edited ");
    common::run(&mut app, "file.newUntitled", Value::Null);
    type_text(&mut app, "draft text");
    common::run(&mut app, "recovery.flush", Value::Null);
    common::settle(&mut app);
    let mut app = common::crash(app, |_| {});
    common::settle(&mut app);
    assert!(
        app.harness
            .query_by_label("Notoir detected unsaved documents from your previous session.")
            .is_some()
    );
    let listed: Vec<String> = app
        .harness
        .state()
        .core
        .recovery
        .prompt_entries()
        .iter()
        .map(|e| e.meta.as_ref().unwrap().title.clone())
        .collect();
    assert!(
        listed.contains(&"a.md".to_string()) && listed.contains(&"Untitled-1".to_string()),
        "{listed:?}"
    );
    assert!(app.harness.query_by_label("Untitled-1").is_some());
    app.harness.get_by_label("Restore All").click();
    app.harness.run_steps(3);
    common::settle(&mut app);
    assert_eq!(doc_text(&app, "a.md").as_deref(), Some("edited original"));
    assert_eq!(doc_text(&app, "Untitled-1").as_deref(), Some("draft text"));
    assert!(doc_state(&app, "a.md").0 && doc_state(&app, "Untitled-1").0);
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "original");
    assert!(app.harness.query_by_label("Restore All").is_none());
    // The restored documents are protected by the new session.
    assert_eq!(recovery_files(&app), 2);
}

// spec 09: "Crash with scratchpads"
#[test]
fn crash_with_scratchpads() {
    let mut app = common::start(|_| {});
    common::run(&mut app, "file.newScratchpad", Value::Null);
    common::run(&mut app, "file.newScratchpad", Value::Null);
    type_text(&mut app, "phone 555");
    common::run(&mut app, "recovery.flush", Value::Null);
    let mut app = common::crash(app, |_| {});
    common::settle(&mut app);
    assert_eq!(doc_text(&app, "Scratch 2").as_deref(), Some("phone 555"));
}

// spec 09: "Conflicting recovery snapshot"
#[test]
fn conflicting_recovery_snapshot_opens_with_banner() {
    let dir = tempfile::tempdir().unwrap();
    let a = write(dir.path(), "a.md", "v1");
    let mut app = common::start(files(vec![a.clone()]));
    common::settle(&mut app);
    type_text(&mut app, "mine ");
    common::run(&mut app, "recovery.flush", Value::Null);
    let mut app = common::crash(app, |_| {});
    std::fs::write(&a, "v2 from elsewhere").unwrap();
    common::settle(&mut app);
    app.harness.get_by_label("Restore All").click();
    app.harness.run_steps(3);
    common::settle(&mut app);
    assert_eq!(doc_text(&app, "a.md").as_deref(), Some("mine v1"));
    assert_eq!(doc_state(&app, "a.md").1, DiskState::ModifiedExternally);
    assert!(
        app.harness
            .query_by_label("This file changed on disk after your unsaved edits.")
            .is_some()
    );
    assert!(app.harness.query_by_label("Keep Recovered").is_some());
    app.harness.get_by_label("Use Disk Version").click();
    app.harness.run_steps(3);
    common::settle(&mut app);
    assert_eq!(doc_text(&app, "a.md").as_deref(), Some("v2 from elsewhere"));
    assert!(!doc_state(&app, "a.md").0);
}

#[test]
fn keep_recovered_then_save_overwrites_without_conflict() {
    let dir = tempfile::tempdir().unwrap();
    let a = write(dir.path(), "a.md", "v1");
    let mut app = common::start(files(vec![a.clone()]));
    common::settle(&mut app);
    type_text(&mut app, "mine ");
    common::run(&mut app, "recovery.flush", Value::Null);
    let mut app = common::crash(app, |_| {});
    std::fs::write(&a, "v2").unwrap();
    common::settle(&mut app);
    app.harness.get_by_label("Restore All").click();
    app.harness.run_steps(3);
    app.harness.get_by_label("Keep Recovered").click();
    app.harness.run_steps(3);
    common::run(&mut app, "file.save", Value::Null);
    common::settle(&mut app);
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "mine v1");
}

#[test]
fn discard_asks_and_moves_sessions_aside() {
    let mut app = common::start(|_| {});
    common::run(&mut app, "file.newUntitled", Value::Null);
    type_text(&mut app, "x");
    common::run(&mut app, "recovery.flush", Value::Null);
    let mut app = common::crash(app, |_| {});
    common::settle(&mut app);
    app.harness.get_by_label("Discard").click();
    app.harness.run_steps(3);
    assert!(
        app.harness
            .query_by_label("Permanently discard 1 unsaved documents?")
            .is_some()
    );
    app.harness.key_press(egui::Key::Enter);
    app.harness.run_steps(3);
    assert!(app.harness.query_by_label("Restore All").is_none());
    let root = app.harness.state().core.recovery.root.clone();
    assert!(
        walk(&root.join(".discarded"))
            .iter()
            .any(|p| p.ends_with("manifest.json"))
    );
}

// spec 09: "Hot exit"
#[test]
fn hot_exit_restores_silently() {
    let dir = tempfile::tempdir().unwrap();
    let a = write(dir.path(), "a.md", "text");
    let mut app = common::start(files(vec![a.clone()]));
    common::settle(&mut app);
    type_text(&mut app, "unsaved ");
    assert!(
        app.harness.state_mut().core.request_quit(),
        "no save prompt"
    );
    assert!(app.harness.state().core.files.prompts.is_empty());
    let mut app = common::restart(app, |_| {});
    common::settle(&mut app);
    assert_eq!(doc_text(&app, "a.md").as_deref(), Some("unsaved text"));
    assert!(doc_state(&app, "a.md").0);
    assert!(
        app.harness.query_by_label("Restore All").is_none(),
        "no recovery prompt"
    );
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "text");
}

// spec 09: "Saving clears recovery"
#[test]
fn saving_clears_recovery() {
    let dir = tempfile::tempdir().unwrap();
    let a = write(dir.path(), "a.md", "x");
    let mut app = common::start(files(vec![a]));
    common::settle(&mut app);
    type_text(&mut app, "y");
    common::run(&mut app, "recovery.flush", Value::Null);
    common::settle(&mut app);
    assert_eq!(recovery_files(&app), 1);
    common::run(&mut app, "file.save", Value::Null);
    common::settle(&mut app);
    assert_eq!(recovery_files(&app), 0);
}

#[test]
fn snapshots_follow_edits_without_explicit_flush() {
    let mut app = common::start(|_| {});
    common::run(&mut app, "file.newUntitled", Value::Null);
    type_text(&mut app, "draft");
    assert!(
        wait_for(&mut app, 4, |a| recovery_files(a) == 1),
        "snapshot 1.5 s after the last edit"
    );
}

// spec 09: "After delay"
#[test]
fn autosave_after_delay() {
    let dir = tempfile::tempdir().unwrap();
    let a = write(dir.path(), "a.md", "x");
    let mut app = common::start(files(vec![a.clone()]));
    common::settle(&mut app);
    set(
        &mut app,
        "files.autoSave",
        SettingValue::Str("afterDelay".into()),
    );
    set(&mut app, "files.autoSaveDelay", SettingValue::Int(200));
    type_text(&mut app, "typed ");
    assert!(wait_for(&mut app, 3, |_| std::fs::read_to_string(&a)
        .unwrap()
        == "typed x"));
    assert!(!doc_state(&app, "a.md").0);
    assert!(
        app.harness.state().core.notifications.toasts().is_empty(),
        "no notification"
    );
}

// spec 09: "Autosave never saves untitled documents"
#[test]
fn autosave_never_saves_untitled() {
    let mut app = common::start(|_| {});
    set(
        &mut app,
        "files.autoSave",
        SettingValue::Str("afterDelay".into()),
    );
    set(&mut app, "files.autoSaveDelay", SettingValue::Int(100));
    common::run(&mut app, "file.newUntitled", Value::Null);
    type_text(&mut app, "draft");
    std::thread::sleep(Duration::from_millis(300));
    common::settle(&mut app);
    assert!(app.harness.state().core.files.prompts.is_empty());
    assert!(doc_state(&app, "Untitled-1").0, "still unsaved");
}

#[test]
fn autosave_on_focus_change() {
    let dir = tempfile::tempdir().unwrap();
    let a = write(dir.path(), "a.md", "a");
    let b = write(dir.path(), "b.md", "b");
    let mut app = common::start(files(vec![a.clone(), b]));
    common::settle(&mut app);
    set(
        &mut app,
        "files.autoSave",
        SettingValue::Str("onFocusChange".into()),
    );
    type_text(&mut app, "1");
    let a_view = {
        let core = &app.harness.state().core;
        core.workbench.layout.active().tabs[0]
    };
    common::run(&mut app, "view.activate", json!({ "view": a_view.0 }));
    common::settle(&mut app);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("b.md")).unwrap(),
        "1b"
    );
    let _ = a;
}

// spec 10: "Clean file reloads automatically"
#[test]
fn clean_file_reloads_automatically() {
    let dir = tempfile::tempdir().unwrap();
    let f = write(dir.path(), "README.md", "old");
    let mut app = common::start(files(vec![f.clone()]));
    common::settle(&mut app);
    std::thread::sleep(Duration::from_millis(200));
    let start = Instant::now();
    std::fs::write(&f, "new content").unwrap();
    assert!(wait_for(&mut app, 5, |a| doc_text(a, "README.md")
        .as_deref()
        == Some("new content")));
    assert!(
        start.elapsed() < Duration::from_secs(1) + Duration::from_millis(600),
        "{:?}",
        start.elapsed()
    );
    assert!(!doc_state(&app, "README.md").0);
    common::run(&mut app, "edit.undo", Value::Null);
    assert_eq!(doc_text(&app, "README.md").as_deref(), Some("old"));
}

// spec 10: "Dirty file is never silently reloaded"
#[test]
fn dirty_file_is_never_silently_reloaded() {
    let dir = tempfile::tempdir().unwrap();
    let f = write(dir.path(), "README.md", "old");
    let mut app = common::start(files(vec![f.clone()]));
    common::settle(&mut app);
    type_text(&mut app, "mine ");
    std::thread::sleep(Duration::from_millis(200));
    std::fs::write(&f, "theirs").unwrap();
    assert!(wait_for(&mut app, 5, |a| a
        .harness
        .query_by_label("“README.md” was changed outside Notoir.")
        .is_some()));
    assert_eq!(doc_text(&app, "README.md").as_deref(), Some("mine old"));
    assert!(app.harness.query_by_label("Reload").is_some());
    assert!(app.harness.query_by_label("Keep My Version").is_some());
    // Keep My Version: the next save overwrites without a conflict prompt.
    app.harness.get_by_label("Keep My Version").click();
    app.harness.run_steps(2);
    common::run(&mut app, "file.save", Value::Null);
    common::settle(&mut app);
    assert!(app.harness.state().core.files.prompts.is_empty());
    assert_eq!(std::fs::read_to_string(&f).unwrap(), "mine old");
}

#[test]
fn reload_from_banner_confirms_and_is_undoable() {
    let dir = tempfile::tempdir().unwrap();
    let f = write(dir.path(), "README.md", "old");
    let mut app = common::start(files(vec![f.clone()]));
    common::settle(&mut app);
    type_text(&mut app, "mine ");
    std::thread::sleep(Duration::from_millis(200));
    std::fs::write(&f, "theirs").unwrap();
    assert!(wait_for(&mut app, 5, |a| a
        .harness
        .query_by_label("Reload")
        .is_some()));
    app.harness.get_by_label("Reload").click();
    app.harness.run_steps(2);
    app.harness.key_press(egui::Key::Enter);
    app.harness.run_steps(2);
    common::settle(&mut app);
    assert_eq!(doc_text(&app, "README.md").as_deref(), Some("theirs"));
    common::run(&mut app, "edit.undo", Value::Null);
    assert_eq!(doc_text(&app, "README.md").as_deref(), Some("mine old"));
}

// spec 10: "Editors that save via temp file and rename"
#[test]
fn temp_file_and_rename_reloads_once() {
    let dir = tempfile::tempdir().unwrap();
    let f = write(dir.path(), "notes.md", "one");
    let mut app = common::start(files(vec![f.clone()]));
    common::settle(&mut app);
    std::thread::sleep(Duration::from_millis(200));
    let tmp = dir.path().join("notes.md.tmp");
    std::fs::write(&tmp, "two").unwrap();
    std::fs::rename(&tmp, &f).unwrap();
    assert!(wait_for(&mut app, 5, |a| doc_text(a, "notes.md")
        .as_deref()
        == Some("two")));
    std::thread::sleep(Duration::from_millis(700));
    common::settle(&mut app);
    assert_eq!(
        doc_state(&app, "notes.md"),
        (false, DiskState::InSync),
        "not marked deleted"
    );
    let undo_depth = {
        common::run(&mut app, "edit.undo", Value::Null);
        doc_text(&app, "notes.md")
    };
    assert_eq!(undo_depth.as_deref(), Some("one"), "one reload step");
}

// spec 10: "Own saves are ignored"
#[test]
fn own_saves_are_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let f = write(dir.path(), "notes.md", "a");
    let mut app = common::start(files(vec![f]));
    common::settle(&mut app);
    type_text(&mut app, "b");
    common::run(&mut app, "file.save", Value::Null);
    std::thread::sleep(Duration::from_millis(800));
    common::settle(&mut app);
    assert!(app.harness.state().core.banners.is_empty());
    assert_eq!(doc_state(&app, "notes.md"), (false, DiskState::InSync));
    assert_eq!(doc_text(&app, "notes.md").as_deref(), Some("ba"));
}

// spec 10: "Touch without content change"
#[test]
fn touch_without_content_change() {
    let dir = tempfile::tempdir().unwrap();
    let f = write(dir.path(), "notes.md", "same");
    let mut app = common::start(files(vec![f.clone()]));
    common::settle(&mut app);
    type_text(&mut app, "x");
    std::thread::sleep(Duration::from_millis(200));
    std::fs::write(&f, "same").unwrap();
    std::thread::sleep(Duration::from_millis(700));
    common::settle(&mut app);
    assert!(app.harness.state().core.banners.is_empty());
    assert_eq!(doc_text(&app, "notes.md").as_deref(), Some("xsame"));
    let core = &app.harness.state().core;
    let undo_steps = core.workbench.documents.values().next().unwrap().revision;
    assert_eq!(undo_steps, 1, "nothing reloaded");
}

// spec 10: "External deletion keeps content"
#[test]
fn external_deletion_keeps_content_and_save_recreates() {
    let dir = tempfile::tempdir().unwrap();
    let f = write(dir.path(), "draft.md", "keep");
    let mut app = common::start(files(vec![f.clone()]));
    common::settle(&mut app);
    std::thread::sleep(Duration::from_millis(200));
    std::fs::remove_file(&f).unwrap();
    assert!(wait_for(&mut app, 5, |a| a
        .harness
        .query_by_label("“draft.md” was deleted on disk.")
        .is_some()));
    assert_eq!(doc_text(&app, "draft.md").as_deref(), Some("keep"));
    assert_eq!(
        doc_state(&app, "draft.md"),
        (true, DiskState::DeletedExternally)
    );
    assert!(
        app.harness.query_by_label("draft.md ●").is_some(),
        "tab marked"
    );
    app.harness.get_by_label("Save").click();
    app.harness.run_steps(2);
    common::settle(&mut app);
    assert_eq!(std::fs::read_to_string(&f).unwrap(), "keep");
    assert_eq!(doc_state(&app, "draft.md"), (false, DiskState::InSync));
}

fn tab_titles(app: &common::TestApp) -> Vec<Vec<String>> {
    let core = &app.harness.state().core;
    core.workbench
        .layout
        .groups_in_order()
        .into_iter()
        .map(|g| {
            core.workbench
                .layout
                .group(g)
                .unwrap()
                .tabs
                .iter()
                .map(|v| core.document_title(core.workbench.view(*v).unwrap().document_id))
                .collect()
        })
        .collect()
}

fn restored(o: &mut notoir::core::StartOptions) {
    o.restore_session = true;
}

// spec 23: "Layout and cursors restored per workspace"
#[test]
fn layout_and_cursors_restored_per_workspace() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("notes");
    let text: String = (1..=60).map(|i| format!("line {i}\n")).collect();
    let a = write(&root, "a.md", &text);
    let b = write(&root, "b.md", "b");
    let c = write(&root, "docs/c.md", "c");
    let root = root.canonicalize().unwrap();
    let mut app = common::start(files(vec![root.clone(), a.clone(), b, c]));
    common::settle(&mut app);
    common::run(&mut app, "workbench.moveTabToNextGroup", Value::Null);
    // Cursor at line 40 of a.md.
    {
        let core = &mut app.harness.state_mut().core;
        let doc = core
            .workbench
            .documents
            .values()
            .find(|d| d.path() == Some(a.as_path()))
            .unwrap()
            .id;
        let view = core.workbench.views.of_document(doc).next().unwrap().id;
        let pos = core
            .workbench
            .document(doc)
            .unwrap()
            .buffer
            .rope()
            .line_to_char(39);
        core.workbench
            .set_selection(view, notoir_editor::Selection::caret(pos))
            .unwrap();
    }
    app.harness.state_mut().core.toggle_dir(&root.join("docs"));
    common::settle(&mut app);
    let before = tab_titles(&app);
    assert_eq!(before.len(), 2);
    let mut app = common::restart(app, restored);
    common::settle(&mut app);
    assert_eq!(tab_titles(&app), before);
    assert_eq!(app.harness.state().core.workspace_name(), Some("notes"));
    let core = &app.harness.state().core;
    let doc = core
        .workbench
        .documents
        .values()
        .find(|d| d.path() == Some(a.as_path()))
        .unwrap();
    let view = core.workbench.views.of_document(doc.id).next().unwrap();
    assert_eq!(
        doc.buffer
            .rope()
            .char_to_line(view.selection.primary().head),
        39
    );
    assert!(
        core.workspace
            .as_ref()
            .unwrap()
            .tree
            .is_expanded(&root.join("docs")),
        "Explorer folders too"
    );
    assert!(
        !root.join(".notoir").exists(),
        "nothing written into the workspace"
    );
}

// spec 23: "Different workspaces have independent sessions"
#[test]
fn different_workspaces_have_independent_sessions() {
    let d = tempfile::tempdir().unwrap();
    let ra = d.path().join("A");
    let rb = d.path().join("B");
    let x = write(&ra, "x.md", "x");
    let y = write(&rb, "y.md", "y");
    let (ra, rb) = (ra.canonicalize().unwrap(), rb.canonicalize().unwrap());
    let mut app = common::start(files(vec![ra.clone(), x]));
    common::settle(&mut app);
    common::run(&mut app, "workspace.close", Value::Null);
    common::settle(&mut app);
    common::run(
        &mut app,
        "workspace.open",
        json!({ "path": rb.to_string_lossy() }),
    );
    common::settle(&mut app);
    app.harness
        .state_mut()
        .core
        .open_path(&y, Default::default());
    common::settle(&mut app);
    common::run(&mut app, "workspace.close", Value::Null);
    common::settle(&mut app);
    common::run(
        &mut app,
        "workspace.open",
        json!({ "path": ra.to_string_lossy() }),
    );
    common::settle(&mut app);
    assert_eq!(tab_titles(&app), vec![vec!["x.md".to_string()]]);
}

// spec 23: "Missing files are skipped"
#[test]
fn missing_files_are_skipped_with_one_notice() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("ws");
    let a = write(&root, "a.md", "a");
    let gone = write(&root, "gone.md", "g");
    let gone2 = write(&root, "gone2.md", "g");
    let root = root.canonicalize().unwrap();
    let mut app = common::start(files(vec![root, a, gone.clone(), gone2.clone()]));
    common::settle(&mut app);
    let (g1, g2) = (gone.clone(), gone2.clone());
    let mut app = common::restart_with(
        app,
        move || {
            std::fs::remove_file(&g1).unwrap();
            std::fs::remove_file(&g2).unwrap();
        },
        restored,
    );
    common::settle(&mut app);
    assert_eq!(tab_titles(&app), vec![vec!["a.md".to_string()]]);
    let core = &app.harness.state().core;
    let notices: Vec<_> = core
        .notifications
        .history()
        .filter(|n| n.message.key == "notifications:sessionMissing")
        .collect();
    assert_eq!(notices.len(), 1);
    let detail = notices[0].detail.clone().unwrap();
    assert!(detail.contains("gone.md") && detail.contains("gone2.md"));
}
