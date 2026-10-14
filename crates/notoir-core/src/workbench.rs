//! `Workbench`: documents, views and the editor layout, mutated only through
//! these methods so selections, dirty state, history and events stay
//! consistent for every origin (UI, MCP, plugins) — docs/specs/01-architecture.md
//! §Mutation Model, 02, 03.

use std::collections::BTreeMap;
use std::sync::Arc;

use notoir_editor::{ChangeError, ChangeSet, EditKind, EditRequest, Selection};

use crate::document::{Document, DocumentOrigin, LoadedText, PathKey};
use crate::events::{AppEvent, ChangeOrigin, EventOutbox};
use crate::ids::{DocumentId, GroupId, IdGen, ViewId};
use crate::layout::{Direction, EditorGroup, LayoutTree};
use crate::view::{EditorView, ViewRegistry};

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum WorkbenchError {
    #[error("no such view")]
    NoSuchView,
    #[error("no such document")]
    NoSuchDocument,
    #[error("no such editor group")]
    NoSuchGroup,
    #[error("document is read-only")]
    ReadOnly,
    #[error(transparent)]
    Change(#[from] ChangeError),
}

/// Result of closing a view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseOutcome {
    Closed,
    /// Last view of a dirty document: the caller must ask Save / Don't Save /
    /// Cancel and then close with `force`.
    NeedsConfirmation(DocumentId),
}

/// Where a new tab goes in its group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabPosition {
    AfterActive,
    End,
    Index(usize),
}

#[derive(Debug)]
pub struct Workbench {
    ids: IdGen,
    pub documents: BTreeMap<DocumentId, Document>,
    pub views: ViewRegistry,
    pub layout: LayoutTree,
    pub events: EventOutbox,
}

impl Default for Workbench {
    fn default() -> Self {
        Self::new()
    }
}

impl Workbench {
    pub fn new() -> Self {
        let mut ids = IdGen::default();
        let first = GroupId(ids.fresh());
        Self {
            ids,
            documents: BTreeMap::new(),
            views: ViewRegistry::default(),
            layout: LayoutTree::new(first),
            events: EventOutbox::default(),
        }
    }

    // ── documents ────────────────────────────────────────────────────────

    pub fn document(&self, id: DocumentId) -> Option<&Document> {
        self.documents.get(&id)
    }

    pub fn document_mut(&mut self, id: DocumentId) -> Option<&mut Document> {
        self.documents.get_mut(&id)
    }

    pub fn find_by_key(&self, key: &PathKey) -> Option<DocumentId> {
        self.documents
            .values()
            .find(|d| d.path_key() == Some(key))
            .map(|d| d.id)
    }

    /// Lowest positive `n` not used by an open Untitled document.
    pub fn next_untitled_number(&self) -> u32 {
        (1..)
            .find(|n| {
                !self
                    .documents
                    .values()
                    .any(|d| d.origin == DocumentOrigin::Untitled { n: *n })
            })
            .unwrap_or(1)
    }

    /// Registers a document. A file whose path key is already open returns the
    /// existing document; its content is never loaded twice.
    pub fn open_document(&mut self, origin: DocumentOrigin, loaded: LoadedText) -> DocumentId {
        if let DocumentOrigin::File { key, .. } = &origin
            && let Some(existing) = self.find_by_key(key)
        {
            return existing;
        }
        let id = DocumentId(self.ids.fresh());
        self.documents.insert(id, Document::new(id, origin, loaded));
        self.events.emit(AppEvent::DocumentOpened { doc: id });
        id
    }

    pub fn mark_saved(
        &mut self,
        doc: DocumentId,
        state: u64,
        content_hash: u64,
    ) -> Result<(), WorkbenchError> {
        let d = self
            .documents
            .get_mut(&doc)
            .ok_or(WorkbenchError::NoSuchDocument)?;
        d.mark_saved(state, content_hash);
        self.events.emit(AppEvent::DocumentSaved { doc });
        Ok(())
    }

    // ── views & tabs ─────────────────────────────────────────────────────

    pub fn view(&self, id: ViewId) -> Option<&EditorView> {
        self.views.get(id)
    }

    pub fn active_view(&self) -> Option<ViewId> {
        self.layout.active().active
    }

