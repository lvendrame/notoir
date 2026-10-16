//! Tool catalogue (docs/specs/29 §Tool Catalogue): wire names, the
//! capabilities each tool needs, English descriptions and input schemas.
//! The app executes the tools; this module only describes them.

use notoir_core::permissions::Capability;
use serde_json::{Value, json};

pub struct ToolDef {
    /// `snake_case` wire name (`document_read`).
    pub name: &'static str,
    /// Every capability the call needs (all are checked).
    pub capabilities: &'static [Capability],
    pub description: &'static str,
    pub read_only: bool,
    pub destructive: bool,
    schema: fn() -> Value,
}

impl ToolDef {
    pub fn input_schema(&self) -> Value {
        (self.schema)()
    }
}

fn object(properties: Value, required: &[&str]) -> Value {
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false
    })
}

const PATH: &str = "Path relative to the workspace root (or absolute inside it).";

fn no_args() -> Value {
    object(json!({}), &[])
}

fn path_only() -> Value {
    object(
        json!({ "path": { "type": "string", "description": PATH } }),
        &["path"],
    )
}

fn read_args() -> Value {
    object(
        json!({
            "path": { "type": "string", "description": PATH },
            "start_line": { "type": "integer", "minimum": 1, "description": "First line to return (1-based)." },
            "end_line": { "type": "integer", "minimum": 1, "description": "Last line to return (inclusive)." }
        }),
        &["path"],
    )
}

fn position() -> Value {
    json!({
        "description": "A position: {line, column} (1-based, column in characters) or {offset} (0-based character offset).",
        "type": "object",
        "properties": {
            "line": { "type": "integer", "minimum": 1 },
            "column": { "type": "integer", "minimum": 1 },
            "offset": { "type": "integer", "minimum": 0 }
        },
        "additionalProperties": false
    })
}

fn list_files_args() -> Value {
    object(
        json!({
            "glob": { "type": "string", "description": "Only paths matching this glob (e.g. \"docs/**/*.md\")." },
            "include_ignored": { "type": "boolean", "description": "Also list files excluded by .gitignore / files.exclude (default false)." },
            "cursor": { "type": "integer", "minimum": 0, "description": "Index to continue from (next_cursor of the previous page)." },
            "limit": { "type": "integer", "minimum": 1, "maximum": 2000, "description": "Page size (default 500)." }
        }),
        &[],
    )
}

fn search_args() -> Value {
    object(
        json!({
            "query": { "type": "string" },
            "regex": { "type": "boolean" },
            "case_sensitive": { "type": "boolean" },
            "whole_word": { "type": "boolean" },
            "include": { "type": "array", "items": { "type": "string" }, "description": "Globs of files to search." },
            "exclude": { "type": "array", "items": { "type": "string" }, "description": "Globs of files to skip." },
            "cursor": { "type": "integer", "minimum": 0 },
            "limit": { "type": "integer", "minimum": 1, "maximum": 1000, "description": "Matches per page (default 200)." }
        }),
        &["query"],
    )
}

fn open_args() -> Value {
    object(
        json!({
            "path": { "type": "string", "description": PATH },
            "show": { "type": "boolean", "description": "Also open it in an editor tab (needs editor control)." }
        }),
        &["path"],
    )
}

fn create_args() -> Value {
    object(
        json!({
            "path": { "type": "string", "description": "New file to create (must not exist). Omit with scratchpad: true." },
            "content": { "type": "string", "description": "Initial content (default empty)." },
            "scratchpad": { "type": "boolean", "description": "Create a Scratchpad instead of a file." }
        }),
        &[],
    )
}

fn write_args() -> Value {
    object(
        json!({
            "path": { "type": "string", "description": PATH },
            "content": { "type": "string", "description": "The complete new content of the buffer." },
            "expected_revision": { "type": "integer", "description": "Fail with revision_conflict if the document changed since this revision." }
        }),
        &["path", "content"],
    )
}

