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

#[test]
fn split_layout_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let mut files = Vec::new();
    for (name, text) in [
        (
            "README.md",
            "# Notoir\n\nA text editor.\n\n- splits\n- tabs\n",
        ),
        ("config.json", "{\n  \"theme\": \"midnight\"\n}\n"),
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
    common::run(&mut app, "view.splitRight", serde_json::Value::Null);
    common::run(
        &mut app,
        "workbench.moveTabToNextGroup",
        serde_json::Value::Null,
    );
    common::run(&mut app, "view.splitDown", serde_json::Value::Null);
    if let Ok(img) = app.harness.render() {
        img.save(out_dir().join("splits.png")).unwrap();
    }
}

#[test]
fn explorer_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("my-notes");
    for (name, text) in [
        ("README.md", "# My notes\n\nIdeas and journal.\n"),
        ("journal/2026-10-14.md", "# Tuesday\n\n- shipped splits\n"),
        ("journal/2026-10-15.md", "# Wednesday\n"),
        ("ideas/editor.md", "x"),
        ("config.json", "{}\n"),
        ("build/out.txt", "x"),
        (".gitignore", "build/\n"),
        ("todo.txt", "milk\n"),
    ] {
        let p = root.join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, text).unwrap();
    }
    let root = root.canonicalize().unwrap();
    let open = root.join("journal/2026-10-14.md");
    let r = root.clone();
    let mut app = common::start(move |o| {
        o.language_override = Some(Language::En);
        o.theme_override = Some(ThemeId::Midnight);
        o.open_paths = vec![(r, None), (open, None)];
    });
    common::settle(&mut app);
    common::run(
        &mut app,
        "explorer.newFile",
        serde_json::json!({ "path": root.join("ideas").to_string_lossy() }),
    );
    common::settle(&mut app);
    app.harness.event(egui::Event::Text("plan.md".into()));
    app.harness.run_steps(3);
    if let Ok(img) = app.harness.render() {
        img.save(out_dir().join("explorer.png")).unwrap();
    }
    app.harness.key_press(egui::Key::Escape);
    app.harness.hover_at(egui::pos2(160.0, 500.0));
    app.harness.run_steps(3);
    if let Ok(img) = app.harness.render() {
        img.save(out_dir().join("explorer-hover.png")).unwrap();
    }
}

#[test]
fn recovery_prompt_and_banner_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("meeting-notes.md");
    std::fs::write(&a, "# Meeting\n\n- agenda\n").unwrap();
    let a = a.canonicalize().unwrap();
    let theme = |o: &mut notoir::core::StartOptions| {
        o.language_override = Some(Language::En);
        o.theme_override = Some(ThemeId::Midnight);
    };
    let path = a.clone();
    let mut app = common::start(move |o| {
        theme(o);
        o.open_paths = vec![(path, None)];
    });
    common::settle(&mut app);
    app.harness
        .event(egui::Event::Text("Decisions: ship M8\n".into()));
    common::run(&mut app, "file.newUntitled", serde_json::Value::Null);
    app.harness
        .event(egui::Event::Text("todo: call Ana\nbuy milk\n".into()));
    common::run(&mut app, "recovery.flush", serde_json::Value::Null);
    let mut app = common::crash(app, theme);
    std::fs::write(&a, "# Meeting\n\nchanged elsewhere\n").unwrap();
    common::settle(&mut app);
    if let Ok(img) = app.harness.render() {
        img.save(out_dir().join("recovery-prompt.png")).unwrap();
    }
    common::run(&mut app, "recovery.restoreAll", serde_json::Value::Null);
    common::settle(&mut app);
    let view = {
        let core = &app.harness.state().core;
        let doc = core
            .workbench
            .documents
            .values()
            .find(|d| d.path().is_some())
            .unwrap()
            .id;
        core.workbench.views.of_document(doc).next().unwrap().id
    };
    common::run(
        &mut app,
        "view.activate",
        serde_json::json!({ "view": view.0 }),
    );
    if let Ok(img) = app.harness.render() {
        img.save(out_dir().join("banner-recovered.png")).unwrap();
    }
}

#[test]
fn find_minimap_and_palette_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let body: String = (1..=400)
        .map(|i| match i % 7 {
            0 => format!("## Section {i}\n"),
            1 => "\n".to_owned(),
            2 => format!("- item {i}: review the notes and the draft\n"),
            3 => format!("    nested detail for note {i}\n"),
            _ => format!("Paragraph {i} mentions the draft and a few other words.\n"),
        })
        .collect();
    let p = dir.path().join("notes.md");
    std::fs::write(&p, body).unwrap();
    let mut app = common::start(move |o| {
        o.language_override = Some(Language::En);
        o.theme_override = Some(ThemeId::Midnight);
        o.open_paths = vec![(p, None)];
    });
    common::settle(&mut app);
    common::run(&mut app, "edit.replace", serde_json::Value::Null);
    app.harness.event(egui::Event::Text("draft".into()));
    app.harness.run_steps(3);
    if let Ok(img) = app.harness.render() {
        img.save(out_dir().join("find-minimap.png")).unwrap();
    }
    app.harness.key_press(egui::Key::Escape);
    app.harness.run_steps(2);
    common::run(
        &mut app,
        "workbench.commandPalette",
        serde_json::Value::Null,
    );
    app.harness.event(egui::Event::Text("tog".into()));
    app.harness.run_steps(3);
    if let Ok(img) = app.harness.render() {
        img.save(out_dir().join("palette.png")).unwrap();
    }
}