    pub fn active_document(&self) -> Option<DocumentId> {
        self.active_view()
            .and_then(|v| self.views.get(v))
            .map(|v| v.document_id)
    }

    /// A view of `doc` already present in `group`, if any.
    pub fn view_in_group(&self, doc: DocumentId, group: GroupId) -> Option<ViewId> {
        let g = self.layout.group(group)?;
        g.tabs.iter().copied().find(|v| {
            self.views
                .get(*v)
                .is_some_and(|view| view.document_id == doc)
        })
    }

    /// Opens a new view of `doc` in `group` and activates it.
    pub fn open_view(
        &mut self,
        doc: DocumentId,
        group: GroupId,
        position: TabPosition,
    ) -> Result<ViewId, WorkbenchError> {
        if !self.documents.contains_key(&doc) {
            return Err(WorkbenchError::NoSuchDocument);
        }
        let id = ViewId(self.ids.fresh());
        self.views.insert(EditorView::new(id, doc));
        self.add_tab(id, group, position)?;
        self.events.emit(AppEvent::EditorViewCreated {
            view: id,
            doc,
            group,
        });
        self.activate_view(id)?;
        Ok(id)
    }

    fn add_tab(
        &mut self,
        view: ViewId,
        group: GroupId,
        position: TabPosition,
    ) -> Result<(), WorkbenchError> {
        let pinned = self.views.get(view).is_some_and(|v| v.pinned);
        let pinned_count = self.pinned_count(group);
        let g = self
            .layout
            .group_mut(group)
            .ok_or(WorkbenchError::NoSuchGroup)?;
        let index = match position {
            TabPosition::AfterActive => g
                .active
                .and_then(|a| g.index_of(a))
                .map_or(g.tabs.len(), |i| i + 1),
            TabPosition::End => g.tabs.len(),
            TabPosition::Index(i) => i,
        };
        // Pinned tabs live in the leading zone; others never enter it.
        let index = if pinned {
            index.min(pinned_count)
        } else {
            index.max(pinned_count)
        };
        g.insert(view, index);
        Ok(())
    }

    fn pinned_count(&self, group: GroupId) -> usize {
        self.layout.group(group).map_or(0, |g| {
            g.tabs
                .iter()
                .filter(|v| self.views.get(**v).is_some_and(|view| view.pinned))
                .count()
        })
    }

    pub fn activate_view(&mut self, view: ViewId) -> Result<(), WorkbenchError> {
        let group = self
            .layout
            .group_of(view)
            .ok_or(WorkbenchError::NoSuchView)?;
        let before = self.active_view();
        self.layout
            .group_mut(group)
            .expect("group exists")
            .activate(view);
        self.layout.active_group = group;
        if before != Some(view) {
            let doc = self.views.get(view).map(|v| v.document_id);
            if let Some(d) = doc.and_then(|d| self.documents.get_mut(&d)) {
                d.buffer.seal();
            }
            self.events.emit(AppEvent::ActiveDocumentChanged {
                doc,
                view: Some(view),
            });
        }
        Ok(())
    }

    pub fn set_selection(
        &mut self,
        view: ViewId,
        selection: Selection,
    ) -> Result<(), WorkbenchError> {
        let v = self.views.get_mut(view).ok_or(WorkbenchError::NoSuchView)?;
        let len = self
            .documents
            .get(&v.document_id)
            .map_or(0, |d| d.buffer.len_chars());
        v.selection = selection.clamp(len);
        Ok(())
    }

    /// Duplicate View: a new view of the same document right after `view`.
    pub fn duplicate_view(&mut self, view: ViewId) -> Result<ViewId, WorkbenchError> {
        let group = self
            .layout
            .group_of(view)
            .ok_or(WorkbenchError::NoSuchView)?;
        let index = self
            .layout
            .group(group)
            .and_then(|g| g.index_of(view))
            .unwrap_or(0)
            + 1;
        self.duplicate_into(view, group, TabPosition::Index(index))
    }

    fn duplicate_into(
        &mut self,
        view: ViewId,
        group: GroupId,
        position: TabPosition,
    ) -> Result<ViewId, WorkbenchError> {
        let source = self
            .views
            .get(view)
            .ok_or(WorkbenchError::NoSuchView)?
            .clone();
        let id = ViewId(self.ids.fresh());
        self.views.insert(source.duplicate(id));
        self.add_tab(id, group, position)?;
        self.events.emit(AppEvent::EditorViewCreated {
            view: id,
            doc: source.document_id,
            group,
        });
        self.activate_view(id)?;
        Ok(id)
    }

