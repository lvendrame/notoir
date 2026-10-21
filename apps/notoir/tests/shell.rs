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
    assert_eq!(app.harness.state().theme_id(), ThemeId::Midnight.id());
    common::run(
        &mut app,
        "preferences.changeTheme",
        json!({ "theme": "notoir-paper" }),
    );
    assert_eq!(app.harness.state().theme_id(), ThemeId::Paper.id());
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
    assert_eq!(
        app.harness.state().theme_id(),
        ThemeId::Paper.id(),
        "preview"
    );
    app.harness.key_press(egui::Key::Escape);
    app.harness.run_steps(3);
    assert_eq!(
        app.harness.state().theme_id(),
        ThemeId::Midnight.id(),
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

// spec 21: "Custom theme inherits missing tokens"
#[test]
fn custom_theme_inherits_missing_tokens_and_reloads() {
    let home = tempfile::tempdir().unwrap();
    let themes = home.path().join("config/themes");
    std::fs::create_dir_all(&themes).unwrap();
    let theme_file = themes.join("deep-sea.toml");
    std::fs::write(
        &theme_file,
        "[meta]\nid = \"deep-sea\"\nname = \"Deep Sea\"\nkind = \"dark\"\n[ui]\n\"bg.editor\" = \"#001122\"\n",
    )
    .unwrap();
    std::fs::write(themes.join("broken.toml"), "[meta]\nid = 3\n").unwrap();
    let mut app = common::start_in(home, |_| {});
    common::settle(&mut app);
    assert!(
        app.harness
            .state()
            .core
            .notifications
            .history()
            .any(|n| n.message.key == "notifications:themeInvalid"),
        "invalid theme reported"
    );
    // Listed in the theme picker.
    common::run(&mut app, "preferences.changeTheme", Value::Null);
    assert!(app.harness.query_by_label_contains("Deep Sea").is_some());
    app.harness.key_press(egui::Key::Escape);
    app.harness.run_steps(3);

    common::run(
        &mut app,
        "preferences.changeTheme",
        json!({ "theme": "deep-sea" }),
    );
    let midnight = ThemeId::Midnight.load();
    {
        let state = app.harness.state();
        assert_eq!(state.theme_id(), "deep-sea");
        let theme = state.theme();
        assert_eq!(
            theme.color(Tok::BgEditor),
            egui::Color32::from_rgb(0, 0x11, 0x22)
        );
        // Every other token comes from Notoir Midnight.
        for tok in Tok::ALL.iter().filter(|t| **t != Tok::BgEditor) {
            assert_eq!(theme.style(*tok), midnight.style(*tok), "{}", tok.name());
        }
    }
    // Editing the file applies at once.
    std::fs::write(
        &theme_file,
        "[meta]\nid = \"deep-sea\"\nname = \"Deep Sea\"\nkind = \"dark\"\n[ui]\n\"bg.editor\" = \"#002233\"\n",
    )
    .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let want = egui::Color32::from_rgb(0, 0x22, 0x33);
    while app.harness.state().theme().color(Tok::BgEditor) != want
        && std::time::Instant::now() < deadline
    {
        common::settle(&mut app);
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert_eq!(app.harness.state().theme().color(Tok::BgEditor), want);
}

// spec 21: "Auto theme follows OS"
#[test]
fn auto_theme_follows_os() {
    let mut app = common::start(|_| {});
    app.harness.input_mut().system_theme = Some(egui::Theme::Dark);
    common::run(
        &mut app,
        "preferences.changeTheme",
        json!({ "theme": "auto" }),
    );
    assert_eq!(app.harness.state().theme_id(), ThemeId::Midnight.id());
    // The OS switches to light mode.
    app.harness.input_mut().system_theme = Some(egui::Theme::Light);
    app.harness.run_steps(3);
    assert_eq!(app.harness.state().theme_id(), ThemeId::Paper.id());
    let paper = ThemeId::Paper.load();
    assert_eq!(
        app.harness.state().theme().color(Tok::BgEditor),
        paper.color(Tok::BgEditor)
    );
}

#[test]
fn explorer_is_first_after_restoring_old_panel_order() {
    use notoir_core::panels::PanelId;
    let mut app = common::start(|_| {});
    {
        let core = &mut app.harness.state_mut().core;
        core.panels
            .primary
            .panels
            .retain(|p| *p != PanelId::Explorer);
        core.panels.primary.panels.push(PanelId::Explorer);
        core.panels
            .ensure_registered(notoir::core::REGISTERED_PANELS);
    }
    app.harness.run_steps(3);
    for label in ["Search", "Source Control"] {
        assert!(
            app.harness.get_by_label("Explorer").rect().top()
                < app.harness.get_by_label(label).rect().top()
        );
    }
}
