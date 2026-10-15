//! Notoir application shell (egui/eframe). See `docs/specs/01-architecture.md`.

/// Product name: never translated (spec 22).
pub const BRAND: &str = "Notoir";

pub mod app;
pub mod cli;
pub mod commands;
pub mod core;
pub mod editor_widget;
pub mod files;
pub mod fonts;
pub mod i18n;
pub mod icons;
pub mod input;
pub mod io;
pub mod logging;
pub mod menus;
pub mod shell;
pub mod theme;
pub mod workspace;