    /// Split Left/Right/Up/Down: a new group next to the active group holding a
    /// duplicate of the active view (empty if the group is empty).
    pub fn split_active(&mut self, dir: Direction) -> GroupId {
        let source_group = self.layout.active_group;
        let active = self.active_view();
        let new = GroupId(self.ids.fresh());
        self.layout.split(source_group, dir, new);
        self.layout.active_group = new;
        if let Some(view) = active {
            let _ = self.duplicate_into(view, new, TabPosition::End);
        }
        self.events.emit(AppEvent::LayoutChanged);
        new
    }

    /// Moves a tab to `group` at `index`; an emptied source group is removed.
    pub fn move_view(
        &mut self,
        view: ViewId,
        group: GroupId,
        index: usize,
    ) -> Result<(), WorkbenchError> {
        let source = self
            .layout
            .group_of(view)
            .ok_or(WorkbenchError::NoSuchView)?;
        if self.layout.group(group).is_none() {
            return Err(WorkbenchError::NoSuchGroup);
        }
        self.layout.group_mut(source).expect("exists").remove(view);
        self.add_tab(view, group, TabPosition::Index(index))?;
        self.remove_group_if_empty(source);
        self.activate_view(view)?;
        self.events.emit(AppEvent::LayoutChanged);
        Ok(())
    }

    /// Drops a tab on a group edge: the view moves into a new group there.
    pub fn move_view_to_split(
        &mut self,
        view: ViewId,
        target: GroupId,
        dir: Direction,
    ) -> Result<GroupId, WorkbenchError> {
        let source = self
            .layout
            .group_of(view)
            .ok_or(WorkbenchError::NoSuchView)?;
        if self.layout.group(target).is_none() {
            return Err(WorkbenchError::NoSuchGroup);
        }
        let new = GroupId(self.ids.fresh());
        self.layout.split(target, dir, new);
        self.layout.group_mut(source).expect("exists").remove(view);
        self.add_tab(view, new, TabPosition::End)?;
        self.remove_group_if_empty(source);
        self.activate_view(view)?;
        self.events.emit(AppEvent::LayoutChanged);
        Ok(new)
    }

    /// Move to Next/Previous Group; creates a group to the right/left when
    /// there is none in that direction.
    pub fn move_view_to_neighbor(
        &mut self,
        view: ViewId,
        forward: bool,
    ) -> Result<GroupId, WorkbenchError> {
        let source = self
            .layout
            .group_of(view)
            .ok_or(WorkbenchError::NoSuchView)?;
        let order = self.layout.groups_in_order();
        let i = order.iter().position(|g| *g == source).unwrap_or(0);
        let target = if forward {
            order.get(i + 1)
        } else {
            i.checked_sub(1).and_then(|j| order.get(j))
        };
        match target {
            Some(&g) => {
                let end = self.layout.group(g).map_or(0, |grp| grp.tabs.len());
                self.move_view(view, g, end)?;
                Ok(g)
            }
            None => {
                let dir = if forward {
                    Direction::Right
                } else {
                    Direction::Left
                };
                self.move_view_to_split(view, source, dir)
            }
        }
    }

    pub fn set_pinned(&mut self, view: ViewId, pinned: bool) -> Result<(), WorkbenchError> {
        let group = self
            .layout
            .group_of(view)
            .ok_or(WorkbenchError::NoSuchView)?;
        self.views
            .get_mut(view)
            .ok_or(WorkbenchError::NoSuchView)?
            .pinned = pinned;
        let g = self.layout.group_mut(group).expect("exists");
        let index = g.index_of(view).expect("in group");
        g.tabs.remove(index);
        let pos = if pinned {
            TabPosition::End
        } else {
            TabPosition::Index(index)
        };
        self.add_tab(view, group, pos)
    }

