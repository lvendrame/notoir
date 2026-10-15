//! Workspaces and the Explorer (docs/specs/07): opening and closing a folder,
//! workspace settings and trust, lazy tree listing, the background index,
//! file watching, and Explorer file operations. Disk work runs on the
//! workspace queue; results come back as [`WsResult`] messages tagged with the
//! workspace generation so results for a closed workspace are dropped.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use notoir_core::explorer::{EntryInfo, ExplorerTree, IndexEntry, NodeKind, RowOptions, SortOrder};
use notoir_core::ids::{DocumentId, ViewId};
use notoir_core::notifications::{Notification, NotificationSource, Severity, Translatable};
use notoir_core::panels::PanelId;
use notoir_core::settings::{ResolvedSettings, SettingsLayer};
use notoir_core::{AppEvent, CommandInvocation, DocumentOrigin, command::InvocationSource};
use notoir_storage::settings_file;
use notoir_storage::state_db::{RecentEntry, RecentKind};
use notoir_workspace::fs_ops::{self, NameError, TargetOs};
use notoir_workspace::listing::{self, Filters};
use notoir_workspace::watcher::WorkspaceWatcher;

use crate::core::AppCore;
use crate::files::{Prompt, Then};
use crate::io::IoResult;

/// Inline name editor in the tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditKind {
    NewFile { dir: PathBuf },
    NewFolder { dir: PathBuf },
    Rename { path: PathBuf },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlineEdit {
    pub kind: EditKind,
    pub text: String,
    /// Translated validation message.
    pub error: Option<String>,
    /// The text field still needs focus (and its initial selection).
    pub focus: bool,
}

/// The last Explorer operation, for `explorer.undo`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UndoOp {
    /// Undo by renaming back.
    Renamed { from: PathBuf, to: PathBuf },
    /// Undo by moving each `to` back to `from`.
    Moved(Vec<(PathBuf, PathBuf)>),
    /// Undo by moving the created paths to the trash.
    Created(Vec<PathBuf>),
}

/// A finished file operation.
#[derive(Clone, Debug, Default)]
pub struct OpOutcome {
    pub renamed: Vec<(PathBuf, PathBuf)>,
    pub removed: Vec<PathBuf>,
    pub created: Vec<PathBuf>,
    /// Open this file afterwards (New File).
    pub open: Option<PathBuf>,
    /// Select this path afterwards.
    pub select: Option<PathBuf>,
    pub undo: Option<UndoOp>,
    /// Error message key + detail.
    pub error: Option<(String, String)>,
    /// Trash failed for these: offer permanent deletion.
    pub trash_failed: Vec<PathBuf>,
}

#[derive(Debug)]
pub enum WsEvent {
    Listed {
        dir: PathBuf,
        entries: Result<Vec<EntryInfo>, String>,
    },
    Indexed {
        entries: Vec<IndexEntry>,
        truncated: bool,
    },
    Changed(Vec<PathBuf>),
    SettingsLoaded(Option<SettingsLayer>),
    Op(Box<OpOutcome>),
}

#[derive(Debug)]
pub struct WsResult {
    pub generation: u64,
    pub event: WsEvent,
}

pub struct Workspace {
    pub root: PathBuf,
    pub key: String,
    pub name: String,
    pub trusted: bool,
    pub generation: u64,
    pub tree: ExplorerTree,
    pub filters: Arc<Filters>,
    pub index: Arc<Vec<IndexEntry>>,
    pub index_ready: bool,
    index_cancel: Arc<AtomicBool>,
    /// Rebuild the index at this time (after file changes).
    pub index_due: Option<u64>,
    pub settings: Option<SettingsLayer>,
    _watcher: Option<WorkspaceWatcher>,
    pub filter: String,
    pub filter_open: bool,
    pub filter_focus: bool,
    pub edit: Option<InlineEdit>,
    /// Explorer Copy: pasted into the selected folder.
    pub clipboard: Option<PathBuf>,
    pub last_op: Option<UndoOp>,
    /// Path to select once its parent directories are listed.
    pub pending_reveal: Option<PathBuf>,
    /// Scroll the selected row into view next frame.
    pub scroll_to_selected: bool,
}

impl Drop for Workspace {
    fn drop(&mut self) {
        self.index_cancel.store(true, Ordering::Relaxed);
    }
}

