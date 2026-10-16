//! The example plugins in `plugins/examples/` (spec 30): status items,
//! plugin settings, declarative panels, outline providers, languages with
//! grammars, formatters; and the Plugin Manager, install from .zip,
//! uninstall and Developer Tools.

mod common;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui_kittest::kittest::Queryable;
use notoir::plugins::PluginStatus;
use notoir_platform::AppPaths;
use notoir_plugins::Manifest;
use notoir_storage::state_db::{PluginApprovalRow, StateDb};
use serde_json::json;

fn wait_for(app: &mut common::TestApp, f: impl Fn(&common::TestApp) -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        common::settle(app);
        if f(app) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

fn example(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../plugins/examples")
        .join(name)
}

/// Installs and approves examples into a fresh home, opens `files` in a
/// workspace holding `contents`.
fn start(
    examples: &[&str],
    contents: &[(&str, &str)],
    files: &[&str],
) -> (tempfile::TempDir, PathBuf, common::TestApp) {
    let home = tempfile::tempdir().unwrap();
    let paths = AppPaths::under(home.path());
    paths.ensure_dirs().unwrap();
    let (db, _) = StateDb::open(&paths.data.join("state.db")).unwrap();
    for name in examples {
        let src = example(name);
        notoir_workspace::fs_ops::copy_recursive(&src, &paths.plugins.join(name)).unwrap();
        let m = Manifest::load(&src).unwrap();
        db.set_plugin_approval(&PluginApprovalRow {
            plugin_id: m.plugin.id.clone(),
            version: m.plugin.version.clone(),
            permissions_hash: m.permissions.hash(),
            approved_at: 1,
            enabled: true,
        })
        .unwrap();
    }
    drop(db);
    let ws = tempfile::tempdir().unwrap();
    let root = ws.path().canonicalize().unwrap();
    for (name, text) in contents {
        std::fs::write(root.join(name), text).unwrap();
    }
    let mut app = common::start_in(home, |_| {});
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .plugins
        .scanned));
    // Open after the plugins registered their languages.
    let mut open = vec![(root.clone(), None)];
    open.extend(files.iter().map(|f| (root.join(f), None)));
    app.harness.state_mut().core.open_cli_paths(&open);
    assert!(wait_for(&mut app, |a| {
        let core = &a.harness.state().core;
        files.is_empty() || core.workbench.active_document().is_some()
    }));
    (ws, root, app)
}

fn text(app: &common::TestApp) -> String {
    let core = &app.harness.state().core;
    let doc = core.workbench.active_document().unwrap();
    core.workbench.document(doc).unwrap().buffer.text()
}

fn status_text(app: &common::TestApp, id: &str) -> Option<String> {
    app.harness
        .state()
        .core
        .plugins
        .status_items
        .iter()
        .find(|s| s.id == id)
        .and_then(|s| s.text.clone())
}

#[test]
fn word_count_status_item_command_and_setting() {
    let doc = "# Title\n\nOne two three.\n\n```\nlet code = 1;\n```\n";
    let (_ws, _root, mut app) = start(&["word-count"], &[("notes.md", doc)], &["notes.md"]);
    // Activated by onLanguage:markdown; the status item shows the count.
    assert!(wait_for(&mut app, |a| status_text(a, "words").as_deref()
        == Some("5 words")));
    app.harness.run_steps(2);
    app.harness.get_by_label("5 words");
    // The plugin setting is registered with its translated title.
    let core = &app.harness.state().core;
    assert!(!core.settings.bool("plugins.word-count.includeCode", None));
    assert_eq!(
        notoir::i18n::t_in(
            notoir::i18n::Language::En,
            "settings:plugins.word-count.includeCode"
        ),
        "Count words in code blocks"
    );
    // Changing it changes the count.
    app.harness.state_mut().core.set_user_setting(
        "plugins.word-count.includeCode",
        Some(notoir_core::settings::SettingValue::Bool(true)),
    );
    common::run(&mut app, "plugin.word-count.count", json!({}));
    assert!(wait_for(&mut app, |a| status_text(a, "words").as_deref()
        == Some("9 words")));
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .notifications
        .history()
        .any(|n| notoir::shell::overlays::message_text(n)
            == "Word Count: This document has 9 words.")));
}

