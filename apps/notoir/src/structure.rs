//! Derived document structure (docs/specs/12, 26): fold ranges and outline
//! symbols per document revision, and the folding, bookmark and symbol
//! operations behind their commands.

use std::collections::HashMap;
use std::sync::Arc;

use notoir_core::ids::{DocumentId, ViewId};
use notoir_core::view::Fold;
use notoir_editor::folding::{self, FoldRange};
use notoir_editor::outline::{self, Symbol};
use notoir_editor::{Range, Selection};

use crate::core::AppCore;

/// Larger documents recompute at most this often while typing.
pub const DEBOUNCE_MS: u64 = 150;
const DEBOUNCE_ABOVE_CHARS: usize = 256 * 1024;

pub struct Structure {
    pub revision: u64,
    pub folds: Arc<Vec<FoldRange>>,
    /// Full-depth symbols (the Outline panel limits the depth it shows).
    pub symbols: Vec<Symbol>,
    at_ms: u64,
}

#[derive(Default)]
pub struct StructureState {
    pub docs: HashMap<DocumentId, Arc<Structure>>,
}

impl AppCore {
    /// The document's structure, recomputed when stale (debounced for large
    /// documents; the previous result is returned meanwhile).
    pub fn structure(&mut self, doc: DocumentId) -> Option<Arc<Structure>> {
        let d = self.workbench.document(doc)?;
        let now = self.now_ms;
        if let Some(s) = self.structure.docs.get(&doc)
            && (s.revision == d.revision
                || (d.buffer.len_chars() > DEBOUNCE_ABOVE_CHARS
                    && now.saturating_sub(s.at_ms) < DEBOUNCE_MS))
        {
            return Some(s.clone());
        }
        let (revision, large, language) = (d.revision, d.large_file, d.language);
        // A plugin outline provider wins over the built-in symbols.
        let plugin_symbols = if large {
            None
        } else {
            self.plugin_outline(doc)
        };
        let d = self.workbench.document(doc)?;
        let rope = d.buffer.rope();
        let s = Arc::new(Structure {
            revision,
            folds: Arc::new(folding::fold_ranges(rope, language, large)),
            symbols: if large {
                Vec::new()
            } else {
                plugin_symbols.unwrap_or_else(|| outline::symbols(rope, language, usize::MAX))
            },
            at_ms: now,
        });
        self.structure.docs.insert(doc, s.clone());
        Some(s)
    }

    fn view_doc(&self, view: ViewId) -> Option<DocumentId> {
        self.workbench.view(view).map(|v| v.document_id)
    }

    fn caret_line(&self, view: ViewId) -> Option<usize> {
        let v = self.workbench.view(view)?;
        let rope = self.workbench.document(v.document_id)?.buffer.rope();
        Some(rope.char_to_line(v.selection.primary().head.min(rope.len_chars())))
    }

    /// Folds `ranges` in `view`; carets inside them move to the header end.
    fn add_folds(&mut self, view: ViewId, ranges: &[FoldRange]) {
        let Some(doc) = self.view_doc(view) else {
            return;
        };
        let Some(rope) = self
            .workbench
            .document(doc)
            .map(|d| d.buffer.rope().clone())
        else {
            return;
        };
        let Some(v) = self.workbench.views.get_mut(view) else {
            return;
        };
        for r in ranges {
            let header = rope.line_to_char(r.start_line);
            let end = rope.line_to_char(r.end_line)
                + notoir_editor::display_map::line_text(&rope, r.end_line)
                    .chars()
                    .count();
            if !v.folds.iter().any(|f| f.header == header) {
                v.folds.push(Fold { header, end });
            }
        }
        let hidden = v.hidden_lines(&rope);
        let len = rope.len_chars();
        let ranges: Vec<Range> = v
            .selection
            .ranges()
            .iter()
            .map(|r| {
                let line = rope.char_to_line(r.head.min(len));
                match hidden.iter().find(|h| h.0 <= line && line <= h.1) {
                    Some(h) => {
                        let header = h.0 - 1;
                        let end = rope.line_to_char(header)
                            + notoir_editor::display_map::line_text(&rope, header)
                                .chars()
                                .count();
                        Range::caret(end)
                    }
                    None => *r,
                }
            })
            .collect();
        let primary = v.selection.primary_index();
        v.selection = Selection::new(ranges, primary);
    }