#[test]
fn settings_welcome_zen_snapshot() {
    let theme = |o: &mut notoir::core::StartOptions| {
        o.language_override = Some(Language::En);
        o.theme_override = Some(ThemeId::Midnight);
    };
    let mut app = common::start(move |o| {
        theme(o);
        o.first_run = true;
    });
    common::settle(&mut app);
    if let Ok(img) = app.harness.render() {
        img.save(out_dir().join("welcome.png")).unwrap();
    }
    common::run(&mut app, "welcome.finish", serde_json::Value::Null);
    common::run(
        &mut app,
        "app.settings",
        serde_json::json!({ "category": "editor" }),
    );
    app.harness.state_mut().core.set_user_setting(
        "editor.fontSize",
        Some(notoir_core::settings::SettingValue::Int(15)),
    );
    app.harness.run_steps(3);
    if let Ok(img) = app.harness.render() {
        img.save(out_dir().join("settings.png")).unwrap();
    }
    common::run(
        &mut app,
        "preferences.keyboardShortcuts",
        serde_json::Value::Null,
    );
    app.harness.run_steps(3);
    if let Ok(img) = app.harness.render() {
        img.save(out_dir().join("shortcuts.png")).unwrap();
    }
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("essay.md");
    let text: String = (0..30).map(|i| format!("Paragraph {i}. Writing without distractions keeps the focus on the words and nothing else.\n\n")).collect();
    std::fs::write(&p, text).unwrap();
    app.harness
        .state_mut()
        .core
        .open_path(&p, Default::default());
    common::settle(&mut app);
    common::run(&mut app, "view.zenMode", serde_json::Value::Null);
    if let Ok(img) = app.harness.render() {
        img.save(out_dir().join("zen.png")).unwrap();
    }
}

#[test]
fn syntax_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("main.rs");
    std::fs::write(
        &p,
        "//! Notoir syntax colours.\n\nuse std::collections::HashMap;\n\n/// A counted word.\n#[derive(Debug, Clone)]\npub struct Word {\n    text: String,\n    count: usize,\n}\n\nfn main() {\n    let mut words: HashMap<&str, usize> = HashMap::new();\n    for w in \"the quick brown fox\".split(' ') {\n        *words.entry(w).or_insert(0) += 1;\n    }\n    println!(\"{} words, max {:?}\", words.len(), words.values().max());\n    let ratio = 3.14_f64 * 2.0; // inline comment\n    if ratio > 6.0 && !words.is_empty() { return; }\n}\n",
    )
    .unwrap();
    for theme in [ThemeId::Midnight, ThemeId::Paper, ThemeId::Dracula] {
        let p = p.clone();
        let mut app = common::start(move |o| {
            o.language_override = Some(Language::En);
            o.theme_override = Some(theme);
            o.open_paths = vec![(p, None)];
        });
        common::settle(&mut app);
        if let Ok(img) = app.harness.render() {
            img.save(out_dir().join(format!("syntax-{}.png", theme.id())))
                .unwrap();
        }
    }
}

#[test]
fn markdown_split_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let png = dir.path().join("logo.png");
    image::RgbaImage::from_fn(120, 40, |x, _| image::Rgba([60 + (x as u8), 110, 220, 255]))
        .save(&png)
        .unwrap();
    let p = dir.path().join("README.md");
    std::fs::write(
        &p,
        "# Notoir\n\nA **fast**, *native* editor with `code`, ~~noise~~ and [links](https://notoir.dev).\n\n![logo](logo.png)\n\n## Tasks\n\n- [x] Splits\n- [ ] Markdown preview\n  - nested item\n\n> Plain files + Git + MCP.\n\n| Feature | Status |\n|:--|--:|\n| Recovery | done |\n| Preview | new |\n\n```rust\nfn main() {\n    println!(\"hi\");\n}\n```\n\n---\n\n1. one\n2. two\n",
    )
    .unwrap();
    let mut app = common::start(move |o| {
        o.language_override = Some(Language::En);
        o.theme_override = Some(ThemeId::Midnight);
        o.open_paths = vec![(p, None)];
    });
    common::settle(&mut app);
    common::run(&mut app, "markdown.showSplit", serde_json::Value::Null);
    common::settle(&mut app);
    app.harness.run_steps(3);
    if let Ok(img) = app.harness.render() {
        img.save(out_dir().join("markdown-split.png")).unwrap();
    }
}