    /// Closes a view. The last view of a dirty document needs confirmation
    /// unless `force`.
    pub fn close_view(
        &mut self,
        view: ViewId,
        force: bool,
    ) -> Result<CloseOutcome, WorkbenchError> {
        let doc = self
            .views
            .get(view)
            .ok_or(WorkbenchError::NoSuchView)?
            .document_id;
        let last = self.views.count_of_document(doc) == 1;
        if last && !force && self.documents.get(&doc).is_some_and(Document::is_dirty) {
            return Ok(CloseOutcome::NeedsConfirmation(doc));
        }
        let group = self
            .layout
            .group_of(view)
            .ok_or(WorkbenchError::NoSuchView)?;
        let was_active = self.active_view() == Some(view);
        self.layout.group_mut(group).expect("exists").remove(view);
        self.views.remove(view);
        self.events.emit(AppEvent::EditorViewClosed { view });
        if last {
            self.documents.remove(&doc);
            self.events.emit(AppEvent::DocumentClosed { doc });
        }
        self.remove_group_if_empty(group);
        if was_active {
            let next = self.active_view();
            match next {
                Some(v) => self.activate_view(v)?,
                None => self.events.emit(AppEvent::ActiveDocumentChanged {
                    doc: None,
                    view: None,
                }),
            }
        }
        Ok(CloseOutcome::Closed)
    }

    fn remove_group_if_empty(&mut self, group: GroupId) {
        if self.layout.group(group).is_some_and(|g| g.tabs.is_empty())
            && self.layout.remove_group(group)
        {
            self.events.emit(AppEvent::LayoutChanged);
        }
    }

    /// Single Editor: merges every group into the active one (active group's
    /// tabs first). Duplicate views of a document collapse to one.
    pub fn single_editor(&mut self) {
        let keep = self.layout.active_group;
        let mut order = vec![keep];
        order.extend(
            self.layout
                .groups_in_order()
                .into_iter()
                .filter(|g| *g != keep),
        );
        let mut merged: Vec<ViewId> = Vec::new();
        let mut seen_docs = Vec::new();
        let mut dropped = Vec::new();
        for g in &order {
            for v in self
                .layout
                .group(*g)
                .map(|g| g.tabs.clone())
                .unwrap_or_default()
            {
                let doc = self.views.get(v).map(|v| v.document_id);
                if seen_docs.contains(&doc) {
                    dropped.push(v);
                } else {
                    seen_docs.push(doc);
                    merged.push(v);
                }
            }
        }
        for g in order.iter().skip(1) {
            self.layout.remove_group(*g);
        }
        for v in dropped {
            self.views.remove(v);
            self.events.emit(AppEvent::EditorViewClosed { view: v });
        }
        let active = self.layout.group(keep).and_then(|g| g.active);
        let g: &mut EditorGroup = self.layout.group_mut(keep).expect("kept");
        g.tabs = merged;
        g.mru.retain(|v| g.tabs.contains(v));
        if active.is_none_or(|a| !g.tabs.contains(&a)) {
            g.active = g.tabs.first().copied();
        }
        self.layout.maximized = None;
        self.events.emit(AppEvent::LayoutChanged);
    }

    /// Maximize Editor Group toggle (panel state is saved by the app).
    pub fn toggle_maximize(&mut self, group: GroupId) {
        self.layout.maximized = match self.layout.maximized {
            Some(g) if g == group => None,
            _ => Some(group),
        };
        self.events.emit(AppEvent::LayoutChanged);
    }

    // ── editing ──────────────────────────────────────────────────────────

    /// A user edit through `view`.
    pub fn edit(
        &mut self,
        view: ViewId,
        changes: ChangeSet,
        selection_after: Selection,
        kind: EditKind,
        now_ms: u64,
    ) -> Result<(), WorkbenchError> {
        let doc = self
            .views
            .get(view)
            .ok_or(WorkbenchError::NoSuchView)?
            .document_id;
        self.apply_edit(
            doc,
            Some(view),
            changes,
            selection_after,
            kind,
            ChangeOrigin::User { view },
            now_ms,
        )
    }

