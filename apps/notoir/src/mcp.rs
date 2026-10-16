//! MCP on the UI thread (docs/specs/29-mcp.md, 32-permissions-and-security.md):
//! the server's lifecycle, registered clients and their permissions, Ask
//! prompts, tool execution through the same document and file services as
//! the UI, and the activity log. The server thread only forwards calls here.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use notoir_core::ids::DocumentId;
use notoir_core::notifications::{Notification, NotificationSource, Severity, Translatable};
use notoir_core::permissions::{
    Capability, ClientId, Decision, GrantScope, MCP_MATRIX, PermissionGate, Principal, RiskClass,
};
use notoir_mcp::paths::{self, PathError};
use notoir_mcp::tools::{self, ToolDef};
use notoir_mcp::{
    ClientAuth, ClientRegistry, McpCall, McpServer, RejectReason, ServerEvent, ToolReply,
};
use notoir_storage::state_db::{McpActivityRow, McpClientRow};
use notoir_workspace::fs_io;
use serde_json::{Value, json};

use crate::core::AppCore;

/// Unanswered permission prompts deny after this long (spec 32).
pub const PROMPT_TIMEOUT_MS: u64 = 120_000;
/// Activity entries kept in memory (the database keeps the rest).
const ACTIVITY_KEEP: usize = 500;
/// Reads above this need a line range (`mcp.maxReadBytes` default).
const MAX_READ_BYTES: usize = 2 * 1024 * 1024;
/// `last_seen` is written at most this often per client.
const SEEN_WRITE_MS: i64 = 10_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientInfo {
    pub id: ClientId,
    /// The name the user gave the client.
    pub name: String,
    /// `clientInfo.name` from the last `initialize` (display only).
    pub reported: Option<String>,
    pub created_at: i64,
    pub last_seen: Option<i64>,
    pub revoked: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActivityEntry {
    /// Unix milliseconds.
    pub at: i64,
    pub client: Option<ClientId>,
    pub tool: String,
    /// Workspace-relative path or other short target; never content.
    pub target: Option<String>,
    /// `ok`, `denied`, `error: <code>`, `rejected: <reason>`.
    pub outcome: String,
    pub duration_ms: Option<u64>,
}

/// A request waiting on the user.
#[derive(Debug)]
pub struct Waiting {
    call: McpCall,
    target: Target,
    started: u64,
}

/// "Local Agent (MCP) wants to save notes.md."
#[derive(Debug)]
pub struct AskPrompt {
    pub id: u64,
    pub client: ClientId,
    pub capability: Capability,
    pub target: Option<String>,
    waiters: Vec<Waiting>,
    pub deadline: u64,
    /// "Always Allow" on a dangerous capability awaits a second click.
    pub confirm_always: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptAnswer {
    Once,
    Session,
    Always,
    Deny,
}

impl PromptAnswer {
    pub fn parse(s: &str) -> Option<PromptAnswer> {
        match s {
            "once" => Some(PromptAnswer::Once),
            "session" => Some(PromptAnswer::Session),
            "always" => Some(PromptAnswer::Always),
            "deny" => Some(PromptAnswer::Deny),
            _ => None,
        }
    }
}

/// A just-created client: the token is shown once.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewClient {
    pub id: ClientId,
    pub name: String,
    pub token: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum McpDialog {
    Connections,
    /// Add Client: the name being typed.
    AddClient(String),
    Permissions(ClientId),
}

/// What a call operates on, resolved before the gate.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Target {
    None,
    Path(PathBuf),
}

/// A save started for a client, answered when the pipeline finishes.
#[derive(Debug)]
struct PendingSave {
    doc: DocumentId,
    call: McpCall,
    target: Option<String>,
    started: u64,
}

pub struct McpState {
    pub server: Option<McpServer>,
    events_tx: Sender<ServerEvent>,
    events_rx: Receiver<ServerEvent>,
    wake: Arc<dyn Fn() + Send + Sync>,
    pub registry: ClientRegistry,
    pub clients: Vec<ClientInfo>,
    /// Open sessions per client.
    pub connected: HashMap<ClientId, usize>,
    pub gate: PermissionGate,
    pub prompts: VecDeque<AskPrompt>,
    next_prompt: u64,
    saves: Vec<PendingSave>,
    /// Newest first.
    pub activity: VecDeque<ActivityEntry>,
    pub new_client: Option<NewClient>,
    pub dialog: Option<McpDialog>,
    /// The status bar popover is open.
    pub popover: bool,
    pub activity_filter: Option<ClientId>,
    /// F6: move keyboard focus to the prompt next frame.
    pub focus_prompt: bool,
    seen_written: HashMap<ClientId, i64>,
    /// The executable named in Claude Desktop snippets (this binary unless
    /// set; snapshots use a neutral path).
    pub bridge_command: Option<String>,
    /// Calls finished off the UI thread, logged and answered here.
    done_tx: Sender<Finished>,
    done_rx: Receiver<Finished>,
}

type Finished = (McpCall, Option<String>, u64, ToolReply);

impl McpState {
    pub fn new(wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        let (events_tx, events_rx) = channel();
        let (done_tx, done_rx) = channel();
        Self {
            done_tx,
            done_rx,
            server: None,
            events_tx,
            events_rx,
            wake,
            registry: ClientRegistry::default(),
            clients: Vec::new(),
            connected: HashMap::new(),
            gate: PermissionGate::default(),
            prompts: VecDeque::new(),
            next_prompt: 1,
            saves: Vec::new(),
            activity: VecDeque::new(),
            new_client: None,
            dialog: None,
            popover: false,
            activity_filter: None,
            focus_prompt: false,
            seen_written: HashMap::new(),
            bridge_command: None,
        }
    }

    /// Delivers a server event as if it came from the server (tests).
    pub fn inject(&self, e: ServerEvent) {
        let _ = self.events_tx.send(e);
    }

    pub fn running(&self) -> bool {
        self.server.is_some()
    }

    pub fn url(&self) -> Option<String> {
        self.server.as_ref().map(McpServer::url)
    }

    /// Clients with at least one open session.
    pub fn connected_count(&self) -> usize {
        self.connected.values().filter(|n| **n > 0).count()
    }

    pub fn client(&self, id: &ClientId) -> Option<&ClientInfo> {
        self.clients.iter().find(|c| &c.id == id)
    }

    pub fn client_name(&self, id: &ClientId) -> String {
        self.client(id)
            .map_or_else(|| id.0.clone(), |c| c.name.clone())
    }

    /// (allowed, ask, denied) counts of a client's matrix.
    pub fn summary(&self, id: &ClientId) -> (usize, usize, usize) {
        let p = Principal::McpClient(id.clone());
        let mut n = (0, 0, 0);
        for (_, caps) in MCP_MATRIX {
            for c in *caps {
                match self.gate.decision(&p, c) {
                    Decision::Allow => n.0 += 1,
                    Decision::Ask => n.1 += 1,
                    Decision::Deny => n.2 += 1,
                }
            }
        }
        n
    }

    /// Config snippets for a client (`token` is a placeholder once the
    /// one-time display is gone).
    pub fn snippets(&self, token: &str) -> (String, String) {
        let port = self
            .server
            .as_ref()
            .map_or(notoir_mcp::DEFAULT_PORT, McpServer::port);
        let url = format!("http://127.0.0.1:{port}/mcp");
        let claude_code = format!(
            "claude mcp add --transport http notoir {url} --header \"Authorization: Bearer {token}\""
        );
        let exe = self.bridge_command.clone().unwrap_or_else(|| {
            std::env::current_exe()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| "notoir".into())
        });
        // Written by hand to keep "command" before "args".
        let q = |v: &str| Value::String(v.to_owned()).to_string();
        let desktop = format!(
            "{{\n  \"mcpServers\": {{\n    \"notoir\": {{\n      \"command\": {},\n      \"args\": [\"mcp-bridge\", \"--token\", {}, \"--port\", \"{port}\"]\n    }}\n  }}\n}}",
            q(&exe),
            q(token)
        );
        (claude_code, desktop)
    }
}