/// Index rebuild delay after file changes.
const INDEX_DEBOUNCE_MS: u64 = 1500;

fn now_epoch_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

fn name_of(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

impl Workspace {
    pub fn row_options(&self, settings: &ResolvedSettings) -> RowOptions {
        RowOptions {
            show_hidden: settings.bool("explorer.showHidden", None),
            show_ignored: settings.bool("explorer.showGitIgnored", None),
            sort: SortOrder::from_setting(settings.str("explorer.sortOrder", None)),
        }
    }

    /// The folder new entries go into for a selection.
    pub fn target_dir(&self, path: Option<&Path>) -> PathBuf {
        match path.and_then(|p| self.tree.node(p)) {
            Some(n) if n.info.kind == NodeKind::Dir => n.path.clone(),
            Some(n) => n.path.parent().map_or(self.root.clone(), Path::to_path_buf),
            None => self.root.clone(),
        }
    }

    /// Relative path for Copy Relative Path (`/`-separated).
    pub fn relative(&self, path: &Path) -> Option<String> {
        path.strip_prefix(&self.root)
            .ok()
            .map(|r| r.to_string_lossy().replace('\\', "/"))
    }

    pub fn contains(&self, path: &Path) -> bool {
        path.starts_with(&self.root)
    }
}

impl AppCore {
    // ── settings ─────────────────────────────────────────────────────────

    /// Resolves settings from the user layer and the workspace layer.
    pub fn resolve_settings(&self) -> ResolvedSettings {
        let (layer, trusted) = match &self.workspace {
            Some(ws) => (ws.settings.clone(), ws.trusted),
            None => (None, false),
        };
        ResolvedSettings::new(
            self.schema.clone(),
            self.user_settings.clone(),
            layer,
            trusted,
        )
    }

    fn apply_resolved_settings(&mut self) {
        let newer = self.resolve_settings();
        let changed = self.settings.changed_keys(&newer);
        self.settings = newer;
        if !changed.is_empty() {
            self.workbench
                .events
                .emit(AppEvent::SettingsChanged { keys: changed });
        }
    }

    // ── open / close ─────────────────────────────────────────────────────

    /// File → Open Workspace… (folder dialog).
    pub fn open_workspace_dialog(&mut self) {
        let opts = notoir_platform::dialogs::FileDialogOptions {
            title: Some(crate::t!("dialogs:openWorkspace")),
            directory: self
                .workspace
                .as_ref()
                .and_then(|w| w.root.parent().map(Path::to_path_buf)),
            file_name: None,
        };
        self.files.dialogs.pick_folder(opts, self.io.poster());
    }

    /// Opens `path` as the workspace; asks first when one is already open.
    pub fn open_workspace(&mut self, path: &Path) {
        let root = match path.canonicalize() {
            Ok(p) if p.is_dir() => p,
            _ => {
                let now = self.now_ms;
                self.notifications.notify(
                    Notification::new(
                        Severity::Error,
                        Translatable::new("notifications:workspaceMissing")
                            .arg("path", path.display()),
                    ),
                    now,
                );
                self.remove_recent_workspace(&path.to_string_lossy());
                return;
            }
        };
        match &self.workspace {
            Some(ws) if ws.root == root => {
                self.panels.show(PanelId::Explorer);
            }
            Some(_) => {
                self.files
                    .prompts
                    .push_back(Prompt::WorkspaceReplace { path: root });
            }
            None => self.set_workspace(root),
        }
    }

    /// Opens a folder in a new Notoir window.
    pub fn open_workspace_in_new_window(&mut self, path: &Path) {
        match std::env::current_exe() {
            Ok(exe) => {
                if let Err(e) = std::process::Command::new(exe)
                    .arg("--new-window")
                    .arg(path)
                    .spawn()
                {
                    tracing::error!(error = %e, "could not start a new window");
                }
            }
            Err(e) => tracing::error!(error = %e, "no executable path"),
        }
    }

    fn set_workspace(&mut self, root: PathBuf) {
        self.ws_generation += 1;
        let generation = self.ws_generation;
        let (_, key) = notoir_workspace::fs_io::identity(&root);
        let key = key.as_str().to_owned();
        let trusted = self
            .state_db
            .as_ref()
            .is_some_and(|db| db.is_workspace_trusted(&key).unwrap_or(false));
        let excludes = self.settings.str_list("files.exclude", None);
        let filters = Arc::new(Filters::new(&root, &excludes));
        let poster = self.fs.poster();
        let watcher = match WorkspaceWatcher::start(&root, move |paths| {
            poster.post(IoResult::Workspace(WsResult {
                generation,
                event: WsEvent::Changed(paths),
            }));
        }) {
            Ok(w) => Some(w),
            Err(e) => {
                tracing::warn!(error = %e, root = %root.display(), "file watching unavailable");
                None
            }
        };
        tracing::info!(root = %root.display(), trusted, "workspace opened");
        self.workspace = Some(Workspace {
            name: name_of(&root),
            tree: ExplorerTree::new(root.clone()),
            root: root.clone(),
            key,
            trusted,
            generation,
            filters,
            index: Arc::new(Vec::new()),
            index_ready: false,
            index_cancel: Arc::new(AtomicBool::new(false)),
            index_due: None,
            settings: None,
            _watcher: watcher,
            filter: String::new(),
            filter_open: false,
            filter_focus: false,
            edit: None,
            clipboard: None,
            last_op: None,
            pending_reveal: None,
            scroll_to_selected: false,
        });
        self.request_listing(&root);
        self.start_index();
        // Workspace settings (`.notoir/settings.toml`) load off the UI thread.
        let file = root.join(".notoir").join("settings.toml");
        self.fs.submit(move || {
            let layer = match settings_file::load_settings_file(&file, None) {
                Ok(loaded) if file.exists() => Some(loaded.layer),
                _ => None,
            };
            Some(IoResult::Workspace(WsResult {
                generation,
                event: WsEvent::SettingsLoaded(layer),
            }))
        });
        self.record_recent_workspace(&root);
        self.panels.show(PanelId::Explorer);
        self.workbench.events.emit(AppEvent::WorkspaceOpened);
        if let Some(path) = self.active_file_path() {
            self.reveal_in_explorer(&path, false);
        }
    }

    /// File → Close Workspace: closes the workspace's files (prompting for
    /// unsaved ones); Scratchpads and outside files stay open.
    pub fn close_workspace(&mut self, next: Option<PathBuf>) {
        let Some(ws) = &self.workspace else {
            if let Some(n) = next {
                self.set_workspace(n);
            }
            return;
        };
        let root = ws.root.clone();
        let mut views = Vec::new();
        let mut dirty: Vec<DocumentId> = Vec::new();
        for (view, doc) in self.workbench.views.iter().map(|v| (v.id, v.document_id)) {
            let Some(d) = self.workbench.document(doc) else {
                continue;
            };
            if d.path().is_some_and(|p| p.starts_with(&root)) {
                views.push(view);
                if d.is_dirty() && !dirty.contains(&doc) {
                    dirty.push(doc);
                }
            }
        }
        let then = Then::CloseWorkspace { views, next };
        if dirty.is_empty() {
            self.run_close_workspace(then);
        } else {
            self.files
                .prompts
                .push_back(Prompt::Unsaved { docs: dirty, then });
        }
    }

    /// Second half of Close Workspace (after prompts).
    pub(crate) fn finish_close_workspace(&mut self, views: Vec<ViewId>, next: Option<PathBuf>) {
        for v in views {
            if self.workbench.view(v).is_some() {
                self.close_view_now(v);
            }
        }
        if self.workspace.take().is_some() {
            tracing::info!("workspace closed");
            self.workbench.events.emit(AppEvent::WorkspaceClosed);
            self.apply_resolved_settings();
        }
        if let Some(n) = next {
            self.set_workspace(n);
        }
    }

    fn run_close_workspace(&mut self, then: Then) {
        if let Then::CloseWorkspace { views, next } = then {
            self.finish_close_workspace(views, next);
        }
    }

    /// Marks the workspace trusted (restricted settings apply).
    pub fn trust_workspace(&mut self) {
        let Some(ws) = self.workspace.as_mut() else {
            return;
        };
        ws.trusted = true;
        if let Some(db) = self.state_db.as_ref() {
            let _ = db.set_workspace_trusted(&ws.key, true, now_epoch_ms());
        }
        self.apply_resolved_settings();
    }

    // ── recent workspaces ────────────────────────────────────────────────

    pub fn load_recent_workspaces(&mut self) {
        if let Some(db) = self.state_db.as_ref() {
            self.files.recent_workspaces = db.recent(RecentKind::Workspace).unwrap_or_default();
        }
    }

    fn record_recent_workspace(&mut self, root: &Path) {
        if let Some(db) = self.state_db.as_ref() {
            let _ = db.record_recent(
                RecentKind::Workspace,
                &root.to_string_lossy(),
                Some(&name_of(root)),
                now_epoch_ms(),
            );
        }
        self.load_recent_workspaces();
    }

    pub fn remove_recent_workspace(&mut self, path: &str) {
        if let Some(db) = self.state_db.as_ref() {
            let _ = db.remove_recent(RecentKind::Workspace, path);
        }
        self.load_recent_workspaces();
    }

    pub fn set_recent_workspace_pinned(&mut self, path: &str, pinned: bool) {
        if let Some(db) = self.state_db.as_ref() {
            let _ = db.set_recent_pinned(RecentKind::Workspace, path, pinned);
        }
        self.load_recent_workspaces();
    }

    pub fn recent_workspaces(&self) -> &[RecentEntry] {
        &self.files.recent_workspaces
    }

    // ── listing, index, watching ─────────────────────────────────────────

    pub fn request_listing(&mut self, dir: &Path) {
        let Some(ws) = self.workspace.as_mut() else {
            return;
        };
        if !ws.tree.mark_loading(dir) {
            return;
        }
        let filters = ws.filters.clone();
        let generation = ws.generation;
        let dir = dir.to_path_buf();
        self.fs.submit(move || {
            let entries = listing::list_dir(&dir, &filters).map_err(|e| e.to_string());
            Some(IoResult::Workspace(WsResult {
                generation,
                event: WsEvent::Listed { dir, entries },
            }))
        });
    }

    fn start_index(&mut self) {
        let Some(ws) = self.workspace.as_mut() else {
            return;
        };
        ws.index_cancel.store(true, Ordering::Relaxed);
        let cancel = Arc::new(AtomicBool::new(false));
        ws.index_cancel = cancel.clone();
        ws.index_due = None;
        let filters = ws.filters.clone();
        let generation = ws.generation;
        self.index_queue.submit(move || {
            let (entries, truncated) = listing::build_index(&filters, &cancel);
            (!cancel.load(Ordering::Relaxed)).then_some(IoResult::Workspace(WsResult {
                generation,
                event: WsEvent::Indexed { entries, truncated },
            }))
        });
    }

    /// Timers: index rebuild after file changes.
    pub fn workspace_tick(&mut self) {
        if let Some(due) = self.workspace.as_ref().and_then(|w| w.index_due)
            && self.now_ms >= due
        {
            self.start_index();
        }
    }

    pub fn workspace_next_due(&self) -> Option<u64> {
        self.workspace.as_ref().and_then(|w| w.index_due)
    }

    pub fn handle_workspace(&mut self, result: WsResult) {
        if self
            .workspace
            .as_ref()
            .is_none_or(|w| w.generation != result.generation)
        {
            return;
        }
        match result.event {
            WsEvent::Listed { dir, entries } => {
                let ws = self.workspace.as_mut().expect("checked");
                match entries {
                    Ok(e) => ws.tree.set_children(&dir, e),
                    Err(e) => {
                        tracing::debug!(dir = %dir.display(), error = %e, "listing failed");
                        ws.tree.listing_failed(&dir);
                    }
                }
                if let Some(p) = ws.pending_reveal.clone() {
                    self.reveal_in_explorer(&p, true);
                }
            }
            WsEvent::Indexed { entries, truncated } => {
                let ws = self.workspace.as_mut().expect("checked");
                if truncated {
                    tracing::warn!(entries = entries.len(), "workspace index truncated");
                }
                ws.index = Arc::new(entries);
                ws.index_ready = true;
            }
            WsEvent::Changed(paths) => self.handle_fs_changes(paths),
            WsEvent::SettingsLoaded(layer) => {
                let restricted: Vec<String> = layer
                    .as_ref()
                    .map(|l| {
                        l.values
                            .keys()
                            .filter(|k| self.schema.get(k).is_some_and(|d| d.restricted))
                            .cloned()
                            .collect()
                    })
                    .unwrap_or_default();
                let ws = self.workspace.as_mut().expect("checked");
                ws.settings = layer;
                let untrusted = !ws.trusted;
                self.apply_resolved_settings();
                if untrusted && !restricted.is_empty() {
                    let now = self.now_ms;
                    self.notifications.notify(
                        Notification::new(
                            Severity::Warning,
                            Translatable::new("notifications:workspaceTrust"),
                        )
                        .detail(restricted.join("\n"))
                        .action(
                            "notifications:trustWorkspace",
                            CommandInvocation::new("workspace.trust", InvocationSource::Internal),
                        )
                        .action(
                            "notifications:notNow",
                            CommandInvocation::new(
                                "workspace.trustLater",
                                InvocationSource::Internal,
                            ),
                        )
                        .source(NotificationSource::Settings)
                        .dedupe("workspace-trust"),
                        now,
                    );
                }
            }
            WsEvent::Op(outcome) => self.handle_op(*outcome),
        }
    }

    /// Watcher batch: re-list the loaded folders that changed.
    fn handle_fs_changes(&mut self, paths: Vec<PathBuf>) {
        let Some(ws) = self.workspace.as_mut() else {
            return;
        };
        let loaded: std::collections::HashSet<PathBuf> =
            ws.tree.loaded_dirs().into_iter().collect();
        let mut relist: Vec<PathBuf> = Vec::new();
        for p in &paths {
            for d in [p.parent().map(Path::to_path_buf), Some(p.clone())]
                .into_iter()
                .flatten()
            {
                if loaded.contains(&d) && !relist.contains(&d) {
                    relist.push(d);
                }
            }
        }
        ws.index_due = Some(self.now_ms + INDEX_DEBOUNCE_MS);
        for d in relist {
            self.relist(&d);
        }
    }

    /// Lists an already loaded directory again.
    fn relist(&mut self, dir: &Path) {
        let Some(ws) = self.workspace.as_mut() else {
            return;
        };
        let filters = ws.filters.clone();
        let generation = ws.generation;
        let dir = dir.to_path_buf();
        self.fs.submit(move || {
            let entries = listing::list_dir(&dir, &filters).map_err(|e| e.to_string());
            Some(IoResult::Workspace(WsResult {
                generation,
                event: WsEvent::Listed { dir, entries },
            }))
        });
    }

    /// Explorer → Refresh: re-reads settings filters, every loaded folder and
    /// the index.
    pub fn refresh_explorer(&mut self) {
        let excludes = self.settings.str_list("files.exclude", None);
        let Some(ws) = self.workspace.as_mut() else {
            return;
        };
        ws.filters = Arc::new(Filters::new(&ws.root, &excludes));
        for d in ws.tree.loaded_dirs() {
            self.relist(&d);
        }
        self.start_index();
    }

    // ── tree interaction ─────────────────────────────────────────────────

    pub fn toggle_dir(&mut self, dir: &Path) {
        let Some(ws) = self.workspace.as_mut() else {
            return;
        };
        ws.tree.selected = Some(dir.to_path_buf());
        if ws.tree.toggle(dir) {
            self.request_listing(dir);
        }
    }

    pub fn expand_dir(&mut self, dir: &Path) {
        let Some(ws) = self.workspace.as_mut() else {
            return;
        };
        if ws.tree.expand(dir) {
            self.request_listing(dir);
        }
    }

    fn active_file_path(&self) -> Option<PathBuf> {
        let doc = self.workbench.active_document()?;
        self.workbench.document(doc)?.path().map(Path::to_path_buf)
    }

    /// Selects `path` in the tree, expanding (and listing) its parents.
    pub fn reveal_in_explorer(&mut self, path: &Path, scroll: bool) {
        let Some(ws) = self.workspace.as_mut() else {
            return;
        };
        if !ws.contains(path) {
            return;
        }
        let need = ws.tree.reveal(path);
        let parents_known = path.parent().is_some_and(|p| {
            ws.tree
                .node(p)
                .is_some_and(|n| matches!(n.children, notoir_core::explorer::Children::Loaded(_)))
        });
        ws.pending_reveal = (!parents_known).then(|| path.to_path_buf());
        ws.scroll_to_selected |= scroll;
        for d in need {
            self.request_listing(&d);
        }
    }

    /// `explorer.autoReveal`: follows the active document.
    pub fn auto_reveal(&mut self, doc: Option<DocumentId>) {
        if !self.settings.bool("explorer.autoReveal", None) {
            return;
        }
        if let Some(path) = doc
            .and_then(|d| self.workbench.document(d))
            .and_then(|d| d.path())
            .map(Path::to_path_buf)
        {
            self.reveal_in_explorer(&path, true);
        }
    }

    // ── file operations ──────────────────────────────────────────────────

    /// Starts New File / New Folder / Rename in the tree.
    pub fn begin_edit(&mut self, kind: EditKind) {
        let Some(ws) = self.workspace.as_mut() else {
            return;
        };
        let text = match &kind {
            EditKind::Rename { path } => name_of(path),
            _ => String::new(),
        };
        let dir = match &kind {
            EditKind::NewFile { dir } | EditKind::NewFolder { dir } => Some(dir.clone()),
            EditKind::Rename { .. } => None,
        };
        ws.filter_open = false;
        ws.filter.clear();
        ws.edit = Some(InlineEdit {
            kind,
            text,
            error: None,
            focus: true,
        });
        if let Some(d) = dir {
            self.expand_dir(&d);
        }
    }

    /// Validates the inline editor text without touching the disk (the tree
    /// knows the folder's entries; the operation itself refuses existing
    /// names too).
    pub fn validate_edit(&self) -> Result<PathBuf, NameError> {
        let ws = self.workspace.as_ref().ok_or(NameError::Empty)?;
        let edit = ws.edit.as_ref().ok_or(NameError::Empty)?;
        let name = edit.text.trim();
        fs_ops::validate_name(name, TargetOs::current())?;
        let (dir, except) = match &edit.kind {
            EditKind::NewFile { dir } | EditKind::NewFolder { dir } => (dir.clone(), None),
            EditKind::Rename { path } => (
                path.parent().map_or(ws.root.clone(), Path::to_path_buf),
                Some(path.clone()),
            ),
        };
        let fold = notoir_workspace::fs_io::case_insensitive_fs();
        let same = |a: &str, b: &str| {
            if fold {
                a.to_lowercase() == b.to_lowercase()
            } else {
                a == b
            }
        };
        if let Some(notoir_core::explorer::Children::Loaded(kids)) =
            ws.tree.node(&dir).map(|n| &n.children)
        {
            let clash = kids
                .iter()
                .any(|k| Some(k) != except.as_ref() && same(&name_of(k), name));
            if clash {
                return Err(NameError::Exists);
            }
        }
        if except.as_ref().is_some_and(|e| name_of(e) == name) {
            // Unchanged name: nothing to do.
            return Err(NameError::Empty);
        }
        Ok(dir.join(name))
    }

    pub fn set_edit_text(&mut self, text: String) {
        let Some(edit) = self.workspace.as_mut().and_then(|w| w.edit.as_mut()) else {
            return;
        };
        edit.text = text;
        let error = match fs_ops::validate_name(edit.text.trim(), TargetOs::current()) {
            Err(NameError::Empty) | Ok(()) => None,
            Err(e) => Some(crate::t!(e.key())),
        };
        if let Some(edit) = self.workspace.as_mut().and_then(|w| w.edit.as_mut()) {
            edit.error = error;
        }
    }

    pub fn cancel_edit(&mut self) {
        if let Some(ws) = self.workspace.as_mut() {
            ws.edit = None;
        }
    }

    /// Enter in the inline editor.
    pub fn commit_edit(&mut self) {
        let target = self.validate_edit();
        let Some(ws) = self.workspace.as_mut() else {
            return;
        };
        let Some(edit) = ws.edit.clone() else { return };
        let target = match target {
            Ok(t) => t,
            Err(NameError::Empty)
                if matches!(edit.kind, EditKind::Rename { .. }) || edit.text.trim().is_empty() =>
            {
                ws.edit = None;
                return;
            }
            Err(e) => {
                if let Some(ed) = ws.edit.as_mut() {
                    ed.error = Some(crate::t!(e.key()));
                }
                return;
            }
        };
        ws.edit = None;
        match edit.kind {
            EditKind::NewFile { .. } => self.run_op(move || {
                fs_ops::create_file(&target).map(|()| OpOutcome {
                    created: vec![target.clone()],
                    open: Some(target.clone()),
                    select: Some(target.clone()),
                    undo: Some(UndoOp::Created(vec![target.clone()])),
                    ..Default::default()
                })
            }),
            EditKind::NewFolder { .. } => self.run_op(move || {
                fs_ops::create_dir(&target).map(|()| OpOutcome {
                    created: vec![target.clone()],
                    select: Some(target.clone()),
                    undo: Some(UndoOp::Created(vec![target.clone()])),
                    ..Default::default()
                })
            }),
            EditKind::Rename { path } => self.rename_path(path, target),
        }
    }

    fn rename_path(&mut self, from: PathBuf, to: PathBuf) {
        self.run_op(move || {
            fs_ops::rename(&from, &to).map(|()| OpOutcome {
                renamed: vec![(from.clone(), to.clone())],
                select: Some(to.clone()),
                undo: Some(UndoOp::Renamed {
                    from: from.clone(),
                    to: to.clone(),
                }),
                ..Default::default()
            })
        });
    }

    /// Runs a file operation on the workspace queue.
    fn run_op(&mut self, op: impl FnOnce() -> std::io::Result<OpOutcome> + Send + 'static) {
        let Some(ws) = self.workspace.as_ref() else {
            return;
        };
        let generation = ws.generation;
        self.fs.submit(move || {
            let outcome = op().unwrap_or_else(|e| OpOutcome {
                error: Some(("notifications:fileOpFailed".into(), e.to_string())),
                ..Default::default()
            });
            Some(IoResult::Workspace(WsResult {
                generation,
                event: WsEvent::Op(Box::new(outcome)),
            }))
        });
    }

    pub fn duplicate_path(&mut self, path: PathBuf) {
        self.run_op(move || {
            fs_ops::duplicate(&path).map(|copy| OpOutcome {
                created: vec![copy.clone()],
                select: Some(copy.clone()),
                undo: Some(UndoOp::Created(vec![copy])),
                ..Default::default()
            })
        });
    }

    /// Explorer Delete: asks, then moves to the trash.
    pub fn request_delete(&mut self, paths: Vec<PathBuf>) {
        if !paths.is_empty() {
            self.files.prompts.push_back(Prompt::Delete {
                paths,
                permanent: false,
            });
        }
    }

    pub(crate) fn delete_paths(&mut self, paths: Vec<PathBuf>, permanent: bool) {
        let dialogs = self.files.dialogs.clone();
        self.run_op(move || {
            if permanent {
                for p in &paths {
                    if p.is_dir() {
                        std::fs::remove_dir_all(p)?;
                    } else {
                        std::fs::remove_file(p)?;
                    }
                }
                return Ok(OpOutcome {
                    removed: paths,
                    ..Default::default()
                });
            }
            Ok(match dialogs.trash(&paths) {
                Ok(()) => OpOutcome {
                    removed: paths,
                    ..Default::default()
                },
                Err(e) => {
                    tracing::warn!(error = %e, "move to trash failed");
                    OpOutcome {
                        trash_failed: paths,
                        ..Default::default()
                    }
                }
            })
        });
    }

    /// Moves (or copies) `srcs` into folder `dir` (Explorer drag and drop,
    /// Paste, OS drops).
    pub fn transfer_into(&mut self, srcs: Vec<PathBuf>, dir: PathBuf, copy: bool) {
        self.run_op(move || {
            let mut out = OpOutcome::default();
            let mut pairs = Vec::new();
            for s in &srcs {
                let r = if copy {
                    fs_ops::copy_into(s, &dir)
                } else {
                    fs_ops::move_into(s, &dir)
                };
                match r {
                    Ok(t) if t == *s => {}
                    Ok(t) => {
                        if copy {
                            out.created.push(t.clone());
                        } else {
                            out.renamed.push((s.clone(), t.clone()));
                        }
                        pairs.push((s.clone(), t.clone()));
                        out.select = Some(t);
                    }
                    Err(e) => {
                        out.error = Some((
                            "notifications:fileOpFailed".into(),
                            format!("{}: {e}", s.display()),
                        ));
                    }
                }
            }
            if !pairs.is_empty() {
                out.undo = Some(if copy {
                    UndoOp::Created(pairs.into_iter().map(|(_, t)| t).collect())
                } else {
                    UndoOp::Moved(pairs)
                });
            }
            Ok(out)
        });
    }

    /// Drop of tree items onto a folder: confirms first when
    /// `explorer.confirmDragAndDrop` is on (copies never ask).
    pub fn request_transfer(&mut self, srcs: Vec<PathBuf>, dir: PathBuf, copy: bool) {
        let srcs: Vec<PathBuf> = srcs
            .into_iter()
            .filter(|s| copy || (s.parent() != Some(dir.as_path()) && !dir.starts_with(s)))
            .collect();
        if srcs.is_empty() {
            return;
        }
        if !copy && self.settings.bool("explorer.confirmDragAndDrop", None) {
            self.files.prompts.push_back(Prompt::Move { srcs, dir });
        } else {
            self.transfer_into(srcs, dir, copy);
        }
    }

    /// Explorer → Undo: reverts the last rename, move or create.
    pub fn undo_explorer_op(&mut self) {
        let Some(op) = self.workspace.as_mut().and_then(|w| w.last_op.take()) else {
            return;
        };
        match op {
            UndoOp::Renamed { from, to } => {
                self.run_op(move || {
                    fs_ops::rename(&to, &from).map(|()| OpOutcome {
                        renamed: vec![(to.clone(), from.clone())],
                        select: Some(from.clone()),
                        ..Default::default()
                    })
                });
            }
            UndoOp::Moved(pairs) => {
                self.run_op(move || {
                    let mut out = OpOutcome::default();
                    for (from, to) in pairs {
                        fs_ops::rename(&to, &from)?;
                        out.renamed.push((to, from));
                    }
                    Ok(out)
                });
            }
            UndoOp::Created(paths) => self.delete_paths(paths, false),
        }
    }

    fn handle_op(&mut self, out: OpOutcome) {
        let key = |p: &Path| notoir_workspace::fs_io::identity(p).1;
        let mut touched: Vec<PathBuf> = Vec::new();
        for (from, to) in &out.renamed {
            self.workbench.rename_paths(from, to, key);
            if let Some(ws) = self.workspace.as_mut() {
                ws.tree.rename(from, to);
            }
            touched.extend(from.parent().map(Path::to_path_buf));
            touched.extend(to.parent().map(Path::to_path_buf));
        }
        for p in &out.removed {
            self.workbench.mark_deleted(p);
            touched.extend(p.parent().map(Path::to_path_buf));
        }
        for p in &out.created {
            touched.extend(p.parent().map(Path::to_path_buf));
        }
        if let Some(ws) = self.workspace.as_mut() {
            if out.undo.is_some() {
                ws.last_op = out.undo.clone();
            }
            ws.index_due = Some(self.now_ms + INDEX_DEBOUNCE_MS);
        }
        let loaded: Vec<PathBuf> = self
            .workspace
            .as_ref()
            .map(|w| w.tree.loaded_dirs())
            .unwrap_or_default();
        touched.sort();
        touched.dedup();
        for d in touched.iter().filter(|d| loaded.contains(d)) {
            self.relist(d);
        }
        if let Some(sel) = &out.select {
            self.reveal_in_explorer(sel, true);
        }
        if let Some(path) = &out.open {
            self.open_path(path, crate::files::OpenAt::default());
        }
        if !out.trash_failed.is_empty() {
            self.files.prompts.push_back(Prompt::Delete {
                paths: out.trash_failed,
                permanent: true,
            });
        }
        if let Some((key, detail)) = out.error {
            let now = self.now_ms;
            self.notifications.notify(
                Notification::new(Severity::Error, Translatable::new(key)).detail(detail),
                now,
            );
        }
    }

    /// Paths for Explorer commands: the `path` argument, else the selection.
    pub fn explorer_target(&self, inv: &CommandInvocation) -> Option<PathBuf> {
        match inv.args.get("path").and_then(|v| v.as_str()) {
            Some(p) => Some(PathBuf::from(p)),
            None => self.workspace.as_ref()?.tree.selected.clone(),
        }
    }

    /// Title for the window (`name — Notoir`).
    pub fn workspace_name(&self) -> Option<&str> {
        self.workspace.as_ref().map(|w| w.name.as_str())
    }

    /// Documents opened from inside the workspace that have a view.
    pub fn is_workspace_file(&self, doc: DocumentId) -> bool {
        let Some(ws) = &self.workspace else {
            return false;
        };
        matches!(self.workbench.document(doc).map(|d| &d.origin), Some(DocumentOrigin::File { path, .. }) if path.starts_with(&ws.root))
    }
}
