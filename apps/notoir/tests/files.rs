//! Files, tabs, Scratchpads and recent files (specs 04, 08, 09, 17, 19, 22, 35).

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::{Event, Key, Modifiers};
use egui_kittest::kittest::Queryable;
use notoir::files::ScriptedDialogs;
use notoir_core::ids::ViewId;
use serde_json::json;

fn ctrl() -> Modifiers {
    Modifiers {
        ctrl: true,
        command: true,
        ..Default::default()
    }
}

fn with_dialogs(d: &ScriptedDialogs) -> impl FnOnce(&mut notoir::core::StartOptions) {
    let d = d.clone();
    move |o| o.dialogs = Some(Arc::new(d))
}

fn write(dir: &Path, rel: &str, text: &str) -> PathBuf {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, text).unwrap();
    p
}

/// Opens files through File → Open (scripted dialog).
fn open_files(app: &mut common::TestApp, dialogs: &ScriptedDialogs, files: Vec<PathBuf>) {
    dialogs.answer_open(files);
    common::run(app, "file.open", serde_json::Value::Null);
    common::settle(app);
}

fn active_text(app: &common::TestApp) -> String {
    let core = &app.harness.state().core;
    core.workbench
        .document(core.workbench.active_document().unwrap())
        .unwrap()
        .buffer
        .text()
}

fn type_text(app: &mut common::TestApp, s: &str) {
    app.harness.event(Event::Text(s.into()));
    app.harness.run_steps(2);
}

fn tab_of(app: &common::TestApp, name: &str) -> ViewId {
    let core = &app.harness.state().core;
    core.workbench
        .layout
        .active()
        .tabs
        .iter()
        .copied()
        .find(|v| core.document_title(core.workbench.view(*v).unwrap().document_id) == name)
        .unwrap_or_else(|| panic!("no tab {name}"))
}

fn tab_names(app: &common::TestApp) -> Vec<String> {
    let core = &app.harness.state().core;
    core.workbench
        .layout
        .active()
        .tabs
        .iter()
        .map(|v| core.document_title(core.workbench.view(*v).unwrap().document_id))
        .collect()
}

// spec 08: "Open a single file"
#[test]
fn open_a_single_file_without_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let file = write(dir.path(), "todo.txt", "milk");
    let d = ScriptedDialogs::default();
    let mut app = common::start(with_dialogs(&d));
    open_files(&mut app, &d, vec![file]);
    assert!(app.harness.query_by_label("todo.txt").is_some());
    type_text(&mut app, "!");
    assert_eq!(active_text(&app), "!milk");
    assert!(app.harness.query_by_label("No folder opened").is_some());
    assert!(app.harness.query_by_label("Open Workspace…").is_some());
}

// spec 04: "Dirty indicator"
// spec 19: "Routine save shows no toast"
#[test]
fn dirty_indicator_and_save() {
    let dir = tempfile::tempdir().unwrap();
    let file = write(dir.path(), "notes.md", "a");
    let d = ScriptedDialogs::default();
    let mut app = common::start(with_dialogs(&d));
    open_files(&mut app, &d, vec![file.clone()]);
    type_text(&mut app, "x");
    assert!(app.harness.query_by_label("notes.md ●").is_some());
    app.harness.key_press_modifiers(ctrl(), Key::S);
    common::settle(&mut app);
    assert!(app.harness.query_by_label("notes.md").is_some());
    assert!(app.harness.query_by_label("notes.md ●").is_none());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "xa");
    assert_eq!(app.harness.state().core.notifications.toasts().len(), 0);
}

// spec 04: "Same-named files are disambiguated"
#[test]
fn same_named_files_are_disambiguated() {
    let dir = tempfile::tempdir().unwrap();
    let a = write(dir.path(), "docs/README.md", "1");
    let b = write(dir.path(), "api/README.md", "2");
    let d = ScriptedDialogs::default();
    let mut app = common::start(with_dialogs(&d));
    open_files(&mut app, &d, vec![a, b]);
    assert!(app.harness.query_by_label("README.md — docs").is_some());
    assert!(app.harness.query_by_label("README.md — api").is_some());
}

