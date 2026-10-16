//! JSON views and commands (docs/specs/25): parse cache (last valid tree kept
//! while the text is invalid), Text / Split / Tree modes, format / minify /
//! sort keys as single undo steps, node selection, and tree edits (value,
//! key, add, delete, move) as minimal text edits in one undo step.

use std::collections::HashMap;
use std::sync::Arc;

use notoir_core::ids::{DocumentId, ViewId};
use notoir_core::notifications::{Notification, Severity, Translatable};
use notoir_core::view::ViewKind;
use notoir_editor::json::{self, Dialect, JsonError, JsonNode};
use notoir_editor::json_edit::{self, NewValue, TreeEditError};
use notoir_editor::{ChangeSet, EditKind, Range, Selection};

use crate::core::AppCore;

/// Re-parse at most this often while typing.
pub const PARSE_DEBOUNCE_MS: u64 = 100;
/// Smaller documents re-parse on every change.
const DEBOUNCE_ABOVE_CHARS: usize = 256 * 1024;

pub struct JsonParsed {
    pub revision: u64,
    /// Last valid structure (kept, greyed out, while the text is invalid).
    pub roots: Arc<Vec<JsonNode>>,
    pub error: Option<JsonError>,
    pub at_ms: u64,
}

/// An edit made in the JSON tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TreeOp {
    /// A scalar's new value as typed (strings unquoted).
    Value(String),
    Key(String),
    /// A new member at the end (objects get a fresh `newKey…` name).
    Add(NewValue),
    Delete,
    Move(isize),
}

/// Translation key for a refused tree edit.
pub fn tree_error_key(e: &TreeEditError) -> &'static str {
    match e {
        TreeEditError::InvalidValue("number") => "json:edit.invalidNumber",
        TreeEditError::InvalidValue(_) => "json:edit.invalidBoolean",
        TreeEditError::DuplicateKey => "json:edit.duplicateKey",
        TreeEditError::NoNode | TreeEditError::NotAllowed => "json:edit.notAllowed",
    }
}

#[derive(Default)]
pub struct JsonState {
    pub parsed: HashMap<DocumentId, JsonParsed>,
}

impl AppCore {
    pub fn json_dialect(&self, doc: DocumentId) -> Option<Dialect> {
        let d = self.workbench.document(doc)?;
        Dialect::for_language(d.language.as_str())
    }

    pub fn is_json(&self, doc: DocumentId) -> bool {
        self.json_dialect(doc).is_some()
    }

    pub fn set_json_mode(&mut self, view: ViewId, mode: &str) {
        let kind = match mode {
            "split" => ViewKind::JsonSplit,
            "tree" => ViewKind::JsonTree,
            _ => ViewKind::Text,
        };
        if let Some(v) = self.workbench.views.get_mut(view) {
            v.kind = kind;
        }
        if kind != ViewKind::JsonTree {
            self.effects.push(crate::core::Effect::FocusEditor(view));
        }
    }

    /// The parsed document (debounced), and whether a newer parse is pending.
    pub fn json_doc(
        &mut self,
        doc: DocumentId,
    ) -> Option<(Arc<Vec<JsonNode>>, Option<JsonError>, bool)> {
        let dialect = self.json_dialect(doc)?;
        let d = self.workbench.document(doc)?;
        let revision = d.revision;
        let now = self.now_ms;
        if let Some(p) = self.json.parsed.get(&doc) {
            if p.revision == revision {
                return Some((p.roots.clone(), p.error.clone(), false));
            }
            if d.buffer.len_chars() > DEBOUNCE_ABOVE_CHARS
                && now.saturating_sub(p.at_ms) < PARSE_DEBOUNCE_MS
            {
                return Some((p.roots.clone(), p.error.clone(), true));
            }
        }
        let parsed = json::parse(&d.buffer.text(), dialect);
        let roots = match (&parsed.error, self.json.parsed.get(&doc)) {
            // Invalid: keep the last good tree.
            (Some(_), Some(prev)) if parsed.roots.is_empty() => prev.roots.clone(),
            _ => Arc::new(parsed.roots),
        };
        self.json.parsed.insert(
            doc,
            JsonParsed {
                revision,
                roots: roots.clone(),
                error: parsed.error.clone(),
                at_ms: now,
            },
        );
        Some((roots, parsed.error, false))
    }