fn epoch_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

fn principal(c: &ClientId) -> Principal {
    Principal::McpClient(c.clone())
}

fn denied(cap: &Capability) -> ToolReply {
    ToolReply::Err {
        code: "permission_denied".into(),
        message: format!("Notoir does not allow {} for this client", cap.key()),
        data: json!({ "capability": cap.key() }),
    }
}

impl AppCore {
    /// Loads clients and permissions, prunes old activity and starts the
    /// server when `mcp.autoStart` is on (restricted: never from an untrusted
    /// workspace).
    pub fn mcp_init(&mut self) {
        if let Some(db) = &self.state_db {
            let days = self.settings.int("mcp.activityRetentionDays", None).max(1);
            let _ = db.prune_mcp_activity(epoch_ms() - days * 86_400_000);
            let rows = db.mcp_clients().unwrap_or_default();
            let mut auth = Vec::new();
            for r in rows {
                let id = ClientId(r.client_id.clone());
                let p = principal(&id);
                let grants = db.mcp_grants(&r.client_id).unwrap_or_default();
                self.mcp.gate.load(
                    &p,
                    grants.into_iter().filter_map(|(c, d)| {
                        Some((Capability::from_key(&c)?, Decision::parse(&d)?))
                    }),
                );
                if r.revoked {
                    self.mcp.gate.revoke(&p);
                }
                auth.push(ClientAuth {
                    id: id.clone(),
                    token_hash: r.token_hash,
                    revoked: r.revoked,
                });
                self.mcp.clients.push(ClientInfo {
                    id,
                    name: r.name,
                    reported: None,
                    created_at: r.created_at,
                    last_seen: r.last_seen,
                    revoked: r.revoked,
                });
            }
            self.mcp.registry.set(auth);
            for r in db.mcp_activity(ACTIVITY_KEEP).unwrap_or_default() {
                self.mcp.activity.push_back(ActivityEntry {
                    at: r.at,
                    client: r.client_id.map(ClientId),
                    tool: r.tool,
                    target: r.target,
                    outcome: r.outcome,
                    duration_ms: r.duration_ms.map(|d| d.max(0) as u64),
                });
            }
        }
    }

