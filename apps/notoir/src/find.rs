//! Find / replace and Go to Line state (docs/specs/11): one find widget per
//! editor view, options shared by all of them. Matches are recomputed when
//! the query or the document changes; the editor paints them and the minimap
//! marks them.

use std::collections::HashMap;
use std::sync::Arc;

use notoir_core::ids::ViewId;
use notoir_core::notifications::{Notification, Severity, Translatable};
use notoir_core::view::ScrollPosition;
use notoir_editor::find::{self, FindQuery};
use notoir_editor::{EditKind, Range, Selection};

use crate::core::AppCore;

/// Toggles shared by every find widget (spec 11: "persist globally").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FindOptions {
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub regex: bool,
}

/// What the cached matches were computed for: revision, query, options, range.
type ComputedFor = (u64, String, FindOptions, Option<(usize, usize)>);

#[derive(Clone, Debug, Default)]
pub struct FindWidget {
    pub open: bool,
    pub replace_open: bool,
    pub query: String,
    pub replacement: String,
    /// Find in selection: the range searched.
    pub within: Option<(usize, usize)>,
    pub matches: Arc<Vec<(usize, usize)>>,
    pub current: Option<usize>,
    pub error: Option<String>,
    /// Give the query field focus next frame.
    pub focus: bool,
    /// "Search wrapped" hint until this time.
    pub wrapped_until: Option<u64>,
    computed: Option<ComputedFor>,
}

/// Go to Line overlay state.
#[derive(Clone, Debug)]
pub struct GotoLine {
    pub view: ViewId,
    pub text: String,
    pub original: (Selection, ScrollPosition),
    pub focus: bool,
}

#[derive(Default)]
pub struct FindState {
    pub options: FindOptions,
    pub widgets: HashMap<ViewId, FindWidget>,
    pub goto: Option<GotoLine>,
    /// Views to centre on their caret next frame.
    pub center: Vec<ViewId>,
    /// Scroll positions to apply next frame (Go to Line cancel).
    pub scroll: HashMap<ViewId, ScrollPosition>,
}

const WRAP_HINT_MS: u64 = 1500;

impl AppCore {
    fn view_selection(&self, view: ViewId) -> Option<Selection> {
        self.workbench.view(view).map(|v| v.selection.clone())
    }

    /// Edit → Find / Replace: opens the widget, seeded with the selection
    /// (one line) or the word at the caret.
    pub fn open_find(&mut self, view: ViewId, replace: bool) {
        let seed = self.view_selection(view).and_then(|sel| {
            let doc = self
                .workbench
                .view(view)
                .and_then(|v| self.workbench.document(v.document_id))?;
            let rope = doc.buffer.rope();
            let p = sel.primary();
            let (a, b) = if p.is_empty() {
                notoir_editor::line_ops::word_at(rope, p.head)
            } else {
                (p.from(), p.to())
            };
            let text = rope.slice(a..b).to_string();
            (!text.is_empty() && !text.contains('\n')).then_some(text)
        });
        let w = self.find.widgets.entry(view).or_default();
        w.open = true;
        w.replace_open = replace || w.replace_open && w.open;
        if replace {
            w.replace_open = true;
        }
        if let Some(seed) = seed {
            w.query = if self.find.options.regex {
                find::escape(&seed)
            } else {
                seed
            };
        }
        w.focus = true;
        w.computed = None;
        self.refresh_find(view, true);
    }

    pub fn close_find(&mut self, view: ViewId) {
        if let Some(w) = self.find.widgets.get_mut(&view) {
            w.open = false;
            w.within = None;
        }
        self.effects.push(crate::core::Effect::FocusEditor(view));
    }

    fn query_for(&self, w: &FindWidget) -> FindQuery {
        let o = self.find.options;
        FindQuery {
            text: w.query.clone(),
            case_sensitive: o.case_sensitive,
            whole_word: o.whole_word,
            regex: o.regex,
            within: w.within,
        }
    }

