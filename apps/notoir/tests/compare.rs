//! Compare (spec 27): sources, diff tabs, navigation, whitespace, hunk copy
//! and live updates.

mod common;

use std::path::{Path, PathBuf};

use egui::{Key, Modifiers};
use egui_kittest::kittest::Queryable;
use notoir_core::ids::{DocumentId, ViewId};
use notoir_editor::diff::Side;
use serde_json::{Value, json};

fn ctrl() -> Modifiers {
    Modifiers {
        ctrl: true,
        command: true,
        ..Default::default()
    }
}

fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, text).unwrap();
    p.canonicalize().unwrap()
}

fn open(files: Vec<PathBuf>) -> common::TestApp {
    let paths = files.into_iter().map(|f| (f, None)).collect();
    let mut app = common::start(move |o| o.open_paths = paths);
    common::settle(&mut app);
    app
}

fn tab_of(app: &common::TestApp, name: &str) -> ViewId {
    let core = &app.harness.state().core;
    core.workbench
        .views
        .iter()
        .find(|v| core.document_title(v.document_id) == name)
        .map(|v| v.id)
        .expect("tab")
}

fn doc_of(app: &common::TestApp, name: &str) -> DocumentId {
    let core = &app.harness.state().core;
    core.workbench.view(tab_of(app, name)).unwrap().document_id
}

fn diff_tab(app: &common::TestApp) -> ViewId {
    *app.harness
        .state()
        .core
        .compare
        .sessions
        .keys()
        .next()
        .expect("diff tab")
}

fn hunks(app: &common::TestApp) -> usize {
    let tab = diff_tab(app);
    app.harness.state().core.compare.sessions[&tab]
        .diff
        .hunks
        .len()
}

fn text(app: &common::TestApp, doc: DocumentId) -> String {
    app.harness
        .state()
        .core
        .workbench
        .document(doc)
        .unwrap()
        .buffer
        .text()
}

/// Replaces a document's text as a user edit.
fn set_text(app: &mut common::TestApp, doc: DocumentId, s: &str) {
    let core = &mut app.harness.state_mut().core;
    let len = core.workbench.document(doc).unwrap().buffer.len_chars();
    core.workbench
        .apply_edit(
            doc,
            None,
            notoir_editor::ChangeSet::replace(0, len, s),
            notoir_editor::Selection::caret(0),
            notoir_editor::EditKind::Other,
            notoir_core::events::ChangeOrigin::Reload,
            0,
        )
        .unwrap();
    app.harness.run_steps(3);
}

// spec 27: "Compare two open tabs"
#[test]
fn compare_two_open_tabs() {
    let dir = tempfile::tempdir().unwrap();
    let old = write(dir.path(), "old.md", "# Title\nfirst\nsame\n");
    let new = write(dir.path(), "new.md", "# Title\nchanged\nsame\nadded\n");
    let mut app = open(vec![old, new]);
    let (old_tab, new_tab) = (tab_of(&app, "old.md"), tab_of(&app, "new.md"));
    common::run(&mut app, "compare.select", json!({ "view": old_tab.0 }));
    common::run(
        &mut app,
        "compare.withSelected",
        json!({ "view": new_tab.0 }),
    );
    common::settle(&mut app);
    assert!(
        app.harness
            .query_by_label("old.md \u{2194} new.md")
            .is_some(),
        "diff tab titled old.md ↔ new.md"
    );
    let tab = diff_tab(&app);
    assert_eq!(app.harness.state().core.workbench.active_view(), Some(tab));
    assert_eq!(hunks(&app), 2, "differences found");
    assert!(app.harness.query_by_label("2 changes").is_some());
    // Highlighted: both panes carry the changed lines.
    let core = &app.harness.state().core;
    let s = &core.compare.sessions[&tab];
    let (l, r) = s
        .decor
        .as_ref()
        .map(|d| (d.1.clone(), d.2.clone()))
        .unwrap();
    assert!(l.changed(1) && !l.inserted);
    assert!(r.changed(1) && r.changed(3) && r.inserted);
    assert!(!r.changed(2));
    // Selection is consumed; the tab menu no longer offers it.
    assert!(core.compare.selected.is_none());
}