    /// `mcp.autoStart` after the workspace (and its trust) is known.
    pub fn mcp_autostart(&mut self) {
        if !self.mcp.running() && self.settings.bool("mcp.autoStart", None) {
            self.mcp_start();
        }
    }

    pub fn mcp_start(&mut self) {
        let port = self.settings.int("mcp.port", None).clamp(0, 65535) as u16;
        self.mcp_start_on(port);
    }

    /// Starts on `port` or the next free one (tests pass 0: any port).
    pub fn mcp_start_on(&mut self, port: u16) {
        if self.mcp.running() {
            return;
        }
        let wake = self.mcp.wake.clone();
        match McpServer::start(
            port,
            self.mcp.registry.clone(),
            self.mcp.events_tx.clone(),
            wake,
        ) {
            Ok(server) => {
                let url = server.url();
                self.mcp.server = Some(server);
                self.notify(
                    Notification::new(
                        Severity::Info,
                        Translatable::new("mcp:started").arg("url", url),
                    )
                    .source(NotificationSource::Mcp),
                );
            }
            Err(e) => {
                tracing::error!(error = %e, "MCP server failed to start");
                self.notify(
                    Notification::new(
                        Severity::Error,
                        Translatable::new("mcp:startFailed").arg("reason", e.to_string()),
                    )
                    .source(NotificationSource::Mcp),
                );
            }
        }
    }

    /// Disconnects every client; waiting requests are rejected.
    pub fn mcp_stop(&mut self) {
        let Some(server) = self.mcp.server.take() else {
            return;
        };
        server.stop();
        let stopped = || ToolReply::error("server_stopped", "The Notoir MCP server stopped");
        for p in std::mem::take(&mut self.mcp.prompts) {
            for w in p.waiters {
                w.call.respond(stopped());
            }
        }
        for s in std::mem::take(&mut self.mcp.saves) {
            s.call.respond(stopped());
        }
        // Calls already queued by the server.
        while let Ok(e) = self.mcp.events_rx.try_recv() {
            if let ServerEvent::Call(c) = e {
                c.respond(stopped());
            }
        }
        self.mcp.connected.clear();
        self.mcp.popover = false;
    }

