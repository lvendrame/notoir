//! Line operations, find/replace, Go to Line, Command Palette, minimap and
//! the keybindings file (specs 05, 11, 13, 15, 16).

mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui::{Event, Key, Modifiers, PointerButton, Pos2};
use egui_kittest::kittest::Queryable;
use notoir::i18n::Language;
use notoir::theme::ThemeId;
use serde_json::Value;

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

fn open(text: &str) -> (tempfile::TempDir, common::TestApp) {
    let dir = tempfile::tempdir().unwrap();
    let f = write(dir.path(), "doc.txt", text);
    let mut app = common::start(move |o| o.open_paths = vec![(f, None)]);
    common::settle(&mut app);
    (dir, app)
}

fn text(app: &common::TestApp) -> String {
    let core = &app.harness.state().core;
    let d = core.workbench.active_document().unwrap();
    core.workbench.document(d).unwrap().buffer.text()
}

fn caret_line(app: &common::TestApp) -> usize {
    let core = &app.harness.state().core;
    let v = core
        .workbench
        .view(core.workbench.active_view().unwrap())
        .unwrap();
    let d = core.workbench.document(v.document_id).unwrap();
    d.buffer.rope().char_to_line(v.selection.primary().head)
}

fn select(app: &mut common::TestApp, a: usize, b: usize) {
    let core = &mut app.harness.state_mut().core;
    let v = core.workbench.active_view().unwrap();
    core.workbench
        .set_selection(
            v,
            notoir_editor::Selection::new(vec![notoir_editor::Range::new(a, b)], 0),
        )
        .unwrap();
    app.harness.run_steps(1);
}

fn type_text(app: &mut common::TestApp, s: &str) {
    app.harness.event(Event::Text(s.into()));
    app.harness.run_steps(2);
}

// spec 11: "Move line down"
#[test]
fn move_line_down_with_shortcut() {
    let (_d, mut app) = open("1\n2\n3");
    app.harness
        .key_press_modifiers(Modifiers::ALT, Key::ArrowDown);
    app.harness.run_steps(2);
    assert_eq!(text(&app), "2\n1\n3");
    assert_eq!(caret_line(&app), 1);
    common::run(&mut app, "edit.undo", Value::Null);
    assert_eq!(text(&app), "1\n2\n3");
}

// spec 11: "Sort lines"
#[test]
fn sort_selected_lines() {
    let (_d, mut app) = open("b\na10\na2");
    select(&mut app, 0, 9);
    common::run(&mut app, "edit.sortLines", Value::Null);
    assert_eq!(text(&app), "a2\na10\nb");
}

#[test]
fn line_shortcuts_duplicate_delete_insert() {
    let (_d, mut app) = open("one\ntwo");
    app.harness
        .key_press_modifiers(ctrl() | Modifiers::SHIFT, Key::D);
    app.harness.run_steps(2);
    assert_eq!(text(&app), "one\none\ntwo");
    app.harness
        .key_press_modifiers(ctrl() | Modifiers::SHIFT, Key::K);
    app.harness.run_steps(2);
    assert_eq!(text(&app), "one\ntwo");
    // The caret is on "two" after deleting the copy: insert below it.
    app.harness.key_press_modifiers(ctrl(), Key::Enter);
    type_text(&mut app, "x");
    assert_eq!(text(&app), "one\ntwo\nx");
}

#[test]
fn read_only_documents_reject_line_operations() {
    let (_d, mut app) = open("1\n2");
    common::run(&mut app, "edit.toggleReadOnly", Value::Null);
    app.harness
        .key_press_modifiers(Modifiers::ALT, Key::ArrowDown);
    app.harness.run_steps(2);
    assert_eq!(text(&app), "1\n2");
}

fn find_field(app: &mut common::TestApp, s: &str) {
    app.harness.event(Event::Text(s.into()));
    app.harness.run_steps(3);
}

// spec 11: "Whole word"
#[test]
fn whole_word_search_counts_matches() {
    let (_d, mut app) = open("cat concat cat");
    app.harness.key_press_modifiers(ctrl(), Key::F);
    app.harness.run_steps(2);
    // The word at the caret seeded the box.
    assert!(app.harness.query_by_label("1 of 3").is_some());
    app.harness.key_press_modifiers(Modifiers::ALT, Key::W);
    app.harness.run_steps(3);
    assert!(app.harness.query_by_label("1 of 2").is_some());
    let core = &app.harness.state().core;
    let v = core.workbench.active_view().unwrap();
    assert_eq!(core.find_highlights(v).0.len(), 2);
}

