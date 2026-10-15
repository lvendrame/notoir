//! Notoir text engine (docs/specs/01-architecture.md): rope buffer operations,
//! transactions and undo, selections, search, folding, outline, syntax state,
//! Markdown/JSON structure and diff model.
//!
//! Pure logic: no egui, no async runtime, no filesystem or network IO.

pub mod buffer;
pub mod change;
pub mod display_map;
pub mod edit_ops;
pub mod encoding;
pub mod find;
pub mod indent_detect;
pub mod language;
pub mod line_ending;
pub mod line_ops;
pub mod markdown;
pub mod motion;
pub mod natural;
pub mod selection;
pub mod syntax;
pub mod wrap;

pub use buffer::{Applied, EditKind, EditRequest, TextBuffer};
pub use change::{Assoc, ChangeError, ChangeSet, Edit};
pub use language::LanguageId;
pub use line_ending::LineEnding;
pub use ropey::Rope;
pub use selection::{Range, Selection};