    /// Per frame: server events, finished saves, prompt timeouts.
    pub fn mcp_tick(&mut self) {
        while let Ok(e) = self.mcp.events_rx.try_recv() {
            match e {
                ServerEvent::Call(call) => self.mcp_call(call),
                ServerEvent::Connected { client, name } => {
                    *self.mcp.connected.entry(client.clone()).or_default() += 1;
                    if let Some(c) = self.mcp.clients.iter_mut().find(|c| c.id == client) {
                        c.reported = Some(name);
                    }
                    self.mcp_seen(&client);
                }
                ServerEvent::Disconnected { client } => {
                    if let Some(n) = self.mcp.connected.get_mut(&client) {
                        *n = n.saturating_sub(1);
                    }
                }
                ServerEvent::Rejected { reason, client } => {
                    let reason = match reason {
                        RejectReason::MissingToken => "missing token",
                        RejectReason::InvalidToken => "invalid token",
                        RejectReason::Host => "host",
                        RejectReason::Origin => "origin",
                        RejectReason::RateLimited => "rate limited",
                    };
                    self.mcp_log(client, "-", None, format!("rejected: {reason}"), None);
                }
            }
        }
        while let Ok((call, target, started, reply)) = self.mcp.done_rx.try_recv() {
            self.mcp_reply(call, target, started, reply);
        }
        self.mcp_finish_saves();
        self.mcp_expire_prompts();
    }

    /// When the next prompt times out (for repaint scheduling).
    pub fn mcp_next_due(&self) -> Option<u64> {
        let timeout = self.settings.int("mcp.requestTimeoutMs", None).max(1000) as u64;
        let saves = self.mcp.saves.iter().map(|s| s.started + timeout);
        self.mcp
            .prompts
            .iter()
            .map(|p| p.deadline)
            .chain(saves)
            .min()
    }

    fn mcp_seen(&mut self, client: &ClientId) {
        let now = epoch_ms();
        if let Some(c) = self.mcp.clients.iter_mut().find(|c| &c.id == client) {
            c.last_seen = Some(now);
        }
        let last = self.mcp.seen_written.get(client).copied().unwrap_or(0);
        if now - last >= SEEN_WRITE_MS {
            self.mcp.seen_written.insert(client.clone(), now);
            if let Some(db) = &self.state_db {
                let _ = db.touch_mcp_client(&client.0, now);
            }
        }
    }

    /// Records an activity entry (never document content).
    fn mcp_log(
        &mut self,
        client: Option<ClientId>,
        tool: &str,
        target: Option<String>,
        outcome: String,
        duration_ms: Option<u64>,
    ) {
        tracing::info!(
            client = client.as_ref().map(|c| c.0.as_str()),
            tool,
            target = target.as_deref(),
            outcome = outcome.as_str(),
            "mcp request"
        );
        let entry = ActivityEntry {
            at: epoch_ms(),
            client,
            tool: tool.to_owned(),
            target,
            outcome,
            duration_ms,
        };
        if let Some(db) = &self.state_db {
            let _ = db.add_mcp_activity(&McpActivityRow {
                at: entry.at,
                client_id: entry.client.as_ref().map(|c| c.0.clone()),
                tool: entry.tool.clone(),
                target: entry.target.clone(),
                outcome: entry.outcome.clone(),
                duration_ms: entry.duration_ms.map(|d| d as i64),
            });
        }
        self.mcp.activity.push_front(entry);
        self.mcp.activity.truncate(ACTIVITY_KEEP);
    }

    fn mcp_target_label(&self, target: &Target) -> Option<String> {
        match target {
            Target::None => None,
            Target::Path(p) => Some(paths::display(
                self.workspace.as_ref().map(|w| w.root.as_path()),
                p,
            )),
        }
    }

    /// Answers a call and logs it.
    fn mcp_reply(&mut self, call: McpCall, target: Option<String>, started: u64, reply: ToolReply) {
        let outcome = match &reply {
            ToolReply::Ok { .. } => "ok".to_owned(),
            ToolReply::Err { code, .. } if code == "permission_denied" => "denied".to_owned(),
            ToolReply::Err { code, .. } => format!("error: {code}"),
        };
        let duration = self.now_ms.saturating_sub(started);
        self.mcp_log(
            Some(call.client.clone()),
            &call.tool,
            target,
            outcome,
            Some(duration),
        );
        call.respond(reply);
    }