// spec 11: "Regex replace with capture groups"
#[test]
fn regex_replace_with_capture_groups_in_widget() {
    let (_d, mut app) = open("2026-10-14");
    common::run(&mut app, "edit.replace", Value::Null);
    common::run(&mut app, "edit.toggleFindRegex", Value::Null);
    // The query box has focus: replace the seeded word.
    app.harness.key_press_modifiers(ctrl(), Key::A);
    find_field(&mut app, r"(\d+)-(\d+)-(\d+)");
    let v = app.harness.state().core.workbench.active_view().unwrap();
    app.harness
        .state_mut()
        .core
        .find
        .widgets
        .get_mut(&v)
        .unwrap()
        .replacement = "$3/$2/$1".into();
    app.harness.get_by_label("Replace All").click();
    app.harness.run_steps(3);
    assert_eq!(text(&app), "14/10/2026");
}

// spec 11: "Replace all is one undo step"
#[test]
fn replace_all_is_one_undo_step() {
    let original: String = (0..17).map(|i| format!("foo {i}\n")).collect();
    let (_d, mut app) = open(&original);
    common::run(&mut app, "edit.replace", Value::Null);
    let v = app.harness.state().core.workbench.active_view().unwrap();
    {
        let w = app
            .harness
            .state_mut()
            .core
            .find
            .widgets
            .get_mut(&v)
            .unwrap();
        w.query = "foo".into();
        w.replacement = "bar".into();
    }
    app.harness.run_steps(2);
    common::run(&mut app, "edit.replaceAll", Value::Null);
    assert_eq!(text(&app).matches("bar").count(), 17);
    assert!(
        app.harness
            .state()
            .core
            .notifications
            .history()
            .any(|n| n.message.key == "notifications:replacedAll")
    );
    common::run(&mut app, "edit.undo", Value::Null);
    assert_eq!(text(&app), original);
}

// spec 11: "Invalid regex"
#[test]
fn invalid_regex_shows_inline_error() {
    let (_d, mut app) = open("a (b) c");
    common::run(&mut app, "edit.find", Value::Null);
    common::run(&mut app, "edit.toggleFindRegex", Value::Null);
    app.harness.key_press_modifiers(ctrl(), Key::A);
    find_field(&mut app, "(");
    let core = &app.harness.state().core;
    let v = core.workbench.active_view().unwrap();
    let w = &core.find.widgets[&v];
    assert!(w.error.is_some());
    assert!(
        core.find_highlights(v).0.is_empty(),
        "no matches highlighted"
    );
    assert!(
        app.harness.query_by_label_contains("unclosed").is_some(),
        "{:?}",
        w.error
    );
}

#[test]
fn find_next_wraps_and_escape_returns_to_text() {
    let (_d, mut app) = open("x a x a x");
    common::run(&mut app, "edit.find", Value::Null);
    app.harness.key_press_modifiers(ctrl(), Key::A);
    find_field(&mut app, "x");
    for _ in 0..3 {
        app.harness.key_press(Key::Enter);
        app.harness.run_steps(2);
    }
    assert!(app.harness.query_by_label("Search wrapped").is_some());
    app.harness.key_press(Key::Escape);
    app.harness.run_steps(3);
    assert!(app.harness.state().core.editor_focused);
}

#[test]
fn go_to_line_previews_and_escape_restores() {
    let body: String = (1..=200).map(|i| format!("line {i}\n")).collect();
    let (_d, mut app) = open(&body);
    common::run(&mut app, "navigate.goToLine", Value::Null);
    type_text(&mut app, "120");
    app.harness.run_steps(2);
    assert_eq!(caret_line(&app), 119);
    let v = app.harness.state().core.workbench.active_view().unwrap();
    let top = app.harness.state().ui.editors.top_line(v).unwrap();
    assert!(top > 80 && top < 119, "centred: top {top}");
    app.harness.key_press(Key::Escape);
    app.harness.run_steps(3);
    assert_eq!(caret_line(&app), 0);
    assert_eq!(app.harness.state().ui.editors.top_line(v), Some(0));
    common::run(&mut app, "navigate.goToLine", Value::Null);
    type_text(&mut app, "50:3");
    app.harness.key_press(Key::Enter);
    app.harness.run_steps(2);
    assert_eq!(caret_line(&app), 49);
}

fn open_palette(app: &mut common::TestApp, query: &str) -> Vec<String> {
    app.harness
        .key_press_modifiers(ctrl() | Modifiers::SHIFT, Key::P);
    app.harness.run_steps(2);
    type_text(app, query);
    app.harness.run_steps(1);
    app.harness
        .state()
        .ui
        .picker
        .as_ref()
        .expect("palette open")
        .items()
        .into_iter()
        .map(|i| i.label)
        .collect()
}

// spec 16: "Fuzzy match"
#[test]
fn palette_fuzzy_match() {
    let (_d, mut app) = open("x");
    let items = open_palette(&mut app, "togmin");
    assert_eq!(
        items.first().map(String::as_str),
        Some("View: Toggle Minimap"),
        "{items:?}"
    );
}