// spec 27: "Compare current changes"
#[test]
fn compare_current_changes_shows_saved_version_read_only() {
    let dir = tempfile::tempdir().unwrap();
    let notes = write(dir.path(), "notes.md", "alpha\nbeta\ngamma\n");
    let mut app = open(vec![notes]);
    let doc = doc_of(&app, "notes.md");
    set_text(&mut app, doc, "alpha\nBETA\ngamma\n");
    common::run(&mut app, "compare.currentChanges", Value::Null);
    common::settle(&mut app);
    let tab = diff_tab(&app);
    let core = &app.harness.state().core;
    let s = &core.compare.sessions[&tab];
    let left = core.workbench.document(s.left.doc).unwrap();
    let right = core.workbench.document(s.right.doc).unwrap();
    assert_eq!(left.buffer.text(), "alpha\nbeta\ngamma\n", "saved version");
    assert!(left.is_read_only(), "left is read-only");
    assert_eq!(s.right.doc, doc, "right is the live buffer");
    assert!(!right.is_read_only(), "right is editable");
    assert_eq!(right.buffer.text(), "alpha\nBETA\ngamma\n");
    assert!(
        app.harness
            .query_by_label("notes.md (Saved) (read-only)")
            .is_some(),
        "left title with the lock"
    );
    assert_eq!(hunks(&app), 1);
    // Typing in the right pane edits the document.
    assert_eq!(
        app.harness.state().core.workbench.active_editor(),
        Some(app.harness.state().core.compare.sessions[&tab].right.view)
    );
}

// spec 27: "Next change navigation"
#[test]
fn next_change_reveals_each_hunk_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let a = write(dir.path(), "a.txt", "1\n2\n3\n4\n5\n6\n7\n8\n");
    let b = write(dir.path(), "b.txt", "1\nX\n3\n4\nY\n6\n7\nZ\n");
    let mut app = open(vec![a, b]);
    let (ta, tb) = (tab_of(&app, "a.txt"), tab_of(&app, "b.txt"));
    common::run(&mut app, "compare.select", json!({ "view": ta.0 }));
    common::run(&mut app, "compare.withSelected", json!({ "view": tb.0 }));
    common::settle(&mut app);
    assert_eq!(hunks(&app), 3);
    let tab = diff_tab(&app);
    let right = app.harness.state().core.compare.sessions[&tab].right;
    let caret_line = |app: &common::TestApp| {
        let core = &app.harness.state().core;
        let head = core
            .workbench
            .view(right.view)
            .unwrap()
            .selection
            .primary()
            .head;
        core.workbench
            .document(right.doc)
            .unwrap()
            .buffer
            .rope()
            .char_to_line(head)
    };
    assert_eq!(caret_line(&app), 0, "cursor before the first hunk");
    for (i, line) in [1, 4, 7].into_iter().enumerate() {
        app.harness.key_press(Key::F7);
        app.harness.run_steps(3);
        let s = &app.harness.state().core.compare.sessions[&tab];
        assert_eq!(s.current, Some(i), "hunk {i} revealed");
        assert_eq!(caret_line(&app), line);
    }
    // Previous goes back.
    app.harness.key_press_modifiers(Modifiers::SHIFT, Key::F7);
    app.harness.run_steps(3);
    assert_eq!(caret_line(&app), 4);
}

