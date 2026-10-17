//! Screenshots for the user guide (`docs/guide/src/images/`): one scene per
//! feature, all in English with the light Notoir Paper theme, 1280×800, on a
//! small "my-notes" workspace. Output goes to `$NOTOIR_DOCS_SHOTS` (defaults to
//! `target/docs-shots`); `scripts/docs-shots.sh` writes them into the guide.
//! Skips without a GPU adapter, like `render.rs`.

mod common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use egui_kittest::kittest::Queryable;
use notoir::i18n::Language;
use notoir::theme::ThemeId;
use serde_json::{Value, json};

fn out_dir() -> PathBuf {
    let dir = std::env::var("NOTOIR_DOCS_SHOTS")
        .map(Into::into)
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/docs-shots")
        });
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Saves the current frame as `<name>.png` without the mouse pointer; false
/// when there is no GPU.
fn shot(app: &mut common::TestApp, name: &str) -> bool {
    app.harness.event(egui::Event::PointerGone);
    shot_with_pointer(app, name)
}

/// Like [`shot`], keeping the pointer (menus open on hover).
fn shot_with_pointer(app: &mut common::TestApp, name: &str) -> bool {
    app.harness.run_steps(3);
    match app.harness.render() {
        Ok(img) => {
            img.save(out_dir().join(format!("{name}.png"))).unwrap();
            true
        }
        Err(e) => {
            eprintln!("render skipped (no GPU adapter?): {e}");
            false
        }
    }
}

fn paper(o: &mut notoir::core::StartOptions) {
    o.language_override = Some(Language::En);
    o.theme_override = Some(ThemeId::Paper);
}

const NOTES: &[(&str, &str)] = &[
    (
        "README.md",
        "# My notes\n\nPersonal notes, plans and a journal. Start at the [[Roadmap]] or the [[Architecture]] overview.\n\n## This week\n\n- [x] Write the user guide\n- [ ] Review the backlog\n- [ ] Ship release 0.2\n\n| Area | Owner | Status |\n|:--|:--|--:|\n| Editor | Ana | done |\n| Git | Ben | review |\n| Docs | Chen | draft |\n\n> Plain files + Git + MCP + native editor.\n\n```rust\nfn main() {\n    println!(\"hello, notes\");\n}\n```\n\n#planning\n",
    ),
    (
        "docs/architecture.md",
        "# Architecture\n\nNotoir is a native editor. See the [[Roadmap]] for what comes next.\n\n## Editor\n\nRope buffer, display map, folding and minimap.\n\n## Workspace\n\nExplorer, file watching and workspace search.\n\n## Git\n\nLocal git2, network operations through the system git.\n\n## MCP\n\nTools behind permissions, one token per client.\n\n## Plugins\n\nLua 5.4 in a sandbox, with declared permissions.\n\n#design #rust\n",
    ),
    (
        "docs/roadmap.md",
        "---\ntags: [planning]\n---\n# Roadmap\n\n## Next\n\n- Better tables in the preview\n- Faster workspace search\n\n## Later\n\n- Sync notes between machines\n\nThe Architecture note explains the layers.\n",
    ),
    (
        "journal/2026-10-15.md",
        "# Thursday\n\n- Paired with Ana on the [[Roadmap]]\n- Fixed the Markdown preview\n\n#journal\n",
    ),
    (
        "journal/2026-10-16.md",
        "# Friday\n\n- Reviewed the [[Architecture]] note\n- Drafted the release notes\n\n#journal\n",
    ),
    (
        "ideas/editor.md",
        "# Editor ideas\n\n- Column selection with the mouse\n- Sticky headings\n",
    ),
    (
        "data/settings.json",
        "{\n  \"name\": \"my-notes\",\n  \"version\": \"0.2.0\",\n  \"private\": true,\n  \"owner\": {\n    \"name\": \"Ana\",\n    \"city\": \"Porto\",\n    \"email\": null\n  },\n  \"tags\": [\"notes\", \"journal\", \"ideas\"],\n  \"sync\": { \"enabled\": false, \"interval\": 300 }\n}\n",
    ),
    (
        "todo.txt",
        "buy milk\ncall the bank\nrenew the passport\nbook the train to Porto\n",
    ),
    (".gitignore", "build/\n"),
];

