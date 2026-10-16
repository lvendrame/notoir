//! MCP activity and logs never hold document contents (spec 29). Its own
//! test binary: the log capture is a thread-local subscriber, which other
//! tests running in parallel would disturb.

mod common;

use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use egui_kittest::kittest::Queryable;
use notoir_mcp::{McpCall, ServerEvent, ToolReply};
use serde_json::json;

fn settle_until(app: &mut common::TestApp, f: impl Fn(&common::TestApp) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !f(app) {
        assert!(Instant::now() < deadline, "timed out");
        common::settle(app);
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn ok(
    app: &mut common::TestApp,
    c: &notoir_core::permissions::ClientId,
    tool: &str,
    args: serde_json::Value,
) {
    let (call, mut rx) = McpCall::new(c.clone(), tool, args);
    app.harness.state().core.mcp.inject(ServerEvent::Call(call));
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Ok(r) = rx.try_recv() {
            assert!(matches!(r, ToolReply::Ok { .. }), "{r:?}");
            return;
        }
        assert!(Instant::now() < deadline, "no reply");
        common::settle(app);
    }
}

fn workspace() -> (tempfile::TempDir, common::TestApp) {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("ws");
    std::fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    std::fs::write(root.join("notes.md"), "A\n").unwrap();
    let paths = vec![(root.clone(), None), (root.join("notes.md"), None)];
    let mut app = common::start(move |o| o.open_paths = paths);
    settle_until(&mut app, |a| {
        a.harness.state().core.workbench.active_document().is_some()
    });
    (d, app)
}

/// Captures log output (the test's own subscriber, same thread).
#[derive(Clone, Default)]
struct LogBuf(Arc<Mutex<Vec<u8>>>);

impl Write for LogBuf {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

// spec 29: "Activity shows operations without content"
#[test]
fn activity_shows_operations_without_content() {
    let buf = LogBuf::default();
    let writer = buf.clone();
    let subscriber = tracing_subscriber::fmt()
        .json()
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(move || writer.clone())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let (_d, mut app) = workspace();
    let c = app
        .harness
        .state_mut()
        .core
        .mcp_add_client("Local Agent")
        .id;
    let secret = "TOP-SECRET-CONTENT";
    ok(
        &mut app,
        &c,
        "document_write",
        json!({ "path": "notes.md", "content": format!("A\n{secret}\n") }),
    );
    let e = app
        .harness
        .state()
        .core
        .mcp
        .activity
        .front()
        .cloned()
        .unwrap();
    assert_eq!(e.client, Some(c.clone()));
    assert_eq!(e.tool, "document_write");
    assert_eq!(e.target.as_deref(), Some("notes.md (+1 −0)"));
    assert_eq!(e.outcome, "ok");
    common::run(&mut app, "mcp.activity", json!({}));
    app.harness
        .query_all_by_label_contains("notes.md (+1 −0)")
        .next()
        .expect("listed in the Activity panel");
    let core = &app.harness.state().core;
    assert!(!format!("{:?}", core.mcp.activity).contains(secret));
    let log = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
    assert!(log.contains("document_write"), "the call is logged");
    assert!(!log.contains(secret), "no content in the log");
}