// spec 27: "Ignore whitespace"
#[test]
fn ignore_whitespace_makes_trailing_spaces_identical() {
    let dir = tempfile::tempdir().unwrap();
    let a = write(dir.path(), "a.txt", "one\ntwo\n");
    let b = write(dir.path(), "b.txt", "one  \ntwo\t\n");
    let mut app = open(vec![a, b]);
    let (ta, tb) = (tab_of(&app, "a.txt"), tab_of(&app, "b.txt"));
    common::run(&mut app, "compare.select", json!({ "view": ta.0 }));
    common::run(&mut app, "compare.withSelected", json!({ "view": tb.0 }));
    common::settle(&mut app);
    assert_eq!(hunks(&app), 1);
    assert!(app.harness.query_by_label("Files are identical").is_none());
    app.harness.get_by_label("Ignore Whitespace").click();
    app.harness.run_steps(3);
    assert_eq!(hunks(&app), 0);
    assert!(app.harness.query_by_label("Files are identical").is_some());
    assert!(app.harness.query_by_label("Identical").is_some());
}

// spec 27: "Copy change to an editable side"
#[test]
fn revert_hunk_copies_saved_text_and_undoes() {
    let dir = tempfile::tempdir().unwrap();
    let notes = write(dir.path(), "notes.md", "a\nb\nc\nd\n");
    let mut app = open(vec![notes]);
    let doc = doc_of(&app, "notes.md");
    set_text(&mut app, doc, "a\nB\nc\nD\n");
    common::run(&mut app, "compare.currentChanges", Value::Null);
    common::settle(&mut app);
    assert_eq!(hunks(&app), 2);
    // Copy left → right on the first hunk ("Revert Change").
    app.harness
        .get_all_by_label("Revert Change")
        .next()
        .expect("revert action")
        .click();
    app.harness.run_steps(3);
    assert_eq!(text(&app, doc), "a\nb\nc\nD\n", "saved text for that hunk");
    assert_eq!(hunks(&app), 1);
    // Undo in the focused (right) pane restores the buffer change.
    app.harness.key_press_modifiers(ctrl(), Key::Z);
    app.harness.run_steps(3);
    assert_eq!(text(&app, doc), "a\nB\nc\nD\n");
    assert_eq!(hunks(&app), 2);
}

/// Without an editable left side, only the copy toward the right exists
/// (spec 27 "No copy toward read-only side" needs Git HEAD, M18).
#[test]
fn no_copy_toward_read_only_side() {
    let dir = tempfile::tempdir().unwrap();
    let notes = write(dir.path(), "notes.md", "a\nb\n");
    let mut app = open(vec![notes]);
    let doc = doc_of(&app, "notes.md");
    set_text(&mut app, doc, "a\nX\n");
    common::run(&mut app, "compare.currentChanges", Value::Null);
    common::settle(&mut app);
    assert!(app.harness.query_by_label("Revert Change").is_some());
    assert!(app.harness.query_by_label("Copy Change to Left").is_none());
    let tab = diff_tab(&app);
    let core = &mut app.harness.state_mut().core;
    assert!(!core.compare_copy_hunk(tab, 0, Side::Left), "refused");
}

// spec 27: "Live update while editing"
#[test]
fn hunk_disappears_when_the_sides_match() {
    let dir = tempfile::tempdir().unwrap();
    let a = write(dir.path(), "a.txt", "same\nleft\n");
    let b = write(dir.path(), "b.txt", "same\nright\n");
    let mut app = open(vec![a, b]);
    let ta = tab_of(&app, "a.txt");
    common::run(&mut app, "compare.with", json!({ "view": ta.0 }));
    // The picker chose b.txt.
    let b_doc = doc_of(&app, "b.txt");
    app.harness.state_mut().ui.picker = None;
    common::run(
        &mut app,
        "compare.pick",
        json!({ "value": format!("d:{}", b_doc.0) }),
    );
    common::settle(&mut app);
    assert_eq!(hunks(&app), 1);
    // Edit the right side (through its pane) to match the left.
    let tab = diff_tab(&app);
    let right = app.harness.state().core.compare.sessions[&tab].right;
    app.harness.state_mut().ui.focus_editor = Some(right.view);
    app.harness.run_steps(2);
    let core = &mut app.harness.state_mut().core;
    let start = "same\n".chars().count();
    core.workbench
        .edit(
            right.view,
            notoir_editor::ChangeSet::replace(start, start + 5, "left"),
            notoir_editor::Selection::caret(start + 4),
            notoir_editor::EditKind::Other,
            0,
        )
        .unwrap();
    app.harness.run_steps(3);
    assert_eq!(text(&app, b_doc), "same\nleft\n");
    assert_eq!(hunks(&app), 0, "hunk disappears");
    assert!(app.harness.query_by_label("Files are identical").is_some());
    assert!(
        app.harness
            .state()
            .core
            .workbench
            .document(b_doc)
            .unwrap()
            .is_dirty(),
        "edits mark the document dirty"
    );
}