fn three_tabs() -> (common::TestApp, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let files = vec![
        write(dir.path(), "a.md", "a"),
        write(dir.path(), "b.md", "b"),
        write(dir.path(), "c.md", "c"),
    ];
    let d = ScriptedDialogs::default();
    let mut app = common::start(with_dialogs(&d));
    open_files(&mut app, &d, files);
    (app, dir)
}

// spec 04: "Close Others keeps pinned tabs"
#[test]
fn close_others_keeps_pinned() {
    let (mut app, _dir) = three_tabs();
    let a = tab_of(&app, "a.md");
    let b = tab_of(&app, "b.md");
    common::run(&mut app, "view.togglePin", json!({ "view": a.0 }));
    common::run(&mut app, "file.closeOthers", json!({ "view": b.0 }));
    assert_eq!(tab_names(&app), ["a.md", "b.md"]);
}

// spec 04: "Context menu acts on the clicked tab"
#[test]
fn context_menu_acts_on_clicked_tab() {
    let (mut app, _dir) = three_tabs();
    let a = tab_of(&app, "a.md");
    common::run(&mut app, "view.activate", json!({ "view": a.0 }));
    app.harness.get_by_label("c.md").click_secondary();
    app.harness.run_steps(3);
    app.harness.get_by_label_contains("Close Ctrl+W").click();
    common::settle(&mut app);
    assert_eq!(tab_names(&app), ["a.md", "b.md"]);
    let core = &app.harness.state().core;
    assert_eq!(core.workbench.active_view(), Some(tab_of(&app, "a.md")));
}

// spec 04: "Close Saved Tabs skips dirty tabs"
#[test]
fn close_saved_tabs_skips_dirty() {
    let (mut app, _dir) = three_tabs();
    let a = tab_of(&app, "a.md");
    common::run(&mut app, "view.activate", json!({ "view": a.0 }));
    type_text(&mut app, "!");
    common::run(&mut app, "file.closeSaved", serde_json::Value::Null);
    assert_eq!(tab_names(&app), ["a.md"]);
}

// spec 04: "Reopen restores position"
#[test]
fn reopen_closed_tab_restores_position() {
    let dir = tempfile::tempdir().unwrap();
    let text: String = (1..=200).map(|i| format!("line {i}\n")).collect();
    let file = write(dir.path(), "notes.md", &text);
    let d = ScriptedDialogs::default();
    let mut app = common::start(with_dialogs(&d));
    open_files(&mut app, &d, vec![file]);
    {
        let core = &mut app.harness.state_mut().core;
        let view = core.workbench.active_view().unwrap();
        let pos = core
            .workbench
            .document(core.workbench.active_document().unwrap())
            .unwrap()
            .buffer
            .rope()
            .line_to_char(119);
        core.workbench
            .set_selection(view, notoir_editor::Selection::caret(pos))
            .unwrap();
    }
    app.harness.key_press_modifiers(ctrl(), Key::W);
    app.harness.run_steps(2);
    assert!(tab_names(&app).is_empty());
    app.harness
        .key_press_modifiers(ctrl() | Modifiers::SHIFT, Key::T);
    common::settle(&mut app);
    assert!(app.harness.query_by_label("Ln 120, Col 1").is_some());
}

// spec 08: "New scratchpad numbering"
#[test]
fn scratchpad_numbering_reuses_gaps() {
    let mut app = common::start(|_| {});
    for _ in 0..3 {
        common::run(&mut app, "file.newScratchpad", serde_json::Value::Null);
    }
    let s2 = tab_of(&app, "Scratch 2");
    common::run(&mut app, "file.close", json!({ "view": s2.0 }));
    common::run(&mut app, "file.newScratchpad", serde_json::Value::Null);
    assert!(tab_names(&app).contains(&"Scratch 2".to_owned()));
}

// spec 08: "Scratchpad survives restart without autosave"
#[test]
fn scratchpad_survives_restart() {
    let mut app = common::start(|_| {});
    common::run(&mut app, "file.newScratchpad", serde_json::Value::Null);
    type_text(&mut app, "buy milk");
    assert!(
        app.harness.state_mut().core.request_quit(),
        "no save prompt for Scratchpads"
    );
    let app = common::restart(app, |_| {});
    assert_eq!(tab_names(&app), ["Scratch 1"]);
    assert_eq!(active_text(&app), "buy milk");
}

