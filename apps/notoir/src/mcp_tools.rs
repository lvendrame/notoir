//! MCP tool implementations (docs/specs/29 §Tool Catalogue): run on the UI
//! thread after the permission gate, through the same document, editor,
//! settings and file services as the UI. IO (loading, searching, Git diffs,
//! disk reads and writes) runs on worker threads; results come back as
//! [`Finished`] and either answer the call or continue it here.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use notoir_core::document::DocumentOrigin;
use notoir_core::events::ChangeOrigin;
use notoir_core::explorer::NodeKind;
use notoir_core::ids::{DocumentId, ViewId};
use notoir_core::permissions::Capability;
use notoir_core::settings::{SettingType, SettingValue};
use notoir_editor::{ChangeSet, Edit, EditKind, Rope, Selection};
use notoir_mcp::ToolReply;
use notoir_mcp::paths;
use notoir_mcp::tools::ToolDef;
use notoir_workspace::fs_io;
use serde_json::{Value, json};

use crate::core::AppCore;
use crate::files::OpenAt;
use crate::mcp::{Finished, PendingSave, Target, Waiting};

/// How long an MCP edit stays highlighted.
pub const HIGHLIGHT_MS: u64 = 1500;
/// Tabless documents loaded for MCP close after this long unused.
const LOADED_IDLE_MS: u64 = 5 * 60_000;
/// `git_diff` output cap.
const MAX_DIFF_BYTES: usize = 1024 * 1024;

/// IO done on a worker thread for a waiting call.
pub(crate) enum AfterIo {
    /// The document is loaded: open it in memory and run the call again.
    Loaded(Box<fs_io::Loaded>),
    Renamed {
        from: PathBuf,
        to: PathBuf,
    },
    Written(PathBuf),
    Created(PathBuf),
}

type IoJob = Box<dyn FnOnce() -> Result<AfterIo, ToolReply> + Send>;
type ReplyJob = Box<dyn FnOnce() -> ToolReply + Send>;

/// What a tool does next.
enum Step {
    Reply(ToolReply),
    /// Reply, with a suffix for the activity target (`notes.md (+3 −1)`).
    Edited(ToolReply, String),
    /// Load this file into memory, then run the call again.
    Load(PathBuf),
    /// Wait for this document's save.
    Save(DocumentId),
    /// IO off the UI thread, then continue with [`AfterIo`].
    Io(IoJob),
    /// Compute the reply off the UI thread.
    Job(ReplyJob),
}

/// Every capability a call needs: the tool's, plus editor control for
/// `document_open { show }` and outside access for paths outside the
/// workspace.
pub(crate) fn capabilities(def: &ToolDef, w: &Waiting) -> Vec<Capability> {
    let mut caps = def.capabilities.to_vec();
    if def.name == "document_open" && w.call.args.get("show").and_then(Value::as_bool) == Some(true)
    {
        caps.push(Capability::EditorControl);
    }
    if matches!(w.target, Target::Outside(_)) {
        caps.push(Capability::FilesystemOutsideWorkspace);
    }
    caps
}

fn err(code: &str, message: impl Into<String>) -> ToolReply {
    ToolReply::error(code, message)
}

fn ok(structured: Value, summary: impl Into<String>) -> ToolReply {
    ToolReply::Ok {
        structured,
        summary: summary.into(),
    }
}

fn arg_str<'a>(args: &'a Value, name: &str) -> Option<&'a str> {
    args.get(name).and_then(Value::as_str)
}

fn arg_u64(args: &Value, name: &str) -> Option<u64> {
    args.get(name).and_then(Value::as_u64)
}

fn arg_bool(args: &Value, name: &str) -> Option<bool> {
    args.get(name).and_then(Value::as_bool)
}

/// 1-based line and column of a char offset.
pub fn line_col(rope: &Rope, offset: usize) -> (usize, usize) {
    let offset = offset.min(rope.len_chars());
    let line = rope.char_to_line(offset);
    (line + 1, offset - rope.line_to_char(line) + 1)
}

/// Chars of `line` (0-based) without its line break.
fn line_len(rope: &Rope, line: usize) -> usize {
    let slice = rope.line(line);
    let mut n = slice.len_chars();
    while n > 0 && matches!(slice.char(n - 1), '\n' | '\r') {
        n -= 1;
    }
    n
}

/// A `{line, column}` (1-based) or `{offset}` position as a char offset.
pub(crate) fn position(rope: &Rope, v: Option<&Value>) -> Result<usize, ToolReply> {
    let v = v.ok_or_else(|| err("invalid_args", "a position is missing"))?;
    if let Some(o) = v.get("offset").and_then(Value::as_u64) {
        let o = o as usize;
        if o > rope.len_chars() {
            return Err(err(
                "invalid_position",
                format!("offset {o} is past the end"),
            ));
        }
        return Ok(o);
    }
    let line = v
        .get("line")
        .and_then(Value::as_u64)
        .ok_or_else(|| err("invalid_args", "a position needs line/column or offset"))?
        as usize;
    let column = v.get("column").and_then(Value::as_u64).unwrap_or(1) as usize;
    if line == 0 || column == 0 || line > rope.len_lines() {
        return Err(err(
            "invalid_position",
            format!("line {line}, column {column} is outside the document"),
        ));
    }
    let len = line_len(rope, line - 1);
    if column > len + 1 {
        return Err(err(
            "invalid_position",
            format!("line {line} has {len} characters; column {column} is past its end"),
        ));
    }
    Ok(rope.line_to_char(line - 1) + column - 1)
}

pub(crate) fn range_json(rope: &Rope, from: usize, to: usize) -> Value {
    let (sl, sc) = line_col(rope, from);
    let (el, ec) = line_col(rope, to);
    json!({
        "start": { "line": sl, "column": sc, "offset": from },
        "end": { "line": el, "column": ec, "offset": to },
    })
}

