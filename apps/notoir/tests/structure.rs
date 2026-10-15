//! Folding, bookmarks, Outline, Go to Symbol and breadcrumbs (specs 12, 26).

mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui::{Event, Key};
use egui_kittest::kittest::Queryable;
use notoir_editor::{Range, Selection};
use serde_json::{Value, json};

fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
    let p = dir.join(name);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&p, text).unwrap();
    p.canonicalize().unwrap()
}

fn open(paths: Vec<PathBuf>) -> common::TestApp {
    let mut app =
        common::start(move |o| o.open_paths = paths.into_iter().map(|p| (p, None)).collect());
    common::settle(&mut app);
    app
}

fn view(app: &common::TestApp) -> notoir_core::ids::ViewId {
    app.harness.state().core.workbench.active_view().unwrap()
}

fn caret_line(app: &common::TestApp) -> usize {
    let core = &app.harness.state().core;
    let v = core.workbench.view(view(app)).unwrap();
    core.workbench
        .document(v.document_id)
        .unwrap()
        .buffer
        .rope()
        .char_to_line(v.selection.primary().head)
}

fn set_caret_line(app: &mut common::TestApp, line: usize) {
    let v = view(app);
    let core = &mut app.harness.state_mut().core;
    let doc = core.workbench.view(v).unwrap().document_id;
    let pos = core
        .workbench
        .document(doc)
        .unwrap()
        .buffer
        .rope()
        .line_to_char(line);
    core.workbench
        .set_selection(v, Selection::caret(pos))
        .unwrap();
}

fn bookmarks(app: &common::TestApp) -> Vec<usize> {
    let core = &app.harness.state().core;
    core.workbench
        .document(core.workbench.active_document().unwrap())
        .unwrap()
        .bookmark_lines()
}

fn hidden(app: &common::TestApp) -> Vec<(usize, usize)> {
    let core = &app.harness.state().core;
    let v = core.workbench.view(view(app)).unwrap();
    v.hidden_lines(
        core.workbench
            .document(v.document_id)
            .unwrap()
            .buffer
            .rope(),
    )
}

fn lines(n: usize) -> String {
    (1..=n).map(|i| format!("line {i}\n")).collect()
}

fn type_text(app: &mut common::TestApp, s: &str) {
    for c in s.chars() {
        app.harness.event(Event::Text(c.to_string()));
        app.harness.run_steps(1);
    }
}

// spec 12: "Bookmarks move with edits"
#[test]
fn bookmarks_move_with_edits() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = open(vec![write(dir.path(), "a.txt", &lines(30))]);
    common::run(&mut app, "bookmarks.toggle", json!({ "line": 9 }));
    assert_eq!(bookmarks(&app), [9]);
    set_caret_line(&mut app, 2);
    app.harness.state_mut().core.editor_focused = true;
    app.harness.event(Event::Text("a\nb\nc\nd\ne\n".into()));
    common::settle(&mut app);
    assert_eq!(bookmarks(&app), [14], "line 10 became line 15");
    // Deleting the whole bookmarked line removes the bookmark.
    set_caret_line(&mut app, 14);
    common::run(&mut app, "edit.deleteLine", Value::Null);
    assert!(bookmarks(&app).is_empty());
}

// spec 12: "Next bookmark wraps"
#[test]
fn next_bookmark_wraps() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = open(vec![write(dir.path(), "a.txt", &lines(60))]);
    common::run(&mut app, "bookmarks.toggle", json!({ "line": 4 }));
    common::run(&mut app, "bookmarks.toggle", json!({ "line": 39 }));
    set_caret_line(&mut app, 40);
    common::run(&mut app, "bookmarks.next", Value::Null);
    assert_eq!(caret_line(&app), 4);
    common::run(&mut app, "bookmarks.previous", Value::Null);
    assert_eq!(caret_line(&app), 39);
}

// spec 12: "Bookmarks persist"
#[test]
fn bookmarks_persist() {
    let dir = tempfile::tempdir().unwrap();
    let p = write(dir.path(), "notes.md", &lines(20));
    let mut app = open(vec![p.clone()]);
    common::run(&mut app, "bookmarks.toggle", json!({ "line": 11 }));
    let app = common::restart(app, move |o| {
        o.open_paths = vec![(p, None)];
        o.restore_session = false;
    });
    let mut app = app;
    common::settle(&mut app);
    assert_eq!(bookmarks(&app), [11], "line 12 is bookmarked");
}

// spec 12: "Fold a Markdown section"
#[test]
fn fold_markdown_section() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = open(vec![write(
        dir.path(),
        "a.md",
        "# Architecture\ntext\n## Editor\nmore\n# Roadmap\n",
    )]);
    common::run(&mut app, "fold.fold", json!({ "line": 0 }));
    app.harness.run_steps(2);
    assert_eq!(hidden(&app), [(1, 3)], "lines 2–4 hidden");
    let v = view(&app);
    // Rows: line 1, line 5 and the empty last line.
    assert_eq!(app.harness.state_mut().ui.editors.total_rows(v), Some(3));
    common::run(&mut app, "fold.unfold", json!({ "line": 0 }));
    app.harness.run_steps(2);
    assert!(hidden(&app).is_empty());
}

// spec 12: "Fold level 1 in JSON"
#[test]
fn fold_level_1_json() {
    let dir = tempfile::tempdir().unwrap();
    let src = "{\n  \"a\": {\n    \"x\": {\n      \"y\": 1\n    }\n  },\n  \"b\": [\n    1\n  ],\n  \"c\": 3\n}\n";
    let mut app = open(vec![write(dir.path(), "a.json", src)]);
    set_caret_line(&mut app, 0);
    common::run(&mut app, "fold.foldLevel1", Value::Null);
    app.harness.run_steps(2);
    assert_eq!(
        hidden(&app),
        [(2, 4), (7, 7)],
        "\"a\" and \"b\" values folded"
    );
}

