//! "Daily editor" milestone (specs 14, 17, 19, 20): Settings tab, keyboard
//! shortcuts editor, first run, Zen Mode and status bar pickers.

mod common;

use std::path::{Path, PathBuf};

use egui::{Key, Modifiers};
use egui_kittest::kittest::Queryable;
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

fn tab_titles(app: &common::TestApp) -> Vec<String> {
    let core = &app.harness.state().core;
    core.workbench
        .layout
        .active()
        .tabs
        .iter()
        .map(|v| core.document_title(core.workbench.view(*v).unwrap().document_id))
        .collect()
}

// spec 20: "Settings opens as a tab"
#[test]
fn settings_opens_as_a_tab() {
    let mut app = common::start(|_| {});
    app.harness.key_press_modifiers(ctrl(), Key::Comma);
    app.harness.run_steps(3);
    assert_eq!(tab_titles(&app), vec!["Settings".to_string()]);
    // Running it again focuses the same tab.
    common::run(&mut app, "app.settings", Value::Null);
    assert_eq!(tab_titles(&app).len(), 1);
    assert!(
        !app.harness
            .state()
            .core
            .workbench
            .documents
            .values()
            .any(|d| d.is_dirty())
    );
}

// spec 20: "Searchable"
#[test]
fn settings_search_filters() {
    let mut app = common::start(|_| {});
    common::run(&mut app, "app.settings", Value::Null);
    app.harness.event(egui::Event::Text("minimap".into()));
    app.harness.run_steps(3);
    assert!(app.harness.query_by_label("Minimap side").is_some());
    assert!(app.harness.query_by_label("Minimap width").is_some());
    assert!(app.harness.query_by_label("Font size").is_none());
    assert!(app.harness.query_by_label("Theme").is_none());
}

#[test]
fn settings_search_matches_english_and_modified_filter() {
    let mut app = common::start(common::with_language(
        notoir::i18n::Language::De,
        notoir::theme::ThemeId::Midnight,
    ));
    common::run(&mut app, "app.settings", Value::Null);
    app.harness.event(egui::Event::Text("font size".into()));
    app.harness.run_steps(3);
    assert!(app.harness.query_by_label("Schriftgröße").is_some());
    app.harness.state_mut().core.set_user_setting(
        "editor.tabSize",
        Some(notoir_core::settings::SettingValue::Int(3)),
    );
    app.harness.state_mut().core.settings_ui.search = "@modified".into();
    app.harness.run_steps(3);
    assert!(app.harness.query_by_label("Tabulatorbreite").is_some());
    assert!(app.harness.query_by_label("Schriftgröße").is_none());
}

#[test]
fn workspace_scope_writes_the_workspace_file() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("ws");
    std::fs::create_dir(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let r = root.clone();
    let mut app = common::start(move |o| o.open_paths = vec![(r, None)]);
    common::settle(&mut app);
    common::run(&mut app, "app.settings", Value::Null);
    app.harness.state_mut().core.settings_ui.workspace_scope = true;
    app.harness.run_steps(2);
    app.harness.state_mut().core.set_setting_in_scope(
        "editor.tabSize",
        Some(notoir_core::settings::SettingValue::Int(2)),
    );
    common::settle(&mut app);
    let text = std::fs::read_to_string(root.join(".notoir/settings.toml")).unwrap();
    assert!(text.contains("tabSize = 2"), "{text}");
    assert_eq!(
        app.harness
            .state()
            .core
            .settings
            .int("editor.tabSize", None),
        2
    );
    assert!(
        !app.harness
            .state()
            .core
            .user_settings
            .values
            .contains_key("editor.tabSize")
    );
}

#[test]
fn shortcut_editor_records_a_chord() {
    let mut app = common::start(|_| {});
    common::run(&mut app, "preferences.keyboardShortcuts", Value::Null);
    app.harness.state_mut().core.settings_ui.kb_search = "save all".into();
    app.harness.run_steps(3);
    app.harness.get_by_label("Change").click();
    app.harness.run_steps(2);
    app.harness
        .key_press_modifiers(ctrl() | Modifiers::ALT | Modifiers::SHIFT, Key::A);
    app.harness.run_steps(1);
    app.harness.key_press(Key::Enter);
    app.harness.run_steps(2);
    common::settle(&mut app);
    common::settle(&mut app);
    let path = app.harness.state().core.paths.keybindings_file();
    let text = std::fs::read_to_string(path).unwrap();
    assert!(
        text.contains("key = \"Mod+Alt+Shift+A\"") && text.contains("command = \"file.saveAll\""),
        "{text}"
    );
    let core = &app.harness.state().core;
    assert!(
        core.keymap
            .chords_for(&"file.saveAll".into())
            .iter()
            .any(|c| c.display(core.platform) == "Ctrl+Alt+Shift+A")
    );
}

fn first_run(o: &mut notoir::core::StartOptions) {
    o.first_run = true;
}

// spec 17: "Skip immediately"
#[test]
fn first_run_skip() {
    let mut app = common::start(first_run);
    common::settle(&mut app);
    assert!(app.harness.query_by_label("Welcome to Notoir").is_some());
    app.harness.get_by_label("Skip").click();
    app.harness.run_steps(3);
    assert!(app.harness.query_by_label("Welcome to Notoir").is_none());
    assert!(
        app.harness.query_by_label("Start writing.").is_some(),
        "empty state"
    );
    assert_eq!(
        app.harness.state().core.theme(true),
        notoir::theme::ThemeId::Midnight
    );
    let mut app = common::restart(app, first_run);
    common::settle(&mut app);
    assert!(app.harness.query_by_label("Welcome to Notoir").is_none());
}

