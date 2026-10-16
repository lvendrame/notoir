//! Notes (docs/specs/34): wiki links, backlinks, tags and the link index of
//! the open workspace. The index is a cache built off the UI thread from the
//! workspace's Markdown files and refreshed from open buffers; it is rebuilt
//! whenever the workspace file index is.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use notoir_core::ids::{DocumentId, ViewId};
use notoir_core::notifications::{Notification, Severity, Translatable};
use notoir_editor::wiki;
use notoir_editor::{ChangeSet, Edit, EditKind, Selection};
use notoir_workspace::links::{self, Backlink, LinkIndex, Resolution};

use crate::core::{AppCore, Effect};
use crate::files::{Answer, OpenAt, Prompt};
use crate::io::IoResult;

/// Buffer edits reach the index after this pause.
const BUFFER_DEBOUNCE_MS: u64 = 300;
/// `[[` completion rows.
const COMPLETION_ROWS: usize = 8;

/// The `[[` completion popup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Completion {
    pub view: ViewId,
    /// Char offset where the typed target starts (after `[[`).
    pub start: usize,
    /// Caret position.
    pub end: usize,
    /// (note name, relative path).
    pub items: Vec<(String, String)>,
    pub selected: usize,
}

/// Confirmations owned by notes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotesPrompt {
    /// Follow a link to a missing note: create it?
    Create { path: PathBuf, name: String },
    /// A renamed note is linked from these notes: update their links?
    UpdateLinks {
        old_stem: String,
        new_stem: String,
        files: Vec<String>,
    },
}

/// What a notes picker lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotesPick {
    /// Several notes match a link: (relative path, heading).
    Targets(Option<String>),
    /// Notes with a tag.
    TagFiles(String),
}

/// Wiki link ranges of a document: (start, end, missing target).
pub type DocLinks = Arc<Vec<(usize, usize, bool)>>;

#[derive(Default)]
pub struct NotesState {
    pub index: LinkIndex,
    /// The index was built at least once for this workspace.
    pub ready: bool,
    /// The workspace index the link index was last built from.
    built_from: Option<usize>,
    building: bool,
    /// Edited notes → when to re-read their buffer.
    dirty: HashMap<DocumentId, u64>,
    /// Link ranges per document as of a revision: (revision, [(start, end, missing)]).
    pub(crate) doc_links: HashMap<DocumentId, (u64, DocLinks)>,
    pub completion: Option<Completion>,
    /// Completion closed with Escape at this caret position.
    dismissed: Option<(ViewId, usize)>,
    /// Unlinked mentions of a note: (note, results); computed in the
    /// background when the Backlinks panel asks.
    pub unlinked: Option<(String, Vec<Backlink>)>,
    unlinked_pending: Option<String>,
    /// Notes picker rows: (value, label, detail).
    pub picker: Vec<(String, String, String)>,
    pub pick: Option<NotesPick>,
    /// Tags panel: the selected tag.
    pub tag_filter: Option<String>,
}

fn rel_of(root: &Path, path: &Path) -> Option<String> {
    path.strip_prefix(root)
        .ok()
        .map(|r| r.to_string_lossy().replace('\\', "/"))
}

impl AppCore {
    fn notes_root(&self) -> Option<PathBuf> {
        self.workspace.as_ref().map(|w| w.root.clone())
    }

    /// The workspace-relative path of a document inside the workspace.
    pub fn note_rel(&self, doc: DocumentId) -> Option<String> {
        let root = self.workspace.as_ref()?.root.clone();
        let path = self.workbench.document(doc)?.path()?.to_path_buf();
        rel_of(&root, &path)
    }

    /// Per frame: (re)build after the workspace index changes, fold edited
    /// buffers in, refresh the completion popup.
    pub fn notes_tick(&mut self) {
        let Some(ws) = &self.workspace else {
            if self.notes.ready || self.notes.built_from.is_some() {
                self.notes = NotesState::default();
            }
            return;
        };
        let ptr = Arc::as_ptr(&ws.index) as usize;
        if ws.index_ready && self.notes.built_from != Some(ptr) && !self.notes.building {
            self.notes.building = true;
            self.notes.built_from = Some(ptr);
            let root = ws.root.clone();
            let generation = ws.generation;
            let files: Vec<String> = ws
                .index
                .iter()
                .filter(|e| e.kind == notoir_core::explorer::NodeKind::File && !e.ignored)
                .map(|e| e.rel.clone())
                .collect();
            self.fs.submit(move || {
                Some(IoResult::NotesIndexed(
                    generation,
                    Box::new(LinkIndex::build(&root, files)),
                ))
            });
        }
        let now = self.now_ms;
        let due: Vec<DocumentId> = self
            .notes
            .dirty
            .iter()
            .filter(|(_, t)| now >= **t)
            .map(|(d, _)| *d)
            .collect();
        for doc in due {
            self.notes.dirty.remove(&doc);
            self.notes_update_from_buffer(doc);
        }
        self.notes_refresh_completion();
    }

