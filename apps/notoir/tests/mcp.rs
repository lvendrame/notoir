//! MCP in the app (specs 29, 32): the permission gate, Ask prompts, the
//! workspace boundary, activity, the status bar indicator, and the real HTTP
//! server with tokens issued by Add Client.

mod common;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use egui::Event;
use egui_kittest::kittest::Queryable;
use notoir_core::permissions::ClientId;
use notoir_mcp::bridge::Bridge;
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

/// A workspace with notes.md and old.md, opened with `files` in tabs.
fn workspace(files: &[&str]) -> (tempfile::TempDir, PathBuf, common::TestApp) {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("ws");
    std::fs::create_dir(&root).unwrap();
    let root = root.canonicalize().unwrap();
    std::fs::write(root.join("notes.md"), "A\n").unwrap();
    std::fs::write(root.join("old.md"), "old\n").unwrap();
    let mut paths = vec![(root.clone(), None)];
    paths.extend(files.iter().map(|f| (root.join(f), None)));
    let mut app = common::start(move |o| o.open_paths = paths);
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .workspace
        .is_some()));
    (d, root, app)
}

/// A registered client (default permissions).
fn client(app: &mut common::TestApp) -> ClientId {
    app.harness
        .state_mut()
        .core
        .mcp_add_client("Local Agent")
        .id
}

