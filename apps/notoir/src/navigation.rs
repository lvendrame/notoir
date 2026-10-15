//! Navigation history wiring (docs/specs/15 §Navigation History): document
//! switches and command jumps are recorded; Back / Forward reopen files when
//! needed.

use notoir_core::document::DocumentOrigin;
use notoir_core::ids::DocumentId;
use notoir_core::navigation::{DocRef, Location};
use notoir_editor::Selection;

use crate::core::AppCore;
use crate::files::OpenAt;

#[derive(Default)]
pub struct NavState {
    pub history: notoir_core::navigation::NavigationHistory,
    /// The active location as of the last frame.
    pub last: Option<(DocumentId, Location)>,
    /// Back/Forward in progress: the next document switch is not a new entry.
    pub navigating: bool,
    /// The command that just ran was Back/Forward (its jump is no entry).
    stepped: bool,
}

impl AppCore {
    fn doc_ref(&self, doc: DocumentId) -> Option<DocRef> {
        let d = self.workbench.document(doc)?;
        Some(match &d.origin {
            DocumentOrigin::File { path, .. } => DocRef::Path(path.clone()),
            DocumentOrigin::Scratchpad { scratch_id, .. } => DocRef::Scratchpad(scratch_id.clone()),
            _ => DocRef::Open(doc),
        })
    }

    /// The active view's caret as a location.
    pub fn current_location(&self) -> Option<(DocumentId, Location)> {
        let v = self.workbench.view(self.workbench.active_view()?)?;
        let rope = self.workbench.document(v.document_id)?.buffer.rope();
        let head = v.selection.primary().head.min(rope.len_chars());
        let line = rope.char_to_line(head);
        Some((
            v.document_id,
            Location {
                doc: self.doc_ref(v.document_id)?,
                line,
                col: head - rope.line_to_char(line),
            },
        ))
    }

    /// Every frame: a change of the active document is a navigation entry.
    pub fn nav_track(&mut self) {
        let now = self.current_location();
        if let (Some((last_doc, last)), Some((doc, loc))) = (self.nav.last.clone(), now.clone())
            && last_doc != doc
        {
            if self.nav.navigating {
                self.nav.navigating = false;
            } else {
                self.nav.history.record(last, loc);
            }
        }
        if now.is_some() {
            self.nav.last = now;
        }
    }

    /// After a command: a jump of more than `navigation.minLineDistance`
    /// lines within the document is an entry.
    pub fn nav_after_command(&mut self, before: Option<(DocumentId, Location)>) {
        if std::mem::take(&mut self.nav.stepped) {
            return;
        }
        let (Some((bd, b)), Some((ad, a))) = (before, self.current_location()) else {
            return;
        };
        let min = self.settings.int("navigation.minLineDistance", None).max(0) as usize;
        if bd == ad && b.line.abs_diff(a.line) > min && !self.nav.navigating {
            self.nav.history.record(b, a.clone());
            self.nav.last = Some((ad, a));
        }
    }

    fn location_valid(&self, l: &Location) -> bool {
        match &l.doc {
            DocRef::Path(p) => self.workbench.documents.values().any(|d| d.path() == Some(p.as_path())) || p.is_file(),
            DocRef::Open(id) => self.workbench.document(*id).is_some(),
            DocRef::Scratchpad(s) => self
                .workbench
                .documents
                .values()
                .any(|d| matches!(&d.origin, DocumentOrigin::Scratchpad { scratch_id, .. } if scratch_id == s)),
        }
    }

    pub fn nav_step(&mut self, forward: bool) {
        self.nav.stepped = true;
        let Some((_, current)) = self.current_location() else {
            return;
        };
        let target = {
            let mut h = std::mem::take(&mut self.nav.history);
            let valid = |l: &Location| self.location_valid(l);
            let t = if forward {
                h.forward(current.clone(), valid)
            } else {
                h.back(current.clone(), valid)
            };
            self.nav.history = h;
            t
        };
        let Some(t) = target else { return };
        if t.doc != current.doc {
            self.nav.navigating = true;
        }
        self.go_to_location(&t);
    }

    fn go_to_location(&mut self, l: &Location) {
        let doc = match &l.doc {
            DocRef::Path(p) => {
                let open = self
                    .workbench
                    .documents
                    .values()
                    .find(|d| d.path() == Some(p.as_path()))
                    .map(|d| d.id);
                match open {
                    Some(d) => d,
                    None => {
                        self.open_path(
                            p,
                            OpenAt {
                                line_col: Some((l.line + 1, l.col + 1)),
                                ..Default::default()
                            },
                        );
                        return;
                    }
                }
            }
            DocRef::Open(id) => *id,
            DocRef::Scratchpad(s) => {
                let Some(d) = self
                    .workbench
                    .documents
                    .values()
                    .find(|d| matches!(&d.origin, DocumentOrigin::Scratchpad { scratch_id, .. } if scratch_id == s))
                    .map(|d| d.id)
                else {
                    return;
                };
                d
            }
        };
        let group = self.workbench.layout.active_group;
        let view = self
            .workbench
            .view_in_group(doc, group)
            .or_else(|| self.workbench.views.of_document(doc).next().map(|v| v.id));
        let view = match view {
            Some(v) => v,
            None => {
                match self
                    .workbench
                    .open_view(doc, group, notoir_core::TabPosition::AfterActive)
                {
                    Ok(v) => v,
                    Err(_) => return,
                }
            }
        };
        let _ = self.workbench.activate_view(view);
        let Some(rope) = self
            .workbench
            .document(doc)
            .map(|d| d.buffer.rope().clone())
        else {
            return;
        };
        let line = l.line.min(rope.len_lines().saturating_sub(1));
        let start = rope.line_to_char(line);
        let len = notoir_editor::display_map::line_text(&rope, line)
            .chars()
            .count();
        let _ = self
            .workbench
            .set_selection(view, Selection::caret(start + l.col.min(len)));
        self.find.center.push(view);
        self.effects.push(crate::core::Effect::FocusEditor(view));
    }
}