    /// Notes: Rebuild Index — drops the cache and builds it again.
    pub fn notes_rebuild(&mut self) {
        self.notes.built_from = None;
        self.notes.building = false;
    }

    pub fn notes_next_due(&self) -> Option<u64> {
        self.notes.dirty.values().copied().min()
    }

    pub fn notes_indexed(&mut self, generation: u64, index: LinkIndex) {
        self.notes.building = false;
        if self.workspace.as_ref().map(|w| w.generation) != Some(generation) {
            return;
        }
        self.notes.index = index;
        self.notes.ready = true;
        self.notes.doc_links.clear();
        // Unsaved buffers win over disk content.
        let dirty: Vec<DocumentId> = self
            .workbench
            .documents
            .values()
            .filter(|d| d.is_dirty())
            .map(|d| d.id)
            .collect();
        for doc in dirty {
            self.notes_update_from_buffer(doc);
        }
    }

    /// A note's buffer changed or was saved.
    pub fn notes_doc_changed(&mut self, doc: DocumentId) {
        if self.note_rel(doc).is_some_and(|r| links::is_note(&r)) {
            self.notes
                .dirty
                .insert(doc, self.now_ms + BUFFER_DEBOUNCE_MS);
        }
    }

    fn notes_update_from_buffer(&mut self, doc: DocumentId) {
        let Some(rel) = self.note_rel(doc) else {
            return;
        };
        let Some(text) = self.workbench.document(doc).map(|d| d.buffer.text()) else {
            return;
        };
        self.notes.index.update(&rel, &text);
        self.notes.doc_links.clear();
    }

    /// Wiki link ranges of a Markdown document for the editor (missing
    /// targets flagged), cached per revision.
    pub fn notes_doc_links(&mut self, doc: DocumentId) -> Arc<Vec<(usize, usize, bool)>> {
        let Some(d) = self.workbench.document(doc) else {
            return Arc::default();
        };
        if d.language != notoir_editor::LanguageId::MARKDOWN || d.large_file {
            return Arc::default();
        }
        if let Some((rev, l)) = self.notes.doc_links.get(&doc)
            && *rev == d.revision
        {
            return l.clone();
        }
        let revision = d.revision;
        let text = d.buffer.text();
        let from = self.note_rel(doc);
        let ready = self.notes.ready;
        let ranges: Vec<(usize, usize, bool)> = wiki::links(&text)
            .into_iter()
            .map(|l| {
                let missing = ready
                    && self.notes.index.resolve(&l.target, from.as_deref()) == Resolution::Missing;
                (l.range.0, l.range.1, missing)
            })
            .collect();
        let ranges = Arc::new(ranges);
        self.notes.doc_links.insert(doc, (revision, ranges.clone()));
        ranges
    }

    /// Whether a link target written in `doc` has no note (preview styling).
    pub fn notes_missing(&self, doc: DocumentId, target: &str) -> bool {
        self.notes.ready
            && self
                .notes
                .index
                .resolve(target, self.note_rel(doc).as_deref())
                == Resolution::Missing
    }

    // ── following links ──────────────────────────────────────────────────

    /// Ctrl/Cmd+click (or Follow Link) at `offset` of the view's document.
    pub fn notes_follow_at(&mut self, view: ViewId, offset: usize) -> bool {
        let Some(doc) = self.workbench.view(view).map(|v| v.document_id) else {
            return false;
        };
        let Some(text) = self.workbench.document(doc).map(|d| d.buffer.text()) else {
            return false;
        };
        let Some(link) = wiki::link_at(&text, offset) else {
            return false;
        };
        let target = match &link.heading {
            Some(h) => format!("{}#{h}", link.target),
            None => link.target.clone(),
        };
        self.notes_follow(doc, &target);
        true
    }

