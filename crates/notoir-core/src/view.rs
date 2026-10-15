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
        }
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

    pub fn iter(&self) -> impl Iterator<Item = &EditorView> {
        self.views.values()
    }

    pub fn of_document(&self, doc: DocumentId) -> impl Iterator<Item = &EditorView> {
        self.views.values().filter(move |v| v.document_id == doc)
    }

    pub fn count_of_document(&self, doc: DocumentId) -> usize {
        self.of_document(doc).count()
    }

    /// Transforms the selections of every view of `doc` except `skip`.
    pub fn map_selections(&mut self, doc: DocumentId, changes: &ChangeSet, skip: Option<ViewId>) {
        for v in self.views.values_mut() {
            if v.document_id == doc && Some(v.id) != skip {
                v.selection = v.selection.map(changes);
            }
        }
    }
}