    /// Resolves a call's path argument against the workspace boundary.
    fn mcp_resolve(&self, call: &McpCall) -> Result<Target, ToolReply> {
        let Some(path) = call.args.get("path") else {
            return Ok(Target::None);
        };
        let Some(path) = path.as_str() else {
            return Err(ToolReply::error("invalid_args", "path must be a string"));
        };
        match &self.workspace {
            Some(ws) => paths::resolve(&ws.root, path)
                .map(Target::Path)
                .map_err(|e| {
                    let msg = match e {
                        PathError::Outside => "The path is outside the workspace",
                        PathError::Invalid => "The path is not valid",
                    };
                    ToolReply::error(e.code(), msg)
                }),
            // No workspace: only open documents are reachable.
            None => {
                let (_, key) = fs_io::identity(std::path::Path::new(path));
                match self.workbench.find_by_key(&key) {
                    Some(doc) => Ok(Target::Path(
                        self.workbench
                            .document(doc)
                            .and_then(|d| d.path())
                            .map(PathBuf::from)
                            .unwrap_or_default(),
                    )),
                    None => Err(ToolReply::error(
                        "path_outside_workspace",
                        "No workspace is open; only open documents are accessible",
                    )),
                }
            }
        }
    }

    /// Gate, then run (or ask).
    fn mcp_call(&mut self, call: McpCall) {
        let started = self.now_ms;
        self.mcp_seen(&call.client);
        let Some(def) = tools::find(&call.tool) else {
            let reply = ToolReply::error("unknown_tool", format!("No tool named {}", call.tool));
            self.mcp_reply(call, None, started, reply);
            return;
        };
        let target = match self.mcp_resolve(&call) {
            Ok(t) => t,
            Err(reply) => {
                let label = call
                    .args
                    .get("path")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                self.mcp_reply(call, label, started, reply);
                return;
            }
        };
        self.mcp_gate(
            Waiting {
                call,
                target,
                started,
            },
            def,
        );
    }

    /// Re-evaluates the gate (also after a prompt answer, so changes made
    /// meanwhile apply).
    fn mcp_gate(&mut self, w: Waiting, def: &'static ToolDef) {
        let p = principal(&w.call.client);
        let mut ask = None;
        for cap in def.capabilities {
            match self.mcp.gate.check(&p, cap, true) {
                Decision::Deny => {
                    let label = self.mcp_target_label(&w.target);
                    self.mcp_reply(w.call, label, w.started, denied(cap));
                    return;
                }
                Decision::Ask => {
                    ask.get_or_insert(cap.clone());
                }
                Decision::Allow => {}
            }
        }
        match ask {
            Some(cap) => self.mcp_ask(w, cap),
            None => self.mcp_run(w),
        }
    }

    /// Queues a prompt; identical requests share one.
    fn mcp_ask(&mut self, w: Waiting, cap: Capability) {
        let target = self.mcp_target_label(&w.target);
        if let Some(p) = self
            .mcp
            .prompts
            .iter_mut()
            .find(|p| p.client == w.call.client && p.capability == cap && p.target == target)
        {
            p.waiters.push(w);
            return;
        }
        let id = self.mcp.next_prompt;
        self.mcp.next_prompt += 1;
        self.mcp.prompts.push_back(AskPrompt {
            id,
            client: w.call.client.clone(),
            capability: cap,
            target,
            deadline: self.now_ms + PROMPT_TIMEOUT_MS,
            waiters: vec![w],
            confirm_always: false,
        });
    }

    /// The user answered the first prompt.
    pub fn mcp_answer(&mut self, answer: PromptAnswer) {
        let Some(front) = self.mcp.prompts.front_mut() else {
            return;
        };
        if answer == PromptAnswer::Always
            && front.capability.risk() == RiskClass::Dangerous
            && !front.confirm_always
        {
            front.confirm_always = true;
            return;
        }
        let Some(prompt) = self.mcp.prompts.pop_front() else {
            return;
        };
        let p = principal(&prompt.client);
        let scope = match answer {
            PromptAnswer::Deny => {
                for w in prompt.waiters {
                    let label = self.mcp_target_label(&w.target);
                    self.mcp_reply(w.call, label, w.started, denied(&prompt.capability));
                }
                return;
            }
            PromptAnswer::Once => GrantScope::Once,
            PromptAnswer::Session => GrantScope::Session,
            PromptAnswer::Always => GrantScope::Persistent,
        };
        self.mcp.gate.grant(&p, &prompt.capability, scope);
        if scope == GrantScope::Persistent {
            self.mcp_persist(&prompt.client);
        }
        for w in prompt.waiters {
            let Some(def) = tools::find(&w.call.tool) else {
                continue;
            };
            if scope == GrantScope::Once {
                // This request only: skip the answered capability, check the rest.
                self.mcp_gate_except(w, def, &prompt.capability);
            } else {
                self.mcp_gate(w, def);
            }
        }
    }

