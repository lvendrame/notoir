//! Theme editing, portable presets, per-file appearance and measured typography.
mod common;

use egui_kittest::kittest::Queryable;
use notoir::theme::{Theme, ThemeId, Tok};
use notoir::theme_manager;
use serde_json::{Value, json};

fn action(app: &mut common::TestApp, args: Value) {
    common::run(app, "themes.action", args);
}
fn edit(app: &mut common::TestApp, path: &[&str], value: Value) {
    action(app, json!({"action":"edit","path":path,"value":value}));
}
fn save(app: &mut common::TestApp) {
    action(app, json!({"action":"save"}));
    common::settle(app);
    assert!(
        app.harness.state().core.theme_manager.error.is_none(),
        "{:?}",
        app.harness.state().core.theme_manager.error
    );
}
fn open(app: &mut common::TestApp) {
    common::run(app, "preferences.manageThemes", Value::Null);
    common::settle(app);
}

// spec 21: "Manage themes in an editor tab"
#[test]
fn tools_opens_theme_manager_without_changing_active_theme() {
    let mut app = common::start(|_| {});
    app.harness.get_by_label("Tools").click();
    app.harness.run_steps(2);
    app.harness.get_by_label_contains("Manage Themes…").click();
    app.harness.run_steps(3);
    assert_eq!(
        app.harness
            .state()
            .core
            .workbench
            .view(app.harness.state().core.workbench.active_view().unwrap())
            .unwrap()
            .kind,
        notoir_core::view::ViewKind::ThemeManager
    );
    assert_eq!(app.harness.state().theme_id(), "notoir-midnight");
    assert!(app.harness.query_by_label("Live Preview").is_some());
    assert!(app.harness.query_by_label("Duplicate").is_some());
    assert!(app.harness.query_by_label("Import…").is_some());
    assert!(app.harness.query_by_label("Export…").is_some());
}

// spec 21: "Theme drafts preview until saved"
#[test]
fn draft_preview_save_and_restart() {
    let mut app = common::start(|_| {});
    open(&mut app);
    let before = app.harness.state().theme.color(Tok::Accent);
    edit(&mut app, &["ui", "accent"], json!("#d15e72"));
    assert_eq!(app.harness.state().theme.color(Tok::Accent), before);
    assert_eq!(
        app.harness
            .state()
            .core
            .theme_manager
            .draft
            .as_ref()
            .unwrap()
            .theme()
            .unwrap()
            .color(Tok::Accent),
        egui::Color32::from_rgb(209, 94, 114)
    );
    save(&mut app);
    assert_eq!(
        app.harness.state().theme.color(Tok::Accent),
        egui::Color32::from_rgb(209, 94, 114)
    );
    let path = app
        .harness
        .state()
        .core
        .theme_manager
        .draft
        .as_ref()
        .unwrap()
        .path
        .clone()
        .unwrap();
    assert!(path.ends_with("defaults/notoir-midnight.toml"));
    assert!(std::fs::read_to_string(path).unwrap().contains("#d15e72"));
    let mut app = common::restart(app, |_| {});
    common::settle(&mut app);
    assert_eq!(
        app.harness.state().theme.color(Tok::Accent),
        egui::Color32::from_rgb(209, 94, 114)
    );
}