    /// Recomputes matches when the query, options or document changed.
    /// `jump`: select the first match at/after the caret (typing in the box).
    pub fn refresh_find(&mut self, view: ViewId, jump: bool) {
        let Some(w) = self.find.widgets.get(&view) else {
            return;
        };
        if !w.open {
            return;
        }
        let Some(doc) = self
            .workbench
            .view(view)
            .and_then(|v| self.workbench.document(v.document_id))
        else {
            return;
        };
        let key = (doc.revision, w.query.clone(), self.find.options, w.within);
        if w.computed.as_ref() == Some(&key) && !jump {
            return;
        }
        let q = self.query_for(w);
        let result = find::find_all(doc.buffer.rope(), &q);
        let caret = self.view_selection(view).map_or(0, |s| s.primary().from());
        let w = self.find.widgets.get_mut(&view).expect("checked");
        w.computed = Some(key);
        match result {
            Ok(m) => {
                w.error = None;
                w.matches = Arc::new(m);
            }
            Err(e) => {
                w.error = Some(e.to_string());
                w.matches = Arc::new(Vec::new());
            }
        }
        let matches = w.matches.clone();
        w.current = matches
            .iter()
            .position(|m| m.0 <= caret && caret < m.1.max(m.0 + 1));
        if jump && let Some((i, _)) = find::next_from(&matches, caret, true) {
            w.current = Some(i);
            let (a, b) = matches[i];
            let _ = self
                .workbench
                .set_selection(view, Selection::new(vec![Range::new(a, b)], 0));
        }
    }

    /// Enter / Shift+Enter, F3 / Shift+F3.
    pub fn find_next(&mut self, view: ViewId, forward: bool) {
        if !self
            .find
            .widgets
            .get(&view)
            .is_some_and(|w| w.open && !w.query.is_empty())
        {
            self.open_find(view, false);
            return;
        }
        self.refresh_find(view, false);
        let Some(sel) = self.view_selection(view) else {
            return;
        };
        let w = self.find.widgets.get_mut(&view).expect("checked");
        let p = sel.primary();
        let pos = if forward { p.to() } else { p.from() };
        let Some((i, wrapped)) = find::next_from(&w.matches, pos, forward) else {
            return;
        };
        w.current = Some(i);
        if wrapped {
            w.wrapped_until = Some(self.now_ms + WRAP_HINT_MS);
        }
        let (a, b) = w.matches[i];
        let _ = self
            .workbench
            .set_selection(view, Selection::new(vec![Range::new(a, b)], 0));
        self.find.center.push(view);
    }

    /// Replace: replaces the current match (when selected) and moves on.
    pub fn replace_one(&mut self, view: ViewId) {
        self.refresh_find(view, false);
        let Some(w) = self.find.widgets.get(&view) else {
            return;
        };
        let Some(sel) = self.view_selection(view) else {
            return;
        };
        let p = sel.primary();
        let hit = w
            .matches
            .iter()
            .copied()
            .find(|m| m.0 == p.from() && m.1 == p.to());
        let Some(m) = hit else {
            self.find_next(view, true);
            return;
        };
        let q = self.query_for(w);
        let with = w.replacement.clone();
        let Some(doc) = self
            .workbench
            .view(view)
            .and_then(|v| self.workbench.document(v.document_id))
        else {
            return;
        };
        let Ok(rep) = find::replacement(doc.buffer.rope(), &q, m, &with) else {
            return;
        };
        let changes = notoir_editor::ChangeSet::replace(m.0, m.1, rep.as_str());
        let after = m.0 + rep.chars().count();
        let now = self.now_ms;
        if self
            .workbench
            .edit(view, changes, Selection::caret(after), EditKind::Other, now)
            .is_ok()
        {
            self.refresh_find(view, false);
            self.find_next(view, true);
        }
    }

    /// Replace All: one undo step; reports the count.
    pub fn replace_all(&mut self, view: ViewId) {
        let Some(w) = self.find.widgets.get(&view) else {
            return;
        };
        let q = self.query_for(w);
        let with = w.replacement.clone();
        let Some(doc) = self
            .workbench
            .view(view)
            .and_then(|v| self.workbench.document(v.document_id))
        else {
            return;
        };
        let Ok(Some((changes, n))) = find::replace_all(doc.buffer.rope(), &q, &with) else {
            return;
        };
        let sel = self.view_selection(view).unwrap_or_default().map(&changes);
        let now = self.now_ms;
        if self
            .workbench
            .edit(view, changes, sel, EditKind::Other, now)
            .is_ok()
        {
            let now = self.now_ms;
            self.notifications.notify(
                Notification::new(
                    Severity::Info,
                    Translatable::new("notifications:replacedAll").arg("count", n),
                ),
                now,
            );
            self.refresh_find(view, false);
        }
    }

    /// Find in selection toggle.
    pub fn toggle_find_in_selection(&mut self, view: ViewId) {
        let sel = self.view_selection(view).map(|s| s.primary());
        if let Some(w) = self.find.widgets.get_mut(&view) {
            w.within = match (w.within, sel) {
                (Some(_), _) => None,
                (None, Some(r)) if !r.is_empty() => Some((r.from(), r.to())),
                _ => None,
            };
        }
        self.refresh_find(view, false);
    }