/// Content (optionally lines `start_line..=end_line`) plus metadata.
pub(crate) fn read_reply(text: &str, args: &Value, max_bytes: usize, mut meta: Value) -> ToolReply {
    let start = arg_u64(args, "start_line");
    let end = arg_u64(args, "end_line");
    let total = text.lines().count().max(1);
    let content = if start.is_some() || end.is_some() {
        let s = start.unwrap_or(1).max(1) as usize;
        let e = end.map_or(total, |e| e as usize).min(total);
        text.split_inclusive('\n')
            .skip(s - 1)
            .take(e.saturating_sub(s - 1))
            .collect::<String>()
    } else {
        text.to_owned()
    };
    if content.len() > max_bytes {
        return ToolReply::Err {
            code: "too_large".into(),
            message: format!(
                "The content is larger than {max_bytes} bytes; pass start_line/end_line"
            ),
            data: json!({ "bytes": content.len(), "lines": total }),
        };
    }
    let summary = format!("{} bytes, {} lines", content.len(), total);
    if let Some(m) = meta.as_object_mut() {
        m.insert("content".into(), Value::String(content));
        m.insert("lines".into(), json!(total));
    }
    ok(meta, summary)
}

/// Disk content, ignoring buffers.
fn read_from_disk(path: &Path, args: &Value, max_bytes: usize) -> ToolReply {
    let opts = fs_io::LoadOptions {
        allow_binary: false,
        ..Default::default()
    };
    match fs_io::load(path, &opts) {
        Ok(loaded) => read_reply(
            &loaded.text.text,
            args,
            max_bytes,
            json!({
                "encoding": loaded.text.encoding.label(),
                "eol": loaded.text.line_ending.label(),
                "bytes": loaded.size,
            }),
        ),
        Err(fs_io::LoadError::NotFound) => err("not_found", "No such file"),
        Err(e) => err("read_failed", e.to_string()),
    }
}

/// Unified diff of `old` → `new` with `a/` `b/` headers.
fn unified(rel: &str, old: &str, new: &str) -> String {
    similar::TextDiff::from_lines(old, new)
        .unified_diff()
        .context_radius(3)
        .header(&format!("a/{rel}"), &format!("b/{rel}"))
        .to_string()
}

/// `(+added −removed)` line counts of an edit list against `rope`.
fn line_delta(rope: &Rope, edits: &[Edit]) -> (usize, usize) {
    let mut added = 0;
    let mut removed = 0;
    for e in edits {
        if e.end > e.start {
            let first = rope.char_to_line(e.start);
            let last = rope.char_to_line(e.end);
            removed += last - first + 1;
        }
        if !e.text.is_empty() {
            added += e.text.matches('\n').count() + usize::from(!e.text.ends_with('\n'));
        }
    }
    (added, removed)
}

impl AppCore {
    fn mcp_max_read(&self) -> usize {
        self.settings.int("mcp.maxReadBytes", None).max(1024) as usize
    }

    /// The open (or MCP-loaded) document at `path`.
    fn mcp_doc(&mut self, path: &Path) -> Option<DocumentId> {
        let (_, key) = fs_io::identity(path);
        let doc = self.workbench.find_by_key(&key)?;
        let now = self.now_ms;
        if let Some(t) = self.mcp.loaded.get_mut(&doc) {
            *t = now;
        }
        Some(doc)
    }

    fn mcp_has_tab(&self, doc: DocumentId) -> bool {
        self.workbench.views.of_document(doc).any(|v| !v.detached)
    }

    /// Gives a tabless document a background tab (edits must be visible
    /// and undoable by the user).
    fn mcp_ensure_tab(&mut self, doc: DocumentId) {
        if self.mcp_has_tab(doc) {
            return;
        }
        let previous = self.workbench.active_view();
        let group = self.workbench.layout.active_group;
        if self
            .workbench
            .open_view(doc, group, notoir_core::TabPosition::End)
            .is_ok()
            && let Some(p) = previous
        {
            let _ = self.workbench.activate_view(p);
        }
        self.mcp.loaded.remove(&doc);
    }

    fn doc_meta(&self, doc: DocumentId) -> Value {
        let Some(d) = self.workbench.document(doc) else {
            return Value::Null;
        };
        let path = d
            .path()
            .map(|p| paths::display(self.workspace.as_ref().map(|w| w.root.as_path()), p));
        json!({
            "path": path,
            "title": self.document_title(doc),
            "revision": d.revision,
            "dirty": d.is_dirty(),
            "language": notoir_editor::language::definition(d.language).name,
            "encoding": d.encoding.label(),
            "eol": d.line_ending.label(),
            "lines": d.buffer.rope().len_lines(),
            "read_only": d.is_read_only(),
            "in_tab": self.mcp_has_tab(doc),
        })
    }

    /// Applies an MCP edit as one undoable transaction, highlighted.
    /// Returns the new revision and `(+added −removed)` lines.
    fn mcp_edit(
        &mut self,
        doc: DocumentId,
        view: Option<ViewId>,
        client: &notoir_core::permissions::ClientId,
        mut edits: Vec<Edit>,
        selection_after: Option<Selection>,
    ) -> Result<(u64, (usize, usize)), ToolReply> {
        let Some(d) = self.workbench.document(doc) else {
            return Err(err("not_open", "The document is not open"));
        };
        if d.is_read_only() {
            return Err(err("read_only", "The document is read-only"));
        }
        edits.sort_by_key(|e| (e.start, e.end));
        let delta = line_delta(d.buffer.rope(), &edits);
        // Highlight ranges in post-edit coordinates.
        let mut shift: isize = 0;
        let mut marks = Vec::new();
        for e in &edits {
            let start = (e.start as isize + shift) as usize;
            let len = e.text.chars().count();
            if len > 0 {
                marks.push((start, start + len));
            }
            shift += len as isize - (e.end - e.start) as isize;
        }
        let end = marks.last().map_or(0, |m| m.1);
        let changes = ChangeSet::new(edits).map_err(|e| err("invalid_edits", e.to_string()))?;
        self.mcp_ensure_tab(doc);
        let selection = selection_after.unwrap_or_else(|| Selection::caret(end));
        self.workbench
            .apply_edit(
                doc,
                view,
                changes,
                selection,
                EditKind::Other,
                ChangeOrigin::Mcp(client.clone()),
                self.now_ms,
            )
            .map_err(|e| err("edit_failed", e.to_string()))?;
        if self.settings.bool("mcp.highlightEdits", None) {
            let now = self.now_ms;
            self.mcp
                .highlights
                .extend(marks.into_iter().map(|(s, e)| (doc, s, e, now)));
        }
        let revision = self.workbench.document(doc).map_or(0, |d| d.revision);
        Ok((revision, delta))
    }