    /// The single edit path for every origin. `view` (if any) receives
    /// `selection_after`; every other view of the document is mapped.
    #[allow(clippy::too_many_arguments)]
    pub fn apply_edit(
        &mut self,
        doc: DocumentId,
        view: Option<ViewId>,
        changes: ChangeSet,
        selection_after: Selection,
        kind: EditKind,
        origin: ChangeOrigin,
        now_ms: u64,
    ) -> Result<(), WorkbenchError> {
        let d = self
            .documents
            .get_mut(&doc)
            .ok_or(WorkbenchError::NoSuchDocument)?;
        if d.is_read_only() && !matches!(origin, ChangeOrigin::Reload) {
            return Err(WorkbenchError::ReadOnly);
        }
        let selection_before = view
            .and_then(|v| self.views.get(v))
            .map(|v| v.selection.clone())
            .unwrap_or_default();
        d.buffer.edit(EditRequest {
            changes: changes.clone(),
            selection_before,
            selection_after: selection_after.clone(),
            kind,
            now_ms,
        })?;
        d.revision += 1;
        let revision = d.revision;
        let len = d.buffer.len_chars();
        self.views.map_selections(doc, &changes, view);
        if let Some(v) = view.and_then(|v| self.views.get_mut(v)) {
            v.selection = selection_after.clamp(len);
        }
        self.events.emit(AppEvent::DocumentChanged {
            doc,
            revision,
            origin,
            changes: Arc::new(changes),
        });
        Ok(())
    }

    pub fn undo(&mut self, view: ViewId) -> Result<bool, WorkbenchError> {
        self.history_step(view, true)
    }

    pub fn redo(&mut self, view: ViewId) -> Result<bool, WorkbenchError> {
        self.history_step(view, false)
    }