/// The "my-notes" workspace, committed to Git, with a few uncommitted
/// changes so the Git decorations have something to show.
fn workspace() -> (tempfile::TempDir, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("my-notes");
    for (rel, text) in NOTES {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    let root = root.canonicalize().unwrap();
    let r = git2::Repository::init(&root).unwrap();
    let mut cfg = r.config().unwrap();
    cfg.set_bool("core.autocrlf", false).unwrap();
    let mut index = r.index().unwrap();
    index
        .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    index.write().unwrap();
    let tree = r.find_tree(index.write_tree().unwrap()).unwrap();
    let sig = git2::Signature::now("Ana", "ana@example.com").unwrap();
    r.commit(Some("HEAD"), &sig, &sig, "Start my notes", &tree, &[])
        .unwrap();
    std::fs::write(
        root.join("ideas/editor.md"),
        "# Editor ideas\n\n- Column selection with the mouse\n- Sticky headings\n- Inline images\n",
    )
    .unwrap();
    std::fs::write(
        root.join("ideas/plugins.md"),
        "# Plugin ideas\n\n- Word goals\n",
    )
    .unwrap();
    (d, root)
}

/// Starts on the workspace with `open` (relative paths) opened in order.
fn start(root: &Path, open: &[&str]) -> common::TestApp {
    let mut paths = vec![(root.to_path_buf(), None)];
    paths.extend(open.iter().map(|rel| (root.join(rel), None)));
    let mut app = common::start(move |o| {
        paper(o);
        o.open_paths = paths;
    });
    wait(&mut app, |a| {
        let core = &a.harness.state().core;
        core.workspace.as_ref().is_some_and(|w| w.index_ready) && core.notes.ready
    });
    app.harness.state_mut().core.notifications.clear();
    common::settle(&mut app);
    app
}

fn wait(app: &mut common::TestApp, f: impl Fn(&common::TestApp) -> bool) -> bool {
    let end = std::time::Instant::now() + Duration::from_secs(20);
    while std::time::Instant::now() < end {
        common::settle(app);
        if f(app) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

fn wait_git(app: &mut common::TestApp) {
    wait(app, |a| {
        let core = &a.harness.state().core;
        core.git.status.of("ideas/plugins.md").is_some()
    });
}

fn active_text(app: &common::TestApp) -> String {
    let core = &app.harness.state().core;
    let doc = core.workbench.active_document().unwrap();
    core.workbench.document(doc).unwrap().buffer.text()
}

/// Puts the selection of the active editor on `ranges` (char offsets).
fn select(app: &mut common::TestApp, ranges: Vec<notoir_editor::Range>) {
    let core = &mut app.harness.state_mut().core;
    let v = core.workbench.active_view().unwrap();
    core.workbench
        .set_selection(v, notoir_editor::Selection::new(ranges, 0))
        .unwrap();
    core.editor_focused = true;
}

fn caret_at(app: &mut common::TestApp, needle: &str) {
    let at = active_text(app).find(needle).unwrap();
    select(app, vec![notoir_editor::Range::caret(at)]);
}

fn hover_menu(app: &mut common::TestApp, labels: &[&str]) {
    for label in labels {
        app.harness
            .query_all_by_label_contains(label)
            .next()
            .unwrap_or_else(|| panic!("menu item {label}"))
            .hover();
        app.harness.run_steps(3);
    }
}

#[test]
fn getting_started() {
    let (_d, root) = workspace();
    let mut app = start(&root, &["README.md"]);
    wait_git(&mut app);
    common::run(&mut app, "markdown.showSplit", Value::Null);
    common::settle(&mut app);
    if !shot(&mut app, "hero") {
        return;
    }
    common::run(&mut app, "markdown.showEditor", Value::Null);
    for rel in ["todo.txt", "docs/architecture.md"] {
        app.harness
            .state_mut()
            .core
            .open_path(&root.join(rel), Default::default());
        common::settle(&mut app);
    }
    common::run(&mut app, "view.showOutline", Value::Null);
    caret_at(&mut app, "Local git2");
    shot(&mut app, "window-tour");

    app.harness
        .query_all_by_label("File")
        .next()
        .expect("File menu")
        .click();
    app.harness.run_steps(3);
    shot_with_pointer(&mut app, "menu-file");
    app.harness.key_press(egui::Key::Escape);
    app.harness.run_steps(2);

    common::run(&mut app, "workbench.commandPalette", Value::Null);
    app.harness.event(egui::Event::Text("split".into()));
    shot(&mut app, "palette");
    app.harness.key_press(egui::Key::Escape);
    app.harness.run_steps(2);

    common::run(&mut app, "navigate.quickOpen", Value::Null);
    app.harness.event(egui::Event::Text("road".into()));
    shot(&mut app, "quick-open");
    app.harness.key_press(egui::Key::Escape);
}

#[test]
fn welcome_and_preferences() {
    let mut app = common::start(|o| {
        paper(o);
        o.first_run = true;
    });
    common::settle(&mut app);
    if !shot(&mut app, "welcome") {
        return;
    }
    common::run(&mut app, "welcome.finish", Value::Null);
    common::run(&mut app, "app.settings", json!({ "category": "editor" }));
    app.harness.state_mut().core.set_user_setting(
        "editor.fontSize",
        Some(notoir_core::settings::SettingValue::Int(15)),
    );
    shot(&mut app, "settings");
    common::run(&mut app, "preferences.keyboardShortcuts", Value::Null);
    shot(&mut app, "keybindings");
    common::run(&mut app, "preferences.changeTheme", Value::Null);
    shot(&mut app, "theme-picker");
    app.harness.key_press(egui::Key::Escape);
    app.harness.run_steps(2);
    common::run(&mut app, "preferences.changeLanguage", Value::Null);
    shot(&mut app, "language-picker");
}

#[test]
fn files_and_recovery() {
    let (_d, root) = workspace();
    let mut app = start(&root, &["README.md", "todo.txt"]);
    // A file changed on disk while it has unsaved edits.
    caret_at(&mut app, "buy milk");
    app.harness.event(egui::Event::Text("oat ".into()));
    common::settle(&mut app);
    std::fs::write(
        root.join("todo.txt"),
        "buy milk\ncall the bank\nrenew the passport\n",
    )
    .unwrap();
    wait(&mut app, |a| !a.harness.state().core.banners.is_empty());
    if !shot(&mut app, "external-change") {
        return;
    }
    let view = app.harness.state().core.workbench.active_view().unwrap();
    common::run(&mut app, "file.close", json!({ "view": view.0 }));
    shot(&mut app, "prompt-unsaved");
    drop(app);

    // Crash with unsaved work, then restart.
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("meeting-notes.md");
    std::fs::write(&a, "# Meeting\n\n- agenda\n").unwrap();
    let a = a.canonicalize().unwrap();
    let path = a.clone();
    let mut app = common::start(move |o| {
        paper(o);
        o.open_paths = vec![(path, None)];
    });
    common::settle(&mut app);
    app.harness
        .event(egui::Event::Text("Decisions: ship 0.2\n".into()));
    common::run(&mut app, "file.newUntitled", Value::Null);
    app.harness
        .event(egui::Event::Text("todo: call Ana\nbuy milk\n".into()));
    common::run(&mut app, "recovery.flush", Value::Null);
    let mut app = common::crash(app, paper);
    std::fs::write(&a, "# Meeting\n\nchanged elsewhere\n").unwrap();
    common::settle(&mut app);
    shot(&mut app, "recovery-prompt");
    common::run(&mut app, "recovery.restoreAll", Value::Null);
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
    common::run(&mut app, "view.activate", json!({ "view": view.0 }));
    shot(&mut app, "recovery-banner");
}

#[test]
fn editing() {
    let (_d, root) = workspace();
    let mut app = start(&root, &["docs/architecture.md"]);
    // Multi-cursor: the name of every section selected.
    let text = active_text(&app);
    let ranges = text
        .match_indices("\n## ")
        .map(|(i, _)| {
            let start = i + 4;
            let end = start + text[start..].find('\n').unwrap();
            notoir_editor::Range::new(start, end)
        })
        .collect();
    select(&mut app, ranges);
    if !shot(&mut app, "multi-cursor") {
        return;
    }
    // On the empty line 2, so the find field starts empty.
    let at = active_text(&app).find("\n\nNotoir is").unwrap() + 1;
    select(&mut app, vec![notoir_editor::Range::caret(at)]);
    common::run(&mut app, "edit.replace", Value::Null);
    app.harness.event(egui::Event::Text("editor".into()));
    shot(&mut app, "find-replace");
    app.harness.key_press(egui::Key::Escape);
    app.harness.run_steps(2);

    common::run(&mut app, "view.showOutline", Value::Null);
    common::run(&mut app, "fold.fold", json!({ "line": 6 }));
    common::run(&mut app, "bookmarks.toggle", json!({ "line": 14 }));
    common::run(&mut app, "bookmarks.toggle", json!({ "line": 18 }));
    caret_at(&mut app, "Tools behind");
    shot(&mut app, "folding-bookmarks");

    common::run(&mut app, "search.workspace", Value::Null);
    {
        let core = &mut app.harness.state_mut().core;
        core.search.query = "Roadmap".into();
        core.start_search();
    }
    wait(&mut app, |a| a.harness.state().core.search.done);
    shot(&mut app, "workspace-search");
}

#[test]
fn tabs_splits_zen() {
    let (_d, root) = workspace();
    let mut app = start(
        &root,
        &[
            "README.md",
            "docs/architecture.md",
            "data/settings.json",
            "todo.txt",
        ],
    );
    caret_at(&mut app, "buy milk");
    app.harness.event(egui::Event::Text("oat ".into()));
    common::run(&mut app, "file.newScratchpad", Value::Null);
    let first = app.harness.state().core.workbench.layout.active().tabs[0];
    common::run(&mut app, "view.togglePin", json!({ "view": first.0 }));
    let readme = app.harness.state().core.workbench.layout.active().tabs[0];
    common::run(&mut app, "view.activate", json!({ "view": readme.0 }));
    shot(&mut app, "tabs");
    drop(app);

    let mut app = start(&root, &["docs/architecture.md", "README.md"]);
    common::run(&mut app, "view.splitRight", Value::Null);
    app.harness
        .state_mut()
        .core
        .open_path(&root.join("data/settings.json"), Default::default());
    common::settle(&mut app);
    common::run(&mut app, "view.splitDown", Value::Null);
    app.harness
        .state_mut()
        .core
        .open_path(&root.join("todo.txt"), Default::default());
    common::settle(&mut app);
    shot(&mut app, "splits");
    common::run(&mut app, "view.singleEditor", Value::Null);
    app.harness
        .state_mut()
        .core
        .open_path(&root.join("docs/architecture.md"), Default::default());
    common::settle(&mut app);
    common::run(&mut app, "view.zenMode", Value::Null);
    common::settle(&mut app);
    shot(&mut app, "zen");
}

#[test]
fn formats() {
    let (_d, root) = workspace();
    let mut app = start(&root, &["README.md"]);
    common::run(&mut app, "markdown.showPreview", Value::Null);
    common::settle(&mut app);
    if !shot(&mut app, "markdown-preview") {
        return;
    }
    app.harness
        .state_mut()
        .core
        .open_path(&root.join("data/settings.json"), Default::default());
    common::settle(&mut app);
    common::run(&mut app, "json.showSplit", Value::Null);
    common::settle(&mut app);
    shot(&mut app, "json-tree");
    let at = active_text(&app).find("300").unwrap() + 3;
    select(&mut app, vec![notoir_editor::Range::caret(at)]);
    app.harness
        .event(egui::Event::Text(",\n    \"broken\": ".into()));
    common::settle(&mut app);
    shot(&mut app, "json-error");
    common::run(&mut app, "edit.undo", Value::Null);

    app.harness
        .state_mut()
        .core
        .open_path(&root.join("docs/roadmap.md"), Default::default());
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
                    "---\ntags: [planning]\n---\n# Roadmap\n\n## Next\n\n- Better tables in the preview\n- Faster workspace search\n- Daily notes\n\n## Later\n\n- Sync notes between devices\n\nThe Architecture note explains the layers.\n",
                ),
                notoir_editor::Selection::caret(0),
                notoir_editor::EditKind::Other,
                notoir_core::events::ChangeOrigin::Reload,
                0,
            )
            .unwrap();
    }
    common::run(&mut app, "compare.currentChanges", Value::Null);
    common::settle(&mut app);
    app.harness.key_press(egui::Key::F7);
    shot(&mut app, "compare");
}

