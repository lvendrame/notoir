//! Workspace search state (docs/specs/15 §Workspace Search): debounced runs on
//! a background thread with cancellation, streamed results, opening results,
//! Replace All across open documents (undoable) and closed files (through the
//! save writer with its conflict check).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use notoir_core::document::DocumentOrigin;
use notoir_core::ids::DocumentId;
use notoir_core::notifications::{Notification, Severity, Translatable};
use notoir_editor::find::{self, FindQuery};
use notoir_editor::{EditKind, Rope, Selection};
use notoir_workspace::search::{self, Batch, FileMatches, SearchOptions};

use crate::core::AppCore;
use crate::files::OpenAt;
use crate::io::IoResult;

/// Typing restarts the search after this pause.
pub const DEBOUNCE_MS: u64 = 300;

#[derive(Default)]
pub struct SearchState {
    pub query: String,
    pub replace: String,
    pub include: String,
    pub exclude: String,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub regex: bool,
    pub results: Vec<FileMatches>,
    pub done: bool,
    pub truncated: bool,
    pub error: Option<String>,
    /// The last finished search read only files the index listed.
    pub indexed: bool,
    pub generation: u64,
    cancel: Option<Arc<AtomicBool>>,
    /// Run at this time (debounce).
    pub due: Option<u64>,
    /// Collapsed file rows (by `rel`).
    pub collapsed: HashSet<String>,
    /// Focus the query box next frame.
    pub focus_query: bool,
    /// Next/Previous Search Result position (file, match).
    pub cursor: Option<(usize, usize)>,
    /// Files changed by the last Replace All (open documents, for Undo).
    pub last_replace: Vec<DocumentId>,
    /// Open files already replaced while closed ones are still being written.
    pending_names: Vec<String>,
}

impl SearchState {
    pub fn total(&self) -> usize {
        self.results.iter().map(|f| f.matches.len()).sum()
    }

    pub fn find_query(&self) -> FindQuery {
        FindQuery {
            text: self.query.clone(),
            case_sensitive: self.case_sensitive,
            whole_word: self.whole_word,
            regex: self.regex,
            within: None,
        }
    }
}

fn split_globs(s: &str) -> Vec<String> {
    s.split(',')
        .map(|g| g.trim().to_owned())
        .filter(|g| !g.is_empty())
        .collect()
}

impl AppCore {
    /// The query or an option changed: search again after the debounce.
    pub fn search_changed(&mut self) {
        self.search.due = Some(self.now_ms + DEBOUNCE_MS);
    }

    pub fn search_tick(&mut self) {
        if self.search.due.is_some_and(|d| self.now_ms >= d) {
            self.search.due = None;
            self.start_search();
        }
    }

    pub fn search_next_due(&self) -> Option<u64> {
        self.search.due
    }

