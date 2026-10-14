//! Editor groups and splits (specs 03, 14): split commands, duplicate views,
//! drag-and-drop docking, maximize, zoom across groups.

mod common;

use std::path::{Path, PathBuf};

use egui::{Event, Key, Modifiers, PointerButton, Pos2};
use egui_kittest::kittest::Queryable;
use notoir_core::layout::{Axis, LayoutNode};
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
    p
}

/// App with `files` opened from the command line.
fn app_with(files: Vec<PathBuf>) -> common::TestApp {
    let mut app =
        common::start(move |o| o.open_paths = files.into_iter().map(|f| (f, None)).collect());
    common::settle(&mut app);
    app
}

fn groups(app: &common::TestApp) -> usize {
    app.harness.state().core.workbench.layout.groups.len()
}

/// Presses on `from`, moves in steps to `to`, releases there (Alt copies).
fn drag(app: &mut common::TestApp, from: Pos2, to: Pos2, alt: bool) {
    let h = &mut app.harness;
    h.hover_at(from);
    h.run_steps(1);
    h.drag_at(from);
    h.run_steps(1);
    for i in 1..=8 {
        let t = i as f32 / 8.0;
        h.hover_at(from + (to - from) * t);
        h.run_steps(1);
    }
    let modifiers = if alt { Modifiers::ALT } else { Modifiers::NONE };
    h.event(Event::PointerButton {
        pos: to,
        button: PointerButton::Primary,
        pressed: false,
        modifiers,
    });
    h.run_steps(3);
}

// spec 03: "Split right creates a second group with a view of the same document"
#[test]
fn split_right_shows_same_document_twice() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app_with(vec![write(dir.path(), "README.md", "x")]);
    app.harness.key_press_modifiers(ctrl(), Key::Backslash);
    app.harness.run_steps(3);
    assert_eq!(groups(&app), 2);
    assert_eq!(
        app.harness.query_all_by_label("README.md").count(),
        2,
        "one tab per group"
    );
    let core = &app.harness.state().core;
    assert_eq!(core.workbench.documents.len(), 1);
    let LayoutNode::Split { axis, .. } = &core.workbench.layout.root else {
        panic!()
    };
    assert_eq!(*axis, Axis::Horizontal);
}

// spec 03: "Nested split layout"
#[test]
fn nested_split_layout() {
    let dir = tempfile::tempdir().unwrap();
    let readme = write(dir.path(), "README.md", "r");
    let notes = write(dir.path(), "notes.md", "n");
    let config = write(dir.path(), "config.json", "{}");
    let mut app = app_with(vec![readme]);
    common::run(&mut app, "view.splitDown", Value::Null);
    app.harness
        .state_mut()
        .core
        .open_path(&notes, Default::default());
    common::settle(&mut app);
    app.harness
        .state_mut()
        .core
        .open_path(&config, Default::default());
    common::settle(&mut app);
    common::run(&mut app, "view.splitRight", Value::Null);
    let core = &app.harness.state().core;
    let LayoutNode::Split { axis, children } = &core.workbench.layout.root else {
        panic!()
    };
    assert_eq!(*axis, Axis::Vertical);
    assert!(matches!(
        &children[1].0,
        LayoutNode::Split {
            axis: Axis::Horizontal,
            ..
        }
    ));
    assert_eq!(groups(&app), 3);
}

// spec 03: "Closing the last tab of a group collapses the split"
#[test]
fn closing_last_tab_collapses_split() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app_with(vec![write(dir.path(), "a.md", "a")]);
    common::run(&mut app, "view.splitRight", Value::Null);
    assert_eq!(groups(&app), 2);
    app.harness.key_press_modifiers(ctrl(), Key::W);
    app.harness.run_steps(3);
    assert_eq!(groups(&app), 1);
    assert_eq!(app.harness.state().core.workbench.documents.len(), 1);
}

// spec 03: "Duplicate views share one buffer but have independent cursors"
#[test]
fn split_views_share_buffer_with_own_cursors() {
    let dir = tempfile::tempdir().unwrap();
    let text: String = (1..=30).map(|i| format!("line {i}\n")).collect();
    let mut app = app_with(vec![write(dir.path(), "a.md", &text)]);
    let left = app.harness.state().core.workbench.active_view().unwrap();
    common::run(&mut app, "view.splitRight", Value::Null);
    let right = app.harness.state().core.workbench.active_view().unwrap();
    assert_ne!(left, right);
    app.harness.key_press(Key::End);
    app.harness.event(Event::Text("hello".into()));
    app.harness.run_steps(2);
    let core = &app.harness.state().core;
    let doc = core.workbench.view(left).unwrap().document_id;
    assert!(
        core.workbench
            .document(doc)
            .unwrap()
            .buffer
            .text()
            .contains("hello")
    );
    assert_eq!(
        core.workbench.view(left).unwrap().selection.primary().head,
        0,
        "left caret unchanged"
    );
}

