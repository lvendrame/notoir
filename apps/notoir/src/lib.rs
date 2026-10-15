//! Notoir application shell (egui/eframe). See `docs/specs/01-architecture.md`.

/// Product name: never translated (spec 22).
pub const BRAND: &str = "Notoir";

pub mod app;
pub mod cli;
pub mod commands;
pub mod compare;
pub mod config;
pub mod core;
pub mod editor_widget;
pub mod external;
pub mod files;
pub mod find;
pub mod fonts;
pub mod i18n;
pub mod icons;
pub mod input;
pub mod io;
pub mod json;
pub mod logging;
pub mod markdown;
pub mod menus;
pub mod navigation;
pub mod quick_open;
pub mod recovery;
pub mod search;
pub mod session;
pub mod settings_tab;
pub mod shell;
pub mod structure;
pub mod theme;
pub mod workspace;
