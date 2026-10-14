//! `TextBuffer`: rope + per-document undo history (AD-3).
//!
//! History is shared by every view of a document. Each history state has an
//! id ([`TextBuffer::state`]) so "undo back to the saved state" can be detected
//! as clean even though the document revision keeps increasing.

use ropey::Rope;

use crate::change::{ChangeError, ChangeSet};
use crate::selection::Selection;

/// Typing coalescing window (spec 11).
pub const COALESCE_MS: u64 = 1_000;
/// Default `editor.undoLimit` in steps.
pub const DEFAULT_UNDO_LIMIT: usize = 10_000;

/// What produced an edit; consecutive typing edits coalesce into one undo step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditKind {
    Insert,
    DeleteBackward,
    DeleteForward,
    /// Paste, line operations, replace-all, reload, MCP/plugin edits…: always
    /// its own undo step.
    Other,
}

#[derive(Clone, Debug)]
pub struct EditRequest {
    pub changes: ChangeSet,
    pub selection_before: Selection,
    pub selection_after: Selection,
    pub kind: EditKind,
    pub now_ms: u64,
}

/// Result of undo/redo: change sets applied in order (for mapping other
/// views' selections and emitting events) and the selection to restore.
#[derive(Clone, Debug)]
pub struct Applied {
    pub changes: Vec<ChangeSet>,
    pub selection: Selection,
}

#[derive(Clone, Debug)]
struct Tx {
    changes: ChangeSet,
    inverse: ChangeSet,
    selection_before: Selection,
    selection_after: Selection,
}

#[derive(Clone, Debug)]
struct Group {
    txs: Vec<Tx>,
    kind: EditKind,
    last_ms: u64,
    ends_with_whitespace: bool,
    sealed: bool,
    state_before: u64,
    state_after: u64,
}

#[derive(Clone, Debug)]
pub struct TextBuffer {
    rope: Rope,
    undo: Vec<Group>,
    redo: Vec<Group>,
    state: u64,
    next_state: u64,
    undo_limit: usize,
}

impl TextBuffer {
    pub fn new(text: &str) -> Self {
        Self {
            rope: Rope::from_str(text),
            undo: Vec::new(),
            redo: Vec::new(),
            state: 0,
            next_state: 1,
            undo_limit: DEFAULT_UNDO_LIMIT,
        }
    }

    pub fn rope(&self) -> &Rope {
        &self.rope
    }

    pub fn text(&self) -> String {
        self.rope.to_string()
    }

    pub fn len_chars(&self) -> usize {
        self.rope.len_chars()
    }

    /// Id of the current history state.
    pub fn state(&self) -> u64 {
        self.state
    }

