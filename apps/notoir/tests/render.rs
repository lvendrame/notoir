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

/// Content wider than the preview column: a wide table, a long code line,
/// long unbroken words and links, in the split and in the preview alone.
const WIDE_MD: &str = "# Wide content\n\nA paragraph with a very long link [https://example.com/a/really/long/path/that/goes/on/and/on/without/any/spaces/at/all/index.html](https://example.com) and Supercalifragilisticexpialidociouswordthatkeepsgoingandgoingwithoutanybreakpointatall in the middle of the text so it has to wrap somewhere sensible.\n\n| Name | Description | Owner | Status | Notes | Created | Updated | Priority |\n|---|---|---|---|---|---|---|---|\n| Recovery | Hot exit keeps unsaved buffers across restarts and crashes | core team | done | Covers scratchpads and untitled files too | 2025-01-10 | 2026-10-01 | high |\n| Preview | Markdown preview with tables, images, wiki links and task lists | editor | new | Wide tables must stay readable at any width | 2025-03-02 | 2026-10-16 | medium |\n\n```rust\nfn main() { let message = \"this line of code is much longer than the preview column and must not be cut off\"; println!(\"{message}\"); }\n```\n\nThe end.\n";

#[test]
fn markdown_wide_content_snapshot() {
    for (name, command, width) in [
        ("markdown-wide-split", "markdown.showSplit", 1280.0),
        ("markdown-wide-preview", "markdown.showPreview", 1280.0),
        // Narrower than the minimum column: the preview scrolls sideways.
        ("markdown-wide-narrow", "markdown.showPreview", 560.0),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("wide.md");
        std::fs::write(&p, WIDE_MD).unwrap();
        let mut app = common::start(move |o| {
            o.language_override = Some(Language::En);
            o.theme_override = Some(ThemeId::Midnight);
            o.open_paths = vec![(p, None)];
        });
        common::settle(&mut app);
        common::run(&mut app, command, serde_json::Value::Null);
        app.harness.set_size(egui::vec2(width, 800.0));
        common::settle(&mut app);
        app.harness.run_steps(3);
        if let Ok(img) = app.harness.render() {
            img.save(out_dir().join(format!("{name}.png"))).unwrap();
        }
    }
}

#[test]
fn json_split_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("package.json");
    std::fs::write(
        &p,
        "{\n  \"name\": \"notoir\",\n  \"version\": \"0.1.0\",\n  \"private\": true,\n  \"user\": {\n    \"name\": \"Ana\",\n    \"age\": 42,\n    \"address\": { \"city\": \"Porto\", \"zip\": null }\n  },\n  \"tags\": [\"editor\", \"native\", \"fast\"],\n  \"ratio\": 3.14\n}\n",
    )
    .unwrap();
    let mut app = common::start(move |o| {
        o.language_override = Some(Language::En);
        o.theme_override = Some(ThemeId::Midnight);
        o.open_paths = vec![(p, None)];
    });
    common::settle(&mut app);
    common::run(&mut app, "json.showSplit", serde_json::Value::Null);
    common::settle(&mut app);
    app.harness.run_steps(3);
    if let Ok(img) = app.harness.render() {
        img.save(out_dir().join("json-split.png")).unwrap();
    }
    // Break it: the last valid tree stays, greyed, under the error banner.
    let core = &mut app.harness.state_mut().core;
    let v = core.workbench.active_view().unwrap();
    let at = core
        .workbench
        .document(core.workbench.active_document().unwrap())
        .unwrap()
        .buffer
        .text()
        .find("3.14")
        .unwrap()
        + 4;
    core.workbench
        .set_selection(v, notoir_editor::Selection::caret(at))
        .unwrap();
    core.editor_focused = true;
    app.harness
        .event(egui::Event::Text("\n  \"broken\": 1".into()));
    common::settle(&mut app);
    app.harness.run_steps(3);
    if let Ok(img) = app.harness.render() {
        img.save(out_dir().join("json-error.png")).unwrap();
    }
}

