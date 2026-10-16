//! The MCP server (docs/specs/29 §Transport, AD-8): rmcp Streamable HTTP on
//! axum, loopback only, on its own thread and Tokio runtime. Every request
//! passes the guard (Host / Origin allow-lists, bearer token, rate limit);
//! tool calls are forwarded to the UI thread as [`ServerEvent::Call`] and
//! answered through a oneshot. The server never touches app state.

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr, TcpListener};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use notoir_core::permissions::ClientId;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, Implementation,
    InitializeRequestParams, InitializeResult, ListToolsResult, PaginatedRequestParams,
    ServerCapabilities, ServerConfig, Tool, ToolAnnotations,
};
use rmcp::service::RequestContext;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler};
use serde_json::{Value, json};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use crate::auth::ClientRegistry;
use crate::tools;

/// Default port (`mcp.port`).
pub const DEFAULT_PORT: u16 = 47823;
/// Ports tried after a busy one.
const PORT_ATTEMPTS: u16 = 20;
/// Per-client token bucket: sustained rate and burst.
const RATE_PER_SEC: f64 = 20.0;
const RATE_BURST: f64 = 50.0;
/// Upper bound on waiting for the UI thread; the app answers sooner (request
/// timeout, prompt timeout).
const HARD_TIMEOUT: Duration = Duration::from_secs(150);

pub type Wake = Arc<dyn Fn() + Send + Sync>;

/// Why a request was turned away before reaching MCP.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RejectReason {
    MissingToken,
    InvalidToken,
    Host,
    Origin,
    RateLimited,
}

impl RejectReason {
    pub fn as_str(self) -> &'static str {
        match self {
            RejectReason::MissingToken => "missing token",
            RejectReason::InvalidToken => "invalid token",
            RejectReason::Host => "host not allowed",
            RejectReason::Origin => "origin not allowed",
            RejectReason::RateLimited => "rate limited",
        }
    }
}

/// The result of a tool, produced on the UI thread.
#[derive(Clone, Debug, PartialEq)]
pub enum ToolReply {
    Ok {
        structured: Value,
        summary: String,
    },
    Err {
        /// `permission_denied`, `revision_conflict`, `path_outside_workspace`…
        code: String,
        message: String,
        /// Extra fields merged into the error object.
        data: Value,
    },
}

impl ToolReply {
    pub fn error(code: &str, message: impl Into<String>) -> ToolReply {
        ToolReply::Err {
            code: code.to_owned(),
            message: message.into(),
            data: Value::Null,
        }
    }

    fn into_result(self) -> CallToolResult {
        match self {
            ToolReply::Ok {
                structured,
                summary,
            } => {
                let mut r = CallToolResult::structured(structured);
                r.content = vec![rmcp::model::ContentBlock::text(summary)];
                r
            }
            ToolReply::Err {
                code,
                message,
                data,
            } => {
                let mut obj = json!({ "error": code, "message": message });
                if let (Some(o), Value::Object(extra)) = (obj.as_object_mut(), data) {
                    o.extend(extra);
                }
                let mut r = CallToolResult::structured_error(obj);
                r.content = vec![rmcp::model::ContentBlock::text(format!(
                    "{code}: {message}"
                ))];
                r
            }
        }
    }
}

/// A tool call waiting for the UI thread.
#[derive(Debug)]
pub struct McpCall {
    pub client: ClientId,
    pub tool: String,
    pub args: Value,
    reply: oneshot::Sender<ToolReply>,
}

impl McpCall {
    /// A call answered through the returned receiver (tests, other hosts).
    pub fn new(
        client: ClientId,
        tool: &str,
        args: Value,
    ) -> (McpCall, oneshot::Receiver<ToolReply>) {
        let (reply, rx) = oneshot::channel();
        (
            McpCall {
                client,
                tool: tool.to_owned(),
                args,
                reply,
            },
            rx,
        )
    }

    pub fn respond(self, reply: ToolReply) {
        let _ = self.reply.send(reply);
    }

    /// The client stopped waiting (timeout, disconnect).
    pub fn abandoned(&self) -> bool {
        self.reply.is_closed()
    }
}

#[derive(Debug)]
pub enum ServerEvent {
    Call(McpCall),
    Rejected {
        reason: RejectReason,
        client: Option<ClientId>,
    },
    /// A session was initialized; `name` is the client's self-reported
    /// `clientInfo.name` (display only).
    Connected {
        client: ClientId,
        name: String,
    },
    Disconnected {
        client: ClientId,
    },
}

struct Shared {
    events: Sender<ServerEvent>,
    wake: Wake,
}

impl Shared {
    fn send(&self, e: ServerEvent) {
        if self.events.send(e).is_ok() {
            (self.wake)();
        }
    }
}

/// Sends `Disconnected` when its session's handler is dropped.
struct SessionGuard {
    client: ClientId,
    shared: Arc<Shared>,
}