// spec 21: "Duplicate a customized theme"
#[test]
fn duplicate_customized_dracula_is_independent() {
    let mut app = common::start(|_| {});
    open(&mut app);
    action(&mut app, json!({"action":"select","id":"dracula"}));
    edit(&mut app, &["ui", "accent"], json!("#654321"));
    edit(
        &mut app,
        &["appearance", "settings", "editor.fontFamily"],
        json!("Manrope"),
    );
    edit(
        &mut app,
        &["appearance", "extensions", "yml", "colors", "syntax.string"],
        json!("#123456"),
    );
    save(&mut app);
    let original = app.harness.state().core.load_theme("dracula");
    action(&mut app, json!({"action":"duplicate"}));
    let copy = app
        .harness
        .state()
        .core
        .theme_manager
        .draft
        .as_ref()
        .unwrap()
        .theme()
        .unwrap();
    assert_ne!(copy.id, "dracula");
    for token in Tok::ALL {
        assert_eq!(
            copy.style(*token),
            original.style(*token),
            "{}",
            token.name()
        );
    }
    assert_eq!(
        copy.appearance.settings["editor.fontFamily"].as_str(),
        Some("Manrope")
    );
    assert_eq!(
        copy.for_document(Some("yaml"), Some("test.yml"))
            .color(Tok::SyntaxString),
        egui::Color32::from_rgb(18, 52, 86)
    );
    save(&mut app);
    let copy_id = copy.id;
    action(&mut app, json!({"action":"select","id":"dracula"}));
    action(&mut app, json!({"action":"reset","confirmed":true}));
    common::settle(&mut app);
    assert_eq!(
        app.harness
            .state()
            .core
            .load_theme(&copy_id)
            .color(Tok::Accent),
        egui::Color32::from_rgb(101, 67, 33)
    );
}

// spec 21: "Reset built-in themes to bundled defaults"
#[test]
fn reset_builtin_preserves_custom_themes_and_explicit_settings() {
    let mut app = common::start(|_| {});
    open(&mut app);
    app.harness.state_mut().core.set_user_setting(
        "editor.fontSize",
        Some(notoir_core::settings::SettingValue::Int(19)),
    );
    for id in ThemeId::ALL {
        action(&mut app, json!({"action":"select","id":id.id()}));
        edit(&mut app, &["ui", "accent"], json!("#123456"));
        edit(
            &mut app,
            &["appearance", "settings", "editor.fontSize"],
            json!(24),
        );
        edit(
            &mut app,
            &[
                "appearance",
                "extensions",
                "jsx",
                "settings",
                "editor.fontFamily",
            ],
            json!("Manrope"),
        );
        save(&mut app);
        action(&mut app, json!({"action":"reset","confirmed":true}));
        common::settle(&mut app);
        let restored = app.harness.state().core.load_theme(id.id());
        for token in Tok::ALL {
            assert_eq!(restored.style(*token), id.load().style(*token));
        }
        assert!(restored.appearance.settings.is_empty());
        assert!(restored.appearance.extensions.is_empty());
        assert!(
            !app.harness
                .state()
                .core
                .paths
                .themes
                .join("defaults")
                .join(format!("{}.toml", id.id()))
                .exists()
        );
    }
    assert_eq!(
        app.harness
            .state()
            .core
            .settings
            .int("editor.fontSize", None),
        19
    );
}