    fn check_revision(&self, doc: DocumentId, args: &Value) -> Result<(), ToolReply> {
        let Some(expected) = arg_u64(args, "expected_revision") else {
            return Ok(());
        };
        let current = self.workbench.document(doc).map_or(0, |d| d.revision);
        if expected == current {
            return Ok(());
        }
        Err(ToolReply::Err {
            code: "revision_conflict".into(),
            message: format!(
                "The document changed since revision {expected}; it is now at revision {current}. Read it again."
            ),
            data: json!({ "current_revision": current, "expected_revision": expected }),
        })
    }

    /// The editor a call targets: `view` argument, else the active editor.
    fn mcp_view(&self, args: &Value) -> Result<ViewId, ToolReply> {
        if let Some(v) = arg_u64(args, "view") {
            let id = ViewId(v);
            return self
                .workbench
                .view(id)
                .map(|_| id)
                .ok_or_else(|| err("no_such_view", format!("No view {v}")));
        }
        self.workbench
            .active_editor()
            .ok_or_else(|| err("no_active_editor", "No editor is active"))
    }

    // ── execution ────────────────────────────────────────────────────────

    pub(crate) fn mcp_run(&mut self, w: Waiting) {
        let step = self.mcp_tool(&w);
        let label = self.mcp_target_label(&w.target);
        match step {
            Step::Reply(reply) => self.mcp_reply(w.call, label, w.started, reply),
            Step::Edited(reply, suffix) => {
                let label = Some(match label {
                    Some(l) => format!("{l} {suffix}"),
                    None => suffix,
                });
                self.mcp_reply(w.call, label, w.started, reply);
            }
            Step::Load(path) => {
                if w.retried {
                    let reply = err("load_failed", "The document could not be opened");
                    self.mcp_reply(w.call, label, w.started, reply);
                    return;
                }
                let done = self.mcp.done_tx.clone();
                let wake = self.mcp.wake.clone();
                std::thread::spawn(move || {
                    let result = fs_io::load(&path, &fs_io::LoadOptions::default())
                        .map(|l| AfterIo::Loaded(Box::new(l)))
                        .map_err(|e| match e {
                            fs_io::LoadError::NotFound => err("not_found", "No such file"),
                            fs_io::LoadError::Binary => {
                                err("binary", "The file looks binary; use filesystem_read")
                            }
                            e => err("read_failed", e.to_string()),
                        });
                    if done.send(Finished::After { w, result }).is_ok() {
                        wake();
                    }
                });
            }
            Step::Save(doc) => self.mcp.saves.push(PendingSave {
                doc,
                call: w.call,
                target: label,
                started: w.started,
            }),
            Step::Io(job) => {
                let done = self.mcp.done_tx.clone();
                let wake = self.mcp.wake.clone();
                std::thread::spawn(move || {
                    let result = job();
                    if done.send(Finished::After { w, result }).is_ok() {
                        wake();
                    }
                });
            }
            Step::Job(job) => {
                let done = self.mcp.done_tx.clone();
                let wake = self.mcp.wake.clone();
                std::thread::spawn(move || {
                    let reply = job();
                    let f = Finished::Reply {
                        call: w.call,
                        target: label,
                        started: w.started,
                        reply,
                    };
                    if done.send(f).is_ok() {
                        wake();
                    }
                });
            }
        }
    }

    /// Continues a call after its IO.
    pub(crate) fn mcp_after(&mut self, mut w: Waiting, result: Result<AfterIo, ToolReply>) {
        let label = self.mcp_target_label(&w.target);
        let io = match result {
            Ok(io) => io,
            Err(reply) => {
                self.mcp_reply(w.call, label, w.started, reply);
                return;
            }
        };
        match io {
            AfterIo::Loaded(loaded) => {
                if self.workbench.find_by_key(&loaded.key).is_none() {
                    let origin = DocumentOrigin::File {
                        path: loaded.path.clone(),
                        key: loaded.key.clone(),
                    };
                    let doc = self.workbench.open_document(origin, loaded.text);
                    self.mcp.loaded.insert(doc, self.now_ms);
                }
                w.retried = true;
                self.mcp_run(w);
            }
            AfterIo::Renamed { from, to } => {
                self.handle_op(crate::workspace::OpOutcome {
                    renamed: vec![(from, to.clone())],
                    ..Default::default()
                });
                let shown = paths::display(self.workspace.as_ref().map(|w| w.root.as_path()), &to);
                let reply = ok(json!({ "path": shown }), format!("Renamed to {shown}"));
                self.mcp_reply(w.call, label, w.started, reply);
            }
            AfterIo::Written(path) => {
                let now = self.now_ms;
                self.mcp
                    .disk_writers
                    .insert(path.clone(), (w.call.client.clone(), now));
                // Open documents see it as an external change (spec 10).
                self.check_changed_paths(std::slice::from_ref(&path));
                let reply = ok(json!({ "written": true }), "Written to disk");
                self.mcp_reply(w.call, label, w.started, reply);
            }
            AfterIo::Created(path) => {
                self.open_path(
                    &path,
                    OpenAt {
                        keep_focus: true,
                        ..Default::default()
                    },
                );
                let shown =
                    paths::display(self.workspace.as_ref().map(|w| w.root.as_path()), &path);
                let reply = ok(
                    json!({ "path": shown, "created": true }),
                    format!("Created {shown}"),
                );
                self.mcp_reply(w.call, label, w.started, reply);
            }
        }
    }

    /// Highlighted MCP edits of `doc` with their remaining strength.
    pub fn mcp_highlights(&self, doc: DocumentId) -> Vec<(usize, usize, f32)> {
        let now = self.now_ms;
        self.mcp
            .highlights
            .iter()
            .filter(|h| h.0 == doc)
            .map(|h| {
                let age = now.saturating_sub(h.3) as f32 / HIGHLIGHT_MS as f32;
                (h.1, h.2, (1.0 - age).clamp(0.0, 1.0))
            })
            .collect()
    }