// spec 08: "Save As converts a Scratchpad into a file"
#[test]
fn save_as_converts_scratchpad() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("idea.md");
    let d = ScriptedDialogs::default();
    let mut app = common::start(with_dialogs(&d));
    common::run(&mut app, "file.newScratchpad", serde_json::Value::Null);
    type_text(&mut app, "idea");
    d.answer_save(Some(target.clone()));
    app.harness.key_press_modifiers(ctrl(), Key::S);
    common::settle(&mut app);
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "idea");
    assert_eq!(tab_names(&app), ["idea.md"]);
    assert!(app.harness.state().core.kept_scratchpads().is_empty());
    let db = app.harness.state().core.state_db.as_ref().unwrap();
    assert!(db.scratchpads().unwrap().is_empty());
}

// spec 08: "Closing a non-empty scratchpad offers to keep it"
#[test]
fn closing_scratchpad_offers_keep() {
    let mut app = common::start(|_| {});
    common::run(&mut app, "file.newScratchpad", serde_json::Value::Null);
    type_text(&mut app, "note");
    app.harness.key_press_modifiers(ctrl(), Key::W);
    app.harness.run_steps(3);
    assert!(
        app.harness
            .query_by_label_contains("Keep “Scratch 1”?")
            .is_some()
    );
    app.harness.get_by_label("Keep Scratchpad").click();
    common::settle(&mut app);
    assert!(tab_names(&app).is_empty());
    let kept = app.harness.state().core.kept_scratchpads();
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].title, "Scratch 1");
}

// spec 08: "Missing recent file"
#[test]
fn missing_recent_file_offers_removal() {
    let dir = tempfile::tempdir().unwrap();
    let gone = dir.path().join("gone.md");
    let mut app = common::start(|_| {});
    app.harness.state_mut().core.record_recent_file(&gone);
    common::run(
        &mut app,
        "file.openRecent",
        json!({ "path": gone.to_string_lossy() }),
    );
    common::settle(&mut app);
    assert!(
        app.harness
            .query_by_label_contains("“gone.md” no longer exists.")
            .is_some()
    );
    app.harness.get_by_label("Remove from Recent").click();
    common::settle(&mut app);
    assert!(app.harness.state().core.files.recent_files.is_empty());
}

// spec 09: "Save refuses to overwrite an externally changed file"
#[test]
fn save_conflict_is_not_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    let file = write(dir.path(), "notes.md", "base");
    let d = ScriptedDialogs::default();
    let mut app = common::start(with_dialogs(&d));
    open_files(&mut app, &d, vec![file.clone()]);
    type_text(&mut app, "mine ");
    std::fs::write(&file, "theirs").unwrap();
    app.harness.key_press_modifiers(ctrl(), Key::S);
    common::settle(&mut app);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "theirs");
    assert!(
        app.harness
            .query_by_label_contains("changed on disk since you opened it")
            .is_some()
    );
    app.harness.key_press(Key::Escape);
    common::settle(&mut app);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "theirs");
}

// spec 09: "Typing during save"
#[test]
fn typing_during_save_stays_dirty() {
    let dir = tempfile::tempdir().unwrap();
    let file = write(dir.path(), "a.md", "");
    let d = ScriptedDialogs::default();
    let mut app = common::start(with_dialogs(&d));
    open_files(&mut app, &d, vec![file]);
    type_text(&mut app, "a");
    app.harness.key_press_modifiers(ctrl(), Key::S);
    app.harness.event(Event::Text("b".into()));
    common::settle(&mut app);
    let core = &app.harness.state().core;
    assert!(
        core.workbench
            .document(core.workbench.active_document().unwrap())
            .unwrap()
            .is_dirty()
    );
}

// spec 09: "Atomic save preserves file on write failure"
// spec 19: "Save failure is sticky with actions"
// spec 35: "Save error reaches the user"
#[cfg(unix)]
#[test]
fn save_failure_keeps_file_and_notifies() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let sub = dir.path().join("locked");
    let file = write(&sub, "notes.md", "original");
    let d = ScriptedDialogs::default();
    let mut app = common::start(with_dialogs(&d));
    open_files(&mut app, &d, vec![file.clone()]);
    type_text(&mut app, "x");
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o444)).unwrap();
    std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o555)).unwrap();
    app.harness.key_press_modifiers(ctrl(), Key::S);
    common::settle(&mut app);
    std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "original");
    let core = &app.harness.state().core;
    assert!(
        core.workbench
            .document(core.workbench.active_document().unwrap())
            .unwrap()
            .is_dirty()
    );
    assert!(
        app.harness
            .query_by_label_contains("Could not save “notes.md”. Permission denied.")
            .is_some()
    );
    for label in ["Save As…", "Retry", "Show Details"] {
        assert!(app.harness.query_by_label(label).is_some(), "{label}");
    }
    app.harness.step();
    assert_eq!(
        app.harness.state().core.notifications.toasts().len(),
        1,
        "errors stay"
    );
}

