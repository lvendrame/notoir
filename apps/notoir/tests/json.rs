//! JSON tree, selection sync, errors and formatting (spec 25).

mod common;

use std::path::{Path, PathBuf};

use egui::Key;
use egui_kittest::kittest::{NodeT, Queryable};
use notoir_core::view::ViewKind;
use notoir_editor::{Range, Selection};
use serde_json::Value;

fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, text).unwrap();
    p.canonicalize().unwrap()
}

fn open(path: PathBuf) -> common::TestApp {
    let mut app = common::start(move |o| o.open_paths = vec![(path, None)]);
    common::settle(&mut app);
    app
}

fn text(app: &common::TestApp) -> String {
    let core = &app.harness.state().core;
    core.workbench
        .document(core.workbench.active_document().unwrap())
        .unwrap()
        .buffer
        .text()
}

fn selection(app: &common::TestApp) -> (usize, usize) {
    let core = &app.harness.state().core;
    let r = core
        .workbench
        .view(core.workbench.active_view().unwrap())
        .unwrap()
        .selection
        .primary();
    (r.from(), r.to())
}

fn select(app: &mut common::TestApp, a: usize, b: usize) {
    let core = &mut app.harness.state_mut().core;
    let v = core.workbench.active_view().unwrap();
    core.workbench
        .set_selection(v, Selection::new(vec![Range::new(a, b)], 0))
        .unwrap();
}

fn mode(app: &mut common::TestApp, label: &str) {
    app.harness.get_by_label(label).click();
    app.harness.run_steps(3);
}

// spec 25: "Tree reflects the document"
#[test]
fn tree_reflects_document() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = open(write(
        dir.path(),
        "user.json",
        r#"{"user":{"name":"John","age":42}}"#,
    ));
    mode(&mut app, "Tree");
    let core = &app.harness.state().core;
    assert_eq!(
        core.workbench
            .view(core.workbench.active_view().unwrap())
            .unwrap()
            .kind,
        ViewKind::JsonTree
    );
    assert!(app.harness.query_by_label("user: {}").is_some());
    assert!(app.harness.query_by_label("name: \"John\"").is_some());
    assert!(app.harness.query_by_label("age: 42").is_some());
}

// spec 25: "Selecting a node selects text"
#[test]
fn selecting_node_selects_text() {
    let dir = tempfile::tempdir().unwrap();
    let src = r#"{"user":{"name":"John","age":42}}"#;
    let mut app = open(write(dir.path(), "user.json", src));
    mode(&mut app, "Split");
    app.harness.get_by_label("age: 42").click();
    app.harness.run_steps(3);
    let at = src.find("42").unwrap();
    assert_eq!(selection(&app), (at, at + 2));
}

// spec 25: "Cursor syncs tree selection"
#[test]
fn cursor_syncs_tree_selection() {
    let dir = tempfile::tempdir().unwrap();
    let src = "{\n  \"user\": {\n    \"address\": { \"city\": \"Porto\" }\n  }\n}\n";
    let mut app = open(write(dir.path(), "user.json", src));
    mode(&mut app, "Split");
    let at = src.find("Porto").unwrap() + 2;
    select(&mut app, at, at);
    app.harness.run_steps(3);
    let node = app.harness.get_by_label("city: \"Porto\"");
    assert_eq!(
        node.accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::True)
    );
}

// spec 25: "Invalid JSON keeps last good tree"
#[test]
fn invalid_json_keeps_last_good_tree() {
    let dir = tempfile::tempdir().unwrap();
    let src = "{\n  \"a\": {\n    \"b\": 1\n  }\n}\n";
    let mut app = open(write(dir.path(), "a.json", src));
    mode(&mut app, "Split");
    assert!(app.harness.query_by_label("b: 1").is_some());
    // Delete the closing brace on the last line.
    let close = src.rfind('}').unwrap();
    select(&mut app, close, close + 1);
    app.harness.state_mut().core.editor_focused = true;
    app.harness.key_press(Key::Backspace);
    common::settle(&mut app);
    app.harness.run_steps(3);
    assert!(!text(&app).trim_end().ends_with('}') || text(&app).trim_end().ends_with("  }"));
    // Greyed tree still shown, with a banner naming line and column.
    assert!(app.harness.query_by_label("b: 1").is_some());
    assert!(
        app.harness
            .query_by_label_contains("line 6, column 1")
            .is_some(),
        "error banner"
    );
    // Still editable.
    let len = text(&app).chars().count();
    select(&mut app, len, len);
    app.harness.event(egui::Event::Text("}".into()));
    common::settle(&mut app);
    app.harness.run_steps(3);
    assert!(text(&app).ends_with('}'));
    assert!(
        app.harness
            .query_by_label_contains("line 6, column 1")
            .is_none()
    );
}

// spec 25: "Format preserves key order and is one undo step"
#[test]
fn format_keeps_order_one_undo() {
    let dir = tempfile::tempdir().unwrap();
    let src = r#"{"b":1,"a":[1,2]}"#;
    let mut app = open(write(dir.path(), "m.json", src));
    common::run(&mut app, "json.format", Value::Null);
    let out = text(&app);
    // Indented with the document's indentation (4 spaces by default).
    assert_eq!(
        out,
        "{\n    \"b\": 1,\n    \"a\": [\n        1,\n        2\n    ]\n}\n"
    );
    common::run(&mut app, "edit.undo", Value::Null);
    assert_eq!(text(&app), src);
}

// spec 25: "Format refuses invalid JSON"
#[test]
fn format_refuses_invalid() {
    let dir = tempfile::tempdir().unwrap();
    let src = "{\"a\" 1}";
    let mut app = open(write(dir.path(), "bad.json", src));
    common::run(&mut app, "json.format", Value::Null);
    app.harness.run_steps(2);
    assert_eq!(text(&app), src);
    assert!(
        app.harness
            .query_by_label_contains("line 1, column 6")
            .is_some(),
        "error toast"
    );
    assert_eq!(selection(&app), (5, 5), "caret at the error");
}

// spec 25: "Comments allowed in tsconfig.json"
#[test]
fn tsconfig_comments_allowed() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = open(write(
        dir.path(),
        "tsconfig.json",
        "{\n  // strict mode\n  \"strict\": true,\n}\n",
    ));
    mode(&mut app, "Tree");
    assert!(app.harness.query_by_label("strict: true").is_some());
    assert!(
        app.harness
            .query_by_label_contains("Invalid JSON")
            .is_none()
    );
    let core = &app.harness.state().core;
    let doc = core.workbench.active_document().unwrap();
    assert!(core.json_diagnostics(doc).is_empty());
}