    /// Closes unused tabless documents and expired highlights.
    pub(crate) fn mcp_cleanup(&mut self) {
        let now = self.now_ms;
        self.mcp
            .highlights
            .retain(|h| now.saturating_sub(h.3) < HIGHLIGHT_MS);
        let idle: Vec<DocumentId> = self
            .mcp
            .loaded
            .iter()
            .filter(|(_, t)| now.saturating_sub(**t) >= LOADED_IDLE_MS)
            .map(|(d, _)| *d)
            .collect();
        for doc in idle {
            self.mcp.loaded.remove(&doc);
            if !self.mcp_has_tab(doc) {
                self.workbench.discard_unused_document(doc);
            }
        }
    }

    fn mcp_tool(&mut self, w: &Waiting) -> Step {
        let args = &w.call.args;
        let path = match &w.target {
            Target::Path(p) | Target::Outside(p) => Some(p.clone()),
            Target::None => None,
        };
        let need_path = || Step::Reply(err("invalid_args", "path is required"));
        match w.call.tool.as_str() {
            "workspace_get" => Step::Reply(self.tool_workspace_get()),
            "workspace_list_files" => self.tool_list_files(args),
            "workspace_search" => self.tool_search(args),
            "document_open" => match path {
                Some(p) => self.tool_document_open(&p, args),
                None => need_path(),
            },
            "document_read" => match path {
                Some(p) => match self.mcp_doc(&p) {
                    Some(doc) => {
                        let text = self
                            .workbench
                            .document(doc)
                            .map(|d| d.buffer.text())
                            .unwrap_or_default();
                        Step::Reply(read_reply(
                            &text,
                            args,
                            self.mcp_max_read(),
                            self.doc_meta(doc),
                        ))
                    }
                    None => Step::Load(p),
                },
                None => need_path(),
            },
            "document_create" => self.tool_document_create(path, args),
            "document_write" | "document_apply_edits" => match path {
                Some(p) => match self.mcp_doc(&p) {
                    Some(doc) => self.tool_document_edit(doc, w),
                    None => Step::Load(p),
                },
                None => need_path(),
            },
            "document_save" => match path {
                Some(p) => self.tool_document_save(&p),
                None => need_path(),
            },
            "document_rename" => match path {
                Some(p) => self.tool_document_rename(p, args),
                None => need_path(),
            },
            "document_delete" => match path {
                Some(p) => Step::Reply(self.tool_document_delete(p)),
                None => need_path(),
            },
            "editor_get_open_tabs" => Step::Reply(self.tool_open_tabs()),
            "editor_get_active_document" => Step::Reply(self.tool_active_document()),
            "editor_get_selection" => Step::Reply(self.tool_selection(args)),
            "editor_open_document" => match path {
                Some(p) => self.tool_editor_open(&p, args),
                None => need_path(),
            },
            "editor_select_range" => self.tool_select_range(path, args),
            "editor_insert" => self.tool_editor_text(w, false),
            "editor_replace" => self.tool_editor_text(w, true),
            "settings_get" => Step::Reply(self.tool_settings_get(args)),
            "settings_update" => Step::Reply(self.tool_settings_update(args)),
            "git_status" => Step::Reply(self.tool_git_status()),
            "git_diff" => self.tool_git_diff(path, args),
            "filesystem_read" => match path {
                Some(p) => {
                    let args = args.clone();
                    let max = self.mcp_max_read();
                    Step::Job(Box::new(move || read_from_disk(&p, &args, max)))
                }
                None => need_path(),
            },
            "filesystem_write" => match path {
                Some(p) => self.tool_fs_write(p, args),
                None => need_path(),
            },
            other => Step::Reply(err("unknown_tool", format!("No tool named {other}"))),
        }
    }

    // ── workspace ────────────────────────────────────────────────────────

    fn tool_workspace_get(&self) -> ToolReply {
        let open = self.workbench.documents.len();
        match &self.workspace {
            Some(ws) => ok(
                json!({
                    "root": ws.root.to_string_lossy(),
                    "name": ws.name,
                    "git": self.git.repo.is_some(),
                    "open_documents": open,
                }),
                format!("Workspace {} ({open} open documents)", ws.name),
            ),
            None => ok(
                json!({ "root": null, "open_documents": open }),
                format!("No workspace is open ({open} open documents)"),
            ),
        }
    }

    fn tool_list_files(&self, args: &Value) -> Step {
        let Some(ws) = &self.workspace else {
            return Step::Reply(err("no_workspace", "No workspace is open"));
        };
        let index = ws.index_ready.then(|| ws.index.clone());
        let filters = ws.filters.clone();
        let glob = arg_str(args, "glob").map(str::to_owned);
        let include_ignored = arg_bool(args, "include_ignored").unwrap_or(false);
        let cursor = arg_u64(args, "cursor").unwrap_or(0) as usize;
        let limit = arg_u64(args, "limit").unwrap_or(500).clamp(1, 2000) as usize;
        Step::Job(Box::new(move || {
            let entries = match index {
                Some(i) => i,
                None => {
                    let cancel = AtomicBool::new(false);
                    Arc::new(notoir_workspace::listing::build_index(&filters, &cancel).0)
                }
            };
            let matcher = glob.map(|g| notoir_workspace::search::glob_set(&[g]));
            let mut files: Vec<&str> = entries
                .iter()
                .filter(|e| e.kind == NodeKind::File && (include_ignored || !e.ignored))
                .filter(|e| matcher.as_ref().is_none_or(|m| m.is_match(&e.rel)))
                .map(|e| e.rel.as_str())
                .collect();
            files.sort_unstable();
            let total = files.len();
            let page: Vec<&str> = files.iter().skip(cursor).take(limit).copied().collect();
            let next = (cursor + page.len() < total).then_some(cursor + page.len());
            let n = page.len();
            ok(
                json!({ "files": page, "total": total, "next_cursor": next }),
                format!("{n} of {total} files"),
            )
        }))
    }