#[test]
fn git() {
    let (_d, root) = workspace();
    std::fs::write(
        root.join("README.md"),
        NOTES[0].1.replace(
            "- [ ] Review the backlog\n",
            "- [x] Review the backlog\n- [ ] Plan the 0.3 release\n",
        ),
    )
    .unwrap();
    let mut app = start(&root, &["README.md"]);
    wait(&mut app, |a| {
        let core = &a.harness.state().core;
        core.git.status.of("ideas/plugins.md").is_some()
            && core
                .workbench
                .active_document()
                .and_then(|d| core.git_changes(d))
                .is_some()
    });
    app.harness.state_mut().core.expand_dir(&root.join("ideas"));
    common::settle(&mut app);
    if !shot(&mut app, "git-gutter") {
        return;
    }
    common::run(&mut app, "git.peekChange", json!({ "line": 8 }));
    common::settle(&mut app);
    shot(&mut app, "git-peek");
    common::run(&mut app, "git.closePeek", Value::Null);
    {
        let core = &mut app.harness.state_mut().core;
        *core.git_message_mut().unwrap() =
            "Add plugin ideas\n\nAlso updates the weekly list.".into();
    }
    common::run(&mut app, "git.showSourceControl", Value::Null);
    common::settle(&mut app);
    shot(&mut app, "scm-panel");
}