    fn mcp_gate_except(&mut self, w: Waiting, def: &'static ToolDef, granted: &Capability) {
        let p = principal(&w.call.client);
        for cap in def.capabilities.iter().filter(|c| *c != granted) {
            match self.mcp.gate.check(&p, cap, true) {
                Decision::Allow => {}
                Decision::Deny => {
                    let label = self.mcp_target_label(&w.target);
                    self.mcp_reply(w.call, label, w.started, denied(cap));
                    return;
                }
                Decision::Ask => {
                    self.mcp_ask(w, cap.clone());
                    return;
                }
            }
        }
        // Revoked meanwhile?
        if self.mcp.gate.is_revoked(&p) {
            let label = self.mcp_target_label(&w.target);
            self.mcp_reply(w.call, label, w.started, denied(granted));
            return;
        }
        self.mcp_run(w);
    }

    fn mcp_expire_prompts(&mut self) {
        let now = self.now_ms;
        let mut keep = VecDeque::new();
        for mut p in std::mem::take(&mut self.mcp.prompts) {
            if now >= p.deadline {
                for w in p.waiters {
                    let label = self.mcp_target_label(&w.target);
                    let reply = ToolReply::Err {
                        code: "permission_denied".into(),
                        message: "Nobody answered the permission prompt in time".into(),
                        data: json!({ "capability": p.capability.key(), "timeout": true }),
                    };
                    self.mcp_reply(w.call, label, w.started, reply);
                }
                continue;
            }
            // Clients that gave up no longer wait.
            p.waiters.retain(|w| !w.call.abandoned());
            if !p.waiters.is_empty() {
                keep.push_back(p);
            }
        }
        self.mcp.prompts = keep;
    }

    // ── tools ────────────────────────────────────────────────────────────

    fn mcp_run(&mut self, w: Waiting) {
        let label = self.mcp_target_label(&w.target);
        let Waiting {
            call,
            target,
            started,
        } = w;
        let path = match &target {
            Target::Path(p) => Some(p.clone()),
            Target::None => None,
        };
        let reply = match call.tool.as_str() {
            "workspace_get" => self.tool_workspace_get(),
            "document_read" => match path {
                Some(p) => match self.tool_document_read(&p, &call.args) {
                    Some(r) => r,
                    None => {
                        // Not open: read from disk off the UI thread.
                        let args = call.args.clone();
                        let done = self.mcp.done_tx.clone();
                        let wake = self.mcp.wake.clone();
                        std::thread::spawn(move || {
                            let reply = read_from_disk(&p, &args);
                            if done.send((call, label, started, reply)).is_ok() {
                                wake();
                            }
                        });
                        return;
                    }
                },
                None => missing_path(),
            },
            "document_save" => match path {
                Some(p) => match self.tool_document_save(&p) {
                    Ok(doc) => {
                        self.mcp.saves.push(PendingSave {
                            doc,
                            call,
                            target: label,
                            started,
                        });
                        return;
                    }
                    Err(r) => r,
                },
                None => missing_path(),
            },
            "document_delete" => match path {
                Some(p) => self.tool_document_delete(p),
                None => missing_path(),
            },
            other => ToolReply::error("unknown_tool", format!("No tool named {other}")),
        };
        self.mcp_reply(call, label, started, reply);
    }

    fn tool_workspace_get(&self) -> ToolReply {
        let open = self.workbench.documents.len();
        match &self.workspace {
            Some(ws) => ToolReply::Ok {
                structured: json!({
                    "root": ws.root.to_string_lossy(),
                    "name": ws.name,
                    "git": self.git.repo.is_some(),
                    "open_documents": open,
                }),
                summary: format!("Workspace {} ({open} open documents)", ws.name),
            },
            None => ToolReply::Ok {
                structured: json!({ "root": null, "open_documents": open }),
                summary: format!("No workspace is open ({open} open documents)"),
            },
        }
    }