// spec 21: "Export and import complete theme presets"
#[test]
fn export_and_import_round_trip_without_overwriting() {
    let dialogs = std::sync::Arc::new(notoir::files::ScriptedDialogs::default());
    let configured = dialogs.clone();
    let mut app = common::start(move |o| o.dialogs = Some(configured));
    open(&mut app);
    edit(
        &mut app,
        &["appearance", "settings", "editor.fontFamily"],
        json!("Manrope"),
    );
    edit(
        &mut app,
        &[
            "appearance",
            "languages",
            "yaml",
            "settings",
            "editor.fontSize",
        ],
        json!(18),
    );
    edit(
        &mut app,
        &["appearance", "extensions", "yml", "colors", "syntax.string"],
        json!({"color":"#aabbcc","bold":true}),
    );
    save(&mut app);
    let exported = app.home.path().join("portable.toml");
    dialogs.answer_save(Some(exported.clone()));
    action(&mut app, json!({"action":"export"}));
    wait_manager(&mut app);
    let text = std::fs::read_to_string(&exported).unwrap();
    let preset = Theme::parse_custom(&text).unwrap();
    assert!(ThemeId::from_id(&preset.id).is_none());
    assert_eq!(
        preset.appearance.settings["editor.fontFamily"].as_str(),
        Some("Manrope")
    );
    dialogs.answer_open(vec![exported.clone()]);
    action(&mut app, json!({"action":"import"}));
    wait_manager(&mut app);
    let imported = app
        .harness
        .state()
        .core
        .theme_manager
        .draft
        .as_ref()
        .unwrap()
        .theme()
        .unwrap();
    assert_ne!(imported.id, preset.id);
    assert_eq!(
        imported
            .for_document(Some("yaml"), Some("a.yml"))
            .style(Tok::SyntaxString),
        preset
            .for_document(Some("yaml"), Some("a.yml"))
            .style(Tok::SyntaxString)
    );
    assert!(
        app.harness
            .state()
            .core
            .load_theme("notoir-midnight")
            .appearance
            .extensions
            .contains_key("yml")
    );
    dialogs.answer_open(vec![exported]);
    action(&mut app, json!({"action":"import"}));
    wait_manager(&mut app);
    assert_ne!(
        app.harness
            .state()
            .core
            .theme_manager
            .draft
            .as_ref()
            .unwrap()
            .id,
        imported.id
    );
}