    /// Follows a link target (`Note`, `Note#Heading`, `folder/Note`) written
    /// in `doc`.
    pub fn notes_follow(&mut self, doc: DocumentId, target: &str) {
        let (name, heading) = match target.split_once('#') {
            Some((n, h)) => (n.trim().to_owned(), Some(h.trim().to_owned())),
            None => (target.trim().to_owned(), None),
        };
        let Some(root) = self.notes_root() else {
            return;
        };
        let from = self.note_rel(doc);
        if name.is_empty() {
            // `[[#Heading]]`: inside this document.
            if let (Some(h), Some(view)) = (heading, self.workbench.active_editor()) {
                self.place_caret_at(
                    view,
                    &OpenAt {
                        anchor: Some(notoir_editor::markdown::slug(&h)),
                        ..Default::default()
                    },
                );
            }
            return;
        }
        match self.notes.index.resolve(&name, from.as_deref()) {
            Resolution::Found(rel) => self.notes_open(&root.join(rel), heading.as_deref()),
            Resolution::Ambiguous(rels) => {
                self.notes.picker = rels
                    .into_iter()
                    .map(|r| {
                        let dir = r.rsplit_once('/').map_or("", |(d, _)| d).to_owned();
                        (r.clone(), links::stem(&r).to_owned(), dir)
                    })
                    .collect();
                self.notes.pick = Some(NotesPick::Targets(heading));
                self.effects.push(Effect::OpenPicker(
                    crate::shell::overlays::PickerKind::Notes,
                ));
            }
            Resolution::Missing => {
                // A new note next to the linking one (or at the root).
                let file = if name.to_lowercase().ends_with(".md") {
                    name.clone()
                } else {
                    format!("{name}.md")
                };
                let dir = from
                    .as_deref()
                    .and_then(|f| f.rsplit_once('/'))
                    .map_or_else(|| root.clone(), |(d, _)| root.join(d));
                let path = if file.contains('/') {
                    root.join(&file)
                } else {
                    dir.join(&file)
                };
                let display = links::stem(&file).to_owned();
                self.files
                    .prompts
                    .push_back(Prompt::Notes(NotesPrompt::Create {
                        path,
                        name: display,
                    }));
            }
        }
    }

    fn notes_open(&mut self, path: &Path, heading: Option<&str>) {
        self.open_path(
            path,
            OpenAt {
                anchor: heading.map(notoir_editor::markdown::slug),
                ..Default::default()
            },
        );
    }

    /// A notes picker row was chosen.
    pub fn notes_pick(&mut self, value: &str) {
        let Some(root) = self.notes_root() else {
            return;
        };
        match self.notes.pick.take() {
            Some(NotesPick::Targets(heading)) => {
                self.notes_open(&root.join(value), heading.as_deref())
            }
            Some(NotesPick::TagFiles(_)) => self.notes_open(&root.join(value), None),
            None => {}
        }
    }

    /// Quick Open `#tag` / Tags panel: the notes with a tag.
    pub fn notes_pick_tag(&mut self, tag: &str) {
        self.notes.picker = self
            .notes
            .index
            .files_with_tag(tag)
            .into_iter()
            .map(|r| {
                let dir = r.rsplit_once('/').map_or("", |(d, _)| d).to_owned();
                (r.clone(), links::stem(&r).to_owned(), dir)
            })
            .collect();
        self.notes.pick = Some(NotesPick::TagFiles(tag.to_owned()));
        self.effects.push(Effect::OpenPicker(
            crate::shell::overlays::PickerKind::Notes,
        ));
    }

    pub fn notes_answer(&mut self, p: NotesPrompt, answer: Answer) {
        if answer != Answer::Primary {
            return;
        }
        match p {
            NotesPrompt::Create { path, .. } => self.create_note_file(path),
            NotesPrompt::UpdateLinks {
                old_stem,
                new_stem,
                files,
            } => self.notes_update_links(&old_stem, &new_stem, &files),
        }
    }

    // ── rename ───────────────────────────────────────────────────────────

    /// After a rename: offer to update links to the old name.
    pub fn notes_renamed(&mut self, from: &Path, to: &Path) {
        let Some(root) = self.notes_root() else {
            return;
        };
        let (Some(old_rel), Some(new_rel)) = (rel_of(&root, from), rel_of(&root, to)) else {
            return;
        };
        if !links::is_note(&old_rel) {
            return;
        }
        let files = self.notes.index.linking_notes(&old_rel);
        let (old_stem, new_stem) = (links::stem(&old_rel), links::stem(&new_rel));
        if files.is_empty() || old_stem == new_stem {
            return;
        }
        self.files
            .prompts
            .push_back(Prompt::Notes(NotesPrompt::UpdateLinks {
                old_stem: old_stem.to_owned(),
                new_stem: new_stem.to_owned(),
                files,
            }));
    }