    fn tool_search(&self, args: &Value) -> Step {
        let Some(text) = arg_str(args, "query").filter(|q| !q.is_empty()) else {
            return Step::Reply(err("invalid_args", "query is required"));
        };
        let strings = |name: &str| -> Vec<String> {
            args.get(name)
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default()
        };
        let cursor = arg_u64(args, "cursor").unwrap_or(0) as usize;
        let limit = arg_u64(args, "limit").unwrap_or(200).clamp(1, 1000) as usize;
        let mut exclude = self.settings.str_list("files.exclude", None);
        exclude.extend(self.settings.str_list("search.exclude", None));
        exclude.extend(strings("exclude"));
        let opts = notoir_workspace::search::SearchOptions {
            root: self.workspace.as_ref().map(|w| w.root.clone()),
            query: notoir_editor::find::FindQuery {
                text: text.to_owned(),
                case_sensitive: arg_bool(args, "case_sensitive").unwrap_or(false),
                whole_word: arg_bool(args, "whole_word").unwrap_or(false),
                regex: arg_bool(args, "regex").unwrap_or(false),
                within: None,
            },
            include: strings("include"),
            exclude,
            max_file_size: self.settings.int("search.maxFileSize", None).max(0) as u64,
            max_results: (cursor + limit + 1).min(20_000),
        };
        // Open documents are searched as held in memory.
        let mut snapshots: HashMap<PathBuf, Rope> = HashMap::new();
        for d in self.workbench.documents.values() {
            if let DocumentOrigin::File { path, .. } = &d.origin {
                snapshots.insert(path.clone(), d.buffer.rope().clone());
            }
        }
        Step::Job(Box::new(move || {
            let cancel = AtomicBool::new(false);
            let mut files = Vec::new();
            let mut error = None;
            notoir_workspace::search::run(&opts, &snapshots, &[], &cancel, |batch| {
                files.extend(batch.files);
                if batch.error.is_some() {
                    error = batch.error;
                }
            });
            if let Some(e) = error {
                return err("invalid_query", e);
            }
            let all: Vec<Value> = files
                .iter()
                .flat_map(|f| {
                    f.matches.iter().map(move |m| {
                        json!({
                            "path": f.rel,
                            "line": m.line + 1,
                            "column": m.start_col + 1,
                            "end_column": m.end_col + 1,
                            "preview": m.preview,
                        })
                    })
                })
                .collect();
            let total = all.len();
            let page: Vec<Value> = all.into_iter().skip(cursor).take(limit).collect();
            let n = page.len();
            let next = (cursor + n < total).then_some(cursor + n);
            ok(
                json!({ "matches": page, "next_cursor": next }),
                format!("{n} matches"),
            )
        }))
    }

    // ── documents ────────────────────────────────────────────────────────

    fn tool_document_open(&mut self, path: &Path, args: &Value) -> Step {
        let Some(doc) = self.mcp_doc(path) else {
            return Step::Load(path.to_path_buf());
        };
        if arg_bool(args, "show") == Some(true) {
            self.mcp.loaded.remove(&doc);
            self.open_path(
                path,
                OpenAt {
                    keep_focus: true,
                    ..Default::default()
                },
            );
        }
        Step::Reply(ok(self.doc_meta(doc), "Open in Notoir"))
    }