// spec 22: "Save error is translated"
#[cfg(unix)]
#[test]
fn save_error_is_translated() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let sub = dir.path().join("locked");
    let file = write(&sub, "notes.md", "original");
    let d = ScriptedDialogs::default();
    let dd = d.clone();
    let mut app = common::start(move |o| {
        with_dialogs(&dd)(o);
        o.language_override = Some(notoir::i18n::Language::Es);
    });
    open_files(&mut app, &d, vec![file.clone()]);
    type_text(&mut app, "x");
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o444)).unwrap();
    std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o555)).unwrap();
    app.harness.key_press_modifiers(ctrl(), Key::S);
    common::settle(&mut app);
    std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(
        app.harness
            .query_by_label_contains("No se pudo guardar «notes.md». Permiso denegado.")
            .is_some()
    );
    let n = app.harness.state().core.notifications.toasts()[0].clone();
    assert!(
        n.detail.unwrap().contains("permission"),
        "raw OS detail stays untranslated"
    );
}

// spec 17: "Type to create a scratchpad"
#[test]
fn typing_on_empty_state_creates_scratchpad() {
    let mut app = common::start(|_| {});
    type_text(&mut app, "hello");
    app.harness.run_steps(2);
    assert_eq!(tab_names(&app), ["Scratch 1"]);
    assert_eq!(active_text(&app), "hello");
}

// spec 19: "Changing line endings from the status bar"
#[test]
fn line_endings_change_from_status_bar() {
    let dir = tempfile::tempdir().unwrap();
    let file = write(dir.path(), "a.txt", "one\ntwo\n");
    let d = ScriptedDialogs::default();
    let mut app = common::start(with_dialogs(&d));
    open_files(&mut app, &d, vec![file.clone()]);
    app.harness.get_by_label("LF").click();
    app.harness.run_steps(3);
    app.harness.get_by_label_contains("CRLF").click();
    app.harness.run_steps(3);
    assert!(app.harness.query_by_label("CRLF").is_some());
    let core = &app.harness.state().core;
    assert!(
        core.workbench
            .document(core.workbench.active_document().unwrap())
            .unwrap()
            .is_dirty()
    );
    app.harness.key_press_modifiers(ctrl(), Key::S);
    common::settle(&mut app);
    assert_eq!(std::fs::read(&file).unwrap(), b"one\r\ntwo\r\n");
}

#[test]
fn quitting_with_unsaved_changes_prompts_without_hot_exit() {
    let mut app = common::start(|_| {});
    app.harness.state_mut().core.set_user_setting(
        "app.hotExit",
        Some(notoir_core::settings::SettingValue::Bool(false)),
    );
    common::run(&mut app, "file.newUntitled", serde_json::Value::Null);
    type_text(&mut app, "draft");
    assert!(!app.harness.state_mut().core.request_quit());
    app.harness.run_steps(2);
    assert!(
        app.harness
            .query_by_label_contains("Do you want to save the changes you made to “Untitled-1”?")
            .is_some()
    );
    app.harness.get_by_label("Don't Save").click();
    app.harness.run_steps(2);
    assert!(app.harness.state_mut().core.request_quit());
}

#[test]
fn command_line_goto_places_caret() {
    let dir = tempfile::tempdir().unwrap();
    let text: String = (1..=50).map(|i| format!("row {i}\n")).collect();
    let file = write(dir.path(), "log.txt", &text);
    let mut app = common::start(move |o| o.open_paths = vec![(file, Some((30, 3)))]);
    common::settle(&mut app);
    assert!(app.harness.query_by_label("Ln 30, Col 3").is_some());
}