fn apply_edits_args() -> Value {
    object(
        json!({
            "path": { "type": "string", "description": PATH },
            "edits": {
                "type": "array",
                "description": "Non-overlapping replacements, in coordinates of the current revision; applied as one undoable step.",
                "items": {
                    "type": "object",
                    "properties": {
                        "start": position(),
                        "end": position(),
                        "text": { "type": "string" }
                    },
                    "required": ["start", "end", "text"],
                    "additionalProperties": false
                }
            },
            "expected_revision": { "type": "integer" }
        }),
        &["path", "edits"],
    )
}

fn rename_args() -> Value {
    object(
        json!({
            "path": { "type": "string", "description": PATH },
            "new_path": { "type": "string", "description": "Destination inside the workspace (must not exist)." }
        }),
        &["path", "new_path"],
    )
}

fn view_args() -> Value {
    object(
        json!({ "view": { "type": "integer", "description": "View id from editor_get_open_tabs (default: the active editor)." } }),
        &[],
    )
}

fn open_document_args() -> Value {
    object(
        json!({
            "path": { "type": "string", "description": PATH },
            "line": { "type": "integer", "minimum": 1 },
            "column": { "type": "integer", "minimum": 1 }
        }),
        &["path"],
    )
}

fn select_args() -> Value {
    object(
        json!({
            "path": { "type": "string", "description": "Document to select in (default: the active editor)." },
            "start": position(),
            "end": position()
        }),
        &["start", "end"],
    )
}

fn text_args() -> Value {
    object(json!({ "text": { "type": "string" } }), &["text"])
}

fn settings_get_args() -> Value {
    object(
        json!({ "prefix": { "type": "string", "description": "Only keys starting with this (e.g. \"editor.\")." } }),
        &[],
    )
}

fn settings_update_args() -> Value {
    object(
        json!({
            "key": { "type": "string" },
            "value": { "description": "New value (boolean, number, string or string list); null resets it." },
            "scope": { "type": "string", "enum": ["user", "workspace"], "description": "Default user." }
        }),
        &["key", "value"],
    )
}

fn git_diff_args() -> Value {
    object(
        json!({
            "path": { "type": "string", "description": "One file (default: every changed file)." },
            "base": { "type": "string", "enum": ["index", "head"], "description": "Compare the work tree with the index (default) or HEAD." }
        }),
        &[],
    )
}

fn fs_write_args() -> Value {
    object(
        json!({
            "path": { "type": "string", "description": PATH },
            "content": { "type": "string" },
            "create": { "type": "boolean", "description": "Allow creating a new file (default true)." }
        }),
        &["path", "content"],
    )
}

macro_rules! tool {
    ($name:literal, [$($cap:expr),+], $read_only:expr, $destructive:expr, $schema:ident, $desc:literal) => {
        ToolDef {
            name: $name,
            capabilities: &[$($cap),+],
            description: $desc,
            read_only: $read_only,
            destructive: $destructive,
            schema: $schema,
        }
    };
}

use Capability as C;