/// Delivers a tool call as the server would.
fn call(
    app: &mut common::TestApp,
    client: &ClientId,
    tool: &str,
    args: Value,
) -> Receiver<ToolReply> {
    let (c, rx) = McpCall::new(client.clone(), tool, args);
    app.harness.state().core.mcp.inject(ServerEvent::Call(c));
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

fn error_code(r: &ToolReply) -> &str {
    match r {
        ToolReply::Err { code, .. } => code,
        ToolReply::Ok { .. } => "ok",
    }
}

fn last_outcome(app: &common::TestApp) -> (String, Option<String>, String) {
    let e = app
        .harness
        .state()
        .core
        .mcp
        .activity
        .front()
        .cloned()
        .unwrap();
    (e.tool, e.target, e.outcome)
}

// spec 32: "Delete denied by default for MCP"
#[test]
fn delete_denied_by_default() {
    let (_d, root, mut app) = workspace(&[]);
    let c = client(&mut app);
    let mut rx = call(&mut app, &c, "document_delete", json!({ "path": "old.md" }));
    let r = reply(&mut app, &mut rx);
    assert_eq!(error_code(&r), "permission_denied");
    assert!(root.join("old.md").exists());
    assert_eq!(
        last_outcome(&app),
        (
            "document_delete".into(),
            Some("old.md".into()),
            "denied".into()
        )
    );
    // No prompt was shown for a denied capability.
    assert!(app.harness.state().core.mcp.prompts.is_empty());
}

fn make_dirty(app: &mut common::TestApp, text: &str) {
    // Prompt buttons may hold keyboard focus: give it back to the editor.
    let core = &mut app.harness.state_mut().core;
    let view = core.workbench.active_editor().unwrap();
    core.effects.push(notoir::core::Effect::FocusEditor(view));
    app.harness.run_steps(2);
    app.harness.event(Event::Text(text.into()));
    app.harness.run_steps(2);
    let core = &app.harness.state().core;
    let doc = core.workbench.active_document().unwrap();
    let d = core.workbench.document(doc).unwrap();
    assert!(d.is_dirty(), "{:?} {:?}", d.path(), d.buffer.text());
}

// spec 32: "Ask once"
#[test]
fn ask_once() {
    let (_d, root, mut app) = workspace(&["notes.md"]);
    let c = client(&mut app);
    make_dirty(&mut app, "B");
    let mut rx = call(&mut app, &c, "document_save", json!({ "path": "notes.md" }));
    app.harness.run_steps(2);
    // A prompt asks the user; nothing is saved yet.
    assert_eq!(app.harness.state().core.mcp.prompts.len(), 1);
    assert!(rx.try_recv().is_err());
    app.harness
        .query_all_by_label_contains("Local Agent")
        .next()
        .expect("prompt names the client");
    app.harness.get_by_label("Allow Once").click();
    app.harness.run_steps(2);
    let r = reply(&mut app, &mut rx);
    assert_eq!(error_code(&r), "ok", "{r:?}");
    assert!(
        std::fs::read_to_string(root.join("notes.md"))
            .unwrap()
            .contains('B')
    );
    // A second save prompts again.
    make_dirty(&mut app, "C");
    let mut rx2 = call(&mut app, &c, "document_save", json!({ "path": "notes.md" }));
    app.harness.run_steps(2);
    assert_eq!(app.harness.state().core.mcp.prompts.len(), 1);
    app.harness.get_by_label("Deny").click();
    app.harness.run_steps(2);
    assert_eq!(error_code(&reply(&mut app, &mut rx2)), "permission_denied");
}

#[test]
fn session_grant_and_identical_requests_share_a_prompt() {
    let (_d, _root, mut app) = workspace(&["notes.md"]);
    let c = client(&mut app);
    make_dirty(&mut app, "B");
    let mut a = call(&mut app, &c, "document_save", json!({ "path": "notes.md" }));
    let mut b = call(&mut app, &c, "document_save", json!({ "path": "notes.md" }));
    assert_eq!(app.harness.state().core.mcp.prompts.len(), 1);
    common::run(&mut app, "mcp.answerPrompt", json!({ "answer": "session" }));
    assert_eq!(error_code(&reply(&mut app, &mut a)), "ok");
    assert_eq!(error_code(&reply(&mut app, &mut b)), "ok");
    // The session grant covers the next save.
    make_dirty(&mut app, "C");
    let mut again = call(&mut app, &c, "document_save", json!({ "path": "notes.md" }));
    assert!(app.harness.state().core.mcp.prompts.is_empty());
    assert_eq!(error_code(&reply(&mut app, &mut again)), "ok");
}

// spec 32: "Prompt timeout denies"
#[test]
fn prompt_timeout_denies() {
    let (_d, root, mut app) = workspace(&["notes.md"]);
    let c = client(&mut app);
    make_dirty(&mut app, "B");
    let mut rx = call(&mut app, &c, "document_save", json!({ "path": "notes.md" }));
    let core = &mut app.harness.state_mut().core;
    let p = core.mcp.prompts.front_mut().unwrap();
    let waited = p.deadline - core.now_ms;
    assert!(
        (notoir::mcp::PROMPT_TIMEOUT_MS - 1000..=notoir::mcp::PROMPT_TIMEOUT_MS).contains(&waited),
        "{waited}"
    );
    // 120 seconds pass.
    p.deadline = 0;
    app.harness.run_steps(2);
    let r = reply(&mut app, &mut rx);
    assert_eq!(error_code(&r), "permission_denied");
    assert!(app.harness.state().core.mcp.prompts.is_empty());
    assert_eq!(
        std::fs::read_to_string(root.join("notes.md")).unwrap(),
        "A\n"
    );
    assert_eq!(last_outcome(&app).2, "denied");
}

// spec 29: "Path escape is rejected"
#[test]
fn path_escape_is_rejected() {
    let (d, _root, mut app) = workspace(&[]);
    std::fs::write(d.path().join("secrets.txt"), "s").unwrap();
    let c = client(&mut app);
    let mut rx = call(
        &mut app,
        &c,
        "document_read",
        json!({ "path": "../secrets.txt" }),
    );
    assert_eq!(
        error_code(&reply(&mut app, &mut rx)),
        "path_outside_workspace"
    );
    let mut rx = call(
        &mut app,
        &c,
        "document_read",
        json!({ "path": "/etc/hosts" }),
    );
    assert_eq!(
        error_code(&reply(&mut app, &mut rx)),
        "path_outside_workspace"
    );
}

// spec 32: "Symlink escape"
#[cfg(unix)]
#[test]
fn symlink_escape() {
    let (_d, root, mut app) = workspace(&[]);
    std::os::unix::fs::symlink("/etc", root.join("link")).unwrap();
    let c = client(&mut app);
    let mut rx = call(
        &mut app,
        &c,
        "document_read",
        json!({ "path": "link/hosts" }),
    );
    assert_eq!(
        error_code(&reply(&mut app, &mut rx)),
        "path_outside_workspace"
    );
}

#[test]
fn document_read_returns_the_buffer_or_the_disk() {
    let (_d, _root, mut app) = workspace(&["notes.md"]);
    let c = client(&mut app);
    make_dirty(&mut app, "B ");
    let mut rx = call(&mut app, &c, "document_read", json!({ "path": "notes.md" }));
    let ToolReply::Ok { structured, .. } = reply(&mut app, &mut rx) else {
        panic!("read failed");
    };
    assert_eq!(structured["content"], "B A\n");
    assert_eq!(structured["dirty"], true);
    // Not open: read from disk off the UI thread.
    let mut rx = call(&mut app, &c, "document_read", json!({ "path": "old.md" }));
    let ToolReply::Ok { structured, .. } = reply(&mut app, &mut rx) else {
        panic!("read failed");
    };
    assert_eq!(structured["content"], "old\n");
    assert_eq!(structured["open"], false);
    // Activity holds the target, never the content.
    let core = &app.harness.state().core;
    for e in &core.mcp.activity {
        assert!(!format!("{e:?}").contains("B A"), "{e:?}");
    }
    let mut rx = call(&mut app, &c, "workspace_get", json!({}));
    let ToolReply::Ok { structured, .. } = reply(&mut app, &mut rx) else {
        panic!("workspace_get failed");
    };
    assert_eq!(structured["name"], "ws");
}

fn status_label(app: &common::TestApp, text: &str) -> bool {
    app.harness.query_by_label(text).is_some()
}

// spec 29: "Status indicator"
#[test]
fn status_indicator() {
    let mut app = common::start(|_| {});
    app.harness.state_mut().core.mcp_start_on(0);
    app.harness.run_steps(2);
    assert!(status_label(&app, "MCP ●0"));
    let a = client(&mut app);
    let b = client(&mut app);
    for c in [&a, &b] {
        app.harness.state().core.mcp.inject(ServerEvent::Connected {
            client: c.clone(),
            name: "agent".into(),
        });
    }
    app.harness.run_steps(3);
    assert!(status_label(&app, "MCP ●2"));
    // The popover lists them and stops the server.
    app.harness.get_by_label("MCP ●2").click();
    app.harness.run_steps(2);
    assert!(app.harness.state().core.mcp.popover);
    app.harness
        .query_all_by_label_contains("Server: http://127.0.0.1:")
        .next()
        .expect("popover shows the URL");
    // The anchored popover settles its size over a few frames.
    app.harness.run_steps(3);
    app.harness.get_by_label("Stop MCP Server").click();
    app.harness.run_steps(3);
    assert!(!app.harness.state().core.mcp.running());
    assert!(!status_label(&app, "MCP ●2"));
}

fn post(url: &str, auth: Option<&str>) -> u16 {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into();
    let mut req = agent
        .post(url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream");
    if let Some(a) = auth {
        req = req.header("Authorization", a);
    }
    req.send(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#)
        .unwrap()
        .status()
        .as_u16()
}

// spec 29: "Requests without token are rejected"
#[test]
fn requests_without_token_are_rejected() {
    let mut app = common::start(|_| {});
    app.harness.state_mut().core.mcp_start_on(0);
    let url = app.harness.state().core.mcp.url().unwrap();
    assert_eq!(post(&url, None), 401);
    assert!(wait_for(&mut app, |a| a
        .harness
        .state()
        .core
        .mcp
        .activity
        .front()
        .is_some_and(|e| e.outcome == "rejected: missing token")));
    common::run(&mut app, "mcp.activity", json!({}));
    app.harness
        .query_all_by_label_contains("rejected: missing token")
        .next()
        .expect("shown in the Activity panel");
}

fn init(bridge: &mut Bridge) -> Value {
    let out = bridge.forward(
        r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"Local Agent","version":"1"}}}"#,
    );
    serde_json::from_str(&out[0]).unwrap()
}