#[test]
fn closing_the_diff_keeps_documents_and_drops_snapshots() {
    let dir = tempfile::tempdir().unwrap();
    let notes = write(dir.path(), "notes.md", "a\n");
    let mut app = open(vec![notes]);
    let doc = doc_of(&app, "notes.md");
    set_text(&mut app, doc, "b\n");
    common::run(&mut app, "compare.currentChanges", Value::Null);
    common::settle(&mut app);
    let tab = diff_tab(&app);
    let snapshot = app.harness.state().core.compare.sessions[&tab].left.doc;
    common::run(&mut app, "file.close", json!({ "view": tab.0 }));
    app.harness.run_steps(3);
    let core = &app.harness.state().core;
    assert!(core.compare.sessions.is_empty());
    assert!(
        core.workbench.document(snapshot).is_none(),
        "snapshot discarded"
    );
    assert!(
        core.workbench.document(doc).is_some(),
        "the file stays open"
    );
    assert_eq!(core.workbench.views.of_document(doc).count(), 1);
}

#[test]
fn compare_files_from_explorer_paths_and_swap() {
    let dir = tempfile::tempdir().unwrap();
    let a = write(dir.path(), "a.txt", "x\n");
    let b = write(dir.path(), "b.txt", "x\ny\n");
    let mut app = open(vec![]);
    common::run(
        &mut app,
        "compare.select",
        json!({ "path": a.to_string_lossy() }),
    );
    common::run(
        &mut app,
        "compare.withSelected",
        json!({ "path": b.to_string_lossy() }),
    );
    common::settle(&mut app);
    assert!(app.harness.query_by_label("a.txt \u{2194} b.txt").is_some());
    assert_eq!(hunks(&app), 1);
    // Files opened only for the diff have no tabs of their own.
    assert_eq!(app.harness.state().core.workbench.views.iter().count(), 1);
    app.harness.get_by_label("Swap Sides").click();
    app.harness.run_steps(3);
    assert!(app.harness.query_by_label("b.txt \u{2194} a.txt").is_some());
    let tab = diff_tab(&app);
    let (l, r) = app.harness.state().core.diff_texts(tab).unwrap();
    assert_eq!(
        (l.to_string(), r.to_string()),
        ("x\ny\n".into(), "x\n".into())
    );
    // Closing it closes those documents again.
    common::run(&mut app, "file.close", json!({ "view": tab.0 }));
    app.harness.run_steps(3);
    assert!(app.harness.state().core.workbench.documents.is_empty());
}

