//! Notoir domain core (docs/specs/01-architecture.md): documents registry,
//! editor views, groups and layout tree, commands, events, settings model,
//! permissions and workspace model.
//!
//! Pure domain logic: no egui, no async runtime, no filesystem or network IO.

pub mod command;
pub mod document;
pub mod events;
pub mod ids;
pub mod keymap;
pub mod layout;
pub mod notifications;
pub mod panels;
pub mod settings;
pub mod view;
pub mod when;
pub mod workbench;

pub use command::{CommandId, CommandInvocation, CommandOutcome, CommandRegistry, CommandSpec};
pub use document::{Document, DocumentOrigin, LoadedText, PathKey};
pub use events::{AppEvent, ChangeOrigin, EventOutbox};
pub use ids::{DocumentId, GroupId, ViewId};
pub use layout::{Direction, LayoutTree};
pub use settings::ResolvedSettings;
pub use view::EditorView;
pub use when::{ContextSnapshot, When};
pub use workbench::{CloseOutcome, TabPosition, Workbench, WorkbenchError};