    /// Matches to paint for a view (when its widget is open).
    pub fn find_highlights(&self, view: ViewId) -> (Arc<Vec<(usize, usize)>>, Option<usize>) {
        match self.find.widgets.get(&view) {
            Some(w) if w.open => (w.matches.clone(), w.current),
            _ => (Arc::new(Vec::new()), None),
        }
    }

    // ── Go to Line ───────────────────────────────────────────────────────

    pub fn open_goto_line(&mut self, view: ViewId) {
        let Some(v) = self.workbench.view(view) else {
            return;
        };
        self.find.goto = Some(GotoLine {
            view,
            text: String::new(),
            original: (v.selection.clone(), v.scroll),
            focus: true,
        });
    }

    /// Previews the typed target (`line`, `line:col`, `+n`, `-n`).
    pub fn goto_preview(&mut self, text: String) {
        let Some(g) = self.find.goto.as_mut() else {
            return;
        };
        g.text = text.clone();
        let view = g.view;
        let origin_line = {
            let Some(doc) = self
                .workbench
                .view(view)
                .and_then(|v| self.workbench.document(v.document_id))
            else {
                return;
            };
            doc.buffer.rope().char_to_line(g.original.0.primary().head)
        };
        let Some((line, col)) = parse_goto(&text, origin_line) else {
            return;
        };
        let Some(doc) = self
            .workbench
            .view(view)
            .and_then(|v| self.workbench.document(v.document_id))
        else {
            return;
        };
        let rope = doc.buffer.rope();
        let line = line.min(rope.len_lines().saturating_sub(1));
        let start = rope.line_to_char(line);
        let len = notoir_editor::display_map::line_text(rope, line)
            .chars()
            .count();
        let pos = start + col.min(len);
        let _ = self.workbench.set_selection(view, Selection::caret(pos));
        self.find.center.push(view);
    }

    /// Enter keeps the position; Esc restores the previous one.
    pub fn close_goto(&mut self, accept: bool) {
        let Some(g) = self.find.goto.take() else {
            return;
        };
        if !accept {
            let _ = self.workbench.set_selection(g.view, g.original.0);
            self.find.scroll.insert(g.view, g.original.1);
        } else if let Some((doc, to)) = self.current_location() {
            // Go to Line is a jump (spec 15): from where the dialog opened.
            let rope = self
                .workbench
                .document(doc)
                .map(|d| d.buffer.rope().clone());
            if let (Some(rope), Some((_, from))) = (rope, self.current_location()) {
                let head = g.original.0.primary().head.min(rope.len_chars());
                let line = rope.char_to_line(head);
                let from = notoir_core::navigation::Location {
                    line,
                    col: head - rope.line_to_char(line),
                    ..from
                };
                let min = self.settings.int("navigation.minLineDistance", None).max(0) as usize;
                if from.line.abs_diff(to.line) > min {
                    self.nav.history.record(from, to.clone());
                    self.nav.last = Some((doc, to));
                }
            }
        }
        self.effects.push(crate::core::Effect::FocusEditor(g.view));
    }
}

/// `120`, `120:5`, `+10`, `-3` → 0-based (line, column).
pub fn parse_goto(text: &str, current_line: usize) -> Option<(usize, usize)> {
    let t = text.trim();
    let (l, c) = match t.split_once(':') {
        Some((l, c)) => (l.trim(), c.trim().parse::<usize>().ok().unwrap_or(1)),
        None => (t, 1),
    };
    let line = if let Some(n) = l.strip_prefix('+') {
        current_line + n.parse::<usize>().ok()?
    } else if let Some(n) = l.strip_prefix('-') {
        current_line.saturating_sub(n.parse::<usize>().ok()?)
    } else {
        l.parse::<usize>().ok()?.saturating_sub(1)
    };
    Some((line, c.saturating_sub(1)))
}

#[cfg(test)]
mod tests {
    use super::parse_goto;

    #[test]
    fn goto_forms() {
        assert_eq!(parse_goto("120", 0), Some((119, 0)));
        assert_eq!(parse_goto("12:5", 0), Some((11, 4)));
        assert_eq!(parse_goto("+10", 5), Some((15, 0)));
        assert_eq!(parse_goto("-10", 5), Some((0, 0)));
        assert_eq!(parse_goto("x", 5), None);
        assert_eq!(parse_goto("", 5), None);
    }
}