    /// `fold.fold` / `fold.unfold` / `fold.toggle` at `line` (default: caret).
    pub fn fold_op(&mut self, view: ViewId, line: Option<usize>, op: &str) {
        let (Some(doc), Some(line)) = (self.view_doc(view), line.or(self.caret_line(view))) else {
            return;
        };
        let Some(s) = self.structure(doc) else { return };
        let Some(rope) = self
            .workbench
            .document(doc)
            .map(|d| d.buffer.rope().clone())
        else {
            return;
        };
        let folded_here = |core: &AppCore| {
            core.workbench.view(view).and_then(|v| {
                v.folds.iter().position(|f| {
                    let h = rope.char_to_line(f.header.min(rope.len_chars()));
                    let e = rope.char_to_line(f.end.min(rope.len_chars()));
                    h == line || (h < line && line <= e)
                })
            })
        };
        let unfold = match op {
            "unfold" => true,
            "toggle" => folded_here(self).is_some(),
            _ => false,
        };
        if unfold {
            if let Some(i) = folded_here(self)
                && let Some(v) = self.workbench.views.get_mut(view)
            {
                v.folds.remove(i);
            }
            return;
        }
        // Innermost unfolded range at the line.
        if let Some(r) = folding::range_at(&s.folds, line) {
            self.add_folds(view, &[r]);
        }
    }

    pub fn fold_all(&mut self, view: ViewId, level: Option<u8>, except_current: bool) {
        let Some(doc) = self.view_doc(view) else {
            return;
        };
        let Some(s) = self.structure(doc) else { return };
        let caret = self.caret_line(view).unwrap_or(0);
        let ranges: Vec<FoldRange> = s
            .folds
            .iter()
            .filter(|r| match level {
                Some(l) => r.level == l,
                None => r.level > 0 || s.folds.len() == 1,
            })
            .filter(|r| {
                // Level folds and Fold All Except Current keep the caret's region open.
                !(level.is_some() || except_current)
                    || !(r.start_line <= caret && caret <= r.end_line)
            })
            .copied()
            .collect();
        self.add_folds(view, &ranges);
    }

    pub fn unfold_all(&mut self, view: ViewId) {
        if let Some(v) = self.workbench.views.get_mut(view) {
            v.folds.clear();
        }
    }

    /// `bookmarks.toggle` at `line` (default: every caret line).
    pub fn toggle_bookmark(&mut self, view: ViewId, line: Option<usize>) {
        let Some(v) = self.workbench.view(view) else {
            return;
        };
        let doc = v.document_id;
        let lines: Vec<usize> = match line {
            Some(l) => vec![l],
            None => {
                let Some(d) = self.workbench.document(doc) else {
                    return;
                };
                let rope = d.buffer.rope();
                let mut ls: Vec<usize> = v
                    .selection
                    .ranges()
                    .iter()
                    .map(|r| rope.char_to_line(r.head.min(rope.len_chars())))
                    .collect();
                ls.dedup();
                ls
            }
        };
        if let Some(d) = self.workbench.document_mut(doc) {
            for l in lines {
                d.toggle_bookmark(l);
            }
        }
        self.persist_bookmarks(doc);
    }

    /// Next / previous bookmark, wrapping in the document or, with
    /// `bookmarks.navigateAcrossFiles`, across open documents.
    pub fn next_bookmark(&mut self, view: ViewId, forward: bool) {
        let (Some(doc), Some(caret)) = (self.view_doc(view), self.caret_line(view)) else {
            return;
        };
        let lines = self
            .workbench
            .document(doc)
            .map(|d| d.bookmark_lines())
            .unwrap_or_default();
        let inside = if forward {
            lines.iter().find(|&&l| l > caret).copied()
        } else {
            lines.iter().rev().find(|&&l| l < caret).copied()
        };
        let target = match inside {
            Some(l) => Some((view, l)),
            None if self.settings.bool("bookmarks.navigateAcrossFiles", None) => {
                self.bookmark_in_other_view(view, forward)
            }
            None => None,
        }
        .or_else(|| {
            // Wrap within the document.
            let l = if forward { lines.first() } else { lines.last() };
            l.map(|&l| (view, l))
        });
        if let Some((v, line)) = target {
            if v != view {
                let _ = self.workbench.activate_view(v);
            }
            self.go_to_line_start(v, line);
        }
    }