/// A tool call over HTTP from a thread (the UI must keep running).
fn call_over_http(
    mut bridge: Bridge,
    line: &'static str,
) -> std::thread::JoinHandle<(Bridge, Value)> {
    std::thread::spawn(move || {
        let out = bridge.forward(line);
        let v = serde_json::from_str(&out[0]).unwrap();
        (bridge, v)
    })
}

// spec 32: "Revocation applies immediately"
#[test]
fn revocation_applies_immediately() {
    let (_d, _root, mut app) = workspace(&[]);
    app.harness.state_mut().core.mcp_start_on(0);
    let port = app.harness.state().core.mcp.server.as_ref().unwrap().port();
    // Add Client issues the token (shown once in Connections).
    common::run(&mut app, "mcp.addClient", json!({ "name": "Local Agent" }));
    let new = app.harness.state().core.mcp.new_client.clone().unwrap();
    app.harness
        .query_all_by_label_contains(&new.token)
        .next()
        .expect("token shown once");
    let mut bridge = Bridge::new(&new.token, port);
    assert_eq!(init(&mut bridge)["result"]["serverInfo"]["name"], "notoir");
    // Modify is allowed by default: a call works end to end.
    let h = call_over_http(
        bridge,
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"workspace_get","arguments":{}}}"#,
    );
    let deadline = Instant::now() + Duration::from_secs(20);
    while !h.is_finished() && Instant::now() < deadline {
        common::settle(&mut app);
    }
    let (mut bridge, v) = h.join().unwrap();
    assert_eq!(v["result"]["structuredContent"]["name"], "ws");
    assert_eq!(app.harness.state().core.mcp.connected_count(), 1);

    common::run(&mut app, "mcp.revokeClient", json!({ "client": new.id.0 }));
    let out = bridge.forward(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#);
    let v: Value = serde_json::from_str(&out[0]).unwrap();
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("rejected the token"),
        "{v}"
    );
    // Revocation persists.
    let core = &app.harness.state().core;
    assert!(core.mcp.client(&new.id).unwrap().revoked);
    assert_eq!(core.mcp.connected_count(), 0);
}