    /// Error diagnostics for the editor squiggle (char ranges).
    pub fn json_diagnostics(&self, doc: DocumentId) -> Vec<(usize, usize)> {
        match self.json.parsed.get(&doc).and_then(|p| p.error.as_ref()) {
            Some(e) => {
                let len = self
                    .workbench
                    .document(doc)
                    .map_or(0, |d| d.buffer.len_chars());
                let start = e.offset.min(len.saturating_sub(1));
                vec![(start, (start + 1).min(len.max(start + 1)))]
            }
            None => Vec::new(),
        }
    }

    fn indent_text(&self, doc: DocumentId) -> String {
        let style = self.indent_style(doc);
        if style.insert_spaces {
            " ".repeat(style.tab_size.max(1))
        } else {
            "\t".to_owned()
        }
    }

    /// Format / Minify / Sort Keys: the whole text replaced as one undo step;
    /// invalid JSON is refused with the error location.
    pub fn json_transform(&mut self, view: ViewId, op: &str) {
        let Some(doc) = self.workbench.view(view).map(|v| v.document_id) else {
            return;
        };
        let Some(dialect) = self.json_dialect(doc) else {
            return;
        };
        let Some(d) = self.workbench.document(doc) else {
            return;
        };
        if d.is_read_only() {
            return;
        }
        let src = d.buffer.text();
        let indent = self.indent_text(doc);
        if op == "sort" && json::has_comments(&src) {
            let now = self.now_ms;
            self.notifications.notify(
                Notification::new(
                    Severity::Warning,
                    Translatable::new("notifications:jsonSortComments"),
                ),
                now,
            );
            return;
        }
        let result = match op {
            "minify" => json::minify(&src, dialect),
            "sort" => json::sort_keys(&src, dialect, &indent),
            _ => json::format(&src, dialect, &indent),
        };
        match result {
            Ok(text) if text != src => {
                let len = d.buffer.len_chars();
                let changes = ChangeSet::replace(0, len, text.as_str());
                let now = self.now_ms;
                let _ =
                    self.workbench
                        .edit(view, changes, Selection::caret(0), EditKind::Other, now);
            }
            Ok(_) => {}
            Err(e) => {
                let now = self.now_ms;
                self.notifications.notify(
                    Notification::new(
                        Severity::Error,
                        Translatable::new("notifications:jsonInvalid")
                            .arg("line", e.line)
                            .arg("column", e.column)
                            .arg("message", &e.message),
                    )
                    .dedupe("json-invalid"),
                    now,
                );
                let _ = self
                    .workbench
                    .set_selection(view, Selection::caret(e.offset));
                self.find.center.push(view);
            }
        }
    }

    /// Tree click: the node's value (or key and value) selected in the text.
    pub fn select_json_range(&mut self, doc: DocumentId, range: (usize, usize)) {
        let Some(view) = self.workbench.views.of_document(doc).next().map(|v| v.id) else {
            return;
        };
        let len = self
            .workbench
            .document(doc)
            .map_or(0, |d| d.buffer.len_chars());
        let (a, b) = (range.0.min(len), range.1.min(len));
        let _ = self
            .workbench
            .set_selection(view, Selection::new(vec![Range::new(a, b)], 0));
        self.find.center.push(view);
    }