    /// Rewrites `[[old]]` links: open documents as one undoable edit each,
    /// closed files on disk.
    fn notes_update_links(&mut self, old: &str, new: &str, files: &[String]) {
        let Some(root) = self.notes_root() else {
            return;
        };
        let mut closed = Vec::new();
        for rel in files {
            let path = root.join(rel);
            let (_, key) = notoir_workspace::fs_io::identity(&path);
            match self.workbench.find_by_key(&key) {
                Some(doc) => {
                    let Some(text) = self.workbench.document(doc).map(|d| d.buffer.text()) else {
                        continue;
                    };
                    let edits: Vec<Edit> = wiki::retarget(&text, old, new)
                        .into_iter()
                        .map(|((a, b), s)| Edit::new(a, b, s))
                        .collect();
                    if edits.is_empty() {
                        continue;
                    }
                    let view = self.workbench.views.of_document(doc).next().map(|v| v.id);
                    let selection = view
                        .and_then(|v| self.workbench.view(v))
                        .map(|v| v.selection.clone())
                        .unwrap_or_else(|| Selection::caret(0));
                    if let Ok(changes) = ChangeSet::new(edits) {
                        let selection = selection.map(&changes);
                        let _ = self.workbench.apply_edit(
                            doc,
                            view,
                            changes,
                            selection,
                            EditKind::Other,
                            notoir_core::ChangeOrigin::Internal,
                            self.now_ms,
                        );
                    }
                }
                None => closed.push(path),
            }
        }
        if !closed.is_empty() {
            let (old, new) = (old.to_owned(), new.to_owned());
            self.io.submit(move || {
                let mut failed = Vec::new();
                for path in closed {
                    let r = std::fs::read_to_string(&path).and_then(|text| {
                        let edits = wiki::retarget(&text, &old, &new);
                        if edits.is_empty() {
                            return Ok(());
                        }
                        let mut chars: Vec<char> = text.chars().collect();
                        for ((a, b), s) in edits.into_iter().rev() {
                            chars.splice(a..b, s.chars());
                        }
                        let out: String = chars.into_iter().collect();
                        notoir_storage::atomic::write(&path, out.as_bytes(), Default::default())
                            .map(drop)
                    });
                    if let Err(e) = r {
                        failed.push(format!("{}: {e}", path.display()));
                    }
                }
                Some(IoResult::NotesLinksUpdated(failed))
            });
        }
    }

    pub fn notes_links_updated(&mut self, failed: Vec<String>) {
        if !failed.is_empty() {
            self.notify(
                Notification::new(
                    Severity::Error,
                    Translatable::new("notes:updateFailed").arg("count", failed.len()),
                )
                .detail(failed.join("\n")),
            );
        }
    }

    // ── completion ───────────────────────────────────────────────────────