#[test]
fn revert_reloads_from_disk_and_is_undoable() {
    let dir = tempfile::tempdir().unwrap();
    let file = write(dir.path(), "a.md", "disk");
    let d = ScriptedDialogs::default();
    let mut app = common::start(with_dialogs(&d));
    open_files(&mut app, &d, vec![file]);
    type_text(&mut app, "edit ");
    common::run(&mut app, "file.revert", serde_json::Value::Null);
    app.harness.get_by_label("Revert").click();
    common::settle(&mut app);
    assert_eq!(active_text(&app), "disk");
    let core = &app.harness.state().core;
    assert!(
        !core
            .workbench
            .document(core.workbench.active_document().unwrap())
            .unwrap()
            .is_dirty()
    );
    app.harness.key_press_modifiers(ctrl(), Key::Z);
    app.harness.run_steps(2);
    assert_eq!(active_text(&app), "edit disk");
}

// spec 04: "Tab plus creates a Scratchpad in its group"
#[test]
fn tab_plus_creates_scratchpad_in_clicked_group() {
    let mut app = common::start(|_| {});
    common::run(&mut app, "file.newUntitled", serde_json::Value::Null);
    let first_group = app.harness.state().core.workbench.layout.active_group;
    common::run(&mut app, "view.splitRight", serde_json::Value::Null);
    let second_group = app.harness.state().core.workbench.layout.active_group;
    assert_ne!(first_group, second_group);
    let buttons: Vec<_> = app.harness.get_all_by_label("New Scratchpad").collect();
    assert_eq!(buttons.len(), 2);
    buttons[0].click();
    app.harness.run_steps(4);
    let core = &app.harness.state().core;
    assert_eq!(core.workbench.layout.active_group, first_group);
    let doc = core
        .workbench
        .document(core.workbench.active_document().unwrap())
        .unwrap();
    assert!(matches!(
        doc.origin,
        notoir_core::document::DocumentOrigin::Scratchpad { .. }
    ));
    assert_eq!(
        core.workbench
            .layout
            .group(second_group)
            .unwrap()
            .tabs
            .len(),
        1
    );
    assert!(core.editor_focused);
}

#[test]
fn tab_plus_is_accessible_with_overflow_and_in_empty_group() {
    let mut app = common::start(|_| {});
    // The empty-state action also has this label; the tab-strip button is first.
    app.harness
        .get_all_by_label("New Scratchpad")
        .next()
        .unwrap()
        .click();
    app.harness.run_steps(3);
    for _ in 0..12 {
        common::run(&mut app, "file.newUntitled", serde_json::Value::Null);
    }
    app.harness.set_size(egui::vec2(640.0, 400.0));
    app.harness.run_steps(3);
    let before = app
        .harness
        .state()
        .core
        .workbench
        .layout
        .active()
        .tabs
        .len();
    app.harness.get_by_label("New Scratchpad").click();
    app.harness.run_steps(3);
    let core = &app.harness.state().core;
    assert_eq!(core.workbench.layout.active().tabs.len(), before + 1);
    assert!(matches!(
        core.workbench
            .document(core.workbench.active_document().unwrap())
            .unwrap()
            .origin,
        notoir_core::document::DocumentOrigin::Scratchpad { .. }
    ));
}

fn overflowing_tabs() -> common::TestApp {
    let mut app = common::start(|_| {});
    for _ in 0..16 {
        common::run(&mut app, "file.newUntitled", serde_json::Value::Null);
    }
    app.harness.set_size(egui::vec2(800.0, 500.0));
    app.harness.run_steps(4);
    app
}

fn scroll_titles(app: &mut common::TestApp, delta: egui::Vec2) {
    let left = app.harness.get_by_label("Scroll Tabs Left").rect().left();
    let y = app
        .harness
        .get_by_label("Scroll Tabs Left")
        .rect()
        .center()
        .y;
    app.harness
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(egui::pos2(left - 20.0, y)));
    app.harness.run_steps(2);
    app.harness
        .input_mut()
        .events
        .push(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta,
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
        });
    app.harness.run_steps(30);
}