pub const TOOLS: &[ToolDef] = &[
    tool!(
        "workspace_get",
        [C::WorkspaceRead],
        true,
        false,
        no_args,
        "Describe the open workspace: root path, name, whether it is a Git repository, and how many documents are open in Notoir."
    ),
    tool!(
        "workspace_list_files",
        [C::WorkspaceRead],
        true,
        false,
        list_files_args,
        "List workspace files (paged, optional glob). Respects .gitignore and files.exclude unless include_ignored is set."
    ),
    tool!(
        "workspace_search",
        [C::WorkspaceSearch],
        true,
        false,
        search_args,
        "Search text or a regex across the workspace. Open documents are searched as Notoir holds them (unsaved edits included). Paged results with line and column."
    ),
    tool!(
        "document_open",
        [C::DocumentRead],
        true,
        false,
        open_args,
        "Load a document into Notoir's memory (no tab unless show is true). Returns its metadata and revision."
    ),
    tool!(
        "document_read",
        [C::DocumentRead],
        true,
        false,
        read_args,
        "Read a document as Notoir holds it in memory: for an open file with unsaved edits this is the edited buffer, not the disk content (use filesystem_read for disk). Returns content (optionally a line range) with dirty, revision, language, encoding and eol."
    ),
    tool!(
        "document_create",
        [C::DocumentCreate],
        false,
        false,
        create_args,
        "Create a new file (written to disk immediately) or a Scratchpad, and open it in Notoir."
    ),
    tool!(
        "document_write",
        [C::DocumentModify],
        false,
        false,
        write_args,
        "Replace the whole buffer of a document in Notoir's memory. The change is undoable, marks the document dirty and is NOT saved: call document_save to write it."
    ),
    tool!(
        "document_apply_edits",
        [C::DocumentModify],
        false,
        false,
        apply_edits_args,
        "Apply range edits to a document in Notoir's memory atomically (one undoable step). Not saved until document_save."
    ),
    tool!(
        "document_save",
        [C::DocumentSave],
        false,
        false,
        path_only,
        "Save an open document to disk through Notoir's normal save pipeline (conflict checks included)."
    ),
    tool!(
        "document_rename",
        [C::FileRename],
        false,
        false,
        rename_args,
        "Rename or move a workspace file; open documents follow it."
    ),
    tool!(
        "document_delete",
        [C::FileDelete],
        false,
        true,
        path_only,
        "Move a workspace file to the system trash."
    ),
    tool!(
        "editor_get_open_tabs",
        [C::EditorRead],
        true,
        false,
        no_args,
        "List editor groups and their tabs (view id, path or title, dirty, pinned, active)."
    ),
    tool!(
        "editor_get_active_document",
        [C::EditorRead],
        true,
        false,
        no_args,
        "The active editor's document (path, view, revision, dirty) and its selections."
    ),
    tool!(
        "editor_get_selection",
        [C::EditorRead, C::DocumentRead],
        true,
        false,
        view_args,
        "The text the user has selected in the active editor (or the given view), with the file and the range of every selection (1-based line/column and 0-based offsets)."
    ),
    tool!(
        "editor_open_document",
        [C::EditorControl],
        false,
        false,
        open_document_args,
        "Open or focus a document in an editor tab, optionally at a line and column. Visible to the user."
    ),
    tool!(
        "editor_select_range",
        [C::EditorControl],
        false,
        false,
        select_args,
        "Set the selection in the active editor (or the document's editor). Visible to the user."
    ),
    tool!(
        "editor_insert",
        [C::DocumentModify, C::EditorRead],
        false,
        false,
        text_args,
        "Insert text at every cursor of the active editor (selected text is kept). Undoable; not saved."
    ),
    tool!(
        "editor_replace",
        [C::DocumentModify, C::EditorRead],
        false,
        false,
        text_args,
        "Replace the current selection(s) of the active editor with text. Undoable; not saved."
    ),
    tool!(
        "settings_get",
        [C::SettingsRead],
        true,
        false,
        settings_get_args,
        "Effective Notoir settings (machine paths are left out)."
    ),
    tool!(
        "settings_update",
        [C::SettingsModify],
        false,
        false,
        settings_update_args,
        "Change a User or Workspace setting. Restricted settings can never be changed by MCP."
    ),
    tool!(
        "git_status",
        [C::GitRead],
        true,
        false,
        no_args,
        "Git state of the workspace: branch, upstream, ahead/behind, staged changes, changes and conflicts."
    ),
    tool!(
        "git_diff",
        [C::GitRead],
        true,
        false,
        git_diff_args,
        "Unified diff of the work tree (disk) against the index or HEAD, for one file or every changed file."
    ),
    tool!(
        "filesystem_read",
        [C::FilesystemRead],
        true,
        false,
        read_args,
        "Read a file from disk directly, ignoring Notoir's unsaved buffers (use document_read for those)."
    ),
    tool!(
        "filesystem_write",
        [C::FilesystemWrite],
        false,
        true,
        fs_write_args,
        "Write a file on disk directly (atomic write). Open documents are not changed: Notoir treats it as an external change."
    ),
];

pub fn find(name: &str) -> Option<&'static ToolDef> {
    TOOLS.iter().find(|t| t.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_wire_safe_and_schemas_are_objects() {
        for t in TOOLS {
            assert!(
                t.name.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "{}",
                t.name
            );
            assert_eq!(t.input_schema()["type"], "object");
            assert!(!t.capabilities.is_empty());
        }
        assert!(find("document_read").is_some());
        assert!(find("document.read").is_none());
        let mut names: Vec<&str> = TOOLS.iter().map(|t| t.name).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), TOOLS.len());
    }
}