    /// The buffer of an open document; `None` when it is not open.
    fn tool_document_read(&self, path: &std::path::Path, args: &Value) -> Option<ToolReply> {
        let (_, key) = fs_io::identity(path);
        let doc = self.workbench.document(self.workbench.find_by_key(&key)?)?;
        let text = doc.buffer.rope().to_string();
        Some(read_reply(
            &text,
            args,
            json!({
                "dirty": doc.is_dirty(),
                "revision": doc.revision,
                "language": notoir_editor::language::definition(doc.language).name,
                "encoding": doc.encoding.label(),
                "eol": doc.line_ending.label(),
                "open": true,
            }),
        ))
    }

    fn tool_document_save(&mut self, path: &std::path::Path) -> Result<DocumentId, ToolReply> {
        let (_, key) = fs_io::identity(path);
        let Some(doc) = self.workbench.find_by_key(&key) else {
            return Err(ToolReply::error(
                "not_open",
                "The document is not open in Notoir; nothing to save",
            ));
        };
        // A save in flight (or nothing to save): wait for / report that.
        let clean = self.workbench.document(doc).is_some_and(|d| !d.is_dirty());
        if !self.saving.contains(&doc) && !clean {
            self.save(doc);
        }
        Ok(doc)
    }

    fn tool_document_delete(&mut self, path: PathBuf) -> ToolReply {
        if !path.exists() {
            return ToolReply::error("not_found", "No such file");
        }
        if path.is_dir() {
            return ToolReply::error("is_directory", "document_delete removes files only");
        }
        let shown = paths::display(self.workspace.as_ref().map(|w| w.root.as_path()), &path);
        self.delete_paths(vec![path], false);
        ToolReply::Ok {
            structured: json!({ "path": shown, "trashed": true }),
            summary: format!("Moved {shown} to the trash"),
        }
    }

    /// Answers saves whose pipeline finished.
    fn mcp_finish_saves(&mut self) {
        let mut waiting = Vec::new();
        for s in std::mem::take(&mut self.mcp.saves) {
            if self.saving.contains(&s.doc) {
                let timeout = self.settings.int("mcp.requestTimeoutMs", None).max(1000) as u64;
                if self.now_ms.saturating_sub(s.started) >= timeout {
                    let reply = ToolReply::error("timeout", "The save did not finish in time");
                    self.mcp_reply(s.call, s.target, s.started, reply);
                } else {
                    waiting.push(s);
                }
                continue;
            }
            let reply = match self.workbench.document(s.doc) {
                Some(d) if !d.is_dirty() => ToolReply::Ok {
                    structured: json!({ "saved": true, "revision": d.revision }),
                    summary: "Saved".into(),
                },
                Some(_) => ToolReply::error(
                    "save_failed",
                    "The document could not be saved (see Notoir for details)",
                ),
                None => ToolReply::error("not_open", "The document was closed"),
            };
            self.mcp_reply(s.call, s.target, s.started, reply);
        }
        self.mcp.saves = waiting;
    }

    // ── clients & permissions ────────────────────────────────────────────

    /// Registers a client; its token is kept in `new_client` for one display.
    pub fn mcp_add_client(&mut self, name: &str) -> NewClient {
        let id = notoir_mcp::auth::generate_client_id();
        let token = notoir_mcp::auth::generate_token();
        let hash = notoir_mcp::auth::hash_token(&token);
        let now = epoch_ms();
        if let Some(db) = &self.state_db {
            let _ = db.insert_mcp_client(&McpClientRow {
                client_id: id.0.clone(),
                name: name.to_owned(),
                token_hash: hash.clone(),
                created_at: now,
                last_seen: None,
                revoked: false,
            });
        }
        self.mcp.registry.add(ClientAuth {
            id: id.clone(),
            token_hash: hash,
            revoked: false,
        });
        self.mcp.clients.push(ClientInfo {
            id: id.clone(),
            name: name.to_owned(),
            reported: None,
            created_at: now,
            last_seen: None,
            revoked: false,
        });
        let created = NewClient {
            id,
            name: name.to_owned(),
            token,
        };
        self.mcp.new_client = Some(created.clone());
        created
    }