#[test]
fn outline_folding_bookmarks_snapshot() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("notes");
    std::fs::create_dir_all(root.join("docs")).unwrap();
    let p = root.join("docs/architecture.md");
    std::fs::write(
        &p,
        "# Introduction\n\nNotoir is a native editor.\n\n# Architecture\n\nLayers and crates.\n\n## Editor\n\nRope buffer, display map, folding.\n\n## Git\n\nLocal git2, network via system git.\n\n## MCP\n\nTools behind permissions.\n\n# Roadmap\n\n- Phase 2: structure\n- Phase 3: Git\n",
    )
    .unwrap();
    let root = root.canonicalize().unwrap();
    let p = p.canonicalize().unwrap();
    let mut app = common::start(move |o| {
        o.language_override = Some(Language::En);
        o.theme_override = Some(ThemeId::Midnight);
        o.open_paths = vec![(root, None), (p, None)];
    });
    common::settle(&mut app);
    common::run(&mut app, "view.showOutline", serde_json::Value::Null);
    common::run(&mut app, "fold.fold", serde_json::json!({ "line": 8 }));
    common::run(
        &mut app,
        "bookmarks.toggle",
        serde_json::json!({ "line": 12 }),
    );
    common::run(
        &mut app,
        "bookmarks.toggle",
        serde_json::json!({ "line": 20 }),
    );
    {
        let core = &mut app.harness.state_mut().core;
        let v = core.workbench.active_view().unwrap();
        let doc = core.workbench.active_document().unwrap();
        let at = core
            .workbench
            .document(doc)
            .unwrap()
            .buffer
            .text()
            .find("Tools")
            .unwrap();
        core.workbench
            .set_selection(v, notoir_editor::Selection::caret(at))
            .unwrap();
    }
    common::settle(&mut app);
    app.harness.run_steps(3);
    if let Ok(img) = app.harness.render() {
        img.save(out_dir().join("outline-folding-bookmarks.png"))
            .unwrap();
    }
}

