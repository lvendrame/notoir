//! Notoir's MCP server (docs/specs/29-mcp.md, AD-8): loopback Streamable HTTP
//! transport with bearer tokens and Host/Origin allow-lists, the tool
//! catalogue, and the stdio bridge. Tool calls are executed by the app on the
//! UI thread through the permission gate (spec 32); this crate holds no app
//! state.

pub mod auth;
pub mod bridge;
pub mod paths;
mod server;
pub mod tools;

pub use auth::{ClientAuth, ClientRegistry};
pub use server::{DEFAULT_PORT, McpCall, McpServer, RejectReason, ServerEvent, ToolReply, Wake};
