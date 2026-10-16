//! MCP tools (spec 29): document vs filesystem semantics, revisions, the
//! editor tools (including `editor_get_selection`), settings, Git, search,
//! files, edit highlighting and content-free activity.

mod common;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use egui::Event;
use egui_kittest::kittest::Queryable;
use notoir_core::permissions::{Capability, ClientId, Decision};
use notoir_mcp::{McpCall, ServerEvent, ToolReply};
use serde_json::{Value, json};
use tokio::sync::oneshot::Receiver;

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

/// A workspace with notes.md ("A\n"), old.md and docs/guide.md, opened
/// with `files` in tabs.
fn workspace(files: &[&str]) -> (tempfile::TempDir, PathBuf, common::TestApp) {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("ws");
    std::fs::create_dir_all(root.join("docs")).unwrap();
    let root = root.canonicalize().unwrap();
    std::fs::write(root.join("notes.md"), "A\n").unwrap();
    std::fs::write(root.join("old.md"), "old\n").unwrap();
    std::fs::write(
        root.join("docs/guide.md"),
        "# Guide\n\nSetup steps\nMore setup\n",
    )
    .unwrap();
    let mut paths = vec![(root.clone(), None)];
    paths.extend(files.iter().map(|f| (root.join(f), None)));
    let mut app = common::start(move |o| o.open_paths = paths);
    assert!(wait_for(&mut app, |a| {
        let core = &a.harness.state().core;
        core.workspace.as_ref().is_some_and(|w| w.index_ready)
    }));
    (d, root, app)
}

fn client(app: &mut common::TestApp) -> ClientId {
    app.harness
        .state_mut()
        .core
        .mcp_add_client("Local Agent")
        .id
}

fn allow(app: &mut common::TestApp, c: &ClientId, cap: Capability) {
    app.harness
        .state_mut()
        .core
        .mcp_set_permission(c, cap, Decision::Allow);
}

fn call(app: &mut common::TestApp, c: &ClientId, tool: &str, args: Value) -> Receiver<ToolReply> {
    let (call, rx) = McpCall::new(c.clone(), tool, args);
    app.harness.state().core.mcp.inject(ServerEvent::Call(call));
    app.harness.run_steps(2);
    rx
}

