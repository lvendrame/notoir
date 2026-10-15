//! Workspace and file-system services (docs/specs/01, 02, 07): loading and
//! saving document files, directory listing and the workspace index, Explorer
//! file operations and file watching.

pub mod fs_io;
pub mod fs_ops;
pub mod listing;
pub mod search;
pub mod watcher;
