//! The HTTP server end to end: guard, sessions, tool calls answered by a
//! stand-in for the UI thread, and the stdio bridge.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use notoir_core::permissions::ClientId;
use notoir_mcp::auth::{generate_token, hash_token};
use notoir_mcp::bridge::Bridge;
use notoir_mcp::{ClientAuth, ClientRegistry, McpServer, RejectReason, ServerEvent, ToolReply};
use serde_json::{Value, json};

struct Fixture {
    server: McpServer,
    events: Receiver<ServerEvent>,
    clients: ClientRegistry,
    token: String,
    client: ClientId,
}

fn start() -> Fixture {
    let clients = ClientRegistry::default();
    let token = generate_token();
    let client = ClientId("c1".into());
    clients.add(ClientAuth {
        id: client.clone(),
        token_hash: hash_token(&token),
        revoked: false,
    });
    let (tx, events) = channel();
    // Port 0: any free port.
    let server = McpServer::start(0, clients.clone(), tx, Arc::new(|| {})).unwrap();
    Fixture {
        server,
        events,
        clients,
        token,
        client,
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into()
}

const LIST: &str = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;

fn rejected(events: &Receiver<ServerEvent>) -> RejectReason {
    loop {
        match events.recv_timeout(Duration::from_secs(5)).unwrap() {
            ServerEvent::Rejected { reason, .. } => return reason,
            _ => continue,
        }
    }
}

#[test]
fn requests_without_token_are_rejected() {
    let f = start();
    let resp = agent()
        .post(&f.server.url())
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .send(LIST)
        .unwrap();
    assert_eq!(resp.status().as_u16(), 401);
    assert_eq!(rejected(&f.events), RejectReason::MissingToken);
    let resp = agent()
        .post(&f.server.url())
        .header("Authorization", "Bearer ntr_wrong")
        .send(LIST)
        .unwrap();
    assert_eq!(resp.status().as_u16(), 401);
    assert_eq!(rejected(&f.events), RejectReason::InvalidToken);
}

// spec 29: "Non-loopback Origin rejected"
#[test]
fn non_loopback_origin_rejected() {
    let f = start();
    let resp = agent()
        .post(&f.server.url())
        .header("Authorization", &format!("Bearer {}", f.token))
        .header("Origin", "https://evil.example")
        .send(LIST)
        .unwrap();
    assert_eq!(resp.status().as_u16(), 403);
    assert_eq!(rejected(&f.events), RejectReason::Origin);
    // A rebinding attack arrives with a foreign Host.
    let resp = agent()
        .post(&f.server.url())
        .header("Authorization", &format!("Bearer {}", f.token))
        .header("Host", "evil.example")
        .send(LIST)
        .unwrap();
    assert_eq!(resp.status().as_u16(), 403);
}

/// Answers tool calls like the app would, until the channel closes.
fn answer_calls(events: Receiver<ServerEvent>) -> std::thread::JoinHandle<Vec<String>> {
    std::thread::spawn(move || {
        let mut seen = Vec::new();
        while let Ok(e) = events.recv_timeout(Duration::from_secs(10)) {
            match e {
                ServerEvent::Call(call) => {
                    let reply = if call.tool == "document_delete" {
                        ToolReply::error("permission_denied", "Delete files is not allowed")
                    } else {
                        ToolReply::Ok {
                            structured: json!({ "root": "/w", "args": call.args }),
                            summary: "Workspace /w".into(),
                        }
                    };
                    seen.push(format!("call {} {}", call.client.0, call.tool));
                    call.respond(reply);
                }
                ServerEvent::Connected { client, name } => {
                    seen.push(format!("connected {} {name}", client.0));
                }
                ServerEvent::Disconnected { client } => {
                    seen.push(format!("disconnected {}", client.0));
                    break;
                }
                ServerEvent::Rejected { reason, .. } => seen.push(format!("rejected {reason:?}")),
            }
        }
        seen
    })
}

#[test]
fn bridge_round_trip_and_revocation() {
    let f = start();
    let port = f.server.port();
    let worker = answer_calls(f.events);
    let mut bridge = Bridge::new(&f.token, port);
    let one = |b: &mut Bridge, line: &str| -> Value {
        let out = b.forward(line);
        assert_eq!(out.len(), 1, "{out:?}");
        serde_json::from_str(&out[0]).unwrap()
    };
    let init = one(
        &mut bridge,
        r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"Test Agent","version":"1"}}}"#,
    );
    assert_eq!(init["result"]["serverInfo"]["name"], "notoir");
    assert!(
        bridge
            .forward(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
            .is_empty()
    );
    let list = one(&mut bridge, LIST);
    let names: Vec<&str> = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"document_read"), "{names:?}");

    let ok = one(
        &mut bridge,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"workspace_get","arguments":{}}}"#,
    );
    assert_eq!(ok["result"]["structuredContent"]["root"], "/w");
    assert_eq!(ok["result"]["isError"], false);
    let denied = one(
        &mut bridge,
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"document_delete","arguments":{"path":"old.md"}}}"#,
    );
    assert_eq!(denied["result"]["isError"], true);
    assert_eq!(
        denied["result"]["structuredContent"]["error"],
        "permission_denied"
    );

    // Revocation applies to the very next request.
    f.clients.revoke(&f.client);
    let after = one(
        &mut bridge,
        r#"{"jsonrpc":"2.0","id":4,"method":"tools/list"}"#,
    );
    assert!(
        after["error"]["message"]
            .as_str()
            .unwrap()
            .contains("rejected the token")
    );

    // Stopping ends the session.
    f.server.stop();
    drop(f.server);
    let seen = worker.join().unwrap();
    assert_eq!(seen[0], "connected c1 Test Agent");
    assert!(
        seen.contains(&"call c1 workspace_get".to_owned()),
        "{seen:?}"
    );
    assert!(
        seen.contains(&"rejected InvalidToken".to_owned()),
        "{seen:?}"
    );
}
