//! Notoir application shell (egui/eframe). See `docs/specs/01-architecture.md`.

/// Product name: never translated (spec 22).
pub const BRAND: &str = "Notoir";

pub mod app;
pub mod cli;
pub mod commands;
pub mod compare;
pub mod config;
pub mod core;
pub mod csv;
pub mod desktop;
#[doc(hidden)]
pub mod docs_ref;
pub mod editor_widget;
pub mod external;
pub mod files;
pub mod find;
pub mod fonts;
pub mod git;
pub mod git_auto;
pub mod git_write;
pub mod help;
pub mod i18n;
pub mod icons;
pub mod input;
pub mod io;
pub mod json;
#[doc(hidden)]
pub mod live;
pub mod logging;
pub mod markdown;
pub mod mcp;
pub mod mcp_tools;
pub mod menus;
pub mod navigation;
pub mod notes;
pub mod plugin_contrib;
pub mod plugins;
pub mod quick_open;
pub mod recovery;
pub mod search;
pub mod session;
pub mod settings_tab;
pub mod shell;
pub mod structure;
pub mod templates;
pub mod text_index;
pub mod theme;
pub mod updates;
pub mod url_document;
pub mod workspace;