// spec 12: "Navigating into a fold unfolds it"
#[test]
fn navigating_into_fold_unfolds() {
    let dir = tempfile::tempdir().unwrap();
    let mut src = String::from("fn main() {\n");
    for i in 0..30 {
        src.push_str(&format!("    let x{i} = {i};\n"));
    }
    src.push_str("}\n");
    let mut app = open(vec![write(dir.path(), "a.rs", &src)]);
    // Fold everything inside main: lines 2..=31 hidden.
    common::run(&mut app, "fold.fold", json!({ "line": 0 }));
    app.harness.run_steps(2);
    assert_eq!(hidden(&app), [(1, 30)]);
    common::run(&mut app, "navigate.goToLine", Value::Null);
    type_text(&mut app, "15");
    app.harness.key_press(Key::Enter);
    app.harness.run_steps(3);
    assert_eq!(caret_line(&app), 14);
    assert!(hidden(&app).is_empty(), "region unfolded");
}

// spec 26: "Markdown outline"
#[test]
fn markdown_outline() {
    let dir = tempfile::tempdir().unwrap();
    let src = "# Introduction\n\nhi\n\n# Architecture\n\n## Editor\n\n## Git\n\n## MCP\n\nmcp\n\n# Roadmap\n";
    let mut app = open(vec![write(dir.path(), "doc.md", src)]);
    common::run(&mut app, "view.showOutline", Value::Null);
    common::settle(&mut app);
    {
        let core = &mut app.harness.state_mut().core;
        let doc = core.workbench.active_document().unwrap();
        let s = core.structure(doc).unwrap();
        let top: Vec<_> = s.symbols.iter().map(|x| x.name.as_str()).collect();
        assert_eq!(top, ["Introduction", "Architecture", "Roadmap"]);
        let kids: Vec<_> = s.symbols[1]
            .children
            .iter()
            .map(|x| x.name.as_str())
            .collect();
        assert_eq!(kids, ["Editor", "Git", "MCP"]);
    }
    for name in [
        "Introduction",
        "Architecture",
        "Editor",
        "Git",
        "MCP",
        "Roadmap",
    ] {
        // ("Git" is also a menu.)
        let menus = usize::from(name == "Git");
        assert!(
            app.harness.query_all_by_label(name).count() > menus,
            "outline row {name}"
        );
    }
    app.harness.get_by_label("MCP").click();
    common::settle(&mut app);
    assert_eq!(caret_line(&app), 10, "jumped to ## MCP");
}

// spec 26: "JSON breadcrumb path"
#[test]
fn json_breadcrumb_path() {
    let dir = tempfile::tempdir().unwrap();
    let src = "{\n  \"editor\": {\n    \"minimap\": {\n      \"enabled\": true\n    }\n  }\n}\n";
    let mut app = open(vec![write(dir.path(), "config.json", src)]);
    let at = src.find("true").unwrap() + 2;
    {
        let v = view(&app);
        let core = &mut app.harness.state_mut().core;
        core.workbench
            .set_selection(v, Selection::single(Range::caret(at)))
            .unwrap();
    }
    common::settle(&mut app);
    let labels: Vec<String> = ["config.json", "editor", "minimap", "enabled"]
        .iter()
        .map(|n| format!("Breadcrumb: {n}"))
        .collect();
    let mut xs = Vec::new();
    for l in &labels {
        let node = app.harness.get_by_label(l);
        xs.push(node.rect().left());
    }
    assert!(xs.windows(2).all(|w| w[0] < w[1]), "in order: {xs:?}");
}

// spec 26: "Folder segment dropdown"
#[test]
fn folder_segment_dropdown() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("notes");
    std::fs::create_dir(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let file = write(&root, "docs/architecture.md", "# A\n");
    write(&root, "docs/roadmap.md", "# R\n");
    let mut app = open(vec![root.clone(), file]);
    app.harness.get_by_label("Breadcrumb: docs").click();
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.harness.query_by_label("roadmap.md").is_none() && Instant::now() < deadline {
        common::settle(&mut app);
        std::thread::sleep(Duration::from_millis(30));
    }
    assert!(
        app.harness.query_by_label("roadmap.md").is_some(),
        "dropdown lists docs"
    );
    app.harness.get_by_label("roadmap.md").click();
    common::settle(&mut app);
    let core = &app.harness.state().core;
    let doc = core
        .workbench
        .document(core.workbench.active_document().unwrap())
        .unwrap();
    assert!(doc.path().unwrap().ends_with("docs/roadmap.md"));
}

#[test]
fn go_to_symbol_previews_and_restores() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = open(vec![write(
        dir.path(),
        "doc.md",
        "# One\n\ntext\n\n# Two\n\n# Three\n",
    )]);
    common::run(&mut app, "navigate.goToSymbol", Value::Null);
    app.harness.run_steps(2);
    type_text(&mut app, "thr");
    app.harness.key_press(Key::Enter);
    common::settle(&mut app);
    assert_eq!(caret_line(&app), 6);
    common::run(&mut app, "navigate.goToSymbol", Value::Null);
    app.harness.run_steps(2);
    app.harness.key_press(Key::ArrowDown);
    app.harness.run_steps(2);
    assert_eq!(caret_line(&app), 4, "preview");
    app.harness.key_press(Key::Escape);
    app.harness.run_steps(2);
    assert_eq!(caret_line(&app), 6, "restored");
}
