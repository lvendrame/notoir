//! YAML trees share the JSON UI and undoable editing pipeline (spec 25).

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

// spec 25: "YAML supports split and tree views"
#[test]
fn yaml_supports_split_and_tree_views() {
    let dir = tempfile::tempdir().unwrap();
    for extension in ["yaml", "yml"] {
        let src = "user:\n  name: 'José'\n  age: 42\n";
        let mut app = open(write(dir.path(), &format!("user.{extension}"), src));
        mode(&mut app, "Split");
        assert_eq!(
            app.harness.state().core.workbench.active_editor().unwrap(),
            app.harness.state().core.workbench.active_view().unwrap()
        );
        app.harness.get_by_label("age: 42").click();
        app.harness.run_steps(3);
        let at = src.chars().position(|c| c == '4').unwrap();
        assert_eq!(selection(&app), (at, at + 2));
        select(&mut app, 18, 18);
        app.harness.run_steps(3);
        assert_eq!(
            app.harness
                .get_by_label("name: \"José\"")
                .accesskit_node()
                .toggled(),
            Some(egui::accesskit::Toggled::True)
        );
        mode(&mut app, "Tree");
        let core = &app.harness.state().core;
        assert_eq!(
            core.workbench
                .view(core.workbench.active_view().unwrap())
                .unwrap()
                .kind,
            ViewKind::JsonTree
        );
        assert!(app.harness.query_by_label("age: 42").is_some());
        mode(&mut app, "Text");
        assert_eq!(text(&app), src);
    }
}

fn tree_edit(app: &mut common::TestApp, mut args: Value) {
    args["view"] = app
        .harness
        .state()
        .core
        .workbench
        .active_view()
        .unwrap()
        .0
        .into();
    common::run(app, "yaml.treeEdit", args);
}

// spec 25: "YAML tree edits preserve source and undo"
#[test]
fn yaml_tree_edits_preserve_source_and_undo() {
    let dir = tempfile::tempdir().unwrap();
    let src = "# keep\nname: 'José' # person\ntags: [one, two]\nmeta: {}\n";
    let mut app = open(write(dir.path(), "user.yml", src));
    mode(&mut app, "Split");
    for args in [
        serde_json::json!({"path":[0,0],"op":"value","text":"Ann"}),
        serde_json::json!({"path":[0,0],"op":"key","text":"person"}),
        serde_json::json!({"path":[0,2],"op":"add","kind":"string"}),
        serde_json::json!({"path":[0,1,0],"op":"delete"}),
        serde_json::json!({"path":[0,1,1],"op":"move","delta":-1}),
    ] {
        tree_edit(&mut app, args);
        let edited = text(&app);
        assert_ne!(edited, src);
        assert!(edited.starts_with("# keep\n") && edited.contains("# person"));
        assert!(notoir_editor::yaml::parse(&edited).error.is_none());
        common::run(&mut app, "edit.undo", Value::Null);
        assert_eq!(text(&app), src);
        common::run(&mut app, "edit.redo", Value::Null);
        assert_eq!(text(&app), edited);
        common::run(&mut app, "edit.undo", Value::Null);
    }
}

// spec 25: "Invalid YAML keeps the last valid tree"
#[test]
fn invalid_yaml_keeps_last_valid_tree_and_recovers() {
    let dir = tempfile::tempdir().unwrap();
    let src = "items: [1, 2]\n";
    let mut app = open(write(dir.path(), "list.yaml", src));
    mode(&mut app, "Split");
    let close = src.find(']').unwrap();
    select(&mut app, close, close + 1);
    app.harness.state_mut().core.editor_focused = true;
    app.harness.key_press(Key::Backspace);
    common::settle(&mut app);
    assert!(app.harness.query_by_label("[0]: 1").is_some());
    assert!(
        app.harness
            .query_by_label_contains("Invalid YAML at line")
            .is_some()
    );
    let invalid = text(&app);
    tree_edit(
        &mut app,
        serde_json::json!({"path":[0,0,0],"op":"value","text":"3"}),
    );
    assert_eq!(text(&app), invalid);
    common::run(&mut app, "edit.undo", Value::Null);
    assert_eq!(text(&app), src);
    assert!(
        app.harness
            .query_by_label_contains("Invalid YAML at line")
            .is_none()
    );
}

#[test]
fn yaml_format_and_language_switches_use_correct_parser() {
    let dir = tempfile::tempdir().unwrap();
    let src = "root:\n    value:    'hello' # keep\n";
    let mut app = open(write(dir.path(), "config.yaml", src));
    mode(&mut app, "Split");
    common::run(&mut app, "editor.formatDocument", Value::Null);
    assert!(text(&app).contains("value: 'hello' # keep"));
    common::run(&mut app, "edit.undo", Value::Null);
    assert_eq!(text(&app), src);
    let core = &mut app.harness.state_mut().core;
    let doc = core.workbench.active_document().unwrap();
    core.set_language(doc, notoir_editor::LanguageId("json"));
    let (roots, error, _) = core.json_doc(doc).unwrap();
    assert!(roots.is_empty() && error.is_some());
    core.set_language(doc, notoir_editor::LanguageId("yaml"));
    let (roots, error, _) = core.json_doc(doc).unwrap();
    assert!(!roots.is_empty() && error.is_none());
}

#[test]
fn yaml_read_only_and_restored_view_modes() {
    let dir = tempfile::tempdir().unwrap();
    let src = "name: 'José'\n";
    let path = write(dir.path(), "config.yml", src);
    let mut app = open(path);
    mode(&mut app, "Tree");
    {
        let core = &mut app.harness.state_mut().core;
        let doc = core.workbench.active_document().unwrap();
        core.workbench.document_mut(doc).unwrap().read_only = notoir_core::document::ReadOnly::User;
    }
    tree_edit(
        &mut app,
        serde_json::json!({"path":[0,0],"op":"value","text":"changed"}),
    );
    assert_eq!(text(&app), src);
    let mut app = common::restart(app, |o| o.restore_session = true);
    common::settle(&mut app);
    let core = &app.harness.state().core;
    assert_eq!(
        core.workbench
            .view(core.workbench.active_view().unwrap())
            .unwrap()
            .kind,
        ViewKind::JsonTree
    );
    assert!(app.harness.query_by_label("name: \"José\"").is_some());
}

#[test]
fn yaml_selection_targets_the_clicked_view_of_a_shared_document() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = open(write(dir.path(), "shared.yaml", "a: one\nb: two\n"));
    let first = app.harness.state().core.workbench.active_view().unwrap();
    common::run(&mut app, "view.splitRight", Value::Null);
    let second = app.harness.state().core.workbench.active_view().unwrap();
    mode(&mut app, "Split");
    app.harness.get_by_label("b: \"two\"").click();
    app.harness.run_steps(3);
    let core = &app.harness.state().core;
    assert_eq!(
        core.workbench.view(first).unwrap().selection.primary().head,
        0
    );
    let selected = core.workbench.view(second).unwrap().selection.primary();
    assert_eq!((selected.from(), selected.to()), (10, 13));
}