    /// `json.goToParent`: caret to the parent node's opening bracket.
    pub fn json_go_to_parent(&mut self, view: ViewId) {
        let Some(doc) = self.workbench.view(view).map(|v| v.document_id) else {
            return;
        };
        let caret = self
            .workbench
            .view(view)
            .map_or(0, |v| v.selection.primary().head);
        let Some((roots, _, _)) = self.json_doc(doc) else {
            return;
        };
        let chain = json::path_at(&roots, caret);
        // Deepest container that starts before the caret, excluding the node
        // the caret is on.
        let target = chain
            .iter()
            .rev()
            .skip(1)
            .find(|n| n.range.0 < caret)
            .or(chain.first());
        if let Some(n) = target {
            let _ = self
                .workbench
                .set_selection(view, Selection::caret(n.range.0));
        }
    }

    /// Applies a tree edit to `view`'s document as one undo step. Refused
    /// (with a notification) while the text does not parse or the tree is
    /// older than the text. Returns the edited node's path afterwards.
    pub fn json_tree_edit(
        &mut self,
        view: ViewId,
        path: &[usize],
        op: TreeOp,
    ) -> Option<Vec<usize>> {
        let doc = self.workbench.view(view)?.document_id;
        let revision = self.workbench.document(doc)?.revision;
        let current = self
            .json
            .parsed
            .get(&doc)
            .filter(|p| p.revision == revision && p.error.is_none())
            .map(|p| p.roots.clone());
        let d = self.workbench.document(doc)?;
        if d.is_read_only() {
            return None;
        }
        let Some(roots) = current else {
            self.json_tree_refused("json:edit.invalidJson");
            return None;
        };
        let src = d.buffer.text();
        let unit = self.indent_text(doc);
        let result = match op {
            TreeOp::Value(v) => {
                json_edit::set_value(&roots, path, &v).map(|e| (e, path.to_vec(), None))
            }
            TreeOp::Key(k) => {
                json_edit::rename_key(&roots, path, &k).map(|e| (e, path.to_vec(), None))
            }
            TreeOp::Add(value) => {
                let container = path.split_first().and_then(|(f, rest)| {
                    rest.iter()
                        .try_fold(roots.get(*f)?, |n, i| n.children.get(*i))
                });
                match container {
                    Some(c) => {
                        let key = (c.kind == json::Kind::Object)
                            .then(|| json_edit::unique_key(c, "newKey"));
                        let mut new_path = path.to_vec();
                        new_path.push(c.children.len());
                        json_edit::add_child(&src, &roots, path, key.as_deref(), value, &unit)
                            .map(|(e, start)| (e, new_path, Some(start)))
                    }
                    None => Err(TreeEditError::NoNode),
                }
            }
            TreeOp::Delete => json_edit::delete(&roots, path).map(|e| {
                let mut p = path.to_vec();
                // The previous sibling (or the parent) is selected next.
                match p.last_mut() {
                    Some(0) => {
                        p.pop();
                    }
                    Some(i) => *i -= 1,
                    None => {}
                }
                (e, p, None)
            }),
            TreeOp::Move(delta) => {
                json_edit::move_child(&src, &roots, path, delta).map(|(e, p)| {
                    // The moved member's start after both swaps.
                    let at = if delta < 0 {
                        e[0].start
                    } else {
                        let grown =
                            e[0].text.chars().count() as isize - (e[0].end - e[0].start) as isize;
                        (e[1].start as isize + grown).max(0) as usize
                    };
                    (e, p, Some(at))
                })
            }
        };
        let (edits, new_path, at) = match result {
            Ok(r) => r,
            Err(e) => {
                self.json_tree_refused(tree_error_key(&e));
                return None;
            }
        };
        if edits.is_empty() {
            return Some(new_path);
        }
        let caret = at.unwrap_or(edits[0].start);
        let changes = ChangeSet::new(edits).ok()?;
        let now = self.now_ms;
        self.workbench
            .edit(view, changes, Selection::caret(caret), EditKind::Other, now)
            .ok()?;
        Some(new_path)
    }

    fn json_tree_refused(&mut self, key: &str) {
        let now = self.now_ms;
        self.notifications.notify(
            Notification::new(Severity::Warning, Translatable::new(key)).dedupe("json-tree-edit"),
            now,
        );
    }
}