// spec 17: "Theme choice previews and applies"
#[test]
fn first_run_theme_choice_applies() {
    let mut app = common::start(first_run);
    common::settle(&mut app);
    app.harness.get_by_label("Notoir Paper").click();
    app.harness.run_steps(3);
    assert_eq!(
        app.harness.state().core.theme(true),
        notoir::theme::ThemeId::Paper
    );
    assert_eq!(
        app.harness.state().theme.name,
        "Notoir Paper",
        "whole UI switched"
    );
}

// spec 17: "Pinned recent shown"
#[test]
fn pinned_recent_workspace_listed_first() {
    let d = tempfile::tempdir().unwrap();
    let a = d.path().join("Personal Notes");
    let b = d.path().join("StringArtIt");
    std::fs::create_dir(&a).unwrap();
    std::fs::create_dir(&b).unwrap();
    let mut app = common::start(|_| {});
    for p in [&a, &b] {
        common::run(
            &mut app,
            "workspace.open",
            json!({ "path": p.to_string_lossy() }),
        );
        common::settle(&mut app);
        common::run(&mut app, "workspace.close", Value::Null);
        common::settle(&mut app);
    }
    let pa = a.canonicalize().unwrap();
    common::run(
        &mut app,
        "workspace.pinRecent",
        json!({ "path": pa.to_string_lossy(), "pinned": true }),
    );
    app.harness.run_steps(2);
    let recent = app.harness.state().core.recent_workspaces();
    assert_eq!(recent[0].display_name.as_deref(), Some("Personal Notes"));
    assert!(recent[0].pinned);
    let first = app.harness.get_by_label("Personal Notes").rect();
    let second = app.harness.get_by_label("StringArtIt").rect();
    assert!(first.top() < second.top());
}

// spec 14: "Enter and exit restores layout"
#[test]
fn zen_mode_enter_and_exit_restores_layout() {
    let dir = tempfile::tempdir().unwrap();
    let f = write(dir.path(), "a.md", "text");
    let mut app = common::start(move |o| o.open_paths = vec![(f, None)]);
    common::settle(&mut app);
    common::run(&mut app, "view.toggleBottomPanel", Value::Null);
    let before = app.harness.state().core.panels.clone();
    assert!(before.primary.visible && before.bottom.visible);
    app.harness.key_press_modifiers(ctrl(), Key::K);
    app.harness.key_press(Key::Z);
    app.harness.run_steps(3);
    {
        let app_state = app.harness.state();
        let core = &app_state.core;
        assert!(core.zen.is_some());
        assert!(!core.panels.primary.visible && !core.panels.bottom.visible);
        assert_eq!(app_state.ui.fullscreen_requested, Some(true));
    }
    assert!(app.harness.query_by_label("EXPLORER").is_none());
    assert!(app.harness.query_by_label("a.md").is_none(), "tabs hidden");
    assert!(
        app.harness.query_by_label("File").is_none(),
        "menu bar hidden"
    );
    let v = app.harness.state().core.workbench.active_view().unwrap();
    let (_, gutter) = app.harness.state().ui.editors.geometry(v).unwrap();
    assert!(gutter.left() > 200.0, "centred column: {gutter:?}");
    app.harness.key_press(Key::Escape);
    app.harness.key_press(Key::Escape);
    app.harness.run_steps(3);
    let app_state = app.harness.state();
    assert!(app_state.core.zen.is_none());
    assert_eq!(app_state.core.panels, before);
    assert_eq!(app_state.ui.fullscreen_requested, Some(false));
}

// spec 19: "Clicking the cursor position opens Go to Line"
#[test]
fn clicking_cursor_position_opens_go_to_line() {
    let dir = tempfile::tempdir().unwrap();
    let f = write(dir.path(), "a.md", "x\ny");
    let mut app = common::start(move |o| o.open_paths = vec![(f, None)]);
    common::settle(&mut app);
    app.harness.get_by_label("Ln 1, Col 1").click();
    app.harness.run_steps(3);
    assert!(app.harness.state().core.find.goto.is_some());
}

#[test]
fn indentation_picker_converts_and_sets_tab_size() {
    let dir = tempfile::tempdir().unwrap();
    let f = write(dir.path(), "a.txt", "    a\n        b\n");
    let mut app = common::start(move |o| o.open_paths = vec![(f, None)]);
    common::settle(&mut app);
    app.harness.get_by_label("Spaces: 4").click();
    app.harness.run_steps(3);
    app.harness
        .get_by_label_contains("Convert Indentation to Tabs")
        .click();
    app.harness.run_steps(3);
    let core = &app.harness.state().core;
    let d = core.workbench.active_document().unwrap();
    assert_eq!(
        core.workbench.document(d).unwrap().buffer.text(),
        "\ta\n\t\tb\n"
    );
    assert!(app.harness.query_by_label("Tab Size: 4").is_some());
    common::run(&mut app, "editor.changeTabSize", json!({ "size": 2 }));
    assert!(app.harness.query_by_label("Tab Size: 2").is_some());
}