#[test]
fn todo_panel_outline_and_language() {
    let notes = "intro\nTODO: write tests\nmore\nFIXME: crash on save\n";
    let list = "# Week\n- [ ] Ship plugins\n- [x] Write spec\n";
    let (_ws, root, mut app) = start(
        &["todo-highlighter"],
        &[("notes.md", notes), ("week.todo", list)],
        &["notes.md"],
    );
    let key = ("todo-highlighter".to_owned(), "todos".to_owned());
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .plugins
        .panel_trees
        .get(&key)
        .is_some_and(|t| t
            .to_string()
            .contains("FIXME: crash on save"))));
    // The Plugins panel renders the tree; a list click reveals the line.
    let core = &mut app.harness.state_mut().core;
    core.exit_maximize();
    core.panels.show(notoir_core::panels::PanelId::Plugins);
    app.harness.run_steps(3);
    app.harness
        .query_all_by_label_contains("2 in this document")
        .next()
        .expect("panel heading");
    app.harness
        .query_all_by_label_contains("FIXME: crash on save")
        .next()
        .expect("list item")
        .click();
    let caret_line = |a: &common::TestApp| {
        let core = &a.harness.state().core;
        let v = core.workbench.active_editor().unwrap();
        let view = core.workbench.view(v).unwrap();
        let rope = core
            .workbench
            .document(view.document_id)
            .unwrap()
            .buffer
            .rope()
            .clone();
        rope.char_to_line(view.selection.primary().head)
    };
    assert!(wait_for(&mut app, |a| caret_line(a) == 3));
    // The `.todo` language: detected, highlighted, outlined by the plugin.
    app.harness
        .state_mut()
        .core
        .open_path(&root.join("week.todo"), notoir::files::OpenAt::default());
    assert!(wait_for(&mut app, |a| {
        let core = &a.harness.state().core;
        core.workbench
            .active_document()
            .and_then(|d| core.workbench.document(d))
            .is_some_and(|d| d.language.as_str() == "todo")
    }));
    let lang = notoir_editor::LanguageId::from_id("todo").unwrap();
    assert!(wait_for(&mut app, |_| notoir_editor::syntax::has_grammar(
        lang
    )));
    let names = |a: &mut common::TestApp| {
        let core = &mut a.harness.state_mut().core;
        let doc = core.workbench.active_document().unwrap();
        core.structure(doc)
            .map(|s| s.symbols.iter().map(|s| s.name.clone()).collect::<Vec<_>>())
            .unwrap_or_default()
    };
    let deadline = Instant::now() + Duration::from_secs(20);
    while names(&mut app) != ["Ship plugins", "Write spec"] {
        assert!(Instant::now() < deadline, "{:?}", names(&mut app));
        common::settle(&mut app);
    }
}

#[test]
fn markdown_table_formatter_formats_and_undoes() {
    let table = "Intro\n\n| a | longer header |\n|---|:-:|\n| value | x |\n";
    let (_ws, _root, mut app) = start(
        &["markdown-table-formatter"],
        &[("table.md", table)],
        &["table.md"],
    );
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .plugins
        .formatters
        .contains_key("markdown")));
    common::run(&mut app, "editor.formatDocument", json!({}));
    let formatted = "Intro\n\n| a     | longer header |\n| ----- | :-----------: |\n| value | x             |\n";
    assert!(
        wait_for(&mut app, |a| text(a) == formatted),
        "{}",
        text(&app)
    );
    common::run(&mut app, "edit.undo", json!({}));
    assert_eq!(text(&app), table);
}