impl Drop for SessionGuard {
    fn drop(&mut self) {
        self.shared.send(ServerEvent::Disconnected {
            client: self.client.clone(),
        });
    }
}

/// One per MCP session.
struct Handler {
    shared: Arc<Shared>,
    session: Mutex<Option<SessionGuard>>,
}

fn client_of(ctx: &RequestContext<RoleServer>) -> Option<ClientId> {
    ctx.extensions
        .get::<axum::http::request::Parts>()
        .and_then(|p| p.extensions.get::<ClientId>())
        .cloned()
}

impl ServerHandler for Handler {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("notoir", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "Notoir is a text editor. document_* tools work on Notoir's in-memory documents \
                 (including unsaved edits); filesystem_* tools work on disk. Paths are relative \
                 to the open workspace.",
            )
    }

    async fn initialize(
        &self,
        request: InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, McpError> {
        context.peer.set_peer_info(request.clone());
        if let Some(client) = client_of(&context) {
            let name = request.client_info.name.clone();
            self.shared.send(ServerEvent::Connected {
                client: client.clone(),
                name,
            });
            *self.session.lock().expect("session lock") = Some(SessionGuard {
                client,
                shared: self.shared.clone(),
            });
        }
        self.negotiate_initialize(&request)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let tools = tools::TOOLS
            .iter()
            .map(|t| {
                let schema = match t.input_schema() {
                    Value::Object(m) => m,
                    _ => Default::default(),
                };
                let mut tool = Tool::new(t.name, t.description, Arc::new(schema));
                tool.annotations = Some(ToolAnnotations::from_raw(
                    None,
                    Some(t.read_only),
                    Some(t.destructive),
                    None,
                    Some(false),
                ));
                tool
            })
            .collect();
        Ok(ListToolsResult::with_all_items(tools))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let Some(client) = client_of(&context) else {
            return Err(McpError::invalid_request("unauthenticated", None));
        };
        if tools::find(&request.name).is_none() {
            return Ok(
                ToolReply::error("unknown_tool", format!("No tool named {}", request.name))
                    .into_result()
                    .into(),
            );
        }
        let args = request.arguments.map(Value::Object).unwrap_or(json!({}));
        let (call, rx) = McpCall::new(client, &request.name, args);
        self.shared.send(ServerEvent::Call(call));
        let reply = tokio::select! {
            r = tokio::time::timeout(HARD_TIMEOUT, rx) => match r {
                Ok(Ok(reply)) => reply,
                Ok(Err(_)) => ToolReply::error("server_stopped", "The Notoir MCP server stopped"),
                Err(_) => ToolReply::error("timeout", "Notoir did not answer in time"),
            },
            () = context.ct.cancelled() => ToolReply::error("cancelled", "Request cancelled"),
        };
        Ok(reply.into_result().into())
    }
}

/// Request guard state.
#[derive(Clone)]
struct Guard {
    port: u16,
    clients: ClientRegistry,
    shared: Arc<Shared>,
    buckets: Arc<Mutex<HashMap<ClientId, (f64, Instant)>>>,
}

impl Guard {
    fn host_ok(&self, headers: &HeaderMap) -> bool {
        let Some(host) = headers.get(header::HOST).and_then(|h| h.to_str().ok()) else {
            return false;
        };
        let port = self.port;
        host == format!("127.0.0.1:{port}") || host == format!("localhost:{port}")
    }

    /// Absent, or a loopback origin (DNS rebinding / browser protection).
    fn origin_ok(headers: &HeaderMap) -> bool {
        let Some(origin) = headers.get(header::ORIGIN) else {
            return true;
        };
        let Ok(origin) = origin.to_str() else {
            return false;
        };
        let Some(rest) = origin
            .strip_prefix("http://")
            .or_else(|| origin.strip_prefix("https://"))
        else {
            return false;
        };
        let host = if let Some(v6) = rest.strip_prefix('[') {
            v6.split(']').next().map(|h| format!("[{h}]"))
        } else {
            rest.split(':').next().map(str::to_owned)
        };
        matches!(
            host.as_deref(),
            Some("127.0.0.1") | Some("localhost") | Some("[::1]")
        )
    }

    fn allow_rate(&self, client: &ClientId) -> bool {
        let mut buckets = self.buckets.lock().expect("bucket lock");
        let now = Instant::now();
        let (tokens, last) = buckets.entry(client.clone()).or_insert((RATE_BURST, now));
        *tokens =
            (*tokens + now.duration_since(*last).as_secs_f64() * RATE_PER_SEC).min(RATE_BURST);
        *last = now;
        if *tokens >= 1.0 {
            *tokens -= 1.0;
            true
        } else {
            false
        }
    }

    fn reject(
        &self,
        status: StatusCode,
        reason: RejectReason,
        client: Option<ClientId>,
    ) -> Response {
        self.shared.send(ServerEvent::Rejected { reason, client });
        let mut r = (status, reason.as_str()).into_response();
        if status == StatusCode::UNAUTHORIZED {
            r.headers_mut().insert(
                header::WWW_AUTHENTICATE,
                axum::http::HeaderValue::from_static("Bearer"),
            );
        }
        r
    }
}