    fn history_step(&mut self, view: ViewId, undo: bool) -> Result<bool, WorkbenchError> {
        let doc = self
            .views
            .get(view)
            .ok_or(WorkbenchError::NoSuchView)?
            .document_id;
        let d = self
            .documents
            .get_mut(&doc)
            .ok_or(WorkbenchError::NoSuchDocument)?;
        let applied = if undo {
            d.buffer.undo()
        } else {
            d.buffer.redo()
        };
        let Some(applied) = applied else {
            return Ok(false);
        };
        let len = d.buffer.len_chars();
        let origin = if undo {
            ChangeOrigin::Undo
        } else {
            ChangeOrigin::Redo
        };
        for changes in applied.changes {
            d.revision += 1;
            self.views.map_selections(doc, &changes, Some(view));
            self.events.emit(AppEvent::DocumentChanged {
                doc,
                revision: d.revision,
                origin: origin.clone(),
                changes: Arc::new(changes),
            });
        }
        if let Some(v) = self.views.get_mut(view) {
            v.selection = applied.selection.clamp(len);
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use notoir_editor::{LanguageId, Range};

    use super::*;

    fn file(name: &str) -> DocumentOrigin {
        let path = PathBuf::from(format!("/w/{name}"));
        DocumentOrigin::File {
            key: PathKey::new(&path, false),
            path,
        }
    }

    fn loaded(text: &str) -> LoadedText {
        LoadedText {
            text: text.into(),
            ..LoadedText::empty(LanguageId::MARKDOWN)
        }
    }

    fn open(wb: &mut Workbench, name: &str, text: &str) -> (DocumentId, ViewId) {
        let doc = wb.open_document(file(name), loaded(text));
        let group = wb.layout.active_group;
        let view = wb.open_view(doc, group, TabPosition::AfterActive).unwrap();
        (doc, view)
    }

    fn type_at(wb: &mut Workbench, view: ViewId, pos: usize, text: &str, now: u64) {
        let n = text.chars().count();
        wb.set_selection(view, Selection::caret(pos)).unwrap();
        wb.edit(
            view,
            ChangeSet::insert(pos, text),
            Selection::caret(pos + n),
            EditKind::Insert,
            now,
        )
        .unwrap();
    }

    fn lines(n: usize) -> String {
        (1..=n).map(|i| format!("line {i}\n")).collect()
    }

    fn line_start(wb: &Workbench, doc: DocumentId, line: usize) -> usize {
        wb.document(doc).unwrap().buffer.rope().line_to_char(line)
    }

    #[test]
    fn opening_an_open_file_reuses_the_document() {
        let mut wb = Workbench::new();
        let (doc, _) = open(&mut wb, "notes.md", "a");
        let right = wb.split_active(Direction::Right);
        let again = wb.open_document(file("notes.md"), loaded("other"));
        assert_eq!(again, doc);
        assert_eq!(wb.documents.len(), 1);
        let v2 = wb.view_in_group(doc, right).unwrap();
        type_at(&mut wb, v2, 1, "b", 0);
        assert_eq!(wb.document(doc).unwrap().buffer.text(), "ab");
    }

    #[test]
    fn split_right_creates_group_with_view_of_same_document() {
        let mut wb = Workbench::new();
        let (doc, _) = open(&mut wb, "README.md", "x");
        let right = wb.split_active(Direction::Right);
        assert_eq!(wb.layout.groups_in_order().len(), 2);
        let v = wb.layout.group(right).unwrap().active.unwrap();
        assert_eq!(wb.view(v).unwrap().document_id, doc);
        assert_eq!(wb.documents.len(), 1);
        wb.layout.check().unwrap();
    }

    #[test]
    fn closing_last_tab_of_a_group_collapses_the_split() {
        let mut wb = Workbench::new();
        open(&mut wb, "a.md", "");
        let right = wb.split_active(Direction::Right);
        let v = wb.layout.group(right).unwrap().active.unwrap();
        assert_eq!(wb.close_view(v, false).unwrap(), CloseOutcome::Closed);
        assert_eq!(wb.layout.groups_in_order().len(), 1);
        assert_eq!(wb.documents.len(), 1, "other view still holds the document");
    }

    #[test]
    fn duplicate_views_share_buffer_with_independent_cursors() {
        let mut wb = Workbench::new();
        let (doc, a) = open(&mut wb, "README.md", &lines(1000));
        let l20 = line_start(&wb, doc, 19);
        wb.set_selection(a, Selection::caret(l20)).unwrap();
        let b = wb.duplicate_view(a).unwrap();
        let l900 = line_start(&wb, doc, 899);
        wb.set_selection(b, Selection::caret(l900)).unwrap();
        assert_eq!(wb.view(a).unwrap().selection.primary(), Range::caret(l20));
        type_at(&mut wb, b, l900, "hello", 0);
        let text = wb.document(doc).unwrap().buffer.text();
        assert!(text.lines().nth(899).unwrap().starts_with("hello"));
        assert_eq!(wb.view(a).unwrap().selection.primary(), Range::caret(l20));
        assert_eq!(wb.layout.active().tabs, vec![a, b]);
    }

    #[test]
    fn selections_follow_remote_edits() {
        let mut wb = Workbench::new();
        let (doc, a) = open(&mut wb, "a.md", &lines(20));
        let b = wb.duplicate_view(a).unwrap();
        let l10 = line_start(&wb, doc, 9);
        wb.set_selection(a, Selection::caret(l10)).unwrap();
        type_at(&mut wb, b, 0, "x\ny\nz\n", 0);
        assert_eq!(
            wb.view(a).unwrap().selection.primary(),
            Range::caret(line_start(&wb, doc, 12))
        );
    }

    #[test]
    fn undo_across_duplicate_views_restores_selection_in_undoing_view() {
        let mut wb = Workbench::new();
        let (doc, a) = open(&mut wb, "a.md", "0123456789");
        let b = wb.duplicate_view(a).unwrap();
        type_at(&mut wb, a, 2, "a", 0);
        type_at(&mut wb, b, 8, "b", 2_000);
        assert!(wb.undo(b).unwrap());
        assert_eq!(wb.document(doc).unwrap().buffer.text(), "01a23456789");
        assert!(wb.undo(b).unwrap());
        assert_eq!(wb.document(doc).unwrap().buffer.text(), "0123456789");
        assert_eq!(wb.view(b).unwrap().selection.primary(), Range::caret(2));
    }

    #[test]
    fn dirty_tracking_and_undo_to_saved() {
        let mut wb = Workbench::new();
        let (doc, v) = open(&mut wb, "notes.md", "");
        assert!(!wb.document(doc).unwrap().is_dirty());
        type_at(&mut wb, v, 0, "x", 0);
        assert!(wb.document(doc).unwrap().is_dirty());
        wb.undo(v).unwrap();
        assert!(!wb.document(doc).unwrap().is_dirty());
    }

    #[test]
    fn closing_last_view_of_dirty_document_needs_confirmation() {
        let mut wb = Workbench::new();
        let (doc, a) = open(&mut wb, "notes.md", "");
        type_at(&mut wb, a, 0, "x", 0);
        let b = wb.duplicate_view(a).unwrap();
        assert_eq!(wb.close_view(b, false).unwrap(), CloseOutcome::Closed);
        assert_eq!(
            wb.close_view(a, false).unwrap(),
            CloseOutcome::NeedsConfirmation(doc)
        );
        assert_eq!(wb.close_view(a, true).unwrap(), CloseOutcome::Closed);
        assert!(wb.documents.is_empty());
        assert_eq!(wb.active_view(), None);
    }

    #[test]
    fn read_only_rejects_user_edits() {
        let mut wb = Workbench::new();
        let doc = wb.open_document(
            file("ro.md"),
            LoadedText {
                read_only: crate::document::ReadOnly::FileSystem,
                ..loaded("a")
            },
        );
        let v = wb
            .open_view(doc, wb.layout.active_group, TabPosition::End)
            .unwrap();
        let err = wb.edit(
            v,
            ChangeSet::insert(0, "x"),
            Selection::caret(1),
            EditKind::Insert,
            0,
        );
        assert_eq!(err, Err(WorkbenchError::ReadOnly));
    }

    #[test]
    fn dropping_tab_on_bottom_edge_creates_split() {
        let mut wb = Workbench::new();
        let (_, a) = open(&mut wb, "a.md", "");
        let (_, b) = open(&mut wb, "b.md", "");
        let top = wb.layout.active_group;
        let bottom = wb.move_view_to_split(b, top, Direction::Down).unwrap();
        assert_eq!(wb.layout.group(top).unwrap().tabs, vec![a]);
        assert_eq!(wb.layout.group(bottom).unwrap().tabs, vec![b]);
        wb.layout.check().unwrap();
    }

    #[test]
    fn move_to_next_group_creates_one_when_missing() {
        let mut wb = Workbench::new();
        open(&mut wb, "a.md", "");
        let (_, b) = open(&mut wb, "b.md", "");
        let g = wb.move_view_to_neighbor(b, true).unwrap();
        assert_eq!(wb.layout.groups_in_order().len(), 2);
        assert_eq!(wb.layout.group_of(b), Some(g));
    }

    #[test]
    fn pinned_tabs_stay_in_leading_zone() {
        let mut wb = Workbench::new();
        let (_, a) = open(&mut wb, "a.md", "");
        let (_, b) = open(&mut wb, "b.md", "");
        let (_, c) = open(&mut wb, "c.md", "");
        wb.set_pinned(c, true).unwrap();
        assert_eq!(wb.layout.active().tabs, vec![c, a, b]);
        wb.activate_view(c).unwrap();
        let (_, d) = open(&mut wb, "d.md", "");
        assert_eq!(wb.layout.active().tabs, vec![c, d, a, b]);
    }

    #[test]
    fn single_editor_merges_groups_and_drops_duplicates() {
        let mut wb = Workbench::new();
        open(&mut wb, "a.md", "");
        wb.split_active(Direction::Right);
        open(&mut wb, "b.md", "");
        wb.single_editor();
        assert_eq!(wb.layout.groups_in_order().len(), 1);
        assert_eq!(wb.layout.active().tabs.len(), 2);
        wb.layout.check().unwrap();
    }

    #[test]
    fn events_are_emitted_in_order() {
        let mut wb = Workbench::new();
        let (_, v) = open(&mut wb, "a.md", "");
        type_at(&mut wb, v, 0, "x", 0);
        let events = wb.events.drain();
        let opened = events
            .iter()
            .position(|e| matches!(e, AppEvent::DocumentOpened { .. }))
            .unwrap();
        let changed = events
            .iter()
            .position(|e| matches!(e, AppEvent::DocumentChanged { .. }))
            .unwrap();
        assert!(opened < changed);
        assert!(wb.events.is_empty());
    }

    #[test]
    fn untitled_numbering_reuses_gaps() {
        let mut wb = Workbench::new();
        for n in [1, 3] {
            wb.open_document(
                DocumentOrigin::Untitled { n },
                LoadedText::empty(LanguageId::PLAINTEXT),
            );
        }
        assert_eq!(wb.next_untitled_number(), 2);
    }
}