#[test]
fn clicking_into_a_group_activates_it() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app_with(vec![write(dir.path(), "a.md", "a")]);
    let left = app.harness.state().core.workbench.active_view().unwrap();
    common::run(&mut app, "view.splitRight", Value::Null);
    app.harness.hover_at(Pos2::new(500.0, 400.0));
    app.harness.drag_at(Pos2::new(500.0, 400.0));
    app.harness.event(Event::PointerButton {
        pos: Pos2::new(500.0, 400.0),
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    app.harness.run_steps(3);
    assert_eq!(app.harness.state().core.workbench.active_view(), Some(left));
}

// spec 03: "Dropping a tab on a group edge creates a split"
#[test]
fn dropping_tab_on_bottom_edge_creates_split() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app_with(vec![
        write(dir.path(), "a.md", "a"),
        write(dir.path(), "b.md", "b"),
    ]);
    let tab = app.harness.get_by_label("b.md").rect().center();
    drag(&mut app, tab, Pos2::new(800.0, 760.0), false);
    assert_eq!(groups(&app), 2);
    let core = &app.harness.state().core;
    let LayoutNode::Split { axis, children } = &core.workbench.layout.root else {
        panic!()
    };
    assert_eq!(*axis, Axis::Vertical);
    let LayoutNode::Group(bottom) = children[1].0 else {
        panic!()
    };
    let names: Vec<String> = core
        .workbench
        .layout
        .group(bottom)
        .unwrap()
        .tabs
        .iter()
        .map(|v| core.document_title(core.workbench.view(*v).unwrap().document_id))
        .collect();
    assert_eq!(names, ["b.md"]);
}

#[test]
fn alt_drop_duplicates_instead_of_moving() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app_with(vec![write(dir.path(), "a.md", "a")]);
    let tab = app.harness.get_by_label("a.md").rect().center();
    drag(&mut app, tab, Pos2::new(1250.0, 400.0), true);
    assert_eq!(groups(&app), 2);
    assert_eq!(app.harness.query_all_by_label("a.md").count(), 2);
}

#[test]
fn dragging_a_tab_reorders_the_strip() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app_with(vec![
        write(dir.path(), "a.md", "a"),
        write(dir.path(), "b.md", "b"),
        write(dir.path(), "c.md", "c"),
    ]);
    let a = app.harness.get_by_label("a.md").rect().center();
    let c = app.harness.get_by_label("c.md").rect();
    drag(&mut app, a, Pos2::new(c.right() - 4.0, c.center().y), false);
    let core = &app.harness.state().core;
    let names: Vec<String> = core
        .workbench
        .layout
        .active()
        .tabs
        .iter()
        .map(|v| core.document_title(core.workbench.view(*v).unwrap().document_id))
        .collect();
    assert_eq!(names, ["b.md", "c.md", "a.md"]);
}

// spec 03: "Maximize and restore"
#[test]
fn maximize_and_restore_panels_exactly() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app_with(vec![write(dir.path(), "a.md", "a")]);
    common::run(&mut app, "view.splitRight", Value::Null);
    common::run(&mut app, "view.toggleBottomPanel", Value::Null);
    let before = app.harness.state().core.panels.clone();
    common::run(&mut app, "workbench.maximizeGroup", Value::Null);
    {
        let core = &app.harness.state().core;
        assert!(!core.panels.primary.visible && !core.panels.bottom.visible);
        assert!(core.workbench.layout.maximized.is_some());
    }
    assert_eq!(
        app.harness.query_all_by_label("a.md").count(),
        1,
        "only the maximized group is shown"
    );
    assert!(app.harness.query_by_label("EXPLORER").is_none());
    common::run(&mut app, "workbench.maximizeGroup", Value::Null);
    assert_eq!(app.harness.state().core.panels, before);
    assert_eq!(app.harness.query_all_by_label("a.md").count(), 2);
}

#[test]
fn opening_a_panel_leaves_maximized_mode() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app_with(vec![write(dir.path(), "a.md", "a")]);
    common::run(&mut app, "workbench.maximizeGroup", Value::Null);
    common::run(&mut app, "view.showExplorer", Value::Null);
    let core = &app.harness.state().core;
    assert!(core.workbench.layout.maximized.is_none());
    assert!(core.panels.primary.visible);
}

// spec 14: "Global zoom affects every view"
#[test]
fn global_zoom_affects_every_view() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app_with(vec![
        write(dir.path(), "a.md", "a"),
        write(dir.path(), "b.md", "b"),
    ]);
    common::run(&mut app, "workbench.moveTabToNextGroup", Value::Null);
    assert_eq!(groups(&app), 2);
    for _ in 0..2 {
        app.harness.key_press_modifiers(ctrl(), Key::Equals);
        app.harness.run_steps(2);
    }
    let core = &app.harness.state().core;
    for g in core.workbench.layout.groups_in_order() {
        let v = core.workbench.layout.group(g).unwrap().active.unwrap();
        assert!((core.zoom_for(v) - 1.25).abs() < 1e-6);
    }
    assert!(app.harness.query_by_label("125%").is_some());
}

// spec 14: "Per-view zoom"
#[test]
fn per_view_zoom_leaves_other_views() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app_with(vec![write(dir.path(), "a.md", "a")]);
    app.harness.state_mut().core.set_user_setting(
        "editor.zoomScope",
        Some(notoir_core::settings::SettingValue::Str("view".into())),
    );
    let left = app.harness.state().core.workbench.active_view().unwrap();
    common::run(&mut app, "view.splitRight", Value::Null);
    let right = app.harness.state().core.workbench.active_view().unwrap();
    common::run(&mut app, "view.activate", json!({ "view": left.0 }));
    common::run(&mut app, "view.zoomIn", Value::Null);
    let core = &app.harness.state().core;
    assert!((core.zoom_for(left) - 1.1).abs() < 1e-6);
    assert!((core.zoom_for(right) - 1.0).abs() < 1e-6);
}