async fn guard(State(g): State<Guard>, mut req: Request, next: Next) -> Response {
    let headers = req.headers();
    if !g.host_ok(headers) {
        return g.reject(StatusCode::FORBIDDEN, RejectReason::Host, None);
    }
    if !Guard::origin_ok(headers) {
        return g.reject(StatusCode::FORBIDDEN, RejectReason::Origin, None);
    }
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer ").map(str::trim));
    let Some(token) = token else {
        return g.reject(StatusCode::UNAUTHORIZED, RejectReason::MissingToken, None);
    };
    let Some(client) = g.clients.authenticate(token) else {
        return g.reject(StatusCode::UNAUTHORIZED, RejectReason::InvalidToken, None);
    };
    if !g.allow_rate(&client) {
        return g.reject(
            StatusCode::TOO_MANY_REQUESTS,
            RejectReason::RateLimited,
            Some(client),
        );
    }
    req.extensions_mut().insert(client);
    next.run(req).await
}

/// A running server. Dropping it stops it.
pub struct McpServer {
    port: u16,
    cancel: CancellationToken,
}

impl McpServer {
    /// Binds `127.0.0.1:port` (or the next free port) and serves on a
    /// background thread.
    pub fn start(
        port: u16,
        clients: ClientRegistry,
        events: Sender<ServerEvent>,
        wake: Wake,
    ) -> std::io::Result<McpServer> {
        let listener = bind(port)?;
        let port = listener.local_addr()?.port();
        listener.set_nonblocking(true)?;
        let cancel = CancellationToken::new();
        let shared = Arc::new(Shared { events, wake });
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("notoir-mcp")
            .enable_all()
            .build()?;
        let token = cancel.clone();
        std::thread::Builder::new()
            .name("notoir-mcp".into())
            .spawn(move || {
                rt.block_on(serve(listener, port, clients, shared, token));
                rt.shutdown_timeout(Duration::from_secs(1));
            })?;
        tracing::info!(port, "MCP server started");
        Ok(McpServer { port, cancel })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/mcp", self.port)
    }

    /// Disconnects every client; requests still waiting get an error.
    pub fn stop(&self) {
        if !self.cancel.is_cancelled() {
            self.cancel.cancel();
            tracing::info!(port = self.port, "MCP server stopped");
        }
    }
}

impl Drop for McpServer {
    fn drop(&mut self) {
        self.stop();
    }
}

fn bind(port: u16) -> std::io::Result<TcpListener> {
    let mut last = None;
    for p in port..port.saturating_add(PORT_ATTEMPTS) {
        match TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, p))) {
            Ok(l) => return Ok(l),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| std::io::Error::other("no port")))
}

async fn serve(
    listener: TcpListener,
    port: u16,
    clients: ClientRegistry,
    shared: Arc<Shared>,
    cancel: CancellationToken,
) {
    let listener = match tokio::net::TcpListener::from_std(listener) {
        Ok(l) => l,
        Err(e) => {
            tracing::error!(error = %e, "MCP listener");
            return;
        }
    };
    let config = StreamableHttpServerConfig::default()
        .with_allowed_hosts([format!("127.0.0.1:{port}"), format!("localhost:{port}")])
        .with_cancellation_token(cancel.child_token());
    let factory_shared = shared.clone();
    let service = StreamableHttpService::new(
        move || {
            Ok(Handler {
                shared: factory_shared.clone(),
                session: Mutex::new(None),
            })
        },
        Arc::new(LocalSessionManager::default()),
        config,
    );
    let state = Guard {
        port,
        clients,
        shared,
        buckets: Default::default(),
    };
    let app = axum::Router::new()
        .route_service("/mcp", service)
        .layer(axum::middleware::from_fn_with_state(state, guard));
    let stop = cancel.clone();
    let server = axum::serve(listener, app).with_graceful_shutdown(async move {
        stop.cancelled().await;
    });
    // Streams may hold the graceful shutdown open; give them a moment.
    tokio::select! {
        r = server => if let Err(e) = r { tracing::error!(error = %e, "MCP server") },
        () = async {
            cancel.cancelled().await;
            tokio::time::sleep(Duration::from_millis(500)).await;
        } => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_allow_list() {
        let mut h = HeaderMap::new();
        assert!(Guard::origin_ok(&h));
        for (o, ok) in [
            ("http://127.0.0.1:47823", true),
            ("http://localhost", true),
            ("https://[::1]:3000", true),
            ("https://evil.example", false),
            ("http://localhost.evil.example", false),
            ("null", false),
        ] {
            h.insert(header::ORIGIN, o.parse().unwrap());
            assert_eq!(Guard::origin_ok(&h), ok, "{o}");
        }
    }
}
