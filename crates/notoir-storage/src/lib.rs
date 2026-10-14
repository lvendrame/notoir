//! Persistent storage (docs/specs/09, 20, 23): settings and keybinding files,
//! the SQLite state store, the Scratchpad store, the recovery store and
//! crash-safe file writes. Paths come from `notoir-platform::AppPaths`.

pub mod atomic;
pub mod recovery;
pub mod scratchpad;
pub mod settings_file;
pub mod state_db;

pub use atomic::{WriteOptions, WriteStrategy, content_hash};
pub use state_db::{OpenReport, RecentEntry, RecentKind, StateDb, StorageError};
