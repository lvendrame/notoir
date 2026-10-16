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

fn no_args() -> Value {
    json!({ "type": "object", "properties": {}, "additionalProperties": false })
}

fn path_only() -> Value {
    json!({
        "type": "object",
        "properties": {
            "path": { "type": "string", "description": "Path relative to the workspace root (or absolute inside it)." }
        },
        "required": ["path"],
        "additionalProperties": false
    })
}

fn read_args() -> Value {
    json!({
        "type": "object",
        "properties": {
            "path": { "type": "string", "description": "Path relative to the workspace root (or absolute inside it)." },
            "start_line": { "type": "integer", "minimum": 1, "description": "First line to return (1-based)." },
            "end_line": { "type": "integer", "minimum": 1, "description": "Last line to return (inclusive)." }
        },
        "required": ["path"],
        "additionalProperties": false
    })
}

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "workspace_get",
        capabilities: &[Capability::WorkspaceRead],
        description: "Describe the open workspace: root path, name, whether it is a Git repository, and how many documents are open in Notoir.",
        read_only: true,
        destructive: false,
        schema: no_args,
    },
    ToolDef {
        name: "document_read",
        capabilities: &[Capability::DocumentRead],
        description: "Read a document as Notoir holds it in memory: for an open file with unsaved edits this is the edited buffer, not the disk content. Returns content (optionally a line range) with dirty, revision, language, encoding and eol.",
        read_only: true,
        destructive: false,
        schema: read_args,
    },
    ToolDef {
        name: "document_save",
        capabilities: &[Capability::DocumentSave],
        description: "Save an open document to disk through Notoir's normal save pipeline (conflict checks included).",
        read_only: false,
        destructive: false,
        schema: path_only,
    },
    ToolDef {
        name: "document_delete",
        capabilities: &[Capability::FileDelete],
        description: "Move a workspace file to the system trash.",
        read_only: false,
        destructive: true,
        schema: path_only,
    },
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
    }
}