// spec 16: "English matching in another language"
#[test]
fn palette_matches_english_in_portuguese() {
    let mut app = common::start(common::with_language(Language::PtBr, ThemeId::Midnight));
    let items = open_palette(&mut app, "toggle minimap");
    assert!(
        items.iter().any(|l| l.ends_with("Alternar Minimapa")),
        "{items:?}"
    );
}

// spec 16: "Editor commands available from palette"
#[test]
fn palette_runs_editor_commands_on_previous_view() {
    let (_d, mut app) = open("a\nb");
    assert!(app.harness.state().core.editor_focused);
    open_palette(&mut app, "duplicate line");
    app.harness.key_press(Key::Enter);
    app.harness.run_steps(3);
    assert_eq!(text(&app), "a\na\nb");
    assert!(app.harness.state().ui.picker.is_none());
}

// spec 16: "Recently used first"
#[test]
fn palette_recently_used_first() {
    let (_d, mut app) = open("x");
    open_palette(&mut app, "toggle word wrap");
    app.harness.key_press(Key::Enter);
    app.harness.run_steps(3);
    let items = open_palette(&mut app, "");
    assert_eq!(items[0], "View: Toggle Word Wrap");
    assert!(app.harness.query_by_label("RECENTLY USED").is_some());
}

#[test]
fn palette_hides_disabled_until_show_all() {
    let mut app = common::start(|_| {});
    let items = open_palette(&mut app, "duplicate line");
    assert!(items.is_empty(), "no editor: {items:?}");
    app.harness
        .key_press_modifiers(ctrl() | Modifiers::SHIFT, Key::P);
    app.harness.run_steps(2);
    let items: Vec<String> = app
        .harness
        .state()
        .ui
        .picker
        .as_ref()
        .unwrap()
        .items()
        .into_iter()
        .map(|i| i.label)
        .collect();
    assert!(items.iter().any(|l| l == "Edit: Duplicate Line"));
}

fn long_doc() -> (tempfile::TempDir, common::TestApp) {
    let body: String = (1..=2000)
        .map(|i| format!("    line {i} of the document\n"))
        .collect();
    open(&body)
}

