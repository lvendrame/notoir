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

#[test]
fn editor_snapshots() {
    for theme in [ThemeId::Midnight, ThemeId::Paper] {
        let mut app = common::start(common::with_language(Language::En, theme));
        let ctrl = egui::Modifiers {
            ctrl: true,
            command: true,
            ..Default::default()
        };
        app.harness.key_press_modifiers(ctrl, egui::Key::N);
        app.harness.run_steps(2);
        let sample = "# Notoir\n\nA fast, native text editor.\n\n\tfn main() {\n\t    println!(\"hello\");   \n\t}\n\n日本語のテキストも表示できます。\nThis is a long line that keeps going so that word wrap has something to do when the window is not wide enough to show it all at once, which is the point of this sample.\n";
        app.harness.event(egui::Event::Paste(sample.into()));
        app.harness.run_steps(2);
        app.harness
            .key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::ArrowUp);
        app.harness
            .key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::ArrowUp);
        app.harness.run_steps(3);
        if let Ok(img) = app.harness.render() {
            img.save(out_dir().join(format!("editor-{}.png", theme.id())))
                .unwrap();
        }
    }
}

#[test]
fn tabs_and_prompt_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let mut files = Vec::new();
    for (name, text) in [
        ("README.md", "# Notoir\n\nNotes.\n"),
        ("config.json", "{\n  \"a\": 1\n}\n"),
        ("todo.txt", "milk\n"),
    ] {
        let p = dir.path().join(name);
        std::fs::write(&p, text).unwrap();
        files.push((p, None));
    }
    let mut app = common::start(move |o| {
        o.language_override = Some(Language::En);
        o.theme_override = Some(ThemeId::Midnight);
        o.open_paths = files;
    });
    common::settle(&mut app);
    app.harness.event(egui::Event::Text("edited ".into()));
    app.harness.run_steps(2);
    common::run(&mut app, "file.newScratchpad", serde_json::Value::Null);
    let first = app.harness.state().core.workbench.layout.active().tabs[0];
    common::run(
        &mut app,
        "view.togglePin",
        serde_json::json!({ "view": first.0 }),
    );
    if let Ok(img) = app.harness.render() {
        img.save(out_dir().join("tabs.png")).unwrap();
    }
    let dirty = app.harness.state().core.workbench.layout.active().tabs[2];
    common::run(
        &mut app,
        "file.close",
        serde_json::json!({ "view": dirty.0 }),
    );
    if let Ok(img) = app.harness.render() {
        img.save(out_dir().join("prompt-unsaved.png")).unwrap();
    }
}
