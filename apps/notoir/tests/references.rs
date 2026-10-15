//! Editor text references and right-click over a selection (spec 37).

mod common;

use std::path::{Path, PathBuf};

use egui::{Event, Modifiers, PointerButton, Pos2};
use egui_kittest::kittest::{NodeT, Queryable};
use notoir_editor::{Range, Selection};

fn write(dir: &Path, rel: &str, text: &str) -> PathBuf {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, text).unwrap();
    p.canonicalize().unwrap()
}

fn lines(n: usize) -> String {
    (1..=n)
        .map(|i| format!("line number {i:02} with some text\n"))
        .collect()
}

fn open(paths: Vec<PathBuf>) -> common::TestApp {
    let paths = paths.into_iter().map(|p| (p, None)).collect();
    let mut app = common::start(move |o| o.open_paths = paths);
    common::settle(&mut app);
    app
}

fn view(app: &common::TestApp) -> notoir_core::ids::ViewId {
    app.harness.state().core.workbench.active_view().unwrap()
}

/// Selects from (line, col) to (line, col), 1-based lines.
fn select(app: &mut common::TestApp, from: (usize, usize), to: (usize, usize)) {
    let v = view(app);
    let core = &mut app.harness.state_mut().core;
    let rope = core
        .workbench
        .document(core.workbench.view(v).unwrap().document_id)
        .unwrap()
        .buffer
        .rope()
        .clone();
    let a = rope.line_to_char(from.0 - 1) + from.1;
    let b = rope.line_to_char(to.0 - 1) + to.1;
    let _ = core
        .workbench
        .set_selection(v, Selection::single(Range::new(a, b)));
    app.harness.run_steps(2);
}

fn right_click(app: &mut common::TestApp, p: Pos2) {
    let h = &mut app.harness;
    h.hover_at(p);
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(Event::PointerButton {
            pos: p,
            button: PointerButton::Secondary,
            pressed,
            modifiers: Modifiers::NONE,
        });
    }
    h.run_steps(3);
}

/// Runs a command and returns what it put on the clipboard.
fn copied(app: &mut common::TestApp, id: &'static str) -> Option<String> {
    let inv = notoir_core::command::CommandInvocation::new(
        id,
        notoir_core::command::InvocationSource::Menu,
    );
    app.harness.state_mut().core.enqueue(inv);
    app.harness.step();
    app.harness
        .output()
        .platform_output
        .commands
        .iter()
        .find_map(|c| match c {
            egui::OutputCommand::CopyText(t) => Some(t.clone()),
            _ => None,
        })
}

// spec 37: "Right-click preserves the caret and selection"
#[test]
fn right_click_keeps_the_selection() {
    let dir = tempfile::tempdir().unwrap();
    let f = write(dir.path(), "myfile.md", &lines(5));
    let mut app = open(vec![f]);
    select(&mut app, (2, 0), (2, 25));
    let v = view(&app);
    let before = app
        .harness
        .state()
        .core
        .workbench
        .view(v)
        .unwrap()
        .selection
        .clone();
    assert_eq!(
        before.primary().head,
        before.primary().to(),
        "caret at one end"
    );
    let (y, row_h) = app
        .harness
        .state_mut()
        .ui
        .editors
        .line_y(v, 1, false)
        .unwrap();
    let (_, gutter) = app.harness.state().ui.editors.geometry(v).unwrap();
    let p = Pos2::new(gutter.right() + 40.0, y + row_h / 2.0);
    right_click(&mut app, p);
    assert!(
        app.harness
            .query_by_label_contains("Copy Full Reference")
            .is_some(),
        "context menu open"
    );
    let after = app
        .harness
        .state()
        .core
        .workbench
        .view(v)
        .unwrap()
        .selection
        .clone();
    assert_eq!(after, before);
}

// spec 37: "Copy full reference for one selected line"
#[test]
fn copy_full_reference_for_one_line() {
    let dir = tempfile::tempdir().unwrap();
    let f = write(dir.path(), "myfile.md", &lines(60));
    let mut app = open(vec![f.clone()]);
    select(&mut app, (45, 3), (45, 9));
    assert_eq!(
        copied(&mut app, "editor.copyFullReference").as_deref(),
        Some(format!("{}:45", f.display()).as_str())
    );
}

// spec 37: "Copy relative reference for selected lines"
#[test]
fn copy_relative_reference_for_lines() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("ws");
    let f = write(&root, "notes/myfile.md", &lines(60));
    let root = root.canonicalize().unwrap();
    let mut app = open(vec![root, f]);
    select(&mut app, (45, 0), (56, 4));
    assert_eq!(
        copied(&mut app, "editor.copyRelativeReference").as_deref(),
        Some("notes/myfile.md:45-56")
    );
    // A selection ending at the start of line 57 stops at line 56.
    select(&mut app, (45, 0), (57, 0));
    assert_eq!(
        copied(&mut app, "editor.copyRelativeReference").as_deref(),
        Some("notes/myfile.md:45-56")
    );
}

// spec 37: "Relative reference is unavailable outside the workspace"
#[test]
fn relative_reference_disabled_outside_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("ws");
    std::fs::create_dir_all(&root).unwrap();
    let outside = write(dir.path(), "outside.md", &lines(3));
    let root = root.canonicalize().unwrap();
    let mut app = open(vec![root, outside]);
    select(&mut app, (1, 0), (1, 4));
    let v = view(&app);
    app.harness.state_mut().ui.focus_editor = Some(v);
    app.harness.run_steps(2);
    let (y, row_h) = app
        .harness
        .state_mut()
        .ui
        .editors
        .line_y(v, 0, false)
        .unwrap();
    let (_, gutter) = app.harness.state().ui.editors.geometry(v).unwrap();
    let p = Pos2::new(gutter.right() + 15.0, y + row_h / 2.0);
    right_click(&mut app, p);
    let full = app.harness.get_by_label_contains("Copy Full Reference");
    assert!(!full.accesskit_node().is_disabled(), "full available");
    let rel = app.harness.get_by_label_contains("Copy Relative Reference");
    assert!(rel.accesskit_node().is_disabled(), "relative disabled");
}

#[test]
fn references_unavailable_without_a_path() {
    let mut app = common::start(|_| {});
    common::run(&mut app, "file.newUntitled", serde_json::Value::Null);
    assert_eq!(copied(&mut app, "editor.copyFullReference"), None);
    let ctx = app.harness.state().core.context(false);
    let spec = app
        .harness
        .state()
        .core
        .commands
        .get(&"editor.copyFullReference".into())
        .unwrap();
    assert!(!spec.enablement.eval(&ctx));
}
