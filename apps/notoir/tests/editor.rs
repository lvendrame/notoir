//! Editor widget (specs 11, 14, 19): typing, editing keys, undo, wrap, zoom.

mod common;

use egui::{Event, Key, Modifiers};
use egui_kittest::kittest::Queryable;
use notoir_core::settings::SettingValue;

fn ctrl() -> Modifiers {
    Modifiers {
        ctrl: true,
        command: true,
        ..Default::default()
    }
}

/// Fresh app with a new Untitled document focused.
fn editor() -> common::TestApp {
    let mut app = common::start(|_| {});
    app.harness.key_press_modifiers(ctrl(), Key::N);
    app.harness.run_steps(3);
    app
}

fn text(app: &common::TestApp) -> String {
    let core = &app.harness.state().core;
    let doc = core.workbench.active_document().expect("document");
    core.workbench.document(doc).unwrap().buffer.text()
}

fn type_text(app: &mut common::TestApp, s: &str) {
    for c in s.chars() {
        app.harness.event(Event::Text(c.to_string()));
    }
    app.harness.run_steps(2);
}

#[test]
fn new_file_takes_typing_and_shows_position() {
    let mut app = editor();
    type_text(&mut app, "hello");
    assert_eq!(text(&app), "hello");
    assert!(app.harness.query_by_label("Ln 1, Col 6").is_some());
    app.harness.key_press(Key::Enter);
    type_text(&mut app, "x");
    assert_eq!(text(&app), "hello\nx");
    assert!(app.harness.query_by_label("Ln 2, Col 2").is_some());
}

// spec 11: "Typing a word is one undo step"
#[test]
fn typing_undo_steps_follow_word_boundaries() {
    let mut app = editor();
    type_text(&mut app, "hello world");
    app.harness.key_press_modifiers(ctrl(), Key::Z);
    app.harness.run_steps(2);
    assert_eq!(text(&app), "hello ");
    app.harness.key_press_modifiers(ctrl(), Key::Z);
    app.harness.run_steps(2);
    assert_eq!(text(&app), "");
}

// spec 11: "Paste is one undo step"
#[test]
fn paste_is_one_undo_step() {
    let mut app = editor();
    app.harness.event(Event::Paste("1\r\n2\n3".into()));
    app.harness.run_steps(2);
    assert_eq!(text(&app), "1\n2\n3", "line endings normalized");
    app.harness.key_press_modifiers(ctrl(), Key::Z);
    app.harness.run_steps(2);
    assert_eq!(text(&app), "");
}

// spec 11: "Backspace deletes a whole emoji"
#[test]
fn backspace_removes_whole_emoji() {
    let mut app = editor();
    app.harness.event(Event::Text("a👍🏽".into()));
    app.harness.run_steps(2);
    app.harness.key_press(Key::Backspace);
    app.harness.run_steps(2);
    assert_eq!(text(&app), "a");
}

#[test]
fn arrows_and_selection_editing() {
    let mut app = editor();
    type_text(&mut app, "abc");
    app.harness.key_press(Key::ArrowLeft);
    app.harness
        .key_press_modifiers(Modifiers::SHIFT, Key::ArrowLeft);
    app.harness.run_steps(2);
    type_text(&mut app, "X");
    assert_eq!(text(&app), "aXc");
    app.harness.key_press_modifiers(ctrl(), Key::A);
    app.harness.run_steps(2);
    type_text(&mut app, "z");
    assert_eq!(text(&app), "z");
}

#[test]
fn workbench_shortcuts_work_while_editing() {
    let mut app = editor();
    assert!(app.harness.state().core.editor_focused);
    app.harness.key_press_modifiers(ctrl(), Key::B);
    app.harness.run_steps(2);
    assert!(!app.harness.state().core.panels.primary.visible);
}

// spec 14: "Toggle word wrap for one view"
#[test]
fn word_wrap_toggles_for_the_active_view() {
    let mut app = editor();
    app.harness.event(Event::Paste("word ".repeat(200)));
    app.harness.run_steps(3);
    let view = app.harness.state().core.workbench.active_view().unwrap();
    let wrapped = app.harness.state_mut().ui.editors.total_rows(view).unwrap();
    assert!(wrapped > 1, "plain text wraps by default ({wrapped} rows)");
    app.harness.key_press_modifiers(Modifiers::ALT, Key::Z);
    app.harness.run_steps(3);
    assert_eq!(app.harness.state_mut().ui.editors.total_rows(view), Some(1));
    // Only this view's override changed.
    let core = &app.harness.state().core;
    assert_eq!(
        core.workbench.view(view).unwrap().wrap_override,
        Some(false)
    );
    assert_eq!(
        core.settings.str("editor.wordWrap", Some("plaintext")),
        "on"
    );
}

// spec 14: "Reset zoom"
// spec 19: "Zoom item visibility"
#[test]
fn zoom_item_shows_only_when_zoomed() {
    let mut app = editor();
    assert!(app.harness.query_by_label("110%").is_none());
    app.harness.key_press_modifiers(ctrl(), Key::Equals);
    app.harness.run_steps(2);
    assert!(app.harness.query_by_label("110%").is_some());
    app.harness.key_press_modifiers(ctrl(), Key::Num0);
    app.harness.run_steps(2);
    assert_eq!(app.harness.state().core.zoom, 100);
    assert!(app.harness.query_by_label("110%").is_none());
}

#[test]
fn read_only_rejects_typing() {
    let mut app = editor();
    let core = &mut app.harness.state_mut().core;
    let doc = core.workbench.active_document().unwrap();
    core.workbench.document_mut(doc).unwrap().read_only = notoir_core::document::ReadOnly::User;
    type_text(&mut app, "x");
    assert_eq!(text(&app), "");
}

#[test]
fn line_numbers_toggle_persists_setting() {
    let mut app = editor();
    common::run(&mut app, "view.toggleLineNumbers", serde_json::Value::Null);
    let core = &app.harness.state().core;
    assert_eq!(
        core.settings.get("editor.lineNumbers", None),
        Some(&SettingValue::Str("off".into()))
    );
}
