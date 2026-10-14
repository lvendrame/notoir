//! Notoir text engine (docs/specs/01-architecture.md): rope buffer operations,
//! transactions and undo, selections, search, folding, outline, syntax state,
//! Markdown/JSON structure and diff model.
//!
//! Pure logic: no egui, no async runtime, no filesystem or network IO.
