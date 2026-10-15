//! `EditorView`: per-tab presentation state over a document
//! (docs/specs/03-editor-views-groups-and-splits.md).

use std::collections::HashMap;

use notoir_editor::{ChangeSet, Selection};

use crate::ids::{DocumentId, ViewId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewKind {
    Text,
    MarkdownPreview,
    MarkdownSplit,
    JsonTree,
    JsonSplit,
    Settings,
    /// First-run / Help → Welcome page.
    Welcome,
    /// Compare tab (spec 27); its panes are detached views.
    Diff,
}

/// Top visible display row plus pixel offsets.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScrollPosition {
    pub top_line: usize,
    pub pixel_offset: f32,
    pub horizontal: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EditorView {
    pub id: ViewId,
    pub document_id: DocumentId,
    pub kind: ViewKind,
    pub selection: Selection,
    pub scroll: ScrollPosition,
    /// Only used when `editor.zoomScope = "view"`.
    pub zoom: Option<f32>,
    pub wrap_override: Option<bool>,
    pub pinned: bool,
    /// Editor share of a Markdown split (0.2–0.8).
    pub preview_ratio: f32,
    /// Folded regions (spec 12), anchored to the text so they survive edits.
    pub folds: Vec<Fold>,
    /// Not a tab: a pane inside another view (the sides of a diff tab).
    pub detached: bool,
}

/// A folded region: the header line (containing `header`) stays visible; the
/// lines after it up to the line containing `end` hide.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fold {
    /// Start of the header line.
    pub header: usize,
    /// A position on the last hidden line (its end).
    pub end: usize,
}

impl EditorView {
    pub fn new(id: ViewId, document_id: DocumentId) -> Self {
        Self {
            id,
            document_id,
            kind: ViewKind::Text,
            selection: Selection::default(),
            scroll: ScrollPosition::default(),
            zoom: None,
            wrap_override: None,
            pinned: false,
            preview_ratio: 0.5,
            folds: Vec::new(),
            detached: false,
        }
    }

    /// Hidden line ranges (inclusive) of the folds in `rope`.
    pub fn hidden_lines(&self, rope: &notoir_editor::Rope) -> Vec<(usize, usize)> {
        let len = rope.len_chars();
        self.folds
            .iter()
            .filter_map(|f| {
                let h = rope.char_to_line(f.header.min(len));
                let e = rope.char_to_line(f.end.min(len));
                (e > h).then_some((h + 1, e))
            })
            .collect()
    }

    /// Unfolds every fold hiding `line`; returns whether any was removed.
    pub fn unfold_line(&mut self, rope: &notoir_editor::Rope, line: usize) -> bool {
        let len = rope.len_chars();
        let before = self.folds.len();
        self.folds.retain(|f| {
            let h = rope.char_to_line(f.header.min(len));
            let e = rope.char_to_line(f.end.min(len));
            !(h < line && line <= e)
        });
        self.folds.len() != before
    }

    /// A copy for Duplicate View / split: same document and position, new id,
    /// never pinned.
    pub fn duplicate(&self, id: ViewId) -> Self {
        Self {
            id,
            pinned: false,
            ..self.clone()
        }
    }
}

#[derive(Debug, Default)]
pub struct ViewRegistry {
    views: HashMap<ViewId, EditorView>,
}

impl ViewRegistry {
    pub fn insert(&mut self, view: EditorView) {
        self.views.insert(view.id, view);
    }

    pub fn remove(&mut self, id: ViewId) -> Option<EditorView> {
        self.views.remove(&id)
    }

    pub fn get(&self, id: ViewId) -> Option<&EditorView> {
        self.views.get(&id)
    }

    pub fn get_mut(&mut self, id: ViewId) -> Option<&mut EditorView> {
        self.views.get_mut(&id)
    }

    /// Tab views (detached panes excluded).
    pub fn iter(&self) -> impl Iterator<Item = &EditorView> {
        self.views.values().filter(|v| !v.detached)
    }

    /// Tab views of `doc`.
    pub fn of_document(&self, doc: DocumentId) -> impl Iterator<Item = &EditorView> {
        self.iter().filter(move |v| v.document_id == doc)
    }

    /// Every view showing `doc`, detached panes included: a document stays
    /// open while any of them does.
    pub fn count_of_document(&self, doc: DocumentId) -> usize {
        self.views.values().filter(|v| v.document_id == doc).count()
    }

    /// Transforms the selections of every view of `doc` except `skip`, and
    /// the folds of every view of `doc`.
    pub fn map_selections(&mut self, doc: DocumentId, changes: &ChangeSet, skip: Option<ViewId>) {
        for v in self.views.values_mut() {
            if v.document_id != doc {
                continue;
            }
            if Some(v.id) != skip {
                v.selection = v.selection.map(changes);
            }
            for f in &mut v.folds {
                f.header = changes.map_pos(f.header, notoir_editor::Assoc::Before);
                f.end = changes.map_pos(f.end, notoir_editor::Assoc::Before);
            }
            v.folds.retain(|f| f.end > f.header);
        }
    }
}