#[test]
fn notes() {
    let (_d, root) = workspace();
    let mut app = start(&root, &["docs/architecture.md"]);
    common::run(&mut app, "notes.showBacklinks", json!({}));
    wait(&mut app, |a| {
        a.harness
            .query_all_by_label_contains("Referenced by")
            .next()
            .is_some()
    });
    if !shot(&mut app, "backlinks") {
        return;
    }
    common::run(&mut app, "notes.showTags", json!({}));
    if let Some(tag) = app.harness.query_all_by_label_contains("#journal").next() {
        tag.click();
    }
    shot(&mut app, "tags");
}

#[test]
fn mcp() {
    let (_d, root) = workspace();
    let mut app = start(&root, &["README.md"]);
    {
        let core = &mut app.harness.state_mut().core;
        core.mcp_start_on(0);
        core.mcp.bridge_command = Some("/Applications/Notoir.app/Contents/MacOS/notoir".into());
    }
    app.harness.get_by_label("Tools").click();
    app.harness.run_steps(3);
    hover_menu(&mut app, &["MCP ⏵", "Copy Setup Command ⏵"]);
    if !shot_with_pointer(&mut app, "mcp-setup-menu") {
        return;
    }
    app.harness
        .query_all_by_label_contains("Claude Code")
        .next()
        .expect("Claude Code item")
        .click();
    app.harness.run_steps(3);
    shot(&mut app, "mcp-setup-copied");

    // The reply channel must outlive the call, or it counts as cancelled.
    let _reply = {
        let core = &mut app.harness.state_mut().core;
        core.notifications.clear();
        core.mcp.new_client = None;
        let client = core.mcp.clients.last().unwrap().id.clone();
        core.mcp.inject(notoir_mcp::ServerEvent::Connected {
            client: client.clone(),
            name: "claude-code".into(),
        });
        let (call, rx) =
            notoir_mcp::McpCall::new(client, "document_save", json!({ "path": "README.md" }));
        core.mcp.inject(notoir_mcp::ServerEvent::Call(call));
        rx
    };
    app.harness.run_steps(4);
    shot(&mut app, "mcp-approval");
    common::run(&mut app, "mcp.answerPrompt", json!({ "answer": "deny" }));
    let mut replies = vec![_reply];
    for (tool, args) in [
        ("workspace_get", json!({})),
        ("workspace_search", json!({ "query": "Roadmap" })),
        ("document_read", json!({ "path": "docs/roadmap.md" })),
    ] {
        let core = &mut app.harness.state_mut().core;
        let client = core.mcp.clients.last().unwrap().id.clone();
        let (call, rx) = notoir_mcp::McpCall::new(client, tool, args);
        core.mcp.inject(notoir_mcp::ServerEvent::Call(call));
        replies.push(rx);
        common::settle(&mut app);
    }
    common::run(&mut app, "mcp.connections", Value::Null);
    shot(&mut app, "mcp-connections");
    common::run(&mut app, "mcp.closeDialog", Value::Null);
    common::run(&mut app, "mcp.activity", Value::Null);
    shot(&mut app, "mcp-activity");
}

#[test]
fn plugins() {
    let home = tempfile::tempdir().unwrap();
    let paths = notoir_platform::AppPaths::under(home.path());
    paths.ensure_dirs().unwrap();
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/examples");
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
    let mut app = common::start_in(home, paper);
    wait(&mut app, |a| a.harness.state().core.plugins.scanned);
    app.harness.state_mut().core.notifications.clear();
    common::run(&mut app, "plugins.manager", json!({}));
    common::run(
        &mut app,
        "plugins.togglePermissions",
        json!({ "plugin": "word-count" }),
    );
    if !shot(&mut app, "plugin-manager") {
        return;
    }
    common::run(
        &mut app,
        "plugins.review",
        json!({ "plugin": "todo-highlighter" }),
    );
    shot(&mut app, "plugin-review");
}
