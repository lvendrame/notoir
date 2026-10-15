//! Opening, saving, reverting and closing documents; Scratchpads; recent
//! files (docs/specs/02, 04, 08, 09).
//!
//! Disk work runs on the IO queue; dialogs report through the [`Poster`].
//! Results come back to the UI thread as [`IoResult`]s and are applied here.

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use notoir_core::command::{CommandInvocation, InvocationSource};
use notoir_core::document::{DiskState, DocumentOrigin, LoadedText};
use notoir_core::events::ChangeOrigin;
use notoir_core::ids::{DocumentId, GroupId, ViewId};
use notoir_core::notifications::{Notification, NotificationSource, Severity, Translatable};
use notoir_core::view::ScrollPosition;
use notoir_core::workbench::{CloseOutcome, TabPosition};
use notoir_editor::encoding::TextEncoding;
use notoir_editor::{ChangeSet, EditKind, LanguageId, Selection};
use notoir_platform::dialogs::FileDialogOptions;
use notoir_storage::scratchpad::ScratchpadStore;
use notoir_storage::state_db::{RecentEntry, RecentKind, ScratchpadRow};
use notoir_workspace::fs_io::{
    self, LoadError, LoadOptions, Loaded, SaveError, SaveRequest, Saved,
};
use serde_json::json;

use crate::core::{AppCore, Effect};
use crate::io::{IoResult, Poster};
use crate::t;

// ── dialogs ──────────────────────────────────────────────────────────────

/// File dialogs, swappable for tests.
pub trait Dialogs: Send + Sync {
    fn pick_files(&self, opts: FileDialogOptions, poster: Poster);
    fn save_file(&self, opts: FileDialogOptions, doc: DocumentId, poster: Poster);
    fn pick_folder(&self, opts: FileDialogOptions, poster: Poster);
    /// Hands a URL to the OS (links). Scripted in tests.
    fn open_external(&self, url: &str);
    /// Moves paths to the OS trash (called on a worker thread).
    fn trash(&self, paths: &[PathBuf]) -> Result<(), String>;
}

pub struct NativeDialogs;

impl Dialogs for NativeDialogs {
    fn pick_files(&self, opts: FileDialogOptions, poster: Poster) {
        notoir_platform::dialogs::pick_files(opts, move |files| {
            poster.post(IoResult::FilesPicked(files))
        });
    }

    fn save_file(&self, opts: FileDialogOptions, doc: DocumentId, poster: Poster) {
        notoir_platform::dialogs::save_file(opts, move |path| {
            poster.post(IoResult::SaveAsPicked { doc, path })
        });
    }

    fn pick_folder(&self, opts: FileDialogOptions, poster: Poster) {
        notoir_platform::dialogs::pick_folder(opts, move |path| {
            poster.post(IoResult::FolderPicked(path))
        });
    }

    fn trash(&self, paths: &[PathBuf]) -> Result<(), String> {
        notoir_platform::trash::move_to_trash(paths)
    }

    fn open_external(&self, url: &str) {
        if let Err(e) = notoir_platform::reveal::open_external(url) {
            tracing::warn!(error = %e, "could not open link");
        }
    }
}

/// Answers dialogs from a queue (tests and automation). An empty queue
/// behaves like Cancel.
#[derive(Clone, Default)]
pub struct ScriptedDialogs {
    opens: Arc<Mutex<VecDeque<Vec<PathBuf>>>>,
    saves: Arc<Mutex<VecDeque<Option<PathBuf>>>>,
    folders: Arc<Mutex<VecDeque<Option<PathBuf>>>>,
    /// Paths "moved to the trash" (deleted from disk, recorded here).
    pub trashed: Arc<Mutex<Vec<PathBuf>>>,
    /// Make the next trash calls fail (permanent-delete fallback).
    pub trash_fails: Arc<std::sync::atomic::AtomicBool>,
    /// URLs handed to the OS.
    pub opened: Arc<Mutex<Vec<String>>>,
}

impl ScriptedDialogs {
    pub fn answer_open(&self, files: Vec<PathBuf>) {
        self.opens
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push_back(files);
    }

    pub fn answer_save(&self, path: Option<PathBuf>) {
        self.saves
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push_back(path);
    }

    pub fn answer_folder(&self, path: Option<PathBuf>) {
        self.folders
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push_back(path);
    }

