//! Renders key screens to PNG (GPU via wgpu) for visual review: every
//! built-in theme × (en, ja, de). Output goes to `$NOTOIR_SNAPSHOT_DIR`
//! (defaults to `target/ui-snapshots`). Skips without a GPU adapter.

mod common;

use notoir::i18n::Language;
use notoir::theme::ThemeId;

fn out_dir() -> std::path::PathBuf {
    let dir = std::env::var("NOTOIR_SNAPSHOT_DIR")
        .map(Into::into)
        .unwrap_or_else(|_| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/ui-snapshots")
        });
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn empty_state_snapshots() {
    for theme in ThemeId::ALL {
        for lang in [Language::En, Language::Ja, Language::De] {
            let mut app = common::start(common::with_language(lang, theme));
            let name = format!("shell-{}-{}.png", theme.id(), lang.code());
            match app.harness.render() {
                Ok(img) => img.save(out_dir().join(name)).unwrap(),
                Err(e) => {
                    eprintln!("render skipped (no GPU adapter?): {e}");
                    return;
                }
            }
        }
    }
}

#[test]
fn overlay_snapshots() {
    let mut app = common::start(common::with_language(Language::En, ThemeId::Midnight));
    egui_kittest::kittest::Queryable::get_by_label(&app.harness, "View").click();
    app.harness.run_steps(3);
    egui_kittest::kittest::Queryable::get_by_label_contains(&app.harness, "Appearance").hover();
    app.harness.run_steps(3);
    if let Ok(img) = app.harness.render() {
        img.save(out_dir().join("shell-menu-view.png")).unwrap();
    }
    app.harness.key_press(egui::Key::Escape);
    common::run(
        &mut app,
        "preferences.changeLanguage",
        serde_json::Value::Null,
    );
    if let Ok(img) = app.harness.render() {
        img.save(out_dir().join("shell-picker-language.png"))
            .unwrap();
    }
}
