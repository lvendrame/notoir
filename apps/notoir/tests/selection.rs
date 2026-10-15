//! Multi-cursor and the Selection menu (spec 11 §Cursor and Selection).

mod common;

use egui::{Event, Key, Modifiers, PointerButton, Pos2};
use notoir_editor::{Range, Selection};
use serde_json::Value;

fn ctrl() -> Modifiers {
    Modifiers {
        ctrl: true,
        command: true,
        ..Default::default()
    }
}

/// A new Untitled document holding `text`, focused, caret at 0.
fn editor(text: &str) -> common::TestApp {
    let mut app = common::start(|_| {});
    app.harness.key_press_modifiers(ctrl(), Key::N);
    app.harness.run_steps(3);
    app.harness.event(Event::Paste(text.into()));
    app.harness.run_steps(2);
    set_sel(&mut app, Selection::caret(0));
    app
}

fn view(app: &common::TestApp) -> notoir_core::ids::ViewId {
    app.harness.state().core.workbench.active_view().unwrap()
}

fn text(app: &common::TestApp) -> String {
    let core = &app.harness.state().core;
    let doc = core.workbench.active_document().unwrap();
    core.workbench.document(doc).unwrap().buffer.text()
}

fn sel(app: &common::TestApp) -> Selection {
    let core = &app.harness.state().core;
    core.workbench.view(view(app)).unwrap().selection.clone()
}

fn set_sel(app: &mut common::TestApp, s: Selection) {
    let v = view(app);
    let _ = app.harness.state_mut().core.workbench.set_selection(v, s);
    app.harness.run_steps(2);
}

fn type_text(app: &mut common::TestApp, s: &str) {
    for c in s.chars() {
        app.harness.event(Event::Text(c.to_string()));
    }
    app.harness.run_steps(2);
}

#[test]
fn select_next_occurrence_then_type_edits_every_match() {
    let mut app = editor("foo bar foo baz foo");
    set_sel(&mut app, Selection::caret(1));
    app.harness.key_press_modifiers(ctrl(), Key::D);
    app.harness.run_steps(2);
    assert_eq!(sel(&app).ranges(), &[Range::new(0, 3)], "word at caret");
    app.harness.key_press_modifiers(ctrl(), Key::D);
    app.harness.run_steps(2);
    assert_eq!(sel(&app).len(), 2);
    type_text(&mut app, "qux");
    assert_eq!(text(&app), "qux bar qux baz foo");
    // One undo step for the multi-cursor typing.
    app.harness.key_press_modifiers(ctrl(), Key::Z);
    app.harness.run_steps(2);
    assert_eq!(text(&app), "foo bar foo baz foo");
}

#[test]
fn select_all_occurrences_and_escape_collapses() {
    let mut app = editor("ab x ab y ab");
    set_sel(&mut app, Selection::caret(0));
    app.harness
        .key_press_modifiers(ctrl() | Modifiers::SHIFT, Key::L);
    app.harness.run_steps(2);
    assert_eq!(sel(&app).len(), 3);
    app.harness.key_press(Key::Escape);
    app.harness.run_steps(2);
    assert_eq!(sel(&app).len(), 1, "Esc keeps the primary cursor only");
    assert_eq!(sel(&app).primary(), Range::new(0, 2));
}

#[test]
fn add_cursor_below_types_on_each_line_and_pastes_lines() {
    let mut app = editor("one\ntwo\nthree");
    set_sel(&mut app, Selection::caret(0));
    let down = ctrl() | Modifiers::ALT;
    app.harness.key_press_modifiers(down, Key::ArrowDown);
    app.harness.key_press_modifiers(down, Key::ArrowDown);
    app.harness.run_steps(2);
    assert_eq!(sel(&app).len(), 3);
    type_text(&mut app, "-");
    assert_eq!(text(&app), "-one\n-two\n-three");
    // As many clipboard lines as cursors: one line each.
    app.harness.event(Event::Paste("a\nb\nc".into()));
    app.harness.run_steps(2);
    assert_eq!(text(&app), "-aone\n-btwo\n-cthree");
    // Other line counts paste the whole text at every cursor.
    app.harness.event(Event::Paste("z".into()));
    app.harness.run_steps(2);
    assert_eq!(text(&app), "-azone\n-bztwo\n-czthree");
}

#[test]
fn alt_click_adds_a_caret() {
    let mut app = editor("first\nsecond\nthird");
    set_sel(&mut app, Selection::caret(0));
    let v = view(&app);
    let (y, row_h) = app
        .harness
        .state_mut()
        .ui
        .editors
        .line_y(v, 2, false)
        .expect("laid out");
    let (_, gutter) = app.harness.state().ui.editors.geometry(v).unwrap();
    let p = Pos2::new(gutter.right() + 300.0, y + row_h / 2.0);
    let h = &mut app.harness;
    h.event(Event::ModifiersChanged(Modifiers::ALT));
    h.hover_at(p);
    h.run_steps(1);
    h.event(Event::PointerButton {
        pos: p,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::ALT,
    });
    h.run_steps(1);
    h.event(Event::PointerButton {
        pos: p,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::ALT,
    });
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(2);
    let s = sel(&app);
    assert_eq!(s.ranges(), &[Range::caret(0), Range::caret(18)]);
    assert_eq!(s.primary(), Range::caret(18), "the new caret is primary");
    type_text(&mut app, "!");
    assert_eq!(text(&app), "!first\nsecond\nthird!");
}

#[test]
fn expand_and_shrink_selection() {
    let mut app = editor("call(alpha, beta)\nnext");
    set_sel(&mut app, Selection::caret(7));
    common::run(&mut app, "selection.expand", Value::Null);
    assert_eq!(sel(&app).primary(), Range::new(5, 10), "word");
    common::run(&mut app, "selection.expand", Value::Null);
    assert_eq!(sel(&app).primary(), Range::new(5, 16), "bracket contents");
    common::run(&mut app, "selection.expand", Value::Null);
    assert_eq!(sel(&app).primary(), Range::new(4, 17), "with brackets");
    common::run(&mut app, "selection.shrink", Value::Null);
    assert_eq!(sel(&app).primary(), Range::new(5, 16));
    common::run(&mut app, "selection.shrink", Value::Null);
    common::run(&mut app, "selection.shrink", Value::Null);
    assert_eq!(sel(&app).primary(), Range::caret(7), "back to the caret");
}

#[test]
fn column_mode_toggle_is_checked_in_the_menu() {
    let mut app = editor("x");
    assert!(!app.harness.state().core.column_mode);
    common::run(&mut app, "selection.columnMode", Value::Null);
    let core = &app.harness.state().core;
    assert!(core.column_mode);
    assert_eq!(
        core.context(false).get("columnSelection"),
        Some(&notoir_core::when::Value::Bool(true))
    );
}
