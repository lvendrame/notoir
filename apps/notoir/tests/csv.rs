//! CSV app behavior against real files and native widgets.
mod common;
use egui_kittest::kittest::Queryable;
use notoir_editor::{ChangeSet, EditKind, Selection};
use serde_json::json;
fn setup(text: &str) -> (tempfile::TempDir, common::TestApp) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("table.csv");
    std::fs::write(&path, text).unwrap();
    let mut app = common::start(|o| o.open_paths = vec![(path, None)]);
    common::settle(&mut app);
    app.harness.get_by_label("Preview").click();
    app.harness.run_steps(3);
    common::settle(&mut app);
    (dir, app)
}
fn action(app: &mut common::TestApp, args: serde_json::Value) {
    common::run(app, "csv.action", args);
    common::settle(app);
}
fn text(app: &common::TestApp) -> String {
    let c = &app.harness.state().core;
    c.workbench
        .document(c.workbench.active_document().unwrap())
        .unwrap()
        .buffer
        .text()
}
// spec 39: "CSV display controls do not change source"
#[test]
fn display_controls() {
    let source = "Name;City\nAlice;Porto\nBob;Lisbon\n";
    let (_dir, mut app) = setup(source);
    let view = app.harness.state().core.workbench.active_view().unwrap();
    assert_eq!(app.harness.state().core.csv.views[&view].detected, ';');
    assert!(app.harness.query_by_label("Alice").is_some());
    app.harness.get_by_label("First row is header").click();
    app.harness.run_steps(3);
    assert!(!app.harness.state().core.csv.views[&view].header);
    assert!(app.harness.query_by_label("Column 1").is_some());
    app.harness.get_by_label("Filter Column 2").click();
    app.harness.run_steps(3);
    app.harness.get_by_label("Hide column").click();
    app.harness.run_steps(3);
    common::settle(&mut app);
    assert!(app.harness.query_by_label("Porto").is_none());
    action(&mut app, json!({"op":"showAll"}));
    assert!(app.harness.query_by_label("Porto").is_some());
    app.harness.get_by_label("Split").click();
    app.harness.run_steps(3);
    assert_eq!(
        app.harness.state().core.workbench.view(view).unwrap().kind,
        notoir_core::view::ViewKind::CsvSplit
    );
    app.harness
        .query_all_by_label("Edit")
        .last()
        .unwrap()
        .click();
    app.harness.run_steps(3);
    assert_eq!(
        app.harness.state().core.workbench.view(view).unwrap().kind,
        notoir_core::view::ViewKind::Text
    );
    assert_eq!(text(&app), source);
}
// spec 39: "Search highlights while column filters narrow rows"
#[test]
fn search_and_filters() {
    let (_dir, mut app) = setup("Name,City\nAlice,Porto\nBob,Lisbon\nAlina,Lisbon\n");
    let view = app.harness.state().core.workbench.active_view().unwrap();
    action(&mut app, json!({"op":"search","value":"ALI"}));
    let s = &app.harness.state().core.csv.views[&view];
    let t = s.parsed.as_ref().unwrap();
    assert_eq!(s.visible_rows(t), vec![1, 2, 3]);
    assert_eq!(s.matches(t, &s.visible_rows(t)), vec![(1, 0), (3, 0)]);
    action(&mut app, json!({"op":"filter","column":1,"value":"LIS"}));
    action(&mut app, json!({"op":"filter","column":0,"value":"ali"}));
    action(&mut app, json!({"op":"visible","column":1,"value":false}));
    let s = &app.harness.state().core.csv.views[&view];
    let t = s.parsed.as_ref().unwrap();
    assert_eq!(s.visible_rows(t), vec![3]);
    assert_eq!(s.matches(t, &s.visible_rows(t)), vec![(3, 0)]);
    assert!(app.harness.query_by_label("Alice").is_none());
    assert!(app.harness.query_by_label("Alina").is_some());
}
// spec 39: "Cell edits preserve CSV source and support undo and save"
#[test]
fn edit_undo_save() {
    let original = "Name,Value,\r\nα,\"old\",\r\nx\r\n";
    let (dir, mut app) = setup(original);
    action(&mut app, json!({"op":"edit","row":1,"column":1}));
    action(&mut app, json!({"op":"commit","value":"a,\"b\nline"}));
    let changed = "Name,Value,\nα,\"a,\"\"b\nline\",\nx\n";
    assert_eq!(text(&app), changed);
    common::run(&mut app, "edit.undo", json!({}));
    common::settle(&mut app);
    assert_eq!(text(&app), original.replace("\r\n", "\n"));
    common::run(&mut app, "edit.redo", json!({}));
    common::settle(&mut app);
    assert_eq!(text(&app), changed);
    action(&mut app, json!({"op":"edit","row":2,"column":2}));
    action(&mut app, json!({"op":"commit","value":"new"}));
    assert!(text(&app).ends_with("x,,new\n"));
    common::run(&mut app, "file.save", json!({}));
    common::settle(&mut app);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("table.csv")).unwrap(),
        text(&app).replace('\n', "\r\n")
    );
}
// spec 39: "Invalid CSV retains the last valid table"
#[test]
fn invalid_and_stale_edit() {
    let (_dir, mut app) = setup("Name,Value\na,b\n");
    let c = &mut app.harness.state_mut().core;
    let view = c.workbench.active_view().unwrap();
    let doc = c.workbench.active_document().unwrap();
    let len = c.workbench.document(doc).unwrap().buffer.len_chars();
    c.workbench
        .edit(
            view,
            ChangeSet::replace(len, len, "\"bad"),
            Selection::caret(len),
            EditKind::Other,
            0,
        )
        .unwrap();
    app.harness.run_steps(3);
    common::settle(&mut app);
    let s = &app.harness.state().core.csv.views[&view];
    assert!(s.error.is_some());
    assert_eq!(s.parsed.as_ref().unwrap().rows[1][0].value, "a");
    let invalid = text(&app);
    action(&mut app, json!({"op":"edit","row":1,"column":1}));
    action(&mut app, json!({"op":"commit","value":"wrong"}));
    assert_eq!(text(&app), invalid);
    common::run(&mut app, "edit.undo", json!({}));
    common::settle(&mut app);
    assert!(app.harness.state().core.csv.views[&view].error.is_none());
}

#[test]
fn filtered_edit_targets_source_and_stale_editor_cannot_overwrite() {
    let (_dir, mut app) = setup("Name,Value\nAlice,first\nBob,second\n");
    action(&mut app, json!({"op":"filter","column":0,"value":"Bob"}));
    action(&mut app, json!({"op":"edit","row":2,"column":1}));
    action(&mut app, json!({"op":"commit","value":"changed"}));
    assert_eq!(text(&app), "Name,Value\nAlice,first\nBob,changed\n");
    action(&mut app, json!({"op":"edit","row":2,"column":1}));
    let core = &mut app.harness.state_mut().core;
    let view = core.workbench.active_view().unwrap();
    core.workbench
        .edit(
            view,
            ChangeSet::replace(0, 0, "New,"),
            Selection::caret(0),
            EditKind::Other,
            10,
        )
        .unwrap();
    core.csv_action(
        &notoir_core::command::CommandInvocation::new(
            "csv.action",
            notoir_core::command::InvocationSource::Internal,
        )
        .with_args(json!({"op":"commit","value":"stale"})),
    );
    assert_eq!(text(&app), "New,Name,Value\nAlice,first\nBob,changed\n");
}
