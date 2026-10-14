//! App shell (specs 00, 16, 17, 21, 22): menus, panels, theme and language.

mod common;

use egui_kittest::kittest::Queryable;
use notoir::i18n::Language;
use notoir::theme::{ThemeId, Tok};
use serde_json::{Value, json};

fn explorer_header_shown(app: &common::TestApp) -> bool {
    app.harness.query_by_label("EXPLORER").is_some()
}

// spec 00: "Fresh install shows only the editor surface"
#[test]
fn fresh_install_shows_only_the_editor_surface() {
    let app = common::start(|_| {});
    let h = &app.harness;
    for menu in ["File", "Edit", "View", "Help"] {
        assert!(h.query_by_label(menu).is_some(), "menu {menu}");
    }
    assert!(h.query_by_label("Start writing.").is_some());
    assert!(explorer_header_shown(&app));
    assert!(
        h.query_by_label("Notifications").is_some(),
        "status bar bell"
    );
    let core = &h.state().core;
    assert!(core.toolbar_visible() && core.status_bar_visible());
    assert!(!core.panels.secondary.visible);
    assert!(!core.panels.bottom.visible);
    assert!(h.query_by_label("OUTPUT").is_none());
}

// spec 17: "Clicking the active activity icon collapses the sidebar"
#[test]
fn clicking_active_activity_icon_collapses_sidebar() {
    let mut app = common::start(|_| {});
    app.harness.get_by_label("Explorer").click();
    app.harness.run_steps(3);
    assert!(!explorer_header_shown(&app));
    app.harness.get_by_label("Explorer").click();
    app.harness.run_steps(3);
    assert!(explorer_header_shown(&app));
}

// spec 17: "Primary sidebar on the right"
#[test]
fn primary_sidebar_moves_to_the_right() {
    let mut app = common::start(|_| {});
    let width = 1280.0;
    assert!(app.harness.get_by_label("EXPLORER").rect().center().x < width / 2.0);
    common::run(&mut app, "view.togglePrimarySidebarPosition", Value::Null);
    assert!(app.harness.get_by_label("EXPLORER").rect().center().x > width / 2.0);
    assert!(
        app.harness.get_by_label("Explorer").rect().center().x > width / 2.0,
        "activity bar follows"
    );
}

#[test]
fn bottom_panel_toggles_with_shortcut() {
    let mut app = common::start(|_| {});
    let ctrl = egui::Modifiers {
        ctrl: true,
        command: true,
        ..Default::default()
    };
    app.harness.key_press_modifiers(ctrl, egui::Key::J);
    app.harness.run_steps(3);
    assert!(app.harness.query_by_label("OUTPUT").is_some());
    app.harness.key_press_modifiers(ctrl, egui::Key::B);
    app.harness.run_steps(3);
    assert!(!explorer_header_shown(&app));
}

// spec 21: "Switch theme instantly"
#[test]
fn switch_theme_instantly() {
    let mut app = common::start(|_| {});
    assert_eq!(app.harness.state().theme_id(), ThemeId::Midnight);
    common::run(
        &mut app,
        "preferences.changeTheme",
        json!({ "theme": "notoir-paper" }),
    );
    assert_eq!(app.harness.state().theme_id(), ThemeId::Paper);
    let paper = ThemeId::Paper.load();
    // The theme is applied to both egui style slots.
    let fill = app
        .harness
        .ctx
        .style_of(egui::Theme::Dark)
        .visuals
        .panel_fill;
    assert_eq!(fill, paper.color(Tok::BgPanel));
    app.harness.state().core.flush_io();
    let text = std::fs::read_to_string(app.home.path().join("config/settings.toml")).unwrap();
    assert!(text.contains("theme = \"notoir-paper\""), "{text}");
}

// spec 16: "Theme picker previews live"
#[test]
fn theme_picker_previews_live_and_escape_restores() {
    let mut app = common::start(|_| {});
    common::run(&mut app, "preferences.changeTheme", Value::Null);
    assert!(app.harness.query_by_label("SELECT COLOR THEME").is_some());
    app.harness.key_press(egui::Key::ArrowDown);
    app.harness.run_steps(3);
    assert_eq!(app.harness.state().theme_id(), ThemeId::Paper, "preview");
    app.harness.key_press(egui::Key::Escape);
    app.harness.run_steps(3);
    assert_eq!(
        app.harness.state().theme_id(),
        ThemeId::Midnight,
        "restored"
    );
    assert!(app.harness.state().ui.picker.is_none());
}

// spec 22: "Pickers list every supported language with native names"
#[test]
fn language_picker_lists_every_language() {
    let mut app = common::start(|_| {});
    common::run(&mut app, "preferences.changeLanguage", Value::Null);
    for lang in Language::ALL {
        assert!(
            app.harness
                .query_by_label_contains(lang.native_label())
                .is_some(),
            "{}",
            lang.native_label()
        );
    }
}

// spec 22: "Switching applies immediately"
#[test]
fn switching_language_applies_immediately() {
    let mut app = common::start(|_| {});
    common::run(
        &mut app,
        "preferences.changeLanguage",
        json!({ "language": "de" }),
    );
    assert!(app.harness.query_by_label("Datei").is_some());
    assert!(
        app.harness
            .query_by_label("Einfach losschreiben.")
            .is_some()
    );
    assert!(app.harness.query_by_label("File").is_none());
}

// spec 22: "Choice persists"
#[test]
fn language_choice_persists() {
    let mut app = common::start(|_| {});
    common::run(
        &mut app,
        "preferences.changeLanguage",
        json!({ "language": "pt-BR" }),
    );
    let app = common::restart(app, |_| {});
    assert_eq!(app.harness.state().core.language, Language::PtBr);
    assert!(app.harness.query_by_label("Arquivo").is_some());
}

// spec 22: "First launch detects OS locale"
#[test]
fn first_launch_detects_os_locale_without_persisting() {
    let app = common::start(|o| o.os_locale = Some("ja-JP".into()));
    assert_eq!(app.harness.state().core.language, Language::Ja);
    assert!(app.harness.query_by_label("ファイル").is_some());
    app.harness.state().core.flush_io();
    let settings =
        std::fs::read_to_string(app.home.path().join("config/settings.toml")).unwrap_or_default();
    assert!(!settings.contains("language"), "{settings}");
}

#[test]
fn settings_parse_error_is_reported_at_startup() {
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join("config")).unwrap();
    std::fs::write(
        home.path().join("config/settings.toml"),
        "[editor\nfontSize = 3\n",
    )
    .unwrap();
    let app = common::start_in(home, |_| {});
    assert!(
        app.harness
            .query_by_label_contains("Your settings file has an error on line 1")
            .is_some()
    );
}

#[test]
fn about_dialog_shows_version() {
    let mut app = common::start(|_| {});
    common::run(&mut app, "help.about", Value::Null);
    let version = format!("Version {}", env!("CARGO_PKG_VERSION"));
    assert!(app.harness.query_by_label(&version).is_some());
}