#[test]
fn search_and_quick_open_snapshot() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("notes");
    let w = |rel: &str, text: &str| {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    };
    w(
        "README.md",
        "# Notoir\n\nMCP integration for agents.\nSee docs/mcp.md for the MCP server.\n",
    );
    w(
        "docs/mcp.md",
        "# MCP\n\nNotoir exposes an MCP server over stdio.\n\n## Permissions\n\nEvery MCP tool asks first.\n",
    );
    w(
        "docs/architecture.md",
        "# Architecture\n\nThe MCP bridge lives in notoir-mcp.\n",
    );
    w("docs/roadmap.md", "# Roadmap\n");
    let root = root.canonicalize().unwrap();
    let readme = root.join("README.md");
    let mut app = common::start(move |o| {
        o.language_override = Some(Language::En);
        o.theme_override = Some(ThemeId::Midnight);
        o.open_paths = vec![(root, None), (readme, None)];
    });
    common::settle(&mut app);
    common::run(&mut app, "search.workspace", serde_json::Value::Null);
    {
        let core = &mut app.harness.state_mut().core;
        core.search.query = "MCP".into();
        core.start_search();
    }
    for _ in 0..100 {
        common::settle(&mut app);
        if app.harness.state().core.search.done {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    app.harness.run_steps(3);
    if let Ok(img) = app.harness.render() {
        img.save(out_dir().join("search-panel.png")).unwrap();
    }
    for _ in 0..100 {
        common::settle(&mut app);
        if app
            .harness
            .state()
            .core
            .workspace
            .as_ref()
            .is_some_and(|w| w.index_ready)
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    common::run(&mut app, "navigate.quickOpen", serde_json::Value::Null);
    app.harness.run_steps(2);
    app.harness.event(egui::Event::Text("arch".into()));
    app.harness.run_steps(3);
    if let Ok(img) = app.harness.render() {
        img.save(out_dir().join("quick-open.png")).unwrap();
    }
}

#[test]
fn compare_snapshot() {
    let d = tempfile::tempdir().unwrap();
    let notes = d.path().join("notes.md");
    std::fs::write(
        &notes,
        "# Notoir\n\nThe editor is fast.\nIt supports splits.\nGit is optional.\n\n## Compare\n\nSide by side.\nOld ending.\n",
    )
    .unwrap();
    let notes = notes.canonicalize().unwrap();
    for (theme, name) in [
        (ThemeId::Midnight, "compare-midnight.png"),
        (ThemeId::Paper, "compare-paper.png"),
    ] {
        let path = notes.clone();
        let mut app = common::start(move |o| {
            o.language_override = Some(Language::En);
            o.theme_override = Some(theme);
            o.open_paths = vec![(path, None)];
        });
        common::settle(&mut app);
        {
            let core = &mut app.harness.state_mut().core;
            let doc = core.workbench.active_document().unwrap();
            let len = core.workbench.document(doc).unwrap().buffer.len_chars();
            core.workbench
                .apply_edit(
                    doc,
                    None,
                    notoir_editor::ChangeSet::replace(
                        0,
                        len,
                        "# Notoir\n\nThe editor is fast.\nIt supports true splits.\nAnd duplicate views.\nGit is optional.\n\n## Compare\n\nSide by side.\n",
                    ),
                    notoir_editor::Selection::caret(0),
                    notoir_editor::EditKind::Other,
                    notoir_core::events::ChangeOrigin::Reload,
                    0,
                )
                .unwrap();
        }
        common::run(&mut app, "compare.currentChanges", serde_json::Value::Null);
        common::settle(&mut app);
        app.harness.key_press(egui::Key::F7);
        app.harness.run_steps(4);
        if let Ok(img) = app.harness.render() {
            img.save(out_dir().join(name)).unwrap();
        }
    }
}

#[test]
fn git_snapshot() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("notes");
    std::fs::create_dir_all(root.join("docs")).unwrap();
    let root = root.canonicalize().unwrap();
    let r = git2::Repository::init(&root).unwrap();
    std::fs::write(
        root.join("README.md"),
        "# Notes\n\nIdeas and plans.\n\n## Plans\n\n- Ship Git decorations\n- Write the guide\n- Review the backlog\n",
    )
    .unwrap();
    std::fs::write(root.join("docs/guide.md"), "# Guide\n").unwrap();
    std::fs::write(root.join("docs/old.md"), "# Old\n").unwrap();
    let mut index = r.index().unwrap();
    index
        .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    index.write().unwrap();
    let tree = r.find_tree(index.write_tree().unwrap()).unwrap();
    let sig = git2::Signature::now("Tester", "t@example.com").unwrap();
    r.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
        .unwrap();
    std::fs::write(
        root.join("README.md"),
        "# Notes\n\nIdeas, plans and drafts.\n\n## Plans\n\n- Ship Git decorations\n- Review the backlog\n- Release 0.2\n- Celebrate\n",
    )
    .unwrap();
    std::fs::write(root.join("idea.md"), "# Idea\n").unwrap();
    std::fs::remove_file(root.join("docs/old.md")).unwrap();
    let readme = root.join("README.md");
    for (theme, name) in [
        (ThemeId::Midnight, "git-midnight.png"),
        (ThemeId::Paper, "git-paper.png"),
    ] {
        let (root, readme) = (root.clone(), readme.clone());
        let mut app = common::start(move |o| {
            o.language_override = Some(Language::En);
            o.theme_override = Some(theme);
            o.open_paths = vec![(root, None), (readme, None)];
        });
        for _ in 0..40 {
            common::settle(&mut app);
            let core = &app.harness.state().core;
            let ready = core.git.status.of("idea.md").is_some()
                && core
                    .workbench
                    .active_document()
                    .and_then(|d| core.git_changes(d))
                    .is_some();
            if ready {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        let docs = app
            .harness
            .state()
            .core
            .workspace
            .as_ref()
            .unwrap()
            .root
            .join("docs");
        app.harness.state_mut().core.expand_dir(&docs);
        common::settle(&mut app);
        app.harness.run_steps(3);
        if let Ok(img) = app.harness.render() {
            img.save(out_dir().join(name)).unwrap();
        }
    }
}

/// Source Control panel with staged and unstaged changes, a draft message
/// and Git Output (docs/design/scm-*.png).
#[test]
fn scm_snapshot() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("notes");
    std::fs::create_dir_all(root.join("docs")).unwrap();
    let root = root.canonicalize().unwrap();
    let r = git2::Repository::init(&root).unwrap();
    std::fs::write(root.join("README.md"), "# Notes\n\nIdeas and plans.\n").unwrap();
    std::fs::write(root.join("docs/guide.md"), "# Guide\n").unwrap();
    std::fs::write(root.join("docs/old.md"), "# Old\n").unwrap();
    let mut index = r.index().unwrap();
    index
        .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    index.write().unwrap();
    let tree = r.find_tree(index.write_tree().unwrap()).unwrap();
    let sig = git2::Signature::now("Tester", "t@example.com").unwrap();
    r.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
        .unwrap();
    std::fs::write(
        root.join("README.md"),
        "# Notes\n\nIdeas, plans and drafts.\n",
    )
    .unwrap();
    std::fs::write(root.join("docs/guide.md"), "# Guide\n\nStep one.\n").unwrap();
    std::fs::write(root.join("idea.md"), "# Idea\n").unwrap();
    std::fs::remove_file(root.join("docs/old.md")).unwrap();
    let mut index = r.index().unwrap();
    index.add_path(std::path::Path::new("idea.md")).unwrap();
    index
        .add_path(std::path::Path::new("docs/guide.md"))
        .unwrap();
    index.write().unwrap();
    for (theme, name) in [
        (ThemeId::Midnight, "scm-midnight.png"),
        (ThemeId::Paper, "scm-paper.png"),
    ] {
        let root = root.clone();
        let mut app = common::start(move |o| {
            o.language_override = Some(Language::En);
            o.theme_override = Some(theme);
            o.open_paths = vec![(root, None)];
        });
        for _ in 0..40 {
            common::settle(&mut app);
            if app.harness.state().core.git.status.staged.len() == 2 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        {
            let core = &mut app.harness.state_mut().core;
            *core.git_message_mut().unwrap() =
                "Add the idea note\n\nAlso expands the guide.".into();
            core.git_log_line("> git fetch --progress".into());
            core.git_log_line("From github.com:example/notes".into());
            core.git_log_line("   1a2b3c4..5d6e7f8  main       -> origin/main".into());
        }
        common::run(&mut app, "git.showSourceControl", serde_json::Value::Null);
        common::run(&mut app, "git.showOutput", serde_json::Value::Null);
        common::settle(&mut app);
        app.harness.run_steps(3);
        if let Ok(img) = app.harness.render() {
            img.save(out_dir().join(name)).unwrap();
        }
    }
}

/// Gutter peek over a modified line (docs/design/git-peek.png).
#[test]
fn git_peek_snapshot() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("notes");
    std::fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let r = git2::Repository::init(&root).unwrap();
    std::fs::write(
        root.join("README.md"),
        "# Notes\n\nIdeas and plans.\n\n## Plans\n\n- Ship Git decorations\n- Write the guide\n",
    )
    .unwrap();
    let mut index = r.index().unwrap();
    index
        .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    index.write().unwrap();
    let tree = r.find_tree(index.write_tree().unwrap()).unwrap();
    let sig = git2::Signature::now("Tester", "t@example.com").unwrap();
    r.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
        .unwrap();
    std::fs::write(
        root.join("README.md"),
        "# Notes\n\nIdeas, plans and drafts.\n\n## Plans\n\n- Ship Git decorations\n- Write the user guide\n- Celebrate\n",
    )
    .unwrap();
    let readme = root.join("README.md");
    let mut app = common::start(move |o| {
        o.language_override = Some(Language::En);
        o.theme_override = Some(ThemeId::Midnight);
        o.open_paths = vec![(root, None), (readme, None)];
    });
    for _ in 0..40 {
        common::settle(&mut app);
        let core = &app.harness.state().core;
        if core
            .workbench
            .active_document()
            .and_then(|d| core.git_changes(d))
            .is_some()
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    common::run(&mut app, "git.peekChange", serde_json::json!({ "line": 7 }));
    common::settle(&mut app);
    app.harness.run_steps(3);
    if let Ok(img) = app.harness.render() {
        img.save(out_dir().join("git-peek.png")).unwrap();
    }
}

/// MCP: status bar indicator, an Ask prompt and the Connections dialog with
/// a new client's one-time token (docs/design/mcp-*.png).
#[test]
fn mcp_snapshots() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("notes");
    std::fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    std::fs::write(root.join("notes.md"), "# Notes\n\nIdeas and plans.\n").unwrap();
    let notes = root.join("notes.md");
    let mut app = common::start(move |o| {
        o.language_override = Some(Language::En);
        o.theme_override = Some(ThemeId::Midnight);
        o.open_paths = vec![(root, None), (notes, None)];
    });
    common::settle(&mut app);
    let core = &mut app.harness.state_mut().core;
    core.mcp_start_on(0);
    core.mcp.bridge_command = Some("/Applications/Notoir.app/Contents/MacOS/notoir".into());
    let client = core.mcp_add_client("Local Agent").id;
    core.mcp.new_client = None;
    core.notifications.clear();
    core.mcp.inject(notoir_mcp::ServerEvent::Connected {
        client: client.clone(),
        name: "local-agent".into(),
    });
    let (call, _rx) = notoir_mcp::McpCall::new(
        client,
        "document_save",
        serde_json::json!({ "path": "notes.md" }),
    );
    core.mcp.inject(notoir_mcp::ServerEvent::Call(call));
    app.harness.run_steps(4);
    if let Ok(img) = app.harness.render() {
        img.save(out_dir().join("mcp-prompt.png")).unwrap();
    }
    common::run(
        &mut app,
        "mcp.answerPrompt",
        serde_json::json!({ "answer": "deny" }),
    );
    common::run(
        &mut app,
        "mcp.addClient",
        serde_json::json!({ "name": "Claude Desktop" }),
    );
    app.harness.run_steps(4);
    if let Ok(img) = app.harness.render() {
        img.save(out_dir().join("mcp-connections.png")).unwrap();
    }
}

/// Plugin Manager with an installed example and a plugin needing review
/// (docs/design/plugin-manager.png).
#[test]
fn plugin_manager_snapshot() {
    let home = tempfile::tempdir().unwrap();
    let paths = notoir_platform::AppPaths::under(home.path());
    paths.ensure_dirs().unwrap();
    let examples = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/examples");
    for name in ["word-count", "todo-highlighter", "markdown-table-formatter"] {
        notoir_workspace::fs_ops::copy_recursive(&examples.join(name), &paths.plugins.join(name))
            .unwrap();
    }
    let (db, _) = notoir_storage::state_db::StateDb::open(&paths.data.join("state.db")).unwrap();
    for name in ["word-count", "markdown-table-formatter"] {
        let m = notoir_plugins::Manifest::load(&examples.join(name)).unwrap();
        db.set_plugin_approval(&notoir_storage::state_db::PluginApprovalRow {
            plugin_id: m.plugin.id.clone(),
            version: m.plugin.version.clone(),
            permissions_hash: m.permissions.hash(),
            approved_at: 1,
            enabled: true,
        })
        .unwrap();
    }
    drop(db);
    let mut app = common::start_in(home, |o| {
        o.language_override = Some(Language::En);
        o.theme_override = Some(ThemeId::Midnight);
    });
    for _ in 0..40 {
        common::settle(&mut app);
        if app.harness.state().core.plugins.scanned {
            break;
        }
    }
    app.harness.state_mut().core.notifications.clear();
    common::run(&mut app, "plugins.manager", serde_json::json!({}));
    common::run(
        &mut app,
        "plugins.togglePermissions",
        serde_json::json!({ "plugin": "word-count" }),
    );
    app.harness.run_steps(4);
    if let Ok(img) = app.harness.render() {
        img.save(out_dir().join("plugin-manager.png")).unwrap();
    }
}

#[test]
fn yaml_split_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("settings.yaml");
    std::fs::write(&p, "# Workspace settings\nname: Notoir\nversion: '0.1.4'\nprivate: true\nuser:\n  name: Ana\n  age: 42\n  address:\n    city: Porto\n    zip: null\ntags: [editor, native, fast]\n").unwrap();
    let mut app = common::start(move |o| {
        o.language_override = Some(Language::En);
        o.theme_override = Some(ThemeId::Midnight);
        o.open_paths = vec![(p, None)];
    });
    common::settle(&mut app);
    common::run(&mut app, "yaml.showSplit", serde_json::Value::Null);
    if let Ok(img) = app.harness.render() {
        img.save(out_dir().join("yaml-split.png")).unwrap();
    }
}