    /// Rejects the token from now on and denies its waiting requests.
    pub fn mcp_revoke(&mut self, id: &ClientId) {
        self.mcp.registry.revoke(id);
        self.mcp.gate.revoke(&principal(id));
        if let Some(db) = &self.state_db {
            let _ = db.set_mcp_client_revoked(&id.0);
        }
        if let Some(c) = self.mcp.clients.iter_mut().find(|c| &c.id == id) {
            c.revoked = true;
        }
        self.mcp.connected.remove(id);
        let mut keep = VecDeque::new();
        for p in std::mem::take(&mut self.mcp.prompts) {
            if &p.client == id {
                for w in p.waiters {
                    let label = self.mcp_target_label(&w.target);
                    self.mcp_reply(w.call, label, w.started, denied(&p.capability));
                }
            } else {
                keep.push_back(p);
            }
        }
        self.mcp.prompts = keep;
    }

    pub fn mcp_set_permission(&mut self, id: &ClientId, cap: Capability, d: Decision) {
        if self.mcp.gate.set(&principal(id), cap, d) {
            self.mcp_persist(id);
        }
    }

    pub fn mcp_reset_permissions(&mut self, id: &ClientId) {
        self.mcp.gate.reset(&principal(id));
        self.mcp_persist(id);
    }

    /// "Revoke All": every capability denied (the client stays registered).
    pub fn mcp_deny_all(&mut self, id: &ClientId) {
        let p = principal(id);
        for (_, caps) in MCP_MATRIX {
            for c in *caps {
                self.mcp.gate.set(&p, c.clone(), Decision::Deny);
            }
        }
        self.mcp.gate.clear_session(&p);
        self.mcp_persist(id);
    }

    fn mcp_persist(&mut self, id: &ClientId) {
        let grants: Vec<(String, String)> = self
            .mcp
            .gate
            .configured(&principal(id))
            .into_iter()
            .map(|(c, d)| (c.key(), d.as_str().to_owned()))
            .collect();
        if let Some(db) = &self.state_db {
            let _ = db.save_mcp_grants(&id.0, &grants);
        }
    }

    pub fn mcp_clear_activity(&mut self) {
        self.mcp.activity.clear();
        if let Some(db) = &self.state_db {
            let _ = db.clear_mcp_activity();
        }
    }
}

fn missing_path() -> ToolReply {
    ToolReply::error("invalid_args", "path is required")
}

/// Content (optionally lines `start_line..=end_line`) plus metadata.
fn read_reply(text: &str, args: &Value, mut meta: Value) -> ToolReply {
    let start = args.get("start_line").and_then(Value::as_u64);
    let end = args.get("end_line").and_then(Value::as_u64);
    let total = text.lines().count().max(1);
    let content = if start.is_some() || end.is_some() {
        let s = start.unwrap_or(1).max(1) as usize;
        let e = end.map_or(total, |e| e as usize).min(total);
        text.split_inclusive('\n')
            .skip(s - 1)
            .take(e.saturating_sub(s - 1))
            .collect::<String>()
    } else {
        if text.len() > MAX_READ_BYTES {
            return ToolReply::Err {
                code: "too_large".into(),
                message: "The document is larger than 2 MiB; pass start_line/end_line".into(),
                data: json!({ "bytes": text.len(), "lines": total }),
            };
        }
        text.to_owned()
    };
    let summary = format!("{} bytes, {} lines", content.len(), total);
    if let Some(m) = meta.as_object_mut() {
        m.insert("content".into(), Value::String(content));
        m.insert("lines".into(), json!(total));
    }
    ToolReply::Ok {
        structured: meta,
        summary,
    }
}

/// `document_read` of a document Notoir does not have open.
fn read_from_disk(path: &std::path::Path, args: &Value) -> ToolReply {
    match fs_io::load(path, &fs_io::LoadOptions::default()) {
        Ok(loaded) => read_reply(
            &loaded.text.text,
            args,
            json!({
                "dirty": false,
                "revision": 0,
                "language": notoir_editor::language::definition(loaded.text.language).name,
                "encoding": loaded.text.encoding.label(),
                "eol": loaded.text.line_ending.label(),
                "open": false,
            }),
        ),
        Err(fs_io::LoadError::NotFound) => ToolReply::error("not_found", "No such file"),
        Err(e) => ToolReply::error("read_failed", e.to_string()),
    }
}