fn reply(app: &mut common::TestApp, rx: &mut Receiver<ToolReply>) -> ToolReply {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Ok(r) = rx.try_recv() {
            return r;
        }
        assert!(Instant::now() < deadline, "no reply");
        common::settle(app);
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Calls a tool and expects success.
fn ok(app: &mut common::TestApp, c: &ClientId, tool: &str, args: Value) -> Value {
    let mut rx = call(app, c, tool, args);
    match reply(app, &mut rx) {
        ToolReply::Ok { structured, .. } => structured,
        e => panic!("{tool} failed: {e:?}"),
    }
}

/// Calls a tool and expects an error; returns (code, data).
fn fail(app: &mut common::TestApp, c: &ClientId, tool: &str, args: Value) -> (String, Value) {
    let mut rx = call(app, c, tool, args);
    match reply(app, &mut rx) {
        ToolReply::Err { code, data, .. } => (code, data),
        r => panic!("{tool} should fail: {r:?}"),
    }
}

fn focus_and_type(app: &mut common::TestApp, text: &str) {
    let core = &mut app.harness.state_mut().core;
    let view = core.workbench.active_editor().unwrap();
    core.effects.push(notoir::core::Effect::FocusEditor(view));
    app.harness.run_steps(2);
    app.harness.event(Event::Text(text.into()));
    app.harness.run_steps(2);
}

fn active_text(app: &common::TestApp) -> String {
    let core = &app.harness.state().core;
    let doc = core.workbench.active_document().unwrap();
    core.workbench.document(doc).unwrap().buffer.text()
}

// spec 29: "document_read returns the unsaved buffer"
#[test]
fn document_read_returns_the_unsaved_buffer() {
    let (_d, root, mut app) = workspace(&["notes.md"]);
    let c = client(&mut app);
    // Disk "A", buffer "A B" (unsaved).
    let core = &mut app.harness.state_mut().core;
    let view = core.workbench.active_editor().unwrap();
    let _ = core
        .workbench
        .set_selection(view, notoir_editor::Selection::caret(1));
    focus_and_type(&mut app, " B");
    assert_eq!(active_text(&app), "A B\n");
    let read = ok(&mut app, &c, "document_read", json!({ "path": "notes.md" }));
    assert_eq!(read["content"], "A B\n");
    assert_eq!(read["dirty"], true);
    let disk = ok(
        &mut app,
        &c,
        "filesystem_read",
        json!({ "path": "notes.md" }),
    );
    assert_eq!(disk["content"], "A\n");
    assert_eq!(
        std::fs::read_to_string(root.join("notes.md")).unwrap(),
        "A\n"
    );
}

// spec 29: "document_write is undoable and does not save"
#[test]
fn document_write_is_undoable_and_does_not_save() {
    let (_d, root, mut app) = workspace(&["notes.md"]);
    let c = client(&mut app);
    let r = ok(
        &mut app,
        &c,
        "document_write",
        json!({ "path": "notes.md", "content": "A\nfrom the agent\n" }),
    );
    assert_eq!(r["dirty"], true);
    assert_eq!(active_text(&app), "A\nfrom the agent\n");
    let core = &app.harness.state().core;
    let doc = core.workbench.active_document().unwrap();
    assert!(core.workbench.document(doc).unwrap().is_dirty());
    assert_eq!(
        std::fs::read_to_string(root.join("notes.md")).unwrap(),
        "A\n"
    );
    // The change is highlighted while it fades.
    assert!(!core.mcp_highlights(doc).is_empty());
    // The user can undo it.
    common::run(&mut app, "edit.undo", json!({}));
    assert_eq!(active_text(&app), "A\n");
}

// spec 29: "Revision conflict"
#[test]
fn revision_conflict() {
    let (_d, _root, mut app) = workspace(&["notes.md"]);
    let c = client(&mut app);
    let read = ok(&mut app, &c, "document_read", json!({ "path": "notes.md" }));
    let read_rev = read["revision"].as_u64().unwrap();
    // The user types: the revision moves on.
    focus_and_type(&mut app, "x");
    let core = &app.harness.state().core;
    let doc = core.workbench.active_document().unwrap();
    let current = core.workbench.document(doc).unwrap().revision;
    assert!(current > read_rev);
    let (code, data) = fail(
        &mut app,
        &c,
        "document_apply_edits",
        json!({
            "path": "notes.md",
            "expected_revision": read_rev,
            "edits": [{ "start": { "line": 1, "column": 1 }, "end": { "line": 1, "column": 1 }, "text": "!" }]
        }),
    );
    assert_eq!(code, "revision_conflict");
    assert_eq!(data["current_revision"], current);
    // With the current revision it applies.
    let r = ok(
        &mut app,
        &c,
        "document_apply_edits",
        json!({
            "path": "notes.md",
            "expected_revision": current,
            "edits": [
                { "start": { "line": 1, "column": 1 }, "end": { "line": 1, "column": 1 }, "text": "!" },
                { "start": { "offset": 2 }, "end": { "offset": 2 }, "text": "?" }
            ]
        }),
    );
    assert_eq!(r["revision"], current + 1);
    assert_eq!(active_text(&app), "!xA?\n");
    let (code, _) = fail(
        &mut app,
        &c,
        "document_apply_edits",
        json!({ "path": "notes.md", "edits": [{ "start": { "line": 9 }, "end": { "line": 9 }, "text": "" }] }),
    );
    assert_eq!(code, "invalid_position");
}

// spec 29: "filesystem_write triggers external change handling"
#[test]
fn filesystem_write_triggers_external_change_handling() {
    let (_d, root, mut app) = workspace(&["notes.md"]);
    let c = client(&mut app);
    focus_and_type(&mut app, "mine ");
    allow(&mut app, &c, Capability::FilesystemWrite);
    ok(
        &mut app,
        &c,
        "filesystem_write",
        json!({ "path": "notes.md", "content": "from the agent\n" }),
    );
    assert_eq!(
        std::fs::read_to_string(root.join("notes.md")).unwrap(),
        "from the agent\n"
    );
    assert!(wait_for(&mut app, |a| !a
        .harness
        .state()
        .core
        .banners
        .is_empty()));
    // The buffer is unchanged and the banner names the client.
    assert_eq!(active_text(&app), "mine A\n");
    app.harness.run_steps(2);
    app.harness
        .query_all_by_label_contains("changed on disk by “Local Agent” (MCP)")
        .next()
        .expect("external-change banner naming the client");
}

#[test]
fn filesystem_write_asks_and_outside_is_denied() {
    let (d, _root, mut app) = workspace(&[]);
    let c = client(&mut app);
    // Raw writes ask every time by default.
    let _rx = call(
        &mut app,
        &c,
        "filesystem_write",
        json!({ "path": "new.txt", "content": "x" }),
    );
    assert_eq!(app.harness.state().core.mcp.prompts.len(), 1);
    common::run(&mut app, "mcp.answerPrompt", json!({ "answer": "deny" }));
    // Outside the workspace needs a capability that is denied by default.
    let outside = d.path().join("outside.txt");
    std::fs::write(&outside, "secret").unwrap();
    let (code, _) = fail(
        &mut app,
        &c,
        "filesystem_read",
        json!({ "path": outside.to_string_lossy() }),
    );
    assert_eq!(code, "permission_denied");
    allow(&mut app, &c, Capability::FilesystemOutsideWorkspace);
    let r = ok(
        &mut app,
        &c,
        "filesystem_read",
        json!({ "path": outside.to_string_lossy() }),
    );
    assert_eq!(r["content"], "secret");
    // document_* never reaches outside.
    let (code, _) = fail(
        &mut app,
        &c,
        "document_read",
        json!({ "path": outside.to_string_lossy() }),
    );
    assert_eq!(code, "path_outside_workspace");
}

#[test]
fn editor_get_selection_returns_text_file_and_range() {
    let (_d, _root, mut app) = workspace(&["docs/guide.md"]);
    let c = client(&mut app);
    // Nothing selected: an empty selection at the caret.
    let r = ok(&mut app, &c, "editor_get_selection", json!({}));
    assert_eq!(r["path"], "docs/guide.md");
    assert_eq!(r["text"], "");
    assert_eq!(r["range"]["start"]["line"], 1);
    // The user selects "Setup steps" (line 3).
    ok(
        &mut app,
        &c,
        "editor_select_range",
        json!({ "start": { "line": 3, "column": 1 }, "end": { "line": 3, "column": 12 } }),
    );
    let r = ok(&mut app, &c, "editor_get_selection", json!({}));
    assert_eq!(r["text"], "Setup steps");
    assert_eq!(r["path"], "docs/guide.md");
    assert_eq!(
        r["range"]["start"],
        json!({ "line": 3, "column": 1, "offset": 9 })
    );
    assert_eq!(
        r["range"]["end"],
        json!({ "line": 3, "column": 12, "offset": 20 })
    );
    assert_eq!(r["selections"].as_array().unwrap().len(), 1);
    // A selection across lines.
    ok(
        &mut app,
        &c,
        "editor_select_range",
        json!({ "path": "docs/guide.md", "start": { "line": 3, "column": 7 }, "end": { "line": 4, "column": 5 } }),
    );
    let r = ok(&mut app, &c, "editor_get_selection", json!({}));
    assert_eq!(r["text"], "steps\nMore");
    // Needs both editor.read and document.read.
    app.harness
        .state_mut()
        .core
        .mcp_set_permission(&c, Capability::DocumentRead, Decision::Deny);
    let (code, _) = fail(&mut app, &c, "editor_get_selection", json!({}));
    assert_eq!(code, "permission_denied");
}

#[test]
fn editor_insert_replace_and_tabs() {
    let (_d, _root, mut app) = workspace(&["notes.md"]);
    let c = client(&mut app);
    ok(
        &mut app,
        &c,
        "editor_select_range",
        json!({ "start": { "line": 1, "column": 1 }, "end": { "line": 1, "column": 2 } }),
    );
    ok(&mut app, &c, "editor_replace", json!({ "text": "Z" }));
    assert_eq!(active_text(&app), "Z\n");
    ok(&mut app, &c, "editor_insert", json!({ "text": "!" }));
    assert_eq!(active_text(&app), "Z!\n");
    let tabs = ok(&mut app, &c, "editor_get_open_tabs", json!({}));
    let tab = &tabs["groups"][0]["tabs"][0];
    assert_eq!(tab["path"], "notes.md");
    assert_eq!(tab["dirty"], true);
    let active = ok(&mut app, &c, "editor_get_active_document", json!({}));
    assert_eq!(active["document"]["path"], "notes.md");
    assert_eq!(active["document"]["selections"][0]["start"]["column"], 3);
    // Open another document at a line.
    ok(
        &mut app,
        &c,
        "editor_open_document",
        json!({ "path": "docs/guide.md", "line": 3 }),
    );
    common::settle(&mut app);
    let active = ok(&mut app, &c, "editor_get_active_document", json!({}));
    assert_eq!(active["document"]["path"], "docs/guide.md");
    assert_eq!(active["document"]["selections"][0]["start"]["line"], 3);
}

#[test]
fn documents_open_without_tabs_and_edits_get_one() {
    let (_d, root, mut app) = workspace(&[]);
    let c = client(&mut app);
    let r = ok(&mut app, &c, "document_open", json!({ "path": "old.md" }));
    assert_eq!(r["in_tab"], false);
    assert!(app.harness.state().core.workbench.active_view().is_none());
    // Reading a file not in memory loads it.
    let r = ok(
        &mut app,
        &c,
        "document_read",
        json!({ "path": "docs/guide.md", "start_line": 3, "end_line": 3 }),
    );
    assert_eq!(r["content"], "Setup steps\n");
    // Editing a tabless document puts it in a tab so the user sees it.
    ok(
        &mut app,
        &c,
        "document_write",
        json!({ "path": "old.md", "content": "new\n" }),
    );
    let tabs = ok(&mut app, &c, "editor_get_open_tabs", json!({}));
    assert_eq!(tabs["groups"][0]["tabs"][0]["path"], "old.md");
    assert_eq!(
        std::fs::read_to_string(root.join("old.md")).unwrap(),
        "old\n"
    );
    // show: true needs editor control and opens a tab.
    let r = ok(
        &mut app,
        &c,
        "document_open",
        json!({ "path": "notes.md", "show": true }),
    );
    assert_eq!(r["path"], "notes.md");
    common::settle(&mut app);
    let tabs = ok(&mut app, &c, "editor_get_open_tabs", json!({}));
    assert_eq!(tabs["groups"][0]["tabs"].as_array().unwrap().len(), 2);
}

#[test]
fn list_search_create_rename() {
    let (_d, root, mut app) = workspace(&["notes.md"]);
    let c = client(&mut app);
    let r = ok(&mut app, &c, "workspace_list_files", json!({}));
    let files: Vec<&str> = r["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f.as_str().unwrap())
        .collect();
    assert_eq!(files, ["docs/guide.md", "notes.md", "old.md"]);
    let r = ok(
        &mut app,
        &c,
        "workspace_list_files",
        json!({ "glob": "docs/**", "limit": 1 }),
    );
    assert_eq!(r["files"], json!(["docs/guide.md"]));
    assert_eq!(r["next_cursor"], Value::Null);
    let r = ok(&mut app, &c, "workspace_list_files", json!({ "limit": 2 }));
    assert_eq!(r["next_cursor"], 2);

    // Search sees unsaved buffers.
    focus_and_type(&mut app, "setup ");
    let r = ok(
        &mut app,
        &c,
        "workspace_search",
        json!({ "query": "setup" }),
    );
    let hits: Vec<(String, u64)> = r["matches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| {
            (
                m["path"].as_str().unwrap().to_owned(),
                m["line"].as_u64().unwrap(),
            )
        })
        .collect();
    assert!(hits.contains(&("notes.md".into(), 1)), "{hits:?}");
    assert!(hits.contains(&("docs/guide.md".into(), 3)), "{hits:?}");
    let (code, _) = fail(
        &mut app,
        &c,
        "workspace_search",
        json!({ "query": "(", "regex": true }),
    );
    assert_eq!(code, "invalid_query");

    // Create asks by default; allow it.
    allow(&mut app, &c, Capability::DocumentCreate);
    ok(
        &mut app,
        &c,
        "document_create",
        json!({ "path": "drafts/idea.md", "content": "idea\n" }),
    );
    assert_eq!(
        std::fs::read_to_string(root.join("drafts/idea.md")).unwrap(),
        "idea\n"
    );
    let (code, _) = fail(
        &mut app,
        &c,
        "document_create",
        json!({ "path": "notes.md" }),
    );
    assert_eq!(code, "exists");

    // Rename asks by default; allow it. The open document follows.
    allow(&mut app, &c, Capability::FileRename);
    ok(
        &mut app,
        &c,
        "document_rename",
        json!({ "path": "notes.md", "new_path": "renamed.md" }),
    );
    assert!(root.join("renamed.md").exists() && !root.join("notes.md").exists());
    let tabs = ok(&mut app, &c, "editor_get_open_tabs", json!({}));
    let paths: Vec<&str> = tabs["groups"][0]["tabs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["path"].as_str().unwrap())
        .collect();
    assert!(paths.contains(&"renamed.md"), "{paths:?}");
    let (code, _) = fail(
        &mut app,
        &c,
        "document_rename",
        json!({ "path": "old.md", "new_path": "../x.md" }),
    );
    assert_eq!(code, "path_outside_workspace");
}

#[test]
fn settings_tools() {
    let (_d, _root, mut app) = workspace(&[]);
    let c = client(&mut app);
    let r = ok(&mut app, &c, "settings_get", json!({ "prefix": "editor." }));
    assert_eq!(r["editor.fontSize"], 14);
    assert!(r.get("files.exclude").is_none());
    // Modify is denied by default.
    let (code, _) = fail(
        &mut app,
        &c,
        "settings_update",
        json!({ "key": "editor.fontSize", "value": 16 }),
    );
    assert_eq!(code, "permission_denied");
    allow(&mut app, &c, Capability::SettingsModify);
    let r = ok(
        &mut app,
        &c,
        "settings_update",
        json!({ "key": "editor.fontSize", "value": 16 }),
    );
    assert_eq!(r["value"], 16);
    assert_eq!(
        app.harness
            .state()
            .core
            .settings
            .int("editor.fontSize", None),
        16
    );
    // Restricted settings never.
    let (code, _) = fail(
        &mut app,
        &c,
        "settings_update",
        json!({ "key": "mcp.autoStart", "value": true }),
    );
    assert_eq!(code, "restricted_setting");
    let (code, _) = fail(
        &mut app,
        &c,
        "settings_update",
        json!({ "key": "editor.fontSize", "value": "big" }),
    );
    assert_eq!(code, "invalid_value");
}

#[test]
fn git_status_and_diff() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("repo");
    std::fs::create_dir(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let r = git2::Repository::init(&root).unwrap();
    std::fs::write(root.join("notes.md"), "one\ntwo\n").unwrap();
    let mut index = r.index().unwrap();
    index.add_path(std::path::Path::new("notes.md")).unwrap();
    index.write().unwrap();
    let tree = r.find_tree(index.write_tree().unwrap()).unwrap();
    let sig = git2::Signature::now("T", "t@example.com").unwrap();
    r.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
        .unwrap();
    std::fs::write(root.join("notes.md"), "one\n2\n").unwrap();
    let open = root.clone();
    let mut app = common::start(move |o| o.open_paths = vec![(open, None)]);
    assert!(wait_for(&mut app, |a| {
        let core = &a.harness.state().core;
        core.git.repo.is_some() && !core.git.status.files.is_empty()
    }));
    let c = client(&mut app);
    let s = ok(&mut app, &c, "git_status", json!({}));
    assert_eq!(s["repository"], true);
    assert_eq!(s["changes"][0]["path"], "notes.md");
    assert_eq!(s["changes"][0]["status"], "modified");
    let diff = ok(
        &mut app,
        &c,
        "git_diff",
        json!({ "path": "notes.md", "base": "head" }),
    );
    let text = diff["diff"].as_str().unwrap();
    assert!(
        text.contains("--- a/notes.md") && text.contains("-two") && text.contains("+2"),
        "{text}"
    );
    let all = ok(&mut app, &c, "git_diff", json!({}));
    assert_eq!(all["files"], json!(["notes.md"]));
}