    pub fn trashed(&self) -> Vec<PathBuf> {
        self.trashed
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

impl Dialogs for ScriptedDialogs {
    fn pick_files(&self, _: FileDialogOptions, poster: Poster) {
        let files = self
            .opens
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pop_front()
            .unwrap_or_default();
        poster.post(IoResult::FilesPicked(files));
    }

    fn save_file(&self, _: FileDialogOptions, doc: DocumentId, poster: Poster) {
        let path = self
            .saves
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pop_front()
            .flatten();
        poster.post(IoResult::SaveAsPicked { doc, path });
    }

    fn pick_folder(&self, _: FileDialogOptions, poster: Poster) {
        let path = self
            .folders
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pop_front()
            .flatten();
        poster.post(IoResult::FolderPicked(path));
    }

    fn open_external(&self, url: &str) {
        self.opened
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(url.to_owned());
    }

    fn trash(&self, paths: &[PathBuf]) -> Result<(), String> {
        if self.trash_fails.load(std::sync::atomic::Ordering::SeqCst) {
            return Err("trash unavailable".into());
        }
        for p in paths {
            let r = if p.is_dir() {
                std::fs::remove_dir_all(p)
            } else {
                std::fs::remove_file(p)
            };
            r.map_err(|e| e.to_string())?;
            self.trashed
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(p.clone());
        }
        Ok(())
    }
}

// ── requests and outcomes ────────────────────────────────────────────────

/// Where to put the caret once a file opens.
#[derive(Clone, Debug, Default)]
pub struct OpenAt {
    /// 1-based line and column.
    pub line_col: Option<(usize, usize)>,
    pub restore: Option<(Selection, ScrollPosition)>,
    /// Leave keyboard focus where it is (single click in the Explorer).
    pub keep_focus: bool,
    /// Markdown heading slug to scroll to (`b.md#setup`).
    pub anchor: Option<String>,
    /// With `line_col`: select this many chars from there (search results).
    pub select_chars: usize,
    /// Open in the group beside the active one (`Mod+Enter`).
    pub side: bool,
}

#[derive(Clone, Debug)]
pub enum LoadPurpose {
    Open {
        group: GroupId,
        at: OpenAt,
        allow_binary: bool,
    },
    Revert(DocumentId),
    Reencode(DocumentId),
    /// Session restore: a saved tab (spec 23).
    Restore {
        group: GroupId,
        key: String,
        tab: Box<crate::session::TabData>,
    },
    /// A Compare side (spec 27): the file itself, or the saved version of
    /// the open document `saved_of`.
    Compare {
        id: u64,
        left: bool,
        saved_of: Option<DocumentId>,
    },
}

#[derive(Debug)]
pub struct LoadOutcome {
    pub path: PathBuf,
    pub purpose: LoadPurpose,
    pub result: Result<Loaded, LoadError>,
}

#[derive(Debug)]
pub struct SaveOutcome {
    pub doc: DocumentId,
    /// History state that was written.
    pub state: u64,
    pub path: PathBuf,
    pub save_as: bool,
    pub encoding: TextEncoding,
    pub result: Result<Saved, SaveError>,
}

// ── prompts ──────────────────────────────────────────────────────────────

/// What happens once the documents a prompt is about are saved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Then {
    CloseViews(Vec<ViewId>),
    Quit,
    /// Close Workspace (then optionally open another one).
    CloseWorkspace {
        views: Vec<ViewId>,
        next: Option<PathBuf>,
    },
    /// Switch Branch after saving.
    GitCheckout(String),
    /// Sync Notes after saving.
    GitSyncNotes,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Prompt {
    /// Save / Don't Save / Cancel for one or more documents.
    Unsaved { docs: Vec<DocumentId>, then: Then },
    /// Keep Scratchpad / Save As… / Delete / Cancel.
    ScratchpadClose { view: ViewId, doc: DocumentId },
    /// File changed on disk since it was loaded: Overwrite / Cancel.
    SaveConflict { doc: DocumentId },
    /// Discard unsaved changes and reload: Revert / Cancel.
    Revert { doc: DocumentId },
    /// Binary-looking file: Open Anyway (read-only) / Cancel.
    Binary { path: PathBuf, group: GroupId },
    /// A workspace is open: Open in This Window / New Window / Cancel.
    WorkspaceReplace { path: PathBuf },
    /// Explorer delete: Move to Trash (or Delete Permanently) / Cancel.
    Delete {
        paths: Vec<PathBuf>,
        permanent: bool,
    },
    /// Explorer drag and drop: Move / Cancel.
    Move { srcs: Vec<PathBuf>, dir: PathBuf },
    /// Recovery Discard: Discard / Cancel.
    DiscardRecovery { count: usize },
    /// A link with an unusual scheme: Open / Cancel.
    OpenLink { url: String },
    /// Workspace Replace All: Replace / Cancel.
    ReplaceAll { count: usize, files: usize },
    /// Source Control confirmations.
    Git(crate::git_write::GitPrompt),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    /// Save / Overwrite / Revert / Open Anyway / Keep (primary action).
    Primary,
    /// Don't Save / Delete.
    Secondary,
    /// Save As… (Scratchpads).
    SaveAs,
    Cancel,
}

/// Work waiting for saves to finish.
#[derive(Clone, Debug)]
pub struct Pending {
    pub waiting: HashSet<DocumentId>,
    pub then: Then,
}

/// A tab closed this session, for Reopen Closed Tab.
#[derive(Clone, Debug)]
pub struct ClosedTab {
    pub path: PathBuf,
    pub group: GroupId,
    pub selection: Selection,
    pub scroll: ScrollPosition,
}

pub const CLOSED_TABS: usize = 20;
/// Scratchpad content is written this long after the last edit.
pub const SCRATCH_DEBOUNCE_MS: u64 = 500;
pub const RECENT_SHOWN: usize = 10;

/// File-related state owned by `AppCore`.
pub struct FileState {
    pub dialogs: Arc<dyn Dialogs>,
    pub prompts: VecDeque<Prompt>,
    pub pending: Vec<Pending>,
    pub closed_tabs: Vec<ClosedTab>,
    pub recent_files: Vec<RecentEntry>,
    pub recent_workspaces: Vec<RecentEntry>,
    pub scratchpads: ScratchpadStore,
    /// Scratchpad documents with unwritten edits → due time.
    pub scratch_due: Vec<(DocumentId, u64)>,
    /// Set after the user confirmed quitting.
    pub quit_confirmed: bool,
}

impl FileState {
    pub fn new(dialogs: Arc<dyn Dialogs>, scratch_dir: PathBuf) -> Self {
        Self {
            dialogs,
            prompts: VecDeque::new(),
            pending: Vec::new(),
            closed_tabs: Vec::new(),
            recent_files: Vec::new(),
            recent_workspaces: Vec::new(),
            scratchpads: ScratchpadStore::new(scratch_dir),
            scratch_due: Vec::new(),
            quit_confirmed: false,
        }
    }
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

fn error_detail(e: &dyn std::fmt::Display, path: &Path) -> String {
    format!("{}: {e}", path.display())
}

impl AppCore {
    fn notify(&mut self, n: Notification) {
        let now = self.now_ms;
        self.notifications.notify(n, now);
    }

    fn load_options(&self) -> LoadOptions {
        LoadOptions::default()
    }

    // ── opening ──────────────────────────────────────────────────────────

    /// File → Open File… (multi-select).
    pub fn open_dialog(&mut self) {
        let dir = self
            .workbench
            .active_document()
            .and_then(|d| self.workbench.document(d))
            .and_then(|d| d.path().and_then(Path::parent).map(Path::to_path_buf));
        let opts = FileDialogOptions {
            title: Some(t!("dialogs:open.title")),
            directory: dir,
            file_name: None,
        };
        self.files.dialogs.pick_files(opts, self.io.poster());
    }

    /// Opens `path` in the active group (focusing it if already open).
    pub fn open_path(&mut self, path: &Path, at: OpenAt) {
        let group = if at.side {
            self.workbench.side_group()
        } else {
            self.workbench.layout.active_group
        };
        let (_, key) = fs_io::identity(path);
        if let Some(doc) = self.workbench.find_by_key(&key) {
            // Already open: focus a view where it is, else add one here.
            let existing = if at.side {
                self.workbench.view_in_group(doc, group)
            } else {
                self.workbench
                    .layout
                    .groups_in_order()
                    .into_iter()
                    .find_map(|g| self.workbench.view_in_group(doc, g))
            };
            let view = match existing {
                Some(v) => v,
                None => match self
                    .workbench
                    .open_view(doc, group, TabPosition::AfterActive)
                {
                    Ok(v) => v,
                    Err(_) => return,
                },
            };
            let _ = self.workbench.activate_view(view);
            self.place_caret(view, &at);
            if !at.keep_focus {
                self.effects.push(Effect::FocusEditor(view));
            }
            return;
        }
        self.load(
            path.to_path_buf(),
            LoadPurpose::Open {
                group,
                at,
                allow_binary: false,
            },
        );
    }

    pub(crate) fn load_for(&mut self, path: PathBuf, purpose: LoadPurpose) {
        self.load(path, purpose);
    }

    fn load(&mut self, path: PathBuf, purpose: LoadPurpose) {
        let mut opts = self.load_options();
        match &purpose {
            LoadPurpose::Restore { .. } | LoadPurpose::Compare { .. } => {}
            LoadPurpose::Open { allow_binary, .. } => opts.allow_binary = *allow_binary,
            LoadPurpose::Reencode(doc) => {
                opts.forced_encoding = self.workbench.document(*doc).map(|d| d.encoding.encoding);
            }
            LoadPurpose::Revert(doc) => {
                opts.forced_encoding = self.workbench.document(*doc).map(|d| d.encoding.encoding);
            }
        }
        self.io.submit(move || {
            let result = fs_io::load(&path, &opts);
            Some(IoResult::Loaded(Box::new(LoadOutcome {
                path,
                purpose,
                result,
            })))
        });
    }

    pub fn handle_loaded(&mut self, outcome: LoadOutcome) {
        let LoadOutcome {
            path,
            purpose,
            result,
        } = outcome;
        if let LoadPurpose::Compare { id, left, saved_of } = purpose {
            self.compare_loaded(id, left, saved_of, result);
            return;
        }
        if let LoadPurpose::Restore { group, key, tab } = purpose {
            match result {
                Ok(loaded) => self.finish_restore_load(loaded, group, &key, &tab),
                Err(e) => {
                    tracing::info!(path = %path.display(), error = %e, "session file not restored");
                    self.restore_slot_done(Some(path.display().to_string()));
                }
            }
            return;
        }
        let loaded = match result {
            Ok(l) => l,
            Err(LoadError::Binary) => {
                if let LoadPurpose::Open { group, .. } = purpose {
                    self.files.prompts.push_back(Prompt::Binary { path, group });
                }
                return;
            }
            Err(LoadError::NotFound) => {
                let missing_recent = self
                    .files
                    .recent_files
                    .iter()
                    .any(|r| Path::new(&r.path) == path);
                let mut n = Notification::new(
                    Severity::Warning,
                    Translatable::new("notifications:fileMissing").arg("name", file_name(&path)),
                );
                if missing_recent {
                    n = n.action(
                        "notifications:removeFromRecent",
                        CommandInvocation::new("file.removeRecent", InvocationSource::Internal)
                            .with_args(json!({ "path": path.to_string_lossy() })),
                    );
                }
                self.notify(n);
                return;
            }
            Err(e) => {
                let key = match e {
                    LoadError::PermissionDenied => "notifications:openDenied",
                    LoadError::TooLarge { .. } => "notifications:openTooLarge",
                    _ => "notifications:openFailed",
                };
                let n = Notification::new(
                    Severity::Error,
                    Translatable::new(key).arg("name", file_name(&path)),
                )
                .detail(error_detail(&e, &path))
                .source(NotificationSource::App);
                self.notify(n);
                return;
            }
        };
        match purpose {
            LoadPurpose::Open { group, at, .. } => self.finish_open(loaded, group, at),
            LoadPurpose::Revert(doc) | LoadPurpose::Reencode(doc) => {
                self.replace_from_disk(doc, loaded)
            }
            LoadPurpose::Restore { .. } | LoadPurpose::Compare { .. } => {
                unreachable!("handled above")
            }
        }
    }

    fn finish_restore_load(
        &mut self,
        loaded: Loaded,
        group: GroupId,
        key: &str,
        tab: &crate::session::TabData,
    ) {
        let origin = DocumentOrigin::File {
            path: loaded.path.clone(),
            key: loaded.key.clone(),
        };
        let doc = self.workbench.open_document(origin, loaded.text);
        self.restore_bookmarks(doc);
        let group = if self.workbench.layout.group(group).is_some() {
            group
        } else {
            self.workbench.layout.active_group
        };
        self.place_restored(doc, group, key, tab);
    }

    pub(crate) fn replace_with_disk(&mut self, doc: DocumentId, loaded: Loaded) {
        self.replace_from_disk(doc, loaded);
    }

    fn finish_open(&mut self, loaded: Loaded, group: GroupId, at: OpenAt) {
        let origin = DocumentOrigin::File {
            path: loaded.path.clone(),
            key: loaded.key.clone(),
        };
        let read_only = loaded.text.read_only;
        let mixed = loaded.text.mixed_line_endings;
        let doc = self.workbench.open_document(origin, loaded.text);
        self.restore_bookmarks(doc);
        let group = if self.workbench.layout.group(group).is_some() {
            group
        } else {
            self.workbench.layout.active_group
        };
        let Ok(view) = self
            .workbench
            .open_view(doc, group, TabPosition::AfterActive)
        else {
            return;
        };
        self.place_caret(view, &at);
        self.apply_markdown_default_mode(view);
        if !at.keep_focus {
            self.effects.push(Effect::FocusEditor(view));
        }
        self.record_recent_file(&loaded.path);
        if read_only == notoir_core::document::ReadOnly::LossyDecode {
            self.notify(Notification::new(
                Severity::Warning,
                Translatable::new("notifications:lossyDecode").arg("name", file_name(&loaded.path)),
            ));
        }
        if mixed {
            self.notify(Notification::new(
                Severity::Info,
                Translatable::new("notifications:mixedLineEndings")
                    .arg("name", file_name(&loaded.path)),
            ));
        }
    }

    fn place_caret(&mut self, view: ViewId, at: &OpenAt) {
        if let Some(anchor) = &at.anchor {
            let Some(doc) = self
                .workbench
                .view(view)
                .and_then(|v| self.workbench.document(v.document_id))
            else {
                return;
            };
            let text = doc.buffer.text();
            let md = notoir_editor::markdown::parse(&text);
            if let Some(h) = notoir_editor::markdown::find_heading(&md, anchor) {
                let line = doc.buffer.rope().char_to_line(h.range.0);
                let _ = self
                    .workbench
                    .set_selection(view, Selection::caret(h.range.0));
                if let Some(v) = self.workbench.views.get_mut(view) {
                    v.scroll.top_line = line;
                }
                self.find.scroll.insert(
                    view,
                    notoir_core::view::ScrollPosition {
                        top_line: line,
                        ..Default::default()
                    },
                );
            }
            return;
        }
        if let Some((sel, scroll)) = &at.restore {
            let _ = self.workbench.set_selection(view, sel.clone());
            if let Some(v) = self.workbench.views.get_mut(view) {
                v.scroll = *scroll;
            }
        } else if let Some((line, col)) = at.line_col {
            let Some(doc) = self
                .workbench
                .view(view)
                .and_then(|v| self.workbench.document(v.document_id))
            else {
                return;
            };
            let rope = doc.buffer.rope();
            let line = line
                .saturating_sub(1)
                .min(rope.len_lines().saturating_sub(1));
            let start = rope.line_to_char(line);
            let len = notoir_editor::display_map::line_text(rope, line)
                .chars()
                .count();
            let pos = start + col.saturating_sub(1).min(len);
            let end = (pos + at.select_chars).min(rope.len_chars());
            let _ = self
                .workbench
                .set_selection(view, Selection::single(notoir_editor::Range::new(pos, end)));
            if let Some(v) = self.workbench.views.get_mut(view) {
                v.scroll.top_line = line.saturating_sub(5);
            }
            self.find.center.push(view);
        }
    }

    /// Revert / Reopen with Encoding: replace the buffer with disk content as
    /// one undoable transaction, then mark it saved.
    fn replace_from_disk(&mut self, doc: DocumentId, loaded: Loaded) {
        let Some(d) = self.workbench.document(doc) else {
            return;
        };
        let len = d.buffer.len_chars();
        let changes = ChangeSet::replace(0, len, loaded.text.text.as_str());
        let caret = 0;
        let read_only = d.read_only;
        if let Some(d) = self.workbench.document_mut(doc) {
            // Reload is allowed on read-only documents (lossy → reencoded).
            d.read_only = notoir_core::document::ReadOnly::No;
        }
        let now = self.now_ms;
        let ok = self
            .workbench
            .apply_edit(
                doc,
                None,
                changes,
                Selection::caret(caret),
                EditKind::Other,
                ChangeOrigin::Reload,
                now,
            )
            .is_ok();
        if let Some(d) = self.workbench.document_mut(doc) {
            d.read_only = if ok { loaded.text.read_only } else { read_only };
            d.encoding = loaded.text.encoding;
            d.line_ending = loaded.text.line_ending;
            d.mixed_line_endings = loaded.text.mixed_line_endings;
            d.disk_state = DiskState::InSync;
            let state = d.buffer.state();
            if let Some(hash) = loaded.text.content_hash {
                d.mark_saved(state, hash);
            }
        }
        self.banners.remove(&doc);
    }

    // ── saving ───────────────────────────────────────────────────────────

    /// File → Save. Untitled documents and Scratchpads go through Save As.
    pub fn save(&mut self, doc: DocumentId) {
        let Some(d) = self.workbench.document(doc) else {
            return;
        };
        match (&d.origin, d.is_read_only()) {
            (DocumentOrigin::File { path, .. }, false) => {
                let path = path.clone();
                let expected = d.saved_content_hash;
                self.start_save(doc, path, false, expected, false);
            }
            _ => self.save_as(doc),
        }
    }

    pub fn save_as(&mut self, doc: DocumentId) {
        let Some(d) = self.workbench.document(doc) else {
            return;
        };
        let (dir, name) = match &d.origin {
            DocumentOrigin::File { path, .. } => {
                (path.parent().map(Path::to_path_buf), Some(file_name(path)))
            }
            DocumentOrigin::Scratchpad { title, .. } => (None, Some(format!("{title}.md"))),
            DocumentOrigin::Untitled { n } => (None, Some(t!("tabs:untitled", n = n) + ".txt")),
            DocumentOrigin::Virtual { title } => (None, Some(title.clone())),
        };
        let opts = FileDialogOptions {
            title: Some(t!("dialogs:saveAs.title")),
            directory: dir,
            file_name: name,
        };
        self.files.dialogs.save_file(opts, doc, self.io.poster());
    }

    /// Save All: every dirty document; Untitled ones ask for a path one by one.
    pub fn save_all(&mut self) {
        let dirty: Vec<DocumentId> = self
            .workbench
            .documents
            .values()
            .filter(|d| d.is_dirty())
            .map(|d| d.id)
            .collect();
        for doc in dirty {
            self.save(doc);
        }
    }

    pub fn handle_save_as_picked(&mut self, doc: DocumentId, path: Option<PathBuf>) {
        let Some(path) = path else {
            // Cancelled: anything waiting on this save is abandoned.
            self.files.pending.retain(|p| !p.waiting.contains(&doc));
            return;
        };
        let (_, key) = fs_io::identity(&path);
        if let Some(other) = self.workbench.find_by_key(&key).filter(|o| *o != doc) {
            if self.workbench.document(other).is_some_and(|d| d.is_dirty()) {
                self.notify(Notification::new(
                    Severity::Error,
                    Translatable::new("notifications:saveAsOpenDirty")
                        .arg("name", file_name(&path)),
                ));
                return;
            }
            let views: Vec<ViewId> = self
                .workbench
                .views
                .of_document(other)
                .map(|v| v.id)
                .collect();
            for v in views {
                let _ = self.workbench.close_view(v, true);
            }
        }
        self.start_save(doc, path, true, None, false);
    }

    /// Save as Administrator… (spec 36): the same save, with the final
    /// write done after administrator authorization.
    pub fn save_as_admin(&mut self, doc: DocumentId) {
        let Some(d) = self.workbench.document(doc) else {
            return;
        };
        if let DocumentOrigin::File { path, .. } = &d.origin {
            let path = path.clone();
            let expected = d.saved_content_hash;
            self.start_save_with(doc, path, false, expected, false, true);
        }
    }

    /// Snapshots the document and writes it on the IO queue.
    pub fn start_save(
        &mut self,
        doc: DocumentId,
        path: PathBuf,
        save_as: bool,
        expected_hash: Option<u64>,
        overwrite: bool,
    ) {
        self.start_save_with(doc, path, save_as, expected_hash, overwrite, false);
    }

    fn start_save_with(
        &mut self,
        doc: DocumentId,
        path: PathBuf,
        save_as: bool,
        expected_hash: Option<u64>,
        overwrite: bool,
        elevated: bool,
    ) {
        self.apply_save_transforms(doc);
        let Some(d) = self.workbench.document(doc) else {
            return;
        };
        let rope = d.buffer.rope().clone();
        let state = d.buffer.state();
        let encoding = d.encoding;
        let line_ending = d.line_ending;
        let atomic = self.settings.bool("files.atomicSave", None);
        let elevation = elevated.then(|| self.elevation.clone());
        self.saving.insert(doc);
        self.io.submit(move || {
            let req = SaveRequest {
                path: path.clone(),
                text: rope.to_string(),
                encoding,
                line_ending,
                expected_hash,
                overwrite,
                atomic,
            };
            let result = match elevation {
                Some(el) => fs_io::save_with(&req, |p, bytes| {
                    use notoir_platform::elevation::ElevationError;
                    match el.write_file(p, bytes) {
                        Ok(()) => Ok(notoir_storage::atomic::WriteStrategy::Atomic),
                        Err(ElevationError::Cancelled) => Err(SaveError::ElevationCancelled),
                        Err(e) => Err(SaveError::Elevation(e.to_string())),
                    }
                }),
                None => fs_io::save(&req),
            };
            Some(IoResult::Saved(Box::new(SaveOutcome {
                doc,
                state,
                path,
                save_as,
                encoding,
                result,
            })))
        });
    }

    /// `files.trimTrailingWhitespaceOnSave` / `files.insertFinalNewline`.
    fn apply_save_transforms(&mut self, doc: DocumentId) {
        let Some(d) = self.workbench.document(doc) else {
            return;
        };
        let lang = Some(d.language.as_str());
        let trim = self
            .settings
            .bool("files.trimTrailingWhitespaceOnSave", lang);
        let final_nl = self.settings.bool("files.insertFinalNewline", lang);
        if !trim && !final_nl {
            return;
        }
        let rope = d.buffer.rope();
        let mut edits = Vec::new();
        if trim {
            for line in 0..rope.len_lines() {
                let text = notoir_editor::display_map::line_text(rope, line);
                let kept = text.trim_end_matches([' ', '\t']).chars().count();
                let total = text.chars().count();
                if kept < total {
                    let start = rope.line_to_char(line);
                    edits.push(notoir_editor::Edit::new(start + kept, start + total, ""));
                }
            }
        }
        let len = rope.len_chars();
        if final_nl && len > 0 && rope.char(len - 1) != '\n' {
            edits.push(notoir_editor::Edit::new(len, len, "\n"));
        }
        if edits.is_empty() {
            return;
        }
        if let Ok(changes) = ChangeSet::new(edits) {
            let now = self.now_ms;
            let view = self.workbench.views.of_document(doc).next().map(|v| v.id);
            let sel = view
                .and_then(|v| self.workbench.view(v))
                .map(|v| v.selection.map(&changes))
                .unwrap_or_default();
            let _ = self.workbench.apply_edit(
                doc,
                view,
                changes,
                sel,
                EditKind::Other,
                ChangeOrigin::Format,
                now,
            );
        }
    }

    pub fn handle_saved(&mut self, outcome: SaveOutcome) {
        let SaveOutcome {
            doc,
            state,
            path,
            save_as,
            encoding,
            result,
        } = outcome;
        self.saving.remove(&doc);
        if self.workbench.document(doc).is_none() {
            return;
        }
        match result {
            Ok(saved) => {
                self.banners.remove(&doc);
                if save_as {
                    let (canonical, key) = fs_io::identity(&path);
                    let scratch = match &self.workbench.document(doc).map(|d| d.origin.clone()) {
                        Some(DocumentOrigin::Scratchpad { scratch_id, .. }) => {
                            Some(scratch_id.clone())
                        }
                        _ => None,
                    };
                    let language = notoir_editor::language::detect(
                        Some(&canonical.to_string_lossy()),
                        "",
                        &[],
                    );
                    if let Some(d) = self.workbench.document_mut(doc) {
                        d.origin = DocumentOrigin::File {
                            path: canonical.clone(),
                            key,
                        };
                        d.encoding = encoding;
                        if !d.language_overridden && language != LanguageId::PLAINTEXT {
                            d.language = language;
                        }
                    }
                    if let (Some(id), Some(db)) = (scratch, self.state_db.as_ref()) {
                        let _ = self.files.scratchpads.delete(db, &id);
                        self.files.scratch_due.retain(|(d, _)| *d != doc);
                    }
                    self.record_recent_file(&canonical);
                }
                let _ = self.workbench.mark_saved(doc, state, saved.hash);
                self.saved_continue(doc);
            }
            Err(SaveError::Conflict) => {
                self.files.prompts.push_back(Prompt::SaveConflict { doc });
            }
            Err(SaveError::ElevationCancelled) => {
                // Edits and the recovery snapshot stay; Save As stays offered.
                self.files.pending.retain(|p| !p.waiting.contains(&doc));
                let n = Notification::new(
                    Severity::Info,
                    Translatable::new("notifications:adminSaveCancelled")
                        .arg("name", file_name(&path)),
                )
                .dedupe(format!("save:{}", doc.0))
                .action(
                    "commands:file.saveAs",
                    CommandInvocation::new("file.saveAs", InvocationSource::Internal)
                        .with_args(json!({ "doc": doc.0 })),
                );
                self.notify(n);
            }
            Err(e) => {
                self.files.pending.retain(|p| !p.waiting.contains(&doc));
                let key = match e {
                    SaveError::Elevation(_) => "notifications:adminSaveFailed",
                    SaveError::PermissionDenied => "notifications:saveDenied",
                    SaveError::DiskFull => "notifications:saveDiskFull",
                    SaveError::Unmappable(_) => "notifications:saveUnmappable",
                    _ => "notifications:saveFailed",
                };
                let args = json!({ "doc": doc.0 });
                let mut n = Notification::new(
                    Severity::Error,
                    Translatable::new(key).arg("name", file_name(&path)),
                )
                .detail(error_detail(&e, &path))
                .dedupe(format!("save:{}", doc.0));
                // Only permission failures of an existing file can elevate.
                if matches!(e, SaveError::PermissionDenied) && !save_as {
                    n = n.action(
                        "commands:file.saveAsAdmin",
                        CommandInvocation::new("file.saveAsAdmin", InvocationSource::Internal)
                            .with_args(args.clone()),
                    );
                }
                let n = n
                    .action(
                        "commands:file.saveAs",
                        CommandInvocation::new("file.saveAs", InvocationSource::Internal)
                            .with_args(args.clone()),
                    )
                    .action(
                        "common:retry",
                        CommandInvocation::new("file.save", InvocationSource::Internal)
                            .with_args(args),
                    );
                self.notify(n);
            }
        }
    }

    /// Runs work that was waiting for `doc` to be saved.
    fn saved_continue(&mut self, doc: DocumentId) {
        let mut ready = Vec::new();
        for p in &mut self.files.pending {
            p.waiting.remove(&doc);
            if p.waiting.is_empty() {
                ready.push(p.then.clone());
            }
        }
        self.files.pending.retain(|p| !p.waiting.is_empty());
        for then in ready {
            self.run_then(then);
        }
    }

    fn run_then(&mut self, then: Then) {
        match then {
            Then::CloseViews(views) => {
                for v in views {
                    self.close_view_now(v);
                }
            }
            Then::Quit => {
                self.files.quit_confirmed = true;
                self.effects.push(Effect::Quit);
            }
            Then::CloseWorkspace { views, next } => self.finish_close_workspace(views, next),
            Then::GitCheckout(branch) => {
                self.git_submit(crate::git_write::LocalOp::Checkout(branch))
            }
            Then::GitSyncNotes => self.git_sync_notes_now(),
        }
    }

    // ── closing ──────────────────────────────────────────────────────────

    /// Close a tab: prompts for unsaved changes / non-empty Scratchpads.
    pub fn request_close(&mut self, view: ViewId) {
        let Some(doc) = self.workbench.view(view).map(|v| v.document_id) else {
            return;
        };
        let last = self.workbench.views.count_of_document(doc) == 1;
        let d = self.workbench.document(doc).expect("view's document");
        if last && matches!(d.origin, DocumentOrigin::Scratchpad { .. }) {
            if d.buffer.len_chars() == 0 {
                self.delete_scratchpad(doc);
                self.close_view_now(view);
            } else {
                self.files
                    .prompts
                    .push_back(Prompt::ScratchpadClose { view, doc });
            }
            return;
        }
        if last && d.is_dirty() {
            self.files.prompts.push_back(Prompt::Unsaved {
                docs: vec![doc],
                then: Then::CloseViews(vec![view]),
            });
        } else {
            self.close_view_now(view);
        }
    }

    /// Closes several tabs, prompting once per dirty document.
    pub fn request_close_many(&mut self, views: Vec<ViewId>) {
        for v in views {
            self.request_close(v);
        }
    }

    /// Closes without prompting (after the user decided).
    pub fn close_view_now(&mut self, view: ViewId) {
        self.remember_closed(view);
        if matches!(
            self.workbench.close_view(view, true),
            Ok(CloseOutcome::Closed)
        ) {
            self.after_close();
        }
    }

    fn remember_closed(&mut self, view: ViewId) {
        let Some(v) = self.workbench.view(view) else {
            return;
        };
        let Some(path) = self
            .workbench
            .document(v.document_id)
            .and_then(|d| d.path())
            .map(Path::to_path_buf)
        else {
            return;
        };
        let group = self
            .workbench
            .layout
            .group_of(view)
            .unwrap_or(self.workbench.layout.active_group);
        self.files.closed_tabs.push(ClosedTab {
            path,
            group,
            selection: v.selection.clone(),
            scroll: v.scroll,
        });
        if self.files.closed_tabs.len() > CLOSED_TABS {
            self.files.closed_tabs.remove(0);
        }
    }

    fn after_close(&mut self) {
        if let Some(v) = self.workbench.active_view() {
            self.effects.push(Effect::FocusEditor(v));
        }
    }

    /// View → Reopen Closed Tab.
    pub fn reopen_closed(&mut self) {
        let Some(tab) = self.files.closed_tabs.pop() else {
            return;
        };
        if self.workbench.layout.group(tab.group).is_some() {
            self.workbench.layout.active_group = tab.group;
        }
        self.open_path(
            &tab.path,
            OpenAt {
                line_col: None,
                restore: Some((tab.selection, tab.scroll)),
                keep_focus: false,
                anchor: None,
                ..Default::default()
            },
        );
    }

    /// Window close / Quit: prompts when documents are unsaved.
    /// Returns true when the app may exit now.
    pub fn request_quit(&mut self) -> bool {
        if self.files.quit_confirmed {
            return true;
        }
        // Hot exit: unsaved work stays in the recovery store (spec 09).
        if self.settings.bool("app.hotExit", None) && self.recovery.session.is_some() {
            return true;
        }
        let dirty: Vec<DocumentId> = self
            .workbench
            .documents
            .values()
            .filter(|d| d.is_dirty())
            .map(|d| d.id)
            .collect();
        if dirty.is_empty() {
            return true;
        }
        if !self.files.prompts.iter().any(|p| {
            matches!(
                p,
                Prompt::Unsaved {
                    then: Then::Quit,
                    ..
                }
            )
        }) {
            self.files.prompts.push_back(Prompt::Unsaved {
                docs: dirty,
                then: Then::Quit,
            });
        }
        false
    }

    /// Applies the user's answer to the prompt at the front of the queue.
    pub fn answer_prompt(&mut self, answer: Answer) {
        let Some(prompt) = self.files.prompts.pop_front() else {
            return;
        };
        match (prompt, answer) {
            (Prompt::Git(p), a) => self.git_answer(p, a),
            (_, Answer::Cancel) => {}
            (Prompt::Unsaved { docs, then }, Answer::Primary) => {
                self.files.pending.push(Pending {
                    waiting: docs.iter().copied().collect(),
                    then,
                });
                for d in docs {
                    self.save(d);
                }
            }
            (Prompt::Unsaved { then, .. }, Answer::Secondary) => {
                if let Then::CloseViews(views) = &then {
                    for v in views {
                        self.close_view_now(*v);
                    }
                } else {
                    // Discarding: views of dirty documents close without saving.
                    self.run_then(then);
                }
            }
            (Prompt::WorkspaceReplace { path }, Answer::Primary) => {
                self.close_workspace(Some(path))
            }
            (Prompt::WorkspaceReplace { path }, Answer::Secondary) => {
                self.open_workspace_in_new_window(&path)
            }
            (Prompt::Delete { paths, permanent }, Answer::Primary) => {
                self.delete_paths(paths, permanent)
            }
            (Prompt::Move { srcs, dir }, Answer::Primary) => self.transfer_into(srcs, dir, false),
            (Prompt::DiscardRecovery { .. }, Answer::Primary) => self.recovery_discard_all(),
            (Prompt::OpenLink { url }, Answer::Primary) => self.files.dialogs.open_external(&url),
            (Prompt::ReplaceAll { .. }, Answer::Primary) => self.replace_results(None),
            (Prompt::ScratchpadClose { view, doc }, Answer::Primary) => {
                self.flush_scratchpad(doc);
                self.set_scratchpad_open(doc, false);
                self.close_view_now(view);
            }
            (Prompt::ScratchpadClose { view, doc }, Answer::SaveAs) => {
                self.files.pending.push(Pending {
                    waiting: [doc].into_iter().collect(),
                    then: Then::CloseViews(vec![view]),
                });
                self.save_as(doc);
            }
            (Prompt::ScratchpadClose { view, doc }, Answer::Secondary) => {
                self.delete_scratchpad(doc);
                self.close_view_now(view);
            }
            (Prompt::SaveConflict { doc }, Answer::Primary) => {
                if let Some(path) = self
                    .workbench
                    .document(doc)
                    .and_then(|d| d.path())
                    .map(Path::to_path_buf)
                {
                    self.start_save(doc, path, false, None, true);
                }
            }
            (Prompt::SaveConflict { doc }, _) => {
                self.files.pending.retain(|p| !p.waiting.contains(&doc))
            }
            (Prompt::Revert { doc }, Answer::Primary) => {
                if let Some(path) = self
                    .workbench
                    .document(doc)
                    .and_then(|d| d.path())
                    .map(Path::to_path_buf)
                {
                    self.load(path, LoadPurpose::Revert(doc));
                }
            }
            (Prompt::Binary { path, group }, Answer::Primary) => {
                self.load(
                    path,
                    LoadPurpose::Open {
                        group,
                        at: OpenAt::default(),
                        allow_binary: true,
                    },
                );
            }
            _ => {}
        }
    }

    // ── Scratchpads ──────────────────────────────────────────────────────

    /// File → New Scratchpad.
    pub fn new_scratchpad(&mut self) {
        let Some(db) = self.state_db.as_ref() else {
            self.notify(Notification::new(
                Severity::Error,
                Translatable::new("notifications:scratchUnavailable"),
            ));
            return;
        };
        let prefix = t!("tabs:scratchPrefix");
        let n = ScratchpadStore::next_number(db, &prefix).unwrap_or(1);
        let title = format!("{prefix} {n}");
        let language = self
            .settings
            .str("scratchpad.defaultLanguage", None)
            .to_owned();
        let now = self.now_ms as i64;
        match self.files.scratchpads.create(db, &title, &language, now) {
            Ok(row) => {
                self.open_scratchpad_row(row, String::new());
            }
            Err(e) => {
                tracing::error!(error = %e, "could not create scratchpad");
                self.notify(
                    Notification::new(
                        Severity::Error,
                        Translatable::new("notifications:scratchUnavailable"),
                    )
                    .detail(e.to_string()),
                );
            }
        }
    }

    /// Typing on the empty state: a new Scratchpad containing `text`.
    pub fn new_scratchpad_with_text(&mut self, text: &str) {
        self.new_scratchpad();
        let Some(view) = self.workbench.active_view() else {
            return;
        };
        let sel = self
            .workbench
            .view(view)
            .map(|v| v.selection.clone())
            .unwrap_or_default();
        if let Some(res) = notoir_editor::edit_ops::insert(&sel, text, EditKind::Insert) {
            let now = self.now_ms;
            let _ = self
                .workbench
                .edit(view, res.changes, res.selection, res.kind, now);
        }
    }

    fn open_scratchpad_row(&mut self, row: ScratchpadRow, content: String) -> Option<ViewId> {
        let language = LanguageId::from_id(&row.language).unwrap_or(LanguageId::MARKDOWN);
        let loaded = LoadedText {
            text: content,
            ..LoadedText::empty(language)
        };
        let origin = DocumentOrigin::Scratchpad {
            scratch_id: row.scratch_id.clone(),
            title: row.title.clone(),
        };
        let doc = self.workbench.open_document(origin, loaded);
        let group = self.workbench.layout.active_group;
        let view = self
            .workbench
            .open_view(doc, group, TabPosition::AfterActive)
            .ok()?;
        self.effects.push(Effect::FocusEditor(view));
        if !row.open {
            self.set_scratchpad_open(doc, true);
        }
        Some(view)
    }

    /// Opens a kept Scratchpad (Open Recent → Scratchpads).
    pub fn open_scratchpad(&mut self, scratch_id: &str) {
        if let Some(doc) = self.workbench.documents.values().find(|d| matches!(&d.origin, DocumentOrigin::Scratchpad { scratch_id: id, .. } if id == scratch_id)).map(|d| d.id) {
            let view = self.workbench.views.of_document(doc).next().map(|v| v.id);
            if let Some(v) = view {
                let _ = self.workbench.activate_view(v);
            }
            return;
        }
        let Some(row) = self
            .state_db
            .as_ref()
            .and_then(|db| db.scratchpads().ok())
            .and_then(|rows| rows.into_iter().find(|r| r.scratch_id == scratch_id))
        else {
            return;
        };
        let content = self
            .files
            .scratchpads
            .read_content(scratch_id)
            .unwrap_or_default();
        self.open_scratchpad_row(row, content);
    }

    /// Reopens Scratchpads that were open when Notoir last exited.
    pub fn restore_scratchpads(&mut self) {
        let Some(rows) = self.state_db.as_ref().and_then(|db| db.scratchpads().ok()) else {
            return;
        };
        for row in rows.into_iter().filter(|r| r.open) {
            let content = self
                .files
                .scratchpads
                .read_content(&row.scratch_id)
                .unwrap_or_default();
            self.open_scratchpad_row(row, content);
        }
    }

    /// Kept Scratchpads (not open in a tab).
    pub fn kept_scratchpads(&self) -> Vec<ScratchpadRow> {
        let open: HashSet<String> = self
            .workbench
            .documents
            .values()
            .filter_map(|d| match &d.origin {
                DocumentOrigin::Scratchpad { scratch_id, .. } => Some(scratch_id.clone()),
                _ => None,
            })
            .collect();
        self.state_db
            .as_ref()
            .and_then(|db| db.scratchpads().ok())
            .unwrap_or_default()
            .into_iter()
            .filter(|r| !open.contains(&r.scratch_id))
            .collect()
    }

    fn scratch_id(&self, doc: DocumentId) -> Option<String> {
        match &self.workbench.document(doc)?.origin {
            DocumentOrigin::Scratchpad { scratch_id, .. } => Some(scratch_id.clone()),
            _ => None,
        }
    }

    fn set_scratchpad_open(&mut self, doc: DocumentId, open: bool) {
        let Some(id) = self.scratch_id(doc) else {
            return;
        };
        let now = self.now_ms as i64;
        if let Some(db) = self.state_db.as_ref()
            && let Some(mut row) = db
                .scratchpads()
                .ok()
                .and_then(|rows| rows.into_iter().find(|r| r.scratch_id == id))
        {
            row.open = open;
            row.updated_at = now;
            let _ = db.upsert_scratchpad(&row);
        }
    }

    fn delete_scratchpad(&mut self, doc: DocumentId) {
        let Some(id) = self.scratch_id(doc) else {
            return;
        };
        self.files.scratch_due.retain(|(d, _)| *d != doc);
        if let Some(db) = self.state_db.as_ref() {
            let _ = self.files.scratchpads.delete(db, &id);
        }
    }

    /// Schedules a content write after edits to a Scratchpad.
    pub fn scratchpad_changed(&mut self, doc: DocumentId) {
        if self.scratch_id(doc).is_none() {
            return;
        }
        let due = self.now_ms + SCRATCH_DEBOUNCE_MS;
        match self.files.scratch_due.iter_mut().find(|(d, _)| *d == doc) {
            Some(entry) => entry.1 = due,
            None => self.files.scratch_due.push((doc, due)),
        }
    }

    fn flush_scratchpad(&mut self, doc: DocumentId) {
        let Some(id) = self.scratch_id(doc) else {
            return;
        };
        let Some(rope) = self
            .workbench
            .document(doc)
            .map(|d| d.buffer.rope().clone())
        else {
            return;
        };
        self.files.scratch_due.retain(|(d, _)| *d != doc);
        let dir = self.files.scratchpads.dir().to_path_buf();
        self.io.submit(move || {
            ScratchpadStore::new(dir)
                .write_content(&id, &rope.to_string())
                .err()
                .map(|e| IoResult::ScratchpadWriteFailed {
                    error: e.to_string(),
                })
        });
    }

    /// Writes Scratchpads whose debounce expired (or all, on exit).
    pub fn flush_scratchpads(&mut self, all: bool) {
        let now = self.now_ms;
        let due: Vec<DocumentId> = self
            .files
            .scratch_due
            .iter()
            .filter(|(_, t)| all || *t <= now)
            .map(|(d, _)| *d)
            .collect();
        for doc in due {
            self.flush_scratchpad(doc);
        }
    }

    /// Next scratchpad write time (repaint scheduling).
    pub fn next_scratch_due(&self) -> Option<u64> {
        self.files.scratch_due.iter().map(|(_, t)| *t).min()
    }

    // ── recent files ─────────────────────────────────────────────────────

    pub fn load_recent(&mut self) {
        if let Some(db) = self.state_db.as_ref() {
            self.files.recent_files = db.recent(RecentKind::File).unwrap_or_default();
        }
    }

    pub fn record_recent_file(&mut self, path: &Path) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        if let Some(db) = self.state_db.as_ref() {
            let _ = db.record_recent(RecentKind::File, &path.to_string_lossy(), None, now);
        }
        self.load_recent();
    }

    pub fn remove_recent_file(&mut self, path: &str) {
        if let Some(db) = self.state_db.as_ref() {
            let _ = db.remove_recent(RecentKind::File, path);
        }
        self.load_recent();
    }

    pub fn set_recent_pinned(&mut self, path: &str, pinned: bool) {
        if let Some(db) = self.state_db.as_ref() {
            let _ = db.set_recent_pinned(RecentKind::File, path, pinned);
        }
        self.load_recent();
    }

    pub fn clear_recent(&mut self) {
        if let Some(db) = self.state_db.as_ref() {
            let _ = db.clear_recent();
        }
        self.load_recent();
    }

    // ── encoding / line endings / language ──────────────────────────────

    pub fn set_line_ending(&mut self, doc: DocumentId, eol: notoir_editor::LineEnding) {
        if let Some(d) = self.workbench.document_mut(doc)
            && d.line_ending != eol
        {
            d.line_ending = eol;
            d.mixed_line_endings = false;
            // Disk bytes would change: unsaved relative to disk.
            d.saved_state = None;
        }
    }

    /// Save with Encoding: changes the encoding used for the next save.
    pub fn set_save_encoding(&mut self, doc: DocumentId, encoding: TextEncoding) {
        if let Some(d) = self.workbench.document_mut(doc)
            && d.encoding != encoding
        {
            d.encoding = encoding;
            d.saved_state = None;
        }
    }

    /// Reopen with Encoding: reloads from disk with `encoding` (prompts first
    /// when there are unsaved changes, via the caller).
    pub fn reopen_with_encoding(&mut self, doc: DocumentId, encoding: TextEncoding) {
        let Some(path) = self
            .workbench
            .document(doc)
            .and_then(|d| d.path())
            .map(Path::to_path_buf)
        else {
            return;
        };
        if let Some(d) = self.workbench.document_mut(doc) {
            d.encoding = encoding;
        }
        self.load(path, LoadPurpose::Reencode(doc));
    }

    pub fn set_language(&mut self, doc: DocumentId, language: LanguageId) {
        if let Some(d) = self.workbench.document_mut(doc) {
            d.language = language;
            d.language_overridden = true;
        }
    }

    /// Display title of a document (tabs, prompts).
    pub fn document_title(&self, doc: DocumentId) -> String {
        if let Some(title) = self.compare_title(doc) {
            return title;
        }
        match self.workbench.document(doc).map(|d| &d.origin) {
            Some(DocumentOrigin::File { path, .. }) => file_name(path),
            Some(DocumentOrigin::Scratchpad { title, .. }) => title.clone(),
            Some(DocumentOrigin::Untitled { n }) => t!("tabs:untitled", n = n),
            // Virtual titles are i18n keys (Settings, Welcome).
            Some(DocumentOrigin::Virtual { title }) => t!(title),
            None => String::new(),
        }
    }
}