    pub fn set_undo_limit(&mut self, limit: usize) {
        self.undo_limit = limit.max(1);
        self.trim();
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Ends typing coalescing (cursor jump, focus change, any other command).
    pub fn seal(&mut self) {
        if let Some(g) = self.undo.last_mut() {
            g.sealed = true;
        }
    }

    /// Applies an edit and records it in history.
    pub fn edit(&mut self, req: EditRequest) -> Result<(), ChangeError> {
        req.changes.check_bounds(self.rope.len_chars())?;
        if req.changes.is_empty() {
            return Ok(());
        }
        let inverse = req.changes.invert(&self.rope);
        req.changes.apply(&mut self.rope);
        let inserted: String = req.changes.edits().iter().map(|e| &*e.text).collect();
        let tx = Tx {
            changes: req.changes,
            inverse,
            selection_before: req.selection_before,
            selection_after: req.selection_after,
        };
        let new_state = self.next_state;
        self.next_state += 1;
        self.redo.clear();

        let starts_word = inserted.chars().next().is_some_and(|c| !c.is_whitespace());
        let coalesce = match self.undo.last() {
            Some(g) => {
                !g.sealed
                    && req.kind != EditKind::Other
                    && g.kind == req.kind
                    && req.now_ms.saturating_sub(g.last_ms) < COALESCE_MS
                    && g.txs
                        .last()
                        .is_some_and(|t| t.selection_after == tx.selection_before)
                    && !inserted.contains('\n')
                    && !(req.kind == EditKind::Insert && g.ends_with_whitespace && starts_word)
            }
            None => false,
        };
        let ends_ws = inserted.chars().last().is_some_and(char::is_whitespace);
        if coalesce {
            let g = self.undo.last_mut().expect("checked above");
            g.txs.push(tx);
            g.last_ms = req.now_ms;
            g.ends_with_whitespace = ends_ws;
            g.state_after = new_state;
        } else {
            self.undo.push(Group {
                txs: vec![tx],
                kind: req.kind,
                last_ms: req.now_ms,
                ends_with_whitespace: ends_ws,
                // A newline or a non-typing edit closes its own group.
                sealed: req.kind == EditKind::Other || inserted.contains('\n'),
                state_before: self.state,
                state_after: new_state,
            });
            self.trim();
        }
        self.state = new_state;
        Ok(())
    }

    pub fn undo(&mut self) -> Option<Applied> {
        let mut group = self.undo.pop()?;
        group.sealed = true;
        let mut changes = Vec::with_capacity(group.txs.len());
        for tx in group.txs.iter().rev() {
            tx.inverse.apply(&mut self.rope);
            changes.push(tx.inverse.clone());
        }
        let selection = group.txs[0].selection_before.clone();
        self.state = group.state_before;
        self.redo.push(group);
        Some(Applied { changes, selection })
    }

    pub fn redo(&mut self) -> Option<Applied> {
        let group = self.redo.pop()?;
        let mut changes = Vec::with_capacity(group.txs.len());
        for tx in &group.txs {
            tx.changes.apply(&mut self.rope);
            changes.push(tx.changes.clone());
        }
        let selection = group
            .txs
            .last()
            .expect("non-empty group")
            .selection_after
            .clone();
        self.state = group.state_after;
        self.undo.push(group);
        Some(Applied { changes, selection })
    }

    fn trim(&mut self) {
        if self.undo.len() > self.undo_limit {
            let excess = self.undo.len() - self.undo_limit;
            self.undo.drain(..excess);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selection::Selection;

    /// Types `text` one char at a time at the end, `step_ms` apart.
    fn type_text(buf: &mut TextBuffer, text: &str, start_ms: u64, step_ms: u64) -> u64 {
        let mut now = start_ms;
        for c in text.chars() {
            let pos = buf.len_chars();
            buf.edit(EditRequest {
                changes: ChangeSet::insert(pos, c.to_string()),
                selection_before: Selection::caret(pos),
                selection_after: Selection::caret(pos + 1),
                kind: EditKind::Insert,
                now_ms: now,
            })
            .unwrap();
            now += step_ms;
        }
        now
    }

    #[test]
    fn typing_a_word_coalesces_and_word_boundary_splits() {
        let mut buf = TextBuffer::new("");
        type_text(&mut buf, "hello world", 0, 50);
        buf.undo().unwrap();
        assert_eq!(buf.text(), "hello ");
        buf.undo().unwrap();
        assert_eq!(buf.text(), "");
        assert!(!buf.can_undo());
    }

    #[test]
    fn pause_splits_undo_steps() {
        let mut buf = TextBuffer::new("");
        let t = type_text(&mut buf, "ab", 0, 10);
        type_text(&mut buf, "cd", t + COALESCE_MS, 10);
        buf.undo().unwrap();
        assert_eq!(buf.text(), "ab");
    }

    #[test]
    fn other_edits_are_single_steps() {
        let mut buf = TextBuffer::new("x");
        buf.edit(EditRequest {
            changes: ChangeSet::insert(1, "1\n2\n3\n"),
            selection_before: Selection::caret(1),
            selection_after: Selection::caret(7),
            kind: EditKind::Other,
            now_ms: 0,
        })
        .unwrap();
        type_text(&mut buf, "y", 10, 10);
        buf.undo().unwrap();
        assert_eq!(buf.text(), "x1\n2\n3\n");
        let applied = buf.undo().unwrap();
        assert_eq!(buf.text(), "x");
        assert_eq!(applied.selection, Selection::caret(1));
    }

    #[test]
    fn state_returns_to_saved_after_undo_and_redo() {
        let mut buf = TextBuffer::new("");
        let saved = buf.state();
        type_text(&mut buf, "x", 0, 1);
        assert_ne!(buf.state(), saved);
        buf.undo().unwrap();
        assert_eq!(buf.state(), saved);
        buf.redo().unwrap();
        assert_ne!(buf.state(), saved);
        assert_eq!(buf.text(), "x");
    }

    #[test]
    fn new_edit_clears_redo() {
        let mut buf = TextBuffer::new("");
        type_text(&mut buf, "a", 0, 1);
        buf.undo().unwrap();
        type_text(&mut buf, "b", 5_000, 1);
        assert!(buf.redo().is_none());
    }

    #[test]
    fn sealing_prevents_coalescing() {
        let mut buf = TextBuffer::new("");
        type_text(&mut buf, "ab", 0, 10);
        buf.seal();
        type_text(&mut buf, "cd", 30, 10);
        buf.undo().unwrap();
        assert_eq!(buf.text(), "ab");
    }

    #[test]
    fn undo_limit_drops_oldest() {
        let mut buf = TextBuffer::new("");
        buf.set_undo_limit(2);
        for i in 0..4 {
            type_text(&mut buf, "a", i * 10_000, 1);
        }
        assert!(buf.undo().is_some());
        assert!(buf.undo().is_some());
        assert!(buf.undo().is_none());
        assert_eq!(buf.text(), "aa");
    }

    #[test]
    fn out_of_bounds_edit_is_rejected() {
        let mut buf = TextBuffer::new("ab");
        let err = buf.edit(EditRequest {
            changes: ChangeSet::delete(1, 5),
            selection_before: Selection::caret(0),
            selection_after: Selection::caret(0),
            kind: EditKind::Other,
            now_ms: 0,
        });
        assert!(err.is_err());
        assert_eq!(buf.text(), "ab");
    }
}