// spec 04: "Overflow titles scroll without changing selection"
#[test]
fn overflow_titles_scroll_without_changing_selection() {
    let mut app = overflowing_tabs();
    // Focus the editor before exercising title-only controls.
    let y = app.harness.get_by_label("Scroll Tabs Left").rect().bottom() + 100.0;
    app.harness
        .event(egui::Event::PointerMoved(egui::pos2(500.0, y)));
    for pressed in [true, false] {
        app.harness.event(egui::Event::PointerButton {
            pos: egui::pos2(500.0, y),
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        app.harness.run_steps(2);
    }
    assert!(app.harness.state().core.editor_focused);
    let selected = app.harness.state().core.workbench.layout.active().active;
    assert!(app.harness.query_by_label("Untitled-16").is_some());
    let last_before = app.harness.get_by_label("Untitled-16").rect();
    assert!(
        last_before.right() <= app.harness.get_by_label("Scroll Tabs Left").rect().left() + 1.0
    );
    app.harness.get_by_label("Scroll Tabs Left").click();
    app.harness.run_steps(4);
    assert!(app.harness.state().core.editor_focused, "after first arrow");
    assert!(app.harness.get_by_label("Untitled-16").rect().left() > last_before.left());
    let manually_scrolled = app.harness.get_by_label("Untitled-16").rect();
    // Manual scrolling must survive further frames without snapping back.
    app.harness.run_steps(6);
    assert_eq!(
        app.harness.get_by_label("Untitled-16").rect(),
        manually_scrolled
    );
    assert_eq!(
        app.harness.state().core.workbench.layout.active().active,
        selected
    );
    for _ in 0..12 {
        app.harness.get_by_label("Scroll Tabs Left").click();
        app.harness.run_steps(3);
    }
    assert!(
        app.harness.state().core.editor_focused,
        "after repeated left"
    );
    let first = app.harness.get_by_label("Untitled-1").rect();
    assert!(first.left() >= 0.0);
    scroll_titles(&mut app, egui::vec2(0.0, -160.0));
    assert!(app.harness.state().core.editor_focused, "after wheel");
    let first_after = app.harness.get_by_label("Untitled-1").rect();
    assert!(
        first_after.left() < first.left(),
        "{first:?} -> {first_after:?}"
    );
    scroll_titles(&mut app, egui::vec2(-160.0, 0.0));
    assert!(app.harness.get_by_label("Untitled-1").rect().left() < first_after.left());
    for _ in 0..12 {
        app.harness.get_by_label("Scroll Tabs Right").click();
        app.harness.run_steps(3);
    }
    let last_after = app.harness.get_by_label("Untitled-16").rect();
    assert!((last_before.left() - last_after.left()).abs() < 1.0);
    assert_eq!(
        app.harness.state().core.workbench.layout.active().active,
        selected
    );
    assert!(app.harness.state().core.editor_focused);
}

// spec 04: "Selected tab is revealed in an overflowing strip"
#[test]
fn selected_tab_is_revealed_in_an_overflowing_strip() {
    let mut app = overflowing_tabs();
    let tabs = app
        .harness
        .state()
        .core
        .workbench
        .layout
        .active()
        .tabs
        .clone();
    common::run(&mut app, "view.activate", json!({"view": tabs[0].0}));
    let first = app.harness.get_by_label("Untitled-1").rect();
    assert!(first.left() >= 0.0);
    app.harness.get_by_label("Show All Tabs").click();
    app.harness.run_steps(3);
    app.harness
        .get_all_by_label("Untitled-16")
        .last()
        .unwrap()
        .click();
    app.harness.run_steps(4);
    assert_eq!(
        app.harness.state().core.workbench.layout.active().active,
        Some(tabs[15])
    );
    app.harness.key_press_modifiers(ctrl(), Key::PageDown);
    app.harness.run_steps(4);
    assert_eq!(
        app.harness.state().core.workbench.layout.active().active,
        Some(tabs[0])
    );
    assert!(app.harness.get_by_label("Untitled-1").rect().left() >= first.left() - 1.0);
    common::run(&mut app, "view.activate", json!({"view": tabs[15].0}));
    let last = app.harness.get_by_label("Untitled-16").rect();
    assert!(last.right() <= app.harness.get_by_label("Scroll Tabs Left").rect().left() + 1.0);
    app.harness.set_size(egui::vec2(640.0, 400.0));
    app.harness.run_steps(4);
    let last = app.harness.get_by_label("Untitled-16").rect();
    assert!(last.right() <= app.harness.get_by_label("Scroll Tabs Left").rect().left() + 1.0);
    common::run(&mut app, "file.close", serde_json::Value::Null);
    let core = &app.harness.state().core;
    let replacement_label = core.document_title(core.workbench.active_document().unwrap());
    let replacement = app.harness.get_by_label(&replacement_label).rect();
    assert!(
        replacement.right() <= app.harness.get_by_label("Scroll Tabs Left").rect().left() + 1.0
    );
}

#[test]
fn overflow_scroll_is_independent_between_groups() {
    let mut app = overflowing_tabs();
    let left_group = app.harness.state().core.workbench.layout.active_group;
    common::run(&mut app, "view.splitRight", serde_json::Value::Null);
    for _ in 0..8 {
        common::run(&mut app, "file.newUntitled", serde_json::Value::Null);
    }
    app.harness.set_size(egui::vec2(1280.0, 600.0));
    app.harness.run_steps(4);
    for pressed in [true, false] {
        app.harness.event(egui::Event::PointerButton {
            pos: egui::pos2(900.0, 250.0),
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        app.harness.run_steps(2);
    }
    assert!(app.harness.state().core.editor_focused);
    let left_before = app.harness.get_by_label("Untitled-15").rect();
    let right_before = app.harness.get_by_label("Untitled-24").rect();
    app.harness
        .get_all_by_label("Scroll Tabs Left")
        .nth(1)
        .unwrap()
        .click();
    app.harness.run_steps(4);
    assert_eq!(app.harness.get_by_label("Untitled-15").rect(), left_before);
    assert!(app.harness.get_by_label("Untitled-24").rect().left() > right_before.left());
    let right_after = app.harness.get_by_label("Untitled-24").rect();
    common::run(
        &mut app,
        "workbench.focusGroup",
        json!({"group":left_group.0}),
    );
    app.harness
        .get_all_by_label("Scroll Tabs Left")
        .next()
        .unwrap()
        .click();
    app.harness.run_steps(4);
    assert_eq!(app.harness.get_by_label("Untitled-24").rect(), right_after);
    assert!(app.harness.state().core.editor_focused);
    assert!(app.harness.get_by_label("Untitled-15").rect().left() > left_before.left());
}

#[test]
fn oversized_tab_aligns_its_leading_edge_and_controls_stay_accessible() {
    let dir = tempfile::tempdir().unwrap();
    let name = format!("{}.txt", "long-name-".repeat(20));
    let path = dir.path().join(&name);
    std::fs::write(&path, "").unwrap();
    let mut app = common::start(move |o| o.open_paths = vec![(path, None)]);
    common::settle(&mut app);
    let view = app
        .harness
        .state()
        .core
        .workbench
        .layout
        .active()
        .active
        .unwrap();
    common::run(&mut app, "file.newUntitled", serde_json::Value::Null);
    app.harness.set_size(egui::vec2(640.0, 400.0));
    app.harness.run_steps(4);
    common::run(&mut app, "view.activate", json!({"view": view.0}));
    let title = app.harness.get_by_label(&name).rect();
    let right = app.harness.get_by_label("Scroll Tabs Left").rect().left();
    assert!(title.left() >= 0.0 && title.left() < right);
    assert!(title.right() > right);
    app.harness.get_by_label("New Scratchpad").click();
    app.harness.run_steps(4);
    assert_eq!(
        app.harness
            .state()
            .core
            .workbench
            .layout
            .active()
            .tabs
            .len(),
        3
    );
    assert!(app.harness.query_by_label("Scratch 1").is_some());
}

#[test]
fn reselecting_active_tab_from_overflow_reveals_it() {
    let mut app = overflowing_tabs();
    let initial = app.harness.get_by_label("Untitled-16").rect();
    for _ in 0..12 {
        app.harness.get_by_label("Scroll Tabs Left").click();
        app.harness.run_steps(3);
    }
    assert!(app.harness.get_by_label("Untitled-16").rect().left() > initial.left());
    app.harness.get_by_label("Show All Tabs").click();
    app.harness.run_steps(3);
    app.harness
        .get_all_by_label("Untitled-16")
        .last()
        .unwrap()
        .click();
    app.harness.run_steps(4);
    let revealed = app.harness.get_by_label("Untitled-16").rect();
    assert!((revealed.left() - initial.left()).abs() < 1.0);
}