    /// Shows `[[` completion while the caret is right after `[[partial`.
    fn notes_refresh_completion(&mut self) {
        let view = self.workbench.active_editor();
        let target = view.and_then(|view| {
            let v = self.workbench.view(view)?;
            if !self.editor_focused || v.selection.len() != 1 || !v.selection.primary().is_empty() {
                return None;
            }
            let d = self.workbench.document(v.document_id)?;
            if d.language != notoir_editor::LanguageId::MARKDOWN {
                return None;
            }
            let rope = d.buffer.rope();
            let head = v.selection.primary().head.min(rope.len_chars());
            let line = rope.char_to_line(head);
            let line_start = rope.line_to_char(line);
            let before: String = rope.slice(line_start..head).to_string();
            let partial = wiki::completion_prefix(&before)?.to_owned();
            Some((view, head - partial.chars().count(), head, partial))
        });
        let Some((view, start, end, partial)) = target else {
            self.notes.completion = None;
            self.notes.dismissed = None;
            return;
        };
        if self.notes.dismissed == Some((view, end)) {
            return;
        }
        let q = partial.to_lowercase();
        let mut items: Vec<(i32, (String, String))> = self
            .notes
            .index
            .note_names()
            .into_iter()
            .filter_map(|(name, rel)| {
                if q.is_empty() {
                    return Some((0, (name, rel)));
                }
                notoir_core::fuzzy::fuzzy_match(&q, &name).map(|m| (m.score, (name, rel)))
            })
            .collect();
        items.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.0.cmp(&b.1.0)));
        let items: Vec<(String, String)> = items
            .into_iter()
            .map(|(_, i)| i)
            .take(COMPLETION_ROWS)
            .collect();
        if items.is_empty() {
            self.notes.completion = None;
            return;
        }
        let selected = self
            .notes
            .completion
            .as_ref()
            .filter(|c| c.view == view && c.start == start)
            .map_or(0, |c| c.selected.min(items.len() - 1));
        self.notes.completion = Some(Completion {
            view,
            start,
            end,
            items,
            selected,
        });
    }

    /// Completion keys (before the editor sees them): up/down/enter/tab/esc.
    pub fn notes_completion_key(&mut self, key: egui::Key) -> bool {
        let Some(c) = self.notes.completion.as_mut() else {
            return false;
        };
        match key {
            egui::Key::ArrowDown => c.selected = (c.selected + 1) % c.items.len(),
            egui::Key::ArrowUp => c.selected = (c.selected + c.items.len() - 1) % c.items.len(),
            egui::Key::Enter | egui::Key::Tab => {
                let i = c.selected;
                self.notes_complete(i);
            }
            egui::Key::Escape => {
                self.notes.dismissed = Some((c.view, c.end));
                self.notes.completion = None;
            }
            _ => return false,
        }
        true
    }

    /// Inserts completion row `i` as `Name]]`.
    pub fn notes_complete(&mut self, i: usize) {
        let Some(c) = self.notes.completion.take() else {
            return;
        };
        let Some((name, _)) = c.items.get(i).cloned() else {
            return;
        };
        let Some(doc) = self.workbench.view(c.view).map(|v| v.document_id) else {
            return;
        };
        let closed = self.workbench.document(doc).is_some_and(|d| {
            let rope = d.buffer.rope();
            let end = (c.end + 2).min(rope.len_chars());
            rope.slice(c.end..end) == "]]"
        });
        let insert = if closed { name } else { format!("{name}]]") };
        let caret = c.start + insert.chars().count() + if closed { 2 } else { 0 };
        let Ok(changes) = ChangeSet::new(vec![Edit::new(c.start, c.end, insert)]) else {
            return;
        };
        let _ = self.workbench.edit(
            c.view,
            changes,
            Selection::caret(caret),
            EditKind::Other,
            self.now_ms,
        );
    }

    // ── backlinks panel ──────────────────────────────────────────────────

    /// The active editor's note.
    pub fn notes_active_rel(&self) -> Option<String> {
        let view = self.workbench.active_editor()?;
        let doc = self.workbench.view(view)?.document_id;
        self.note_rel(doc).filter(|r| links::is_note(r))
    }

    /// Unlinked mentions of `rel` (computed in the background).
    pub fn notes_find_unlinked(&mut self, rel: &str) {
        if self.notes.unlinked_pending.as_deref() == Some(rel)
            || self.notes.unlinked.as_ref().is_some_and(|(r, _)| r == rel)
        {
            return;
        }
        let Some(root) = self.notes_root() else {
            return;
        };
        self.notes.unlinked_pending = Some(rel.to_owned());
        let name = links::stem(rel).to_owned();
        let target = rel.to_owned();
        let notes: Vec<String> = self
            .notes
            .index
            .note_names()
            .into_iter()
            .map(|(_, r)| r)
            .filter(|r| r != rel)
            .collect();
        // Open buffers are searched as edited.
        let mut buffers = HashMap::new();
        for r in &notes {
            let (_, key) = notoir_workspace::fs_io::identity(&root.join(r));
            if let Some(d) = self
                .workbench
                .find_by_key(&key)
                .and_then(|d| self.workbench.document(d))
            {
                buffers.insert(r.clone(), d.buffer.text());
            }
        }
        self.fs.submit(move || {
            let mut found = Vec::new();
            for r in notes {
                let text = match buffers.remove(&r) {
                    Some(t) => t,
                    None => match std::fs::read_to_string(root.join(&r)) {
                        Ok(t) => t,
                        Err(_) => continue,
                    },
                };
                let lines: Vec<&str> = text.split('\n').collect();
                for (line, _) in wiki::mentions(&text, &name) {
                    found.push(Backlink {
                        source: r.clone(),
                        line,
                        snippet: lines
                            .get(line)
                            .map(|l| l.trim().to_owned())
                            .unwrap_or_default(),
                    });
                }
            }
            Some(IoResult::NotesUnlinked(target, found))
        });
    }

    pub fn notes_unlinked_found(&mut self, rel: String, found: Vec<Backlink>) {
        if self.notes.unlinked_pending.as_deref() == Some(rel.as_str()) {
            self.notes.unlinked_pending = None;
        }
        self.notes.unlinked = Some((rel, found));
    }

    /// Opens a note at a 0-based line (Backlinks rows).
    pub fn notes_open_line(&mut self, rel: &str, line: usize) {
        if let Some(root) = self.notes_root() {
            self.open_path(
                &root.join(rel),
                OpenAt {
                    line_col: Some((line + 1, 1)),
                    ..Default::default()
                },
            );
        }
    }
}
