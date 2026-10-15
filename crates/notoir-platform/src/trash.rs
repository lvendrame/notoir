//! Moving files to the OS trash (docs/specs/07 §Delete, 31).

use std::path::PathBuf;

/// Moves `paths` to the Recycle Bin / Trash. Errors carry the OS message;
/// callers then offer permanent deletion.
pub fn move_to_trash(paths: &[PathBuf]) -> Result<(), String> {
    trash::delete_all(paths).map_err(|e| e.to_string())
}