    fn tool_document_create(&mut self, path: Option<PathBuf>, args: &Value) -> Step {
        let content = arg_str(args, "content").unwrap_or("").to_owned();
        if arg_bool(args, "scratchpad") == Some(true) {
            self.new_scratchpad_with_text(&content);
            let doc = self.workbench.active_document();
            let meta = doc.map(|d| self.doc_meta(d)).unwrap_or(Value::Null);
            return Step::Reply(ok(
                json!({ "scratchpad": true, "document": meta }),
                "Scratchpad created",
            ));
        }
        let Some(path) = path else {
            return Step::Reply(err(
                "invalid_args",
                "path is required (or scratchpad: true)",
            ));
        };
        if path.exists() {
            return Step::Reply(err("exists", "A file with that name already exists"));
        }
        Step::Io(Box::new(move || {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| err("write_failed", e.to_string()))?;
            }
            notoir_storage::atomic::write(&path, content.as_bytes(), Default::default())
                .map_err(|e| err("write_failed", e.to_string()))?;
            Ok(AfterIo::Created(path))
        }))
    }

    fn tool_document_edit(&mut self, doc: DocumentId, w: &Waiting) -> Step {
        let args = &w.call.args;
        if let Err(r) = self.check_revision(doc, args) {
            return Step::Reply(r);
        }
        let Some(rope) = self
            .workbench
            .document(doc)
            .map(|d| d.buffer.rope().clone())
        else {
            return Step::Reply(err("not_open", "The document is not open"));
        };
        let edits = if w.call.tool == "document_write" {
            let Some(content) = arg_str(args, "content") else {
                return Step::Reply(err("invalid_args", "content is required"));
            };
            // The smallest single replacement: common prefix and suffix kept.
            let old: Vec<char> = rope.chars().collect();
            let new: Vec<char> = content.chars().collect();
            let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
            let suffix = old[prefix..]
                .iter()
                .rev()
                .zip(new[prefix..].iter().rev())
                .take_while(|(a, b)| a == b)
                .count();
            if prefix == old.len() && prefix == new.len() {
                Vec::new()
            } else {
                let text: String = new[prefix..new.len() - suffix].iter().collect();
                vec![Edit::new(prefix, old.len() - suffix, text)]
            }
        } else {
            let Some(list) = args.get("edits").and_then(Value::as_array) else {
                return Step::Reply(err("invalid_args", "edits must be an array"));
            };
            let mut edits = Vec::new();
            for e in list {
                let start = match position(&rope, e.get("start")) {
                    Ok(p) => p,
                    Err(r) => return Step::Reply(r),
                };
                let end = match position(&rope, e.get("end")) {
                    Ok(p) => p,
                    Err(r) => return Step::Reply(r),
                };
                let text = e.get("text").and_then(Value::as_str).unwrap_or("");
                edits.push(Edit::new(start, end, text));
            }
            edits
        };
        if edits.is_empty() {
            let rev = self.workbench.document(doc).map_or(0, |d| d.revision);
            return Step::Reply(ok(
                json!({ "revision": rev, "changed": false }),
                "No change",
            ));
        }
        match self.mcp_edit(doc, None, &w.call.client, edits, None) {
            Ok((revision, (added, removed))) => Step::Edited(
                ok(
                    json!({
                        "revision": revision,
                        "dirty": true,
                        "lines_added": added,
                        "lines_removed": removed,
                    }),
                    format!("Edited (+{added} −{removed}); not saved"),
                ),
                format!("(+{added} −{removed})"),
            ),
            Err(r) => Step::Reply(r),
        }
    }

    fn tool_document_save(&mut self, path: &Path) -> Step {
        let Some(doc) = self.mcp_doc(path) else {
            return Step::Reply(err(
                "not_open",
                "The document is not open in Notoir; nothing to save",
            ));
        };
        // A save in flight (or nothing to save): wait for / report that.
        let clean = self.workbench.document(doc).is_some_and(|d| !d.is_dirty());
        if !self.saving.contains(&doc) && !clean {
            self.save(doc);
        }
        Step::Save(doc)
    }

    fn tool_document_rename(&mut self, from: PathBuf, args: &Value) -> Step {
        let Some(ws) = &self.workspace else {
            return Step::Reply(err("no_workspace", "No workspace is open"));
        };
        let Some(new_path) = arg_str(args, "new_path") else {
            return Step::Reply(err("invalid_args", "new_path is required"));
        };
        let to = match paths::resolve(&ws.root, new_path) {
            Ok(p) => p,
            Err(e) => {
                return Step::Reply(err(e.code(), "The new path is not inside the workspace"));
            }
        };
        if !from.exists() {
            return Step::Reply(err("not_found", "No such file"));
        }
        if to.exists() {
            return Step::Reply(err("exists", "The destination already exists"));
        }
        Step::Io(Box::new(move || {
            if let Some(dir) = to.parent() {
                std::fs::create_dir_all(dir).map_err(|e| err("rename_failed", e.to_string()))?;
            }
            notoir_workspace::fs_ops::rename(&from, &to)
                .map_err(|e| err("rename_failed", e.to_string()))?;
            Ok(AfterIo::Renamed { from, to })
        }))
    }

    fn tool_document_delete(&mut self, path: PathBuf) -> ToolReply {
        if !path.exists() {
            return err("not_found", "No such file");
        }
        if path.is_dir() {
            return err("is_directory", "document_delete removes files only");
        }
        let shown = paths::display(self.workspace.as_ref().map(|w| w.root.as_path()), &path);
        self.delete_paths(vec![path], false);
        ok(
            json!({ "path": shown, "trashed": true }),
            format!("Moved {shown} to the trash"),
        )
    }

    // ── editor ───────────────────────────────────────────────────────────

    fn tool_open_tabs(&self) -> ToolReply {
        let wb = &self.workbench;
        let active = wb.active_view();
        let groups: Vec<Value> = wb
            .layout
            .groups_in_order()
            .into_iter()
            .filter_map(|g| wb.layout.group(g))
            .map(|g| {
                let tabs: Vec<Value> = g
                    .tabs
                    .iter()
                    .filter_map(|v| wb.view(*v))
                    .map(|v| {
                        let d = wb.document(v.document_id);
                        json!({
                            "view": v.id.0,
                            "path": d.and_then(|d| d.path()).map(|p| paths::display(
                                self.workspace.as_ref().map(|w| w.root.as_path()), p)),
                            "title": self.document_title(v.document_id),
                            "dirty": d.is_some_and(|d| d.is_dirty()),
                            "pinned": v.pinned,
                            "active": g.active == Some(v.id),
                        })
                    })
                    .collect();
                json!({ "group": g.id.0, "active": g.id == wb.layout.active_group, "tabs": tabs })
            })
            .collect();
        let n: usize = groups
            .iter()
            .map(|g| g["tabs"].as_array().map_or(0, Vec::len))
            .sum();
        ok(
            json!({ "groups": groups, "active_view": active.map(|v| v.0) }),
            format!("{n} tabs"),
        )
    }

    fn selections_json(&self, view: ViewId, with_text: bool) -> Vec<Value> {
        let Some(v) = self.workbench.view(view) else {
            return Vec::new();
        };
        let Some(d) = self.workbench.document(v.document_id) else {
            return Vec::new();
        };
        let rope = d.buffer.rope();
        v.selection
            .ranges()
            .iter()
            .map(|r| {
                let (from, to) = (r.from().min(rope.len_chars()), r.to().min(rope.len_chars()));
                let mut j = range_json(rope, from, to);
                j["empty"] = json!(from == to);
                j["reversed"] = json!(r.head < r.anchor);
                if with_text {
                    j["text"] = json!(rope.slice(from..to).to_string());
                }
                j
            })
            .collect()
    }

    fn tool_active_document(&self) -> ToolReply {
        let Some(view) = self.workbench.active_editor() else {
            return ok(json!({ "document": null }), "No editor is active");
        };
        let doc = self.workbench.view(view).map(|v| v.document_id);
        let mut meta = doc.map(|d| self.doc_meta(d)).unwrap_or(Value::Null);
        meta["view"] = json!(view.0);
        meta["selections"] = json!(self.selections_json(view, false));
        let title = doc.map(|d| self.document_title(d)).unwrap_or_default();
        ok(json!({ "document": meta }), format!("Active: {title}"))
    }

    /// `editor_get_selection`: the selected text, its file and range(s).
    fn tool_selection(&self, args: &Value) -> ToolReply {
        let view = match self.mcp_view(args) {
            Ok(v) => v,
            Err(r) => return r,
        };
        let Some(v) = self.workbench.view(view) else {
            return err("no_such_view", "The view is gone");
        };
        let doc = v.document_id;
        let primary = v.selection.primary_index();
        let selections = self.selections_json(view, true);
        let main = selections.get(primary).cloned().unwrap_or(Value::Null);
        let path = self
            .workbench
            .document(doc)
            .and_then(|d| d.path())
            .map(|p| paths::display(self.workspace.as_ref().map(|w| w.root.as_path()), p));
        let text = main["text"].as_str().unwrap_or("").to_owned();
        let summary = if text.is_empty() {
            "Nothing selected".to_owned()
        } else {
            format!("{} characters selected", text.chars().count())
        };
        ok(
            json!({
                "view": view.0,
                "path": path,
                "title": self.document_title(doc),
                "revision": self.workbench.document(doc).map_or(0, |d| d.revision),
                "text": text,
                "range": { "start": main["start"], "end": main["end"] },
                "selections": selections,
            }),
            summary,
        )
    }

    fn tool_editor_open(&mut self, path: &Path, args: &Value) -> Step {
        let Some(doc) = self.mcp_doc(path) else {
            return Step::Load(path.to_path_buf());
        };
        self.mcp.loaded.remove(&doc);
        let line_col = arg_u64(args, "line").map(|l| {
            (
                l.max(1) as usize,
                arg_u64(args, "column").unwrap_or(1).max(1) as usize,
            )
        });
        self.open_path(
            path,
            OpenAt {
                line_col,
                keep_focus: true,
                ..Default::default()
            },
        );
        let mut meta = self.doc_meta(doc);
        meta["view"] = json!(self.workbench.active_editor().map(|v| v.0));
        Step::Reply(ok(meta, "Opened in an editor"))
    }

    fn tool_select_range(&mut self, path: Option<PathBuf>, args: &Value) -> Step {
        let view = match path {
            Some(p) => {
                let Some(doc) = self.mcp_doc(&p) else {
                    return Step::Reply(err(
                        "not_open",
                        "Open the document first (editor_open_document)",
                    ));
                };
                let wb = &self.workbench;
                let in_active = wb.view_in_group(doc, wb.layout.active_group);
                match in_active.or_else(|| {
                    wb.views
                        .of_document(doc)
                        .find(|v| !v.detached)
                        .map(|v| v.id)
                }) {
                    Some(v) => v,
                    None => {
                        return Step::Reply(err(
                            "not_in_editor",
                            "The document has no editor tab (editor_open_document)",
                        ));
                    }
                }
            }
            None => match self.mcp_view(&json!({})) {
                Ok(v) => v,
                Err(r) => return Step::Reply(r),
            },
        };
        let Some(rope) = self
            .workbench
            .view(view)
            .and_then(|v| self.workbench.document(v.document_id))
            .map(|d| d.buffer.rope().clone())
        else {
            return Step::Reply(err("no_such_view", "The view is gone"));
        };
        let (start, end) = match (
            position(&rope, args.get("start")),
            position(&rope, args.get("end")),
        ) {
            (Ok(s), Ok(e)) => (s, e),
            (Err(r), _) | (_, Err(r)) => return Step::Reply(r),
        };
        let _ = self.workbench.activate_view(view);
        let _ = self.workbench.set_selection(
            view,
            Selection::single(notoir_editor::selection::Range::new(start, end)),
        );
        self.find.center.push(view);
        Step::Reply(ok(
            json!({ "view": view.0, "range": range_json(&rope, start.min(end), start.max(end)) }),
            "Selected",
        ))
    }

    fn tool_editor_text(&mut self, w: &Waiting, replace: bool) -> Step {
        let Some(text) = arg_str(&w.call.args, "text") else {
            return Step::Reply(err("invalid_args", "text is required"));
        };
        let view = match self.mcp_view(&json!({})) {
            Ok(v) => v,
            Err(r) => return Step::Reply(r),
        };
        let Some(v) = self.workbench.view(view) else {
            return Step::Reply(err("no_such_view", "The view is gone"));
        };
        let doc = v.document_id;
        let ranges: Vec<(usize, usize)> = v
            .selection
            .ranges()
            .iter()
            .map(|r| {
                if replace {
                    (r.from(), r.to())
                } else {
                    (r.head, r.head)
                }
            })
            .collect();
        let primary = v.selection.primary_index();
        let edits: Vec<Edit> = ranges
            .iter()
            .map(|(s, e)| Edit::new(*s, *e, text))
            .collect();
        // Carets end after each inserted text.
        let len = text.chars().count();
        let mut shift: isize = 0;
        let mut sorted = ranges.clone();
        sorted.sort();
        let mut carets = Vec::new();
        for (s, e) in &sorted {
            let at = (*s as isize + shift) as usize + len;
            carets.push(notoir_editor::selection::Range::caret(at));
            shift += len as isize - (*e - *s) as isize;
        }
        let selection = Selection::new(carets, primary.min(sorted.len().saturating_sub(1)));
        match self.mcp_edit(doc, Some(view), &w.call.client, edits, Some(selection)) {
            Ok((revision, (added, removed))) => Step::Edited(
                ok(
                    json!({ "view": view.0, "revision": revision, "cursors": ranges.len() }),
                    if replace {
                        "Replaced the selection"
                    } else {
                        "Inserted"
                    },
                ),
                format!("(+{added} −{removed})"),
            ),
            Err(r) => Step::Reply(r),
        }
    }

    // ── settings ─────────────────────────────────────────────────────────

    fn tool_settings_get(&self, args: &Value) -> ToolReply {
        let prefix = arg_str(args, "prefix").unwrap_or("");
        let mut out = serde_json::Map::new();
        for def in self.schema.iter() {
            // Machine paths stay private.
            let k = def.key.to_ascii_lowercase();
            if !def.key.starts_with(prefix) || k.contains("path") || k.contains("directory") {
                continue;
            }
            if let Some(v) = self.settings.get(def.key, None) {
                out.insert(def.key.to_owned(), setting_json(v));
            }
        }
        let n = out.len();
        ok(Value::Object(out), format!("{n} settings"))
    }

    fn tool_settings_update(&mut self, args: &Value) -> ToolReply {
        let Some(key) = arg_str(args, "key") else {
            return err("invalid_args", "key is required");
        };
        let Some(def) = self.schema.get(key) else {
            return err("unknown_setting", format!("No setting {key}"));
        };
        if def.restricted {
            return err(
                "restricted_setting",
                format!("{key} is restricted and can never be changed by MCP"),
            );
        }
        let workspace = arg_str(args, "scope") == Some("workspace");
        if workspace && (!def.scopes.workspace || self.workspace.is_none()) {
            return err(
                "invalid_scope",
                format!("{key} cannot be set for the workspace"),
            );
        }
        let value = match args.get("value") {
            None | Some(Value::Null) => None,
            Some(v) => match setting_value(&def.ty, v) {
                Some(sv) if def.ty.accepts(&sv) => Some(sv),
                _ => return err("invalid_value", format!("Invalid value for {key}")),
            },
        };
        let key = def.key;
        if workspace {
            self.set_workspace_setting(key, value);
        } else {
            self.set_user_setting(key, value);
        }
        let now = self
            .settings
            .get(key, None)
            .map(setting_json)
            .unwrap_or(Value::Null);
        ok(
            json!({ "key": key, "value": now }),
            format!("{key} updated"),
        )
    }

    // ── git ──────────────────────────────────────────────────────────────

    fn tool_git_status(&self) -> ToolReply {
        let Some(repo) = &self.git.repo else {
            return ok(json!({ "repository": false }), "Not a Git repository");
        };
        let s = &self.git.status;
        let list = |m: &std::collections::BTreeMap<String, notoir_git::FileStatus>| -> Vec<Value> {
            m.iter()
                .map(|(p, st)| json!({ "path": p, "status": format!("{st:?}").to_lowercase() }))
                .collect()
        };
        let conflicts: Vec<&String> = s.conflicts().collect();
        let changes: Vec<Value> = s
            .changes()
            .map(|(p, st)| json!({ "path": p, "status": format!("{st:?}").to_lowercase() }))
            .collect();
        let branch = s.head.branch.clone();
        ok(
            json!({
                "repository": true,
                "root": repo.workdir.to_string_lossy(),
                "branch": branch,
                "commit": s.head.short_sha,
                "upstream": s.head.upstream,
                "ahead": s.head.ahead,
                "behind": s.head.behind,
                "staged": list(&s.staged),
                "changes": changes,
                "conflicts": conflicts,
                "truncated": s.truncated,
            }),
            format!(
                "{} · {} changed",
                branch.as_deref().unwrap_or("detached"),
                s.files.len()
            ),
        )
    }

    fn tool_git_diff(&self, path: Option<PathBuf>, args: &Value) -> Step {
        let Some(repo) = self.git.repo.clone() else {
            return Step::Reply(err(
                "no_repository",
                "The workspace is not a Git repository",
            ));
        };
        let base = match arg_str(args, "base") {
            Some("head") => notoir_git::DiffBase::Head,
            _ => notoir_git::DiffBase::Index,
        };
        let rels: Vec<String> = match &path {
            Some(p) => match repo.relative(p) {
                Some(r) => vec![r],
                None => {
                    return Step::Reply(err(
                        "not_in_repository",
                        "The file is not in the repository",
                    ));
                }
            },
            None => self
                .git
                .status
                .files
                .iter()
                .filter(|(_, s)| **s != notoir_git::FileStatus::Ignored)
                .map(|(p, _)| p.clone())
                .collect(),
        };
        let backend = self.git.backend.clone();
        Step::Job(Box::new(move || {
            let mut out = String::new();
            let mut truncated = false;
            for rel in &rels {
                let old = match backend.read_base(&repo, rel, base) {
                    Ok(b) => b.map(|b| String::from_utf8_lossy(&b).into_owned()),
                    Err(e) => return err("git_failed", e.to_string()),
                };
                let new = std::fs::read(repo.absolute(rel))
                    .ok()
                    .map(|b| String::from_utf8_lossy(&b).into_owned());
                let d = unified(
                    rel,
                    old.as_deref().unwrap_or(""),
                    new.as_deref().unwrap_or(""),
                );
                if out.len() + d.len() > MAX_DIFF_BYTES {
                    truncated = true;
                    break;
                }
                out.push_str(&d);
            }
            let n = rels.len();
            ok(
                json!({ "diff": out, "files": rels, "truncated": truncated }),
                format!("Diff of {n} files"),
            )
        }))
    }

    // ── filesystem ───────────────────────────────────────────────────────

    fn tool_fs_write(&mut self, path: PathBuf, args: &Value) -> Step {
        let Some(content) = arg_str(args, "content").map(str::to_owned) else {
            return Step::Reply(err("invalid_args", "content is required"));
        };
        let create = arg_bool(args, "create").unwrap_or(true);
        Step::Io(Box::new(move || {
            if path.is_dir() {
                return Err(err("is_directory", "The path is a directory"));
            }
            if !path.exists() {
                if !create {
                    return Err(err("not_found", "No such file (pass create: true)"));
                }
                if let Some(dir) = path.parent() {
                    std::fs::create_dir_all(dir).map_err(|e| err("write_failed", e.to_string()))?;
                }
            }
            notoir_storage::atomic::write(&path, content.as_bytes(), Default::default())
                .map_err(|e| err("write_failed", e.to_string()))?;
            Ok(AfterIo::Written(path))
        }))
    }
}

fn setting_json(v: &SettingValue) -> Value {
    match v {
        SettingValue::Bool(b) => json!(b),
        SettingValue::Int(i) => json!(i),
        SettingValue::Float(f) => json!(f),
        SettingValue::Str(s) => json!(s),
        SettingValue::List(l) => json!(l),
    }
}

fn setting_value(ty: &SettingType, v: &Value) -> Option<SettingValue> {
    Some(match ty {
        SettingType::Bool => SettingValue::Bool(v.as_bool()?),
        SettingType::Int { .. } => SettingValue::Int(v.as_i64()?),
        SettingType::Float { .. } => SettingValue::Float(v.as_f64()?),
        SettingType::Enum(_) | SettingType::Str => SettingValue::Str(v.as_str()?.to_owned()),
        SettingType::StrList => SettingValue::List(
            v.as_array()?
                .iter()
                .map(|s| s.as_str().map(str::to_owned))
                .collect::<Option<Vec<_>>>()?,
        ),
    })
}