#[test]
fn clients_and_permissions_persist_across_restarts() {
    let mut app = common::start(|_| {});
    common::run(
        &mut app,
        "mcp.addClient",
        json!({ "name": "Claude Desktop" }),
    );
    let id = app.harness.state().core.mcp.new_client.clone().unwrap().id;
    common::run(
        &mut app,
        "mcp.setPermission",
        json!({ "client": id.0, "capability": "file.delete", "decision": "ask" }),
    );
    common::run(&mut app, "mcp.permissions", json!({ "client": id.0 }));
    app.harness
        .query_all_by_label_contains("MCP Access — Claude Desktop")
        .next()
        .expect("permissions dialog");
    let mut app = common::restart(app, |_| {});
    let core = &app.harness.state().core;
    assert_eq!(core.mcp.clients[0].name, "Claude Desktop");
    // The token is never shown again.
    assert!(core.mcp.new_client.is_none());
    let p = notoir_core::permissions::Principal::McpClient(id.clone());
    assert_eq!(
        core.mcp
            .gate
            .decision(&p, &notoir_core::permissions::Capability::FileDelete),
        notoir_core::permissions::Decision::Ask
    );
    common::run(&mut app, "mcp.resetPermissions", json!({ "client": id.0 }));
    let core = &app.harness.state().core;
    assert_eq!(
        core.mcp
            .gate
            .decision(&p, &notoir_core::permissions::Capability::FileDelete),
        notoir_core::permissions::Decision::Deny
    );
}