fn wait_manager(app: &mut common::TestApp) {
    for _ in 0..80 {
        common::settle(app);
        if !app.harness.state().core.theme_manager.busy {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("theme operation did not finish");
}

// spec 21: "Language and extension rules resolve independently"
#[test]
fn language_extension_and_explicit_precedence() {
    let source = r##"
[meta]
id = "rules"
name = "Rules"
kind = "dark"
[appearance.settings]
"editor.fontSize" = 15
[appearance.languages.yaml.settings]
"editor.fontSize" = 17
[appearance.languages.yaml.colors]
"syntax.string" = "#112233"
[appearance.extensions.yml.settings]
"editor.fontFamily" = "Manrope"
[appearance.extensions.yml.colors]
"syntax.string" = "#445566"
[appearance.extensions.ts.settings]
"editor.fontSize" = 19
[appearance.extensions."d.ts".settings]
"editor.fontSize" = 21
"##;
    let theme = Theme::parse_custom(source).unwrap();
    assert_eq!(
        theme
            .for_document(Some("yaml"), Some("a.YML"))
            .color(Tok::SyntaxString),
        egui::Color32::from_rgb(68, 85, 102)
    );
    assert_eq!(
        theme
            .for_document(Some("yaml"), Some("a.yaml"))
            .color(Tok::SyntaxString),
        egui::Color32::from_rgb(17, 34, 51)
    );
    assert_eq!(
        theme.appearance.settings_layer(None, Some("a.d.ts")).values["editor.fontSize"],
        notoir_core::settings::SettingValue::Int(21)
    );
    let mut user = notoir_core::settings::SettingsLayer::default();
    user.set(
        "editor.fontSize",
        notoir_core::settings::SettingValue::Int(23),
    );
    let resolved = notoir_core::settings::ResolvedSettings::new(
        std::sync::Arc::new(notoir_core::settings::Schema::builtin()),
        user,
        None,
        false,
    )
    .with_theme_defaults(theme.appearance.settings_layer(Some("yaml"), Some("a.yml")));
    assert_eq!(resolved.int("editor.fontSize", Some("yaml")), 23);
    assert_eq!(resolved.str("editor.fontFamily", Some("yaml")), "Manrope");
    assert_eq!(
        theme.appearance.settings_layer(Some("yaml"), None).values["editor.fontSize"],
        notoir_core::settings::SettingValue::Int(17)
    );
}

// spec 21: "Protect dirty drafts and external theme edits"
#[test]
fn dirty_close_and_conflicting_save_keep_draft() {
    let mut app = common::start(|_| {});
    open(&mut app);
    edit(&mut app, &["ui", "accent"], json!("#abcdef"));
    let view = app.harness.state().core.workbench.active_view().unwrap();
    common::run(&mut app, "file.newUntitled", Value::Null);
    app.harness.state_mut().core.request_close(view);
    assert_eq!(app.harness.state().core.workbench.active_view(), Some(view));
    app.harness.run_steps(2);
    assert!(app.harness.state().core.workbench.view(view).is_some());
    assert!(app.harness.state().core.theme_manager.pending.is_some());
    app.harness.state_mut().core.theme_manager.pending = None;
    save(&mut app);
    let path = app
        .harness
        .state()
        .core
        .theme_manager
        .draft
        .as_ref()
        .unwrap()
        .path
        .clone()
        .unwrap();
    edit(&mut app, &["ui", "accent"], json!("#fedcba"));
    let external = std::fs::read_to_string(&path)
        .unwrap()
        .replace("#abcdef", "#123456");
    std::fs::write(&path, &external).unwrap();
    action(&mut app, json!({"action":"save"}));
    common::settle(&mut app);
    assert!(app.harness.state().core.theme_manager.conflict);
    assert!(
        app.harness
            .state()
            .core
            .theme_manager
            .draft
            .as_ref()
            .unwrap()
            .dirty()
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), external);
    action(&mut app, json!({"action":"save","overwrite":true}));
    common::settle(&mut app);
    assert!(app.harness.state().core.theme_manager.error.is_none());
    assert!(std::fs::read_to_string(path).unwrap().contains("#fedcba"));
}

#[test]
fn invalid_values_and_duplicate_extensions_are_rejected() {
    let head = "[meta]\nid='rules'\nname='Rules'\nkind='dark'\n";
    for tail in [
        "[appearance.settings]\n'editor.fontSize'=999\n",
        "[appearance.extensions.yml.settings]\n'workbench.fontSize'=14\n",
        "[appearance.extensions.yml.colors]\n'accent'='#ffffff'\n",
        "[appearance.extensions.yml.settings]\n'editor.fontSize'=14\n[appearance.extensions.'.YML'.settings]\n'editor.fontSize'=16\n",
        "[appearance.extensions.'*.jsx'.settings]\n'editor.fontSize'=14\n",
    ] {
        assert!(
            Theme::parse_custom(&format!("{head}{tail}")).is_err(),
            "{tail}"
        );
    }
}

#[test]
fn editing_preserves_comments_unknown_fields_and_derivations() {
    let source = "# My theme\n[meta]\nid='mine'\nname='Mine' # keep\nkind='dark'\n[ui]\naccent='#123456'\n'accent.soft'={base='accent',alpha=0.14}\n[future]\ncustom=true\n";
    let edited =
        theme_manager::edit_source(source, &json!({"path":["meta","name"],"value":"Renamed"}))
            .unwrap();
    assert!(edited.contains("# My theme"));
    assert!(edited.contains("# keep"));
    assert!(edited.contains("custom=true"));
    assert!(edited.contains("base='accent'"));
    assert_eq!(Theme::parse_custom(&edited).unwrap().name, "Renamed");
}

// spec 21: "Proportional fonts preserve caret and click geometry"
#[test]
fn proportional_font_carets_clicks_and_vertical_motion() {
    let mut app = common::start(|_| {});
    common::run(&mut app, "file.newUntitled", Value::Null);
    app.harness
        .event(egui::Event::Paste("Wiii\nWWWW\ne\u{301}日\n".into()));
    app.harness.run_steps(2);
    app.harness.state_mut().core.set_user_setting(
        "editor.fontFamily",
        Some(notoir_core::settings::SettingValue::Str("Manrope".into())),
    );
    let view = app.harness.state().core.workbench.active_view().unwrap();
    let mut positions = Vec::new();
    for pos in [0, 1, 2, 3, 4] {
        app.harness
            .state_mut()
            .core
            .workbench
            .set_selection(view, notoir_editor::Selection::caret(pos))
            .unwrap();
        app.harness.run_steps(2);
        positions.push(app.harness.state().ui.editors.caret_rect(view).unwrap());
    }
    assert!(
        positions[1].left() - positions[0].left()
            > (positions[2].left() - positions[1].left()) * 1.5
    );
    app.harness
        .event(egui::Event::PointerMoved(positions[2].center()));
    app.harness.event(egui::Event::PointerButton {
        pos: positions[2].center(),
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: Default::default(),
    });
    app.harness.event(egui::Event::PointerButton {
        pos: positions[2].center(),
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Default::default(),
    });
    app.harness.run_steps(2);
    assert_eq!(
        app.harness
            .state()
            .core
            .workbench
            .view(view)
            .unwrap()
            .selection
            .primary()
            .head,
        2
    );
    app.harness.key_press(egui::Key::ArrowDown);
    app.harness.run_steps(2);
    let down = app
        .harness
        .state()
        .core
        .workbench
        .view(view)
        .unwrap()
        .selection
        .primary()
        .head;
    assert!(matches!(down, 6 | 7));
    app.harness.key_press(egui::Key::ArrowUp);
    app.harness.run_steps(2);
    assert_eq!(
        app.harness
            .state()
            .core
            .workbench
            .view(view)
            .unwrap()
            .selection
            .primary()
            .head,
        2
    );
}

#[test]
fn file_rules_can_be_added_edited_and_removed_through_commands() {
    let mut app = common::start(|_| {});
    open(&mut app);
    action(&mut app, json!({"action":"addRule","key":".YML"}));
    let draft = app
        .harness
        .state()
        .core
        .theme_manager
        .draft
        .as_ref()
        .unwrap();
    assert!(
        draft
            .theme()
            .unwrap()
            .appearance
            .extensions
            .contains_key("yml"),
        "{}",
        draft.source
    );
    action(&mut app, json!({"action":"addRule","key":"yml"}));
    assert!(app.harness.state().core.theme_manager.error.is_some());
    edit(
        &mut app,
        &[
            "appearance",
            "extensions",
            "yml",
            "settings",
            "editor.fontSize",
        ],
        json!(20),
    );
    assert_eq!(
        app.harness
            .state()
            .core
            .theme_manager
            .draft
            .as_ref()
            .unwrap()
            .theme()
            .unwrap()
            .appearance
            .settings_layer(Some("yaml"), Some("a.yml"))
            .values["editor.fontSize"],
        notoir_core::settings::SettingValue::Int(20)
    );
    action(&mut app, json!({"action":"removeRule"}));
    assert!(
        app.harness
            .state()
            .core
            .theme_manager
            .draft
            .as_ref()
            .unwrap()
            .theme()
            .unwrap()
            .appearance
            .extensions
            .is_empty()
    );
}

#[test]
fn color_reset_button_restores_inherited_builtin_value() {
    let mut app = common::start(|_| {});
    open(&mut app);
    edit(&mut app, &["ui", "bg.app"], json!("#123456"));
    app.harness
        .query_all_by_label("Reset")
        .next()
        .unwrap()
        .click();
    app.harness.run_steps(3);
    let draft = app
        .harness
        .state()
        .core
        .theme_manager
        .draft
        .as_ref()
        .unwrap();
    assert_eq!(
        draft.theme().unwrap().color(Tok::BgApp),
        ThemeId::Midnight.load().color(Tok::BgApp)
    );
}

#[test]
fn create_rename_activate_and_delete_a_custom_preset() {
    let mut app = common::start(|_| {});
    open(&mut app);
    action(&mut app, json!({"action":"create"}));
    edit(&mut app, &["meta", "name"], json!("Personal"));
    edit(
        &mut app,
        &["appearance", "settings", "editor.fontSize"],
        json!(18),
    );
    save(&mut app);
    let id = app
        .harness
        .state()
        .core
        .theme_manager
        .draft
        .as_ref()
        .unwrap()
        .id
        .clone();
    let path = app
        .harness
        .state()
        .core
        .theme_manager
        .draft
        .as_ref()
        .unwrap()
        .path
        .clone()
        .unwrap();
    action(&mut app, json!({"action":"use"}));
    assert_eq!(app.harness.state().theme_id(), id);
    assert_eq!(
        app.harness
            .state()
            .core
            .settings
            .int("editor.fontSize", None),
        18
    );
    edit(&mut app, &["meta", "name"], json!("Renamed"));
    save(&mut app);
    assert_eq!(app.harness.state().core.load_theme(&id).name, "Renamed");
    assert_eq!(
        app.harness
            .state()
            .core
            .theme_manager
            .draft
            .as_ref()
            .unwrap()
            .id,
        id
    );
    action(&mut app, json!({"action":"delete","confirmed":true}));
    common::settle(&mut app);
    assert_eq!(app.harness.state().theme_id(), "notoir-midnight");
    assert!(!path.exists());
    assert!(
        !app.harness
            .state()
            .core
            .custom_themes
            .iter()
            .any(|theme| theme.id == id)
    );
}

#[test]
fn failed_save_and_invalid_import_preserve_the_draft() {
    let dialogs = std::sync::Arc::new(notoir::files::ScriptedDialogs::default());
    let configured = dialogs.clone();
    let mut app = common::start(move |o| o.dialogs = Some(configured));
    open(&mut app);
    action(&mut app, json!({"action":"create"}));
    let source = app
        .harness
        .state()
        .core
        .theme_manager
        .draft
        .as_ref()
        .unwrap()
        .source
        .clone();
    let directory = app.home.path().join("directory-not-a-theme-file");
    std::fs::create_dir(&directory).unwrap();
    app.harness
        .state_mut()
        .core
        .theme_manager
        .draft
        .as_mut()
        .unwrap()
        .path = Some(directory);
    action(&mut app, json!({"action":"save"}));
    common::settle(&mut app);
    assert!(app.harness.state().core.theme_manager.error.is_some());
    assert!(
        app.harness
            .state()
            .core
            .theme_manager
            .draft
            .as_ref()
            .unwrap()
            .dirty()
    );
    assert_eq!(
        app.harness
            .state()
            .core
            .theme_manager
            .draft
            .as_ref()
            .unwrap()
            .source,
        source
    );
    let invalid = app.home.path().join("invalid.toml");
    std::fs::write(&invalid, "[meta\nnot a theme").unwrap();
    dialogs.answer_open(vec![invalid]);
    action(&mut app, json!({"action":"import","discard":true}));
    wait_manager(&mut app);
    assert!(app.harness.state().core.theme_manager.error.is_some());
    assert!(app.harness.state().core.custom_themes.is_empty());
}

#[test]
fn imported_inline_and_mixed_case_extension_rules_remain_editable() {
    let source = "appearance={extensions={'.YML'={settings={'editor.fontSize'=18}}}}\n[meta]\nid='mine'\nname='Mine'\nkind='dark'\n";
    let edited = theme_manager::edit_source(source,&json!({"path":["appearance","extensions","yml","colors","syntax.string"],"value":"#123456"})).unwrap();
    let theme = Theme::parse_custom(&edited).unwrap();
    assert_eq!(
        theme
            .for_document(Some("yaml"), Some("a.yml"))
            .color(Tok::SyntaxString),
        egui::Color32::from_rgb(18, 52, 86)
    );
    let removed = theme_manager::edit_source(
        &edited,
        &json!({"path":["appearance","extensions","yml"],"value":null}),
    )
    .unwrap();
    assert!(
        Theme::parse_custom(&removed)
            .unwrap()
            .appearance
            .extensions
            .is_empty()
    );
}
