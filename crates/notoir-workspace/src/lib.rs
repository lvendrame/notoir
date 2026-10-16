//! Workspace and file-system services (docs/specs/01, 02, 07): loading and
//! saving document files, directory listing and the workspace index, Explorer
//! file operations, file watching and the persistent search index.

pub mod fs_io;
pub mod fs_ops;
pub mod links;
pub mod listing;
pub mod search;
pub mod text_index;
pub mod watcher;
