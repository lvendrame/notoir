//! Lua plugin host (docs/specs/30-lua-plugins.md, AD-9): manifests and
//! permissions, discovery, and one sandboxed Lua 5.4 VM per plugin on its own
//! thread. Every API call is decided by the app on the UI thread through the
//! permission gate (spec 32); this crate holds no app state.

pub mod manifest;
pub mod runtime;

pub use manifest::{API_VERSION, Discovered, Manifest, ManifestError, Permissions, scan};
pub use runtime::{
    ApiError, ApiResult, ErrorKind, FromPlugin, PluginEvents, PluginRequest, PluginRuntime,
    RuntimeConfig, ToPlugin, Wake,
};