    /// Cancels any running search and starts a new one.
    pub fn start_search(&mut self) {
        if let Some(c) = self.search.cancel.take() {
            c.store(true, Ordering::Relaxed);
        }
        self.search.generation += 1;
        self.search.results.clear();
        self.search.error = None;
        self.search.truncated = false;
        self.search.cursor = None;
        if self.search.query.is_empty() {
            self.search.done = true;
            return;
        }
        self.search.done = false;
        let mut exclude = self.settings.str_list("files.exclude", None);
        exclude.extend(self.settings.str_list("search.exclude", None));
        exclude.extend(split_globs(&self.search.exclude));
        let opts = SearchOptions {
            root: self.workspace.as_ref().map(|w| w.root.clone()),
            query: self.search.find_query(),
            include: split_globs(&self.search.include),
            exclude,
            max_file_size: self.settings.int("search.maxFileSize", None).max(0) as u64,
            max_results: self.settings.int("search.maxResults", None).max(1) as usize,
            index: self.text_index_dir(),
        };
        // Open documents are searched in memory (unsaved edits included).
        let mut snapshots: HashMap<PathBuf, Rope> = HashMap::new();
        let mut unsaved: Vec<(String, Rope)> = Vec::new();
        let ids: Vec<DocumentId> = self.workbench.documents.keys().copied().collect();
        for id in ids {
            let Some(d) = self.workbench.document(id) else {
                continue;
            };
            let rope = d.buffer.rope().clone();
            match &d.origin {
                DocumentOrigin::File { path, .. } => {
                    snapshots.insert(path.clone(), rope);
                }
                DocumentOrigin::Virtual { .. } => {}
                _ => unsaved.push((self.document_title(id), rope)),
            }
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.search.cancel = Some(cancel.clone());
        let generation = self.search.generation;
        let poster = self.fs.poster();
        let spawned = std::thread::Builder::new()
            .name("notoir-search".into())
            .spawn(move || {
                search::run(&opts, &snapshots, &unsaved, &cancel, |batch| {
                    if !cancel.load(Ordering::Relaxed) {
                        poster.post(IoResult::Search(Box::new((generation, batch))));
                    }
                });
            });
        if let Err(e) = spawned {
            tracing::error!(error = %e, "could not start search thread");
        }
    }

    pub fn handle_search(&mut self, generation: u64, batch: Batch) {
        if generation != self.search.generation {
            return;
        }
        self.search.results.extend(batch.files);
        if batch.done {
            self.search.indexed = batch.indexed;
            self.search.done = true;
            self.search.truncated = batch.truncated;
            self.search.error = batch.error;
            self.search.cancel = None;
        }
    }

    /// Opens a result: the file (or open Untitled/Scratchpad by title) with
    /// the match selected.
    pub fn open_search_result(&mut self, file: usize, m: usize, side: bool) {
        let Some(f) = self.search.results.get(file).cloned() else {
            return;
        };
        let Some(lm) = f.matches.get(m).cloned() else {
            return;
        };
        self.search.cursor = Some((file, m));
        let at = OpenAt {
            line_col: Some((lm.line + 1, lm.start_col + 1)),
            select_chars: lm.end_col - lm.start_col,
            side,
            ..Default::default()
        };
        if f.path.as_os_str().is_empty() {
            // Untitled / Scratchpad.
            let doc = self
                .workbench
                .documents
                .keys()
                .copied()
                .find(|d| self.document_title(*d) == f.rel);
            if let Some(doc) = doc {
                let view = self.workbench.views.of_document(doc).next().map(|v| v.id);
                if let Some(v) = view {
                    let _ = self.workbench.activate_view(v);
                    if let Some(rope) = self
                        .workbench
                        .document(doc)
                        .map(|d| d.buffer.rope().clone())
                    {
                        let start = rope
                            .line_to_char(lm.line.min(rope.len_lines().saturating_sub(1)))
                            + lm.start_col;
                        let end = start + lm.end_col - lm.start_col;
                        let sel = Selection::single(notoir_editor::Range::new(
                            start.min(rope.len_chars()),
                            end.min(rope.len_chars()),
                        ));
                        let _ = self.workbench.set_selection(v, sel);
                        self.find.center.push(v);
                    }
                }
            }
            return;
        }
        self.open_path(&f.path, at);
    }

    /// Next / Previous Search Result through the panel's results.
    pub fn search_step(&mut self, forward: bool) -> bool {
        let flat: Vec<(usize, usize)> = self
            .search
            .results
            .iter()
            .enumerate()
            .flat_map(|(fi, f)| (0..f.matches.len()).map(move |mi| (fi, mi)))
            .collect();
        if flat.is_empty() {
            return false;
        }
        let i = match self
            .search
            .cursor
            .and_then(|c| flat.iter().position(|x| *x == c))
        {
            Some(i) if forward => (i + 1) % flat.len(),
            Some(i) => (i + flat.len() - 1) % flat.len(),
            None if forward => 0,
            None => flat.len() - 1,
        };
        let (f, m) = flat[i];
        self.open_search_result(f, m, false);
        true
    }

    /// Removes a file (or one match) from the results.
    pub fn dismiss_result(&mut self, file: usize, m: Option<usize>) {
        match m {
            Some(m) => {
                if let Some(f) = self.search.results.get_mut(file) {
                    if m < f.matches.len() {
                        f.matches.remove(m);
                    }
                    if f.matches.is_empty() {
                        self.search.results.remove(file);
                    }
                }
            }
            None => {
                if file < self.search.results.len() {
                    self.search.results.remove(file);
                }
            }
        }
        self.search.cursor = None;
    }

    /// Replace All: confirmation first.
    pub fn request_replace_all(&mut self) {
        let total = self.search.total();
        if total == 0 || self.search.query.is_empty() {
            return;
        }
        self.files
            .prompts
            .push_back(crate::files::Prompt::ReplaceAll {
                count: total,
                files: self.search.results.len(),
            });
    }

    /// Replaces in the listed files (`only`: one file / one match).
    pub fn replace_results(&mut self, only: Option<(usize, Option<usize>)>) {
        let q = self.search.find_query();
        let with = self.search.replace.clone();
        let targets: Vec<(usize, FileMatches)> = match only {
            Some((f, _)) => self
                .search
                .results
                .get(f)
                .cloned()
                .map(|x| vec![(f, x)])
                .unwrap_or_default(),
            None => self.search.results.iter().cloned().enumerate().collect(),
        };
        let mut changed_docs = Vec::new();
        let mut closed = Vec::new();
        let mut names = Vec::new();
        for (_, f) in &targets {
            // An open document: one transaction (undoable).
            let doc = if f.path.as_os_str().is_empty() {
                self.workbench
                    .documents
                    .keys()
                    .copied()
                    .find(|d| self.document_title(*d) == f.rel)
            } else {
                self.workbench
                    .documents
                    .values()
                    .find(|d| d.path() == Some(f.path.as_path()))
                    .map(|d| d.id)
            };
            match doc {
                Some(doc) => {
                    let Some(d) = self.workbench.document(doc) else {
                        continue;
                    };
                    if d.is_read_only() {
                        continue;
                    }
                    let rope = d.buffer.rope().clone();
                    let changes = match only.and_then(|(_, m)| m) {
                        Some(mi) => {
                            let Some(lm) = f.matches.get(mi) else {
                                continue;
                            };
                            let start = rope.line_to_char(lm.line) + lm.start_col;
                            let end = rope.line_to_char(lm.line) + lm.end_col;
                            let text = find::replacement(&rope, &q, (start, end), &with)
                                .unwrap_or_else(|_| with.clone());
                            Some(notoir_editor::ChangeSet::replace(start, end, text.as_str()))
                        }
                        None => find::replace_all(&rope, &q, &with)
                            .ok()
                            .flatten()
                            .map(|(c, _)| c),
                    };
                    let Some(changes) = changes else { continue };
                    let sel = Selection::caret(0);
                    let now = self.now_ms;
                    let first_view = self.workbench.views.of_document(doc).next().map(|v| v.id);
                    let res = match first_view {
                        Some(v) => {
                            let sel = self
                                .workbench
                                .view(v)
                                .map(|v| v.selection.map(&changes))
                                .unwrap_or(sel);
                            self.workbench.edit(v, changes, sel, EditKind::Other, now)
                        }
                        None => self.workbench.apply_edit(
                            doc,
                            None,
                            changes,
                            sel,
                            EditKind::Other,
                            notoir_core::events::ChangeOrigin::Internal,
                            now,
                        ),
                    };
                    if res.is_ok() {
                        changed_docs.push(doc);
                        names.push(f.rel.clone());
                    }
                }
                None => closed.push(f.clone()),
            }
        }
        self.search.last_replace = changed_docs;
        // Closed files: load, replace, save with the conflict check, off the UI thread.
        let single = only.and_then(|(_, m)| m);
        if !closed.is_empty() {
            let q2 = q.clone();
            self.fs.submit(move || {
                let mut done = Vec::new();
                let mut failed = Vec::new();
                for f in closed {
                    let r = match single {
                        // One match of a closed file: replace the n-th occurrence only.
                        Some(mi) => replace_nth(&f, mi, &q2, &with),
                        None => search::replace_in_file(&f.path, &q2, &with),
                    };
                    match r {
                        Ok(n) if n > 0 => done.push(f.rel.clone()),
                        Ok(_) => {}
                        Err(e) => failed.push((f.rel.clone(), e)),
                    }
                }
                Some(IoResult::ReplaceDone(Box::new((done, failed))))
            });
        } else {
            self.replace_finished(names, Vec::new());
            return;
        }
        self.search.pending_names = names;
    }

    pub fn replace_finished(&mut self, mut names: Vec<String>, failed: Vec<(String, String)>) {
        names.extend(std::mem::take(&mut self.search.pending_names));
        let now = self.now_ms;
        if !names.is_empty() {
            let mut n = Notification::new(
                Severity::Info,
                Translatable::new("notifications:replacedInFiles").arg("count", names.len()),
            )
            .detail(names.join("\n"));
            if !self.search.last_replace.is_empty() {
                n = n.action(
                    "notifications:undo",
                    notoir_core::command::CommandInvocation::new(
                        "search.undoReplace",
                        notoir_core::command::InvocationSource::Internal,
                    ),
                );
            }
            self.notifications.notify(n, now);
        }
        for (name, e) in failed {
            self.notifications.notify(
                Notification::new(
                    Severity::Error,
                    Translatable::new("notifications:replaceFailed")
                        .arg("name", &name)
                        .arg("error", &e),
                ),
                now,
            );
        }
        self.start_search();
    }

    /// Undo of the last Replace All in the open documents it changed.
    pub fn undo_replace(&mut self) {
        for doc in std::mem::take(&mut self.search.last_replace) {
            let first_view = self.workbench.views.of_document(doc).next().map(|v| v.id);
            if let Some(v) = first_view {
                let _ = self.workbench.undo(v);
            }
        }
        self.start_search();
    }
}

/// Replaces only the `n`-th match of a closed file.
fn replace_nth(f: &FileMatches, n: usize, q: &FindQuery, with: &str) -> Result<usize, String> {
    use notoir_workspace::fs_io::{self, LoadOptions, SaveRequest};
    let lm = f.matches.get(n).ok_or("gone")?;
    let loaded = fs_io::load(&f.path, &LoadOptions::default()).map_err(|e| e.to_string())?;
    let mut rope = Rope::from_str(&loaded.text.text);
    if lm.line >= rope.len_lines() {
        return Ok(0);
    }
    let start = rope.line_to_char(lm.line) + lm.start_col;
    let end = rope.line_to_char(lm.line) + lm.end_col;
    let text = find::replacement(&rope, q, (start, end), with).map_err(|e| e.to_string())?;
    notoir_editor::ChangeSet::replace(start, end, text.as_str()).apply(&mut rope);
    fs_io::save(&SaveRequest {
        path: loaded.path.clone(),
        text: rope.to_string(),
        encoding: loaded.text.encoding,
        line_ending: loaded.text.line_ending,
        expected_hash: loaded.text.content_hash,
        overwrite: false,
        atomic: true,
    })
    .map_err(|e| e.to_string())?;
    Ok(1)
}