#[test]
fn panes_scroll_together() {
    let dir = tempfile::tempdir().unwrap();
    let body: String = (0..300).map(|i| format!("line {i}\n")).collect();
    let a = write(dir.path(), "a.txt", &body);
    let b = write(
        dir.path(),
        "b.txt",
        &body.replace("line 150\n", "changed\n"),
    );
    let mut app = open(vec![a.clone(), b.clone()]);
    let (ta, tb) = (tab_of(&app, "a.txt"), tab_of(&app, "b.txt"));
    common::run(&mut app, "compare.select", json!({ "view": ta.0 }));
    common::run(&mut app, "compare.withSelected", json!({ "view": tb.0 }));
    common::settle(&mut app);
    app.harness.key_press(Key::F7);
    app.harness.run_steps(6);
    let tab = diff_tab(&app);
    let s = &app.harness.state().core.compare.sessions[&tab];
    let (l, r) = (s.left.view, s.right.view);
    let ui = &app.harness.state().ui;
    let (ly, ry) = (
        ui.editors.scroll_px(l).unwrap().0,
        ui.editors.scroll_px(r).unwrap().0,
    );
    assert!(ly > 100.0, "scrolled to the change ({ly})");
    assert!((ly - ry).abs() < 0.5, "{ly} vs {ry}");
}

#[test]
fn horizontal_scrollbars_reach_long_lines_and_stay_synchronized() {
    let dir = tempfile::tempdir().unwrap();
    let a = write(
        dir.path(),
        "wide.txt",
        &format!("{}END\n", "\t日本e\u{301}".repeat(100)),
    );
    let b = write(dir.path(), "short.txt", "short\n");
    let mut app = open(vec![a, b]);
    let (ta, tb) = (tab_of(&app, "wide.txt"), tab_of(&app, "short.txt"));
    common::run(&mut app, "compare.select", json!({ "view": ta.0 }));
    common::run(&mut app, "compare.withSelected", json!({ "view": tb.0 }));
    common::settle(&mut app);
    let tab = diff_tab(&app);
    let session = &app.harness.state().core.compare.sessions[&tab];
    let (left, right, doc) = (session.left.view, session.right.view, session.left.doc);
    let editors = &app.harness.state().ui.editors;
    let lg = editors.geometry(left).unwrap().1;
    let rg = editors.geometry(right).unwrap().1;
    let pane_width = rg.left() - lg.left() - 26.0;
    let end = egui::pos2(lg.left() + pane_width - 12.0, lg.bottom() - 5.0);
    let start = egui::pos2(lg.right() + 20.0, end.y);
    let selection = app
        .harness
        .state()
        .core
        .workbench
        .view(left)
        .unwrap()
        .selection
        .clone();
    // Click the track at its far end with a standard primary mouse button.
    app.harness.hover_at(end);
    app.harness.run_steps(1);
    for pressed in [true, false] {
        app.harness.event(egui::Event::PointerButton {
            pos: end,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        app.harness.run_steps(1);
    }
    app.harness.run_steps(6);
    let x = app.harness.state().ui.editors.scroll_px(left).unwrap().1;
    assert!(x > 3000.0, "reaches the far end of the long line: {x}");
    assert!((x - app.harness.state().ui.editors.scroll_px(right).unwrap().1).abs() < 0.5);
    assert_eq!(
        app.harness
            .state()
            .core
            .workbench
            .view(left)
            .unwrap()
            .selection,
        selection
    );

    // Drag the thumb back to the start without changing the selection.
    app.harness.drag_at(end);
    app.harness.run_steps(1);
    for i in 1..=8 {
        app.harness.hover_at(end + (start - end) * (i as f32 / 8.0));
        app.harness.run_steps(1);
    }
    app.harness.drop_at(start);
    app.harness.run_steps(6);
    let after_drag = app.harness.state().ui.editors.scroll_px(left).unwrap().1;
    assert!(after_drag < x / 4.0, "thumb dragged back: {after_drag}");
    assert!((after_drag - app.harness.state().ui.editors.scroll_px(right).unwrap().1).abs() < 0.5);

    app.harness.set_size(egui::vec2(1000.0, 600.0));
    app.harness.run_steps(4);
    set_text(&mut app, doc, "now short\n");
    common::settle(&mut app);
    assert_eq!(
        app.harness.state().ui.editors.scroll_px(left).unwrap().1,
        0.0
    );
    assert_eq!(
        app.harness.state().ui.editors.scroll_px(right).unwrap().1,
        0.0
    );
}