// spec 30: "Malformed manifest"
#[test]
fn manager_lists_plugins_and_invalid_manifests() {
    let home = tempfile::tempdir().unwrap();
    let paths = AppPaths::under(home.path());
    paths.ensure_dirs().unwrap();
    let broken = paths.plugins.join("broken");
    std::fs::create_dir_all(&broken).unwrap();
    std::fs::write(
        broken.join("plugin.toml"),
        "[plugin]\nid = \"broken\nname = 1\n",
    )
    .unwrap();
    notoir_workspace::fs_ops::copy_recursive(
        &example("word-count"),
        &paths.plugins.join("word-count"),
    )
    .unwrap();
    let mut app = common::start_in(home, |_| {});
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .plugins
        .scanned));
    common::run(&mut app, "plugins.manager", json!({}));
    app.harness.run_steps(3);
    app.harness.get_by_label("Installed (2)");
    app.harness
        .query_all_by_label_contains("Word Count  1.0.0")
        .next()
        .expect("listed");
    app.harness
        .query_all_by_label_contains("Needs review")
        .next()
        .expect("unapproved plugin");
    app.harness.get_by_label("Invalid");
    app.harness
        .query_all_by_label_contains("plugin.toml line 2")
        .next()
        .expect("parse error shown");
    // Search filters the list.
    app.harness.state_mut().core.plugins.manager.search = "word".into();
    app.harness.run_steps(2);
    assert!(app.harness.query_by_label("Invalid").is_none());
}

/// A zip of an example plugin (inside a top folder, like a download).
fn zip_example(name: &str, out: &Path) {
    let file = std::fs::File::create(out).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default();
    let root = example(name);
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).unwrap() {
            let p = e.unwrap().path();
            let rel = p
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if p.is_dir() {
                stack.push(p);
            } else {
                zip.start_file(format!("{name}/{rel}"), opts).unwrap();
                zip.write_all(&std::fs::read(&p).unwrap()).unwrap();
            }
        }
    }
    zip.finish().unwrap();
}

#[test]
fn install_from_zip_uninstall_and_developer_tools() {
    let (_ws, _root, mut app) = start(&[], &[], &[]);
    let dir = tempfile::tempdir().unwrap();
    let zip = dir.path().join("word-count.zip");
    zip_example("word-count", &zip);
    app.harness.state_mut().core.plugins_install_zip(zip);
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .plugins
        .review
        .is_some()));
    app.harness.run_steps(2);
    app.harness.get_by_label("Install").click();
    assert!(wait_for(&mut app, |a| {
        a.harness
            .state()
            .core
            .plugins
            .entry("word-count")
            .is_some_and(|e| e.status == PluginStatus::Enabled)
    }));
    let installed = AppPaths::under(app.home.path()).plugins.join("word-count");
    assert!(installed.join("lib/count.lua").is_file());

    // Developer Tools: activation reason, API calls, reload one plugin.
    common::run(&mut app, "plugin.word-count.count", json!({}));
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .plugins
        .entry("word-count")
        .is_some_and(
            |e| e.api_calls.contains_key("notify") || e.api_calls.contains_key("editor.active")
        )));
    app.harness.state_mut().core.plugins.dev_plugin = Some("word-count".into());
    common::run(&mut app, "plugins.developerTools", json!({}));
    app.harness
        .query_all_by_label_contains("Activated by: onCommand:count")
        .next()
        .expect("activation reason");
    app.harness
        .query_all_by_label_contains("API calls:")
        .next()
        .expect("api calls");
    app.harness.get_by_label("Reload this plugin").click();
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .plugins
        .entry("word-count")
        .is_some_and(
            |e| e.status == PluginStatus::Enabled && e.activated_by.is_none()
        )));

    // Uninstall asks, then moves the folder to the trash.
    common::run(
        &mut app,
        "plugins.uninstall",
        json!({ "plugin": "word-count" }),
    );
    app.harness
        .query_all_by_label_contains("Uninstall “Word Count”?")
        .next()
        .expect("confirmation");
    common::run(
        &mut app,
        "plugins.uninstallAnswer",
        json!({ "approve": true }),
    );
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .plugins
        .entry("word-count")
        .is_none()));
    let core = &app.harness.state().core;
    assert!(
        core.commands
            .get(&notoir_core::command::CommandId::from(
                "plugin.word-count.count".to_owned()
            ))
            .is_none()
    );
}