// spec 13: "Click to jump"
#[test]
fn minimap_click_centres_line() {
    let (_d, mut app) = long_doc();
    let v = app.harness.state().core.workbench.active_view().unwrap();
    let (mini, _) = app.harness.state().ui.editors.geometry(v).unwrap();
    let mini = mini.expect("minimap shown");
    // The minimap scrolls with long documents; at the top, line n is at n * 2 px.
    let y = mini.top() + 290.0 * 2.0;
    let p = Pos2::new(mini.center().x, y);
    app.harness.hover_at(p);
    app.harness.event(Event::PointerButton {
        pos: p,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    app.harness.event(Event::PointerButton {
        pos: p,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    app.harness.run_steps(3);
    let top = app.harness.state().ui.editors.top_line(v).unwrap();
    let visible = 30;
    assert!(
        (top as i64 - (290 - visible / 2) as i64).abs() <= visible as i64 / 2 + 2,
        "top {top}"
    );
}

// spec 13: "Drag viewport"
#[test]
fn minimap_drag_scrolls_continuously() {
    let (_d, mut app) = long_doc();
    let v = app.harness.state().core.workbench.active_view().unwrap();
    let mini = app
        .harness
        .state()
        .ui
        .editors
        .geometry(v)
        .unwrap()
        .0
        .unwrap();
    let start = Pos2::new(mini.center().x, mini.top() + 10.0);
    app.harness.hover_at(start);
    app.harness.run_steps(1);
    app.harness.event(Event::PointerButton {
        pos: start,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    app.harness.run_steps(1);
    let mut last = 0;
    for i in 1..=5 {
        app.harness
            .hover_at(start + egui::vec2(0.0, i as f32 * 20.0));
        app.harness.run_steps(1);
        let top = app.harness.state().ui.editors.top_line(v).unwrap();
        assert!(top > last, "step {i}: {top} after {last}");
        last = top;
    }
    app.harness.event(Event::PointerButton {
        pos: start,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    app.harness.run_steps(1);
}

// spec 13: "Position left"
#[test]
fn minimap_on_the_left_comes_before_the_gutter() {
    let (_d, mut app) = long_doc();
    common::run(
        &mut app,
        "view.minimapSide",
        serde_json::json!({ "side": "left" }),
    );
    let v = app.harness.state().core.workbench.active_view().unwrap();
    let (mini, gutter) = app.harness.state().ui.editors.geometry(v).unwrap();
    let mini = mini.unwrap();
    assert!(mini.right() <= gutter.left() + 0.5, "{mini:?} {gutter:?}");
}

// spec 13: "Hidden in large-file mode"
#[test]
fn minimap_hidden_in_large_file_mode() {
    let (_d, mut app) = long_doc();
    {
        let core = &mut app.harness.state_mut().core;
        let d = core.workbench.active_document().unwrap();
        core.workbench.document_mut(d).unwrap().large_file = true;
    }
    app.harness.run_steps(2);
    let v = app.harness.state().core.workbench.active_view().unwrap();
    assert!(
        app.harness
            .state()
            .ui
            .editors
            .geometry(v)
            .unwrap()
            .0
            .is_none()
    );
}

fn wait_for(app: &mut common::TestApp, secs: u64, f: impl Fn(&common::TestApp) -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        common::settle(app);
        if f(app) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(40));
    }
    false
}

#[test]
fn keybindings_file_changes_apply_without_restart() {
    let mut app = common::start(|_| {});
    let path = app.harness.state().core.paths.keybindings_file();
    std::thread::sleep(Duration::from_millis(200));
    std::fs::write(
        &path,
        "[[binding]]\nkey = \"Mod+Shift+S\"\ncommand = \"file.saveAll\"\n",
    )
    .unwrap();
    assert!(wait_for(&mut app, 5, |a| {
        let core = &a.harness.state().core;
        !core.keymap.chords_for(&"file.saveAll".into()).is_empty()
            && core
                .keymap
                .chords_for(&"file.saveAll".into())
                .iter()
                .any(|k| k.display(core.platform) == "Ctrl+Shift+S")
    }));
}

#[test]
fn open_keybindings_file_creates_template_and_saving_applies() {
    let mut app = common::start(|_| {});
    common::run(&mut app, "preferences.openKeybindingsFile", Value::Null);
    common::settle(&mut app);
    assert!(text(&app).contains("[[binding]]"));
    app.harness.key_press_modifiers(ctrl(), Key::End);
    type_text(
        &mut app,
        "\n[[binding]]\nkey = \"Mod+Alt+M\"\ncommand = \"view.toggleMinimap\"\n",
    );
    app.harness.key_press_modifiers(ctrl(), Key::S);
    app.harness.run_steps(2);
    assert!(wait_for(&mut app, 5, |a| {
        let core = &a.harness.state().core;
        !core
            .keymap
            .chords_for(&"view.toggleMinimap".into())
            .is_empty()
    }));
}

#[test]
fn settings_file_changes_apply_without_restart() {
    let mut app = common::start(|_| {});
    let path = app.harness.state().core.paths.user_settings_file();
    std::thread::sleep(Duration::from_millis(200));
    std::fs::write(&path, "[editor]\ntabSize = 7\n").unwrap();
    assert!(wait_for(&mut app, 5, |a| a
        .harness
        .state()
        .core
        .settings
        .int("editor.tabSize", None)
        == 7));
}

/// Regression: clicking Next / Previous must not also place the caret in
/// the editor under the widget (which restarted the search from there).
#[test]
fn find_buttons_walk_every_match() {
    let body: String = (1..=40).map(|i| format!("item {i} x\n")).collect();
    let (_d, mut app) = open(&body);
    common::run(&mut app, "edit.find", Value::Null);
    app.harness.key_press_modifiers(ctrl(), Key::A);
    find_field(&mut app, "x");
    let current = |app: &common::TestApp| {
        let core = &app.harness.state().core;
        let v = core.workbench.active_view().unwrap();
        core.find.widgets[&v].current
    };
    let mut seen = Vec::new();
    for _ in 0..6 {
        app.harness.get_by_label("Next Match (Enter)").click();
        app.harness.run_steps(3);
        seen.push(current(&app).unwrap());
    }
    let start = seen[0];
    assert_eq!(
        seen,
        (start..start + 6).collect::<Vec<_>>(),
        "advances one match per click"
    );
    for _ in 0..3 {
        app.harness
            .get_by_label("Previous Match (Shift+Enter)")
            .click();
        app.harness.run_steps(3);
    }
    assert_eq!(current(&app), Some(start + 2));
}

#[test]
fn replace_button_replaces_current_match_only() {
    let (_d, mut app) = open("a x b x c x\n");
    common::run(&mut app, "edit.replace", Value::Null);
    app.harness.key_press_modifiers(ctrl(), Key::A);
    find_field(&mut app, "x");
    let v = app.harness.state().core.workbench.active_view().unwrap();
    app.harness
        .state_mut()
        .core
        .find
        .widgets
        .get_mut(&v)
        .unwrap()
        .replacement = "Y".into();
    app.harness.get_by_label("Replace").click();
    app.harness.run_steps(3);
    assert_eq!(text(&app), "a Y b x c x\n");
}