    fn bookmark_in_other_view(&self, view: ViewId, forward: bool) -> Option<(ViewId, usize)> {
        let group = self.workbench.layout.group_of(view)?;
        let tabs = self.workbench.layout.group(group)?.tabs.clone();
        let i = tabs.iter().position(|t| *t == view)?;
        let n = tabs.len();
        (1..n).find_map(|k| {
            let t = tabs[if forward {
                (i + k) % n
            } else {
                (i + n - k) % n
            }];
            let d = self
                .workbench
                .document(self.workbench.view(t)?.document_id)?;
            let lines = d.bookmark_lines();
            let l = if forward { lines.first() } else { lines.last() };
            l.map(|&l| (t, l))
        })
    }

    fn go_to_line_start(&mut self, view: ViewId, line: usize) {
        let Some(doc) = self.view_doc(view) else {
            return;
        };
        let Some(pos) = self.workbench.document(doc).map(|d| {
            let rope = d.buffer.rope();
            rope.line_to_char(line.min(rope.len_lines().saturating_sub(1)))
        }) else {
            return;
        };
        let _ = self.workbench.set_selection(view, Selection::caret(pos));
        self.find.center.push(view);
    }

    pub fn clear_bookmarks(&mut self, doc: Option<DocumentId>) {
        let ids: Vec<DocumentId> = match doc {
            Some(d) => vec![d],
            None => self.workbench.documents.keys().copied().collect(),
        };
        for id in ids {
            if let Some(d) = self.workbench.document_mut(id) {
                d.bookmarks.clear();
            }
            self.persist_bookmarks(id);
        }
    }

    /// Selects a symbol / range start and centres it (outline, breadcrumbs,
    /// Go to Symbol).
    pub fn reveal_range(&mut self, view: ViewId, start: usize, end: usize) {
        let len = self
            .view_doc(view)
            .and_then(|d| self.workbench.document(d))
            .map_or(0, |d| d.buffer.len_chars());
        let _ = self.workbench.set_selection(
            view,
            Selection::single(Range::new(start.min(len), end.min(len))),
        );
        self.find.center.push(view);
        self.effects.push(crate::core::Effect::FocusEditor(view));
    }

    fn bookmark_map(&self) -> serde_json::Map<String, serde_json::Value> {
        self.state_db
            .as_ref()
            .and_then(|db| db.load_bookmarks(&self.session_key()).ok().flatten())
            .and_then(|j| serde_json::from_str::<serde_json::Value>(&j).ok())
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default()
    }

    /// Saves a file's bookmarks (by line) in the workspace session store.
    pub fn persist_bookmarks(&mut self, doc: DocumentId) {
        let Some(d) = self.workbench.document(doc) else {
            return;
        };
        let Some(key) = d.path_key().map(|k| k.as_str().to_owned()) else {
            return;
        };
        let lines = d.bookmark_lines();
        let mut map = self.bookmark_map();
        if lines.is_empty() {
            map.remove(&key);
        } else {
            map.insert(key, serde_json::json!(lines));
        }
        let json = serde_json::Value::Object(map).to_string();
        let now = crate::session::epoch_ms();
        if let Some(db) = self.state_db.as_ref()
            && let Err(e) = db.save_bookmarks(&self.session_key(), &json, now)
        {
            tracing::warn!(error = %e, "saving bookmarks failed");
        }
    }

    /// Restores a just-opened file's bookmarks.
    pub fn restore_bookmarks(&mut self, doc: DocumentId) {
        let Some(key) = self
            .workbench
            .document(doc)
            .and_then(|d| d.path_key())
            .map(|k| k.as_str().to_owned())
        else {
            return;
        };
        let map = self.bookmark_map();
        let Some(lines) = map.get(&key).and_then(|v| v.as_array()) else {
            return;
        };
        let lines: Vec<usize> = lines
            .iter()
            .filter_map(|l| l.as_u64())
            .map(|l| l as usize)
            .collect();
        if let Some(d) = self.workbench.document_mut(doc) {
            d.set_bookmark_lines(&lines);
        }
    }
}
