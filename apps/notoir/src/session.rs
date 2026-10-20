//! Session persistence (docs/specs/23): per workspace (or "no workspace"),
//! the layout tree, tabs with cursors and scroll, panels, Explorer expanded
//! folders and zoom are stored as versioned JSON in the state DB, written on
//! the IO thread at most every 2 s, on focus loss and on exit.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use notoir_core::document::DocumentOrigin;
use notoir_core::ids::{DocumentId, GroupId, ViewId};
use notoir_core::layout::LayoutShape;
use notoir_core::notifications::{Notification, Severity, Translatable};
use notoir_core::panels::PanelLayout;
use notoir_core::view::ScrollPosition;
use notoir_core::workbench::TabPosition;
use notoir_editor::{Range, Selection};
use notoir_storage::state_db::StateDb;
use serde::{Deserialize, Serialize};

use crate::core::AppCore;
use crate::files::LoadPurpose;

pub const SESSION_VERSION: u32 = 1;
/// Session rows of windows without a workspace.
pub const NO_WORKSPACE_KEY: &str = "::no-workspace::0";
/// Debounce between session writes.
pub const WRITE_INTERVAL_MS: u64 = 2000;
/// Explorer expanded folders kept per workspace.
const MAX_EXPANDED: usize = 500;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum TabSource {
    File {
        path: String,
    },
    Scratchpad {
        id: String,
    },
    /// Untitled documents live in the recovery store under this entry id.
    Untitled {
        entry: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TabData {
    #[serde(flatten)]
    pub source: TabSource,
    #[serde(default)]
    pub pinned: bool,
    /// A preview tab (spec 04).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub preview: bool,
    /// (anchor, head) per range.
    #[serde(default)]
    pub selection: Vec<(usize, usize)>,
    #[serde(default)]
    pub primary: usize,
    #[serde(default)]
    pub top_line: usize,
    #[serde(default)]
    pub wrap: Option<bool>,
    #[serde(default)]
    pub zoom: Option<f32>,
    /// Folded regions as (header char, end char).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub folds: Vec<(usize, usize)>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GroupData {
    pub tabs: Vec<TabData>,
    pub active: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionData {
    pub v: u32,
    /// Workspace root as opened (the DB key may be case-folded).
    #[serde(default)]
    pub root: Option<String>,
    pub layout: LayoutShape,
    pub groups: Vec<GroupData>,
    #[serde(default)]
    pub active_group: usize,
    #[serde(default)]
    pub panels: Option<PanelLayout>,
    #[serde(default)]
    pub expanded: Vec<String>,
    #[serde(default)]
    pub zoom: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowGeometry {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    #[serde(default)]
    pub maximized: bool,
    #[serde(default)]
    pub fullscreen: bool,
}

/// Tabs still loading during a restore.
#[derive(Default)]
pub struct RestoreState {
    /// Tab keys per group in saved order.
    pub order: HashMap<GroupId, Vec<String>>,
    /// Saved active tab key per group.
    pub active: HashMap<GroupId, String>,
    pub active_group: Option<GroupId>,
    pub pending: usize,
    pub missing: Vec<String>,
}

#[derive(Default)]
pub struct SessionState {
    pub last_written: Option<(String, String)>,
    pub next_write: u64,
    pub restore: Option<RestoreState>,
    /// The window geometry (reported by the app each frame).
    pub geometry: Option<WindowGeometry>,
    pub last_geometry: Option<String>,
}

pub(crate) fn epoch_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

thread_local! {
    /// The IO thread's own state DB connection (opened on first use).
    static IO_DB: std::cell::RefCell<Option<StateDb>> = const { std::cell::RefCell::new(None) };
}

/// Runs `f` with the IO thread's DB connection.
fn with_io_db(path: &Path, f: impl FnOnce(&StateDb)) {
    IO_DB.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            match StateDb::open(path) {
                Ok((db, _)) => *slot = Some(db),
                Err(e) => {
                    tracing::warn!(error = %e, "state DB unavailable for session writes");
                    return;
                }
            }
        }
        if let Some(db) = slot.as_ref() {
            f(db);
        }
    });
}

/// Restorable window geometry for `main` (before the window exists).
pub fn load_geometry(data_dir: &Path) -> Option<WindowGeometry> {
    let path = data_dir.join("state.db");
    if !path.exists() {
        return None;
    }
    let (db, _) = StateDb::open(&path).ok()?;
    let (_, json) = db.load_window(0).ok()??;
    serde_json::from_str(&json).ok()
}

impl AppCore {
    /// The DB key of this window's session.
    pub fn session_key(&self) -> String {
        self.workspace
            .as_ref()
            .map_or_else(|| NO_WORKSPACE_KEY.to_owned(), |w| w.key.clone())
    }

    /// Sort key of a tab for restore ordering.
    fn tab_key(&self, doc: DocumentId) -> Option<String> {
        let d = self.workbench.document(doc)?;
        match &d.origin {
            DocumentOrigin::File { key, .. } => Some(format!("file:{}", key.as_str())),
            DocumentOrigin::Scratchpad { scratch_id, .. } => Some(format!("scratch:{scratch_id}")),
            DocumentOrigin::Untitled { .. } => self
                .recovery
                .entries
                .get(&doc)
                .map(|e| format!("untitled:{e}"))
                .or_else(|| {
                    self.recovery
                        .restored_untitled
                        .iter()
                        .find(|(_, d)| **d == doc)
                        .map(|(e, _)| format!("untitled:{e}"))
                }),
            DocumentOrigin::Virtual { .. } => None,
        }
    }

    /// The current session as data.
    pub fn capture_session(&self) -> SessionData {
        let wb = &self.workbench;
        let order = wb.layout.groups_in_order();
        let groups = order
            .iter()
            .map(|g| {
                let group = wb.layout.group(*g).expect("in order");
                let mut tabs = Vec::new();
                let mut active = None;
                for v in &group.tabs {
                    let Some(view) = wb.view(*v) else { continue };
                    let Some(doc) = wb.document(view.document_id) else {
                        continue;
                    };
                    let source = match &doc.origin {
                        DocumentOrigin::File { path, .. } => TabSource::File {
                            path: path.to_string_lossy().into_owned(),
                        },
                        DocumentOrigin::Scratchpad { scratch_id, .. } => TabSource::Scratchpad {
                            id: scratch_id.clone(),
                        },
                        DocumentOrigin::Untitled { .. } => match self.recovery.entries.get(&doc.id)
                        {
                            Some(e) => TabSource::Untitled { entry: e.clone() },
                            None => continue,
                        },
                        DocumentOrigin::Virtual { .. } => continue,
                    };
                    if group.active == Some(*v) {
                        active = Some(tabs.len());
                    }
                    tabs.push(TabData {
                        source,
                        pinned: view.pinned,
                        preview: view.preview,
                        selection: view
                            .selection
                            .ranges()
                            .iter()
                            .map(|r| (r.anchor, r.head))
                            .collect(),
                        primary: view.selection.primary_index(),
                        top_line: view.scroll.top_line,
                        wrap: view.wrap_override,
                        zoom: view.zoom,
                        folds: view.folds.iter().map(|f| (f.header, f.end)).collect(),
                    });
                }
                GroupData { tabs, active }
            })
            .collect();
        let mut expanded: Vec<String> = self
            .workspace
            .as_ref()
            .map(|w| {
                w.tree
                    .expanded_dirs()
                    .into_iter()
                    .filter(|p| *p != w.root)
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        expanded.truncate(MAX_EXPANDED);
        let panels = self
            .maximize_saved
            .clone()
            .unwrap_or_else(|| self.panels.clone());
        SessionData {
            v: SESSION_VERSION,
            root: self
                .workspace
                .as_ref()
                .map(|w| w.root.to_string_lossy().into_owned()),
            layout: wb.layout.shape(),
            groups,
            active_group: order
                .iter()
                .position(|g| *g == wb.layout.active_group)
                .unwrap_or(0),
            panels: Some(panels),
            expanded,
            zoom: Some(self.zoom),
        }
    }

    /// Writes the session if it changed (every 2 s, or now with `force`).
    pub fn write_session(&mut self, only_when_due: bool) {
        if only_when_due && self.now_ms < self.session.next_write {
            return;
        }
        self.session.next_write = self.now_ms + WRITE_INTERVAL_MS;
        if self.session.restore.is_some() {
            // Never overwrite a session that is still being restored.
            return;
        }
        let key = self.session_key();
        let Ok(json) = serde_json::to_string(&self.capture_session()) else {
            return;
        };
        let geometry = self
            .session
            .geometry
            .and_then(|g| serde_json::to_string(&g).ok());
        let changed = self.session.last_written.as_ref() != Some(&(key.clone(), json.clone()));
        let geometry_changed = geometry.is_some() && geometry != self.session.last_geometry;
        if !changed && !geometry_changed {
            return;
        }
        self.session.last_written = Some((key.clone(), json.clone()));
        self.session.last_geometry = geometry.clone();
        let path = self.paths.data.join("state.db");
        let ws_key = self.workspace.as_ref().map(|w| w.key.clone());
        self.io.submit(move || {
            with_io_db(&path, |db| {
                if changed && let Err(e) = db.save_session(&key, &json, epoch_ms()) {
                    tracing::warn!(error = %e, "could not save session");
                }
                let geometry = geometry.unwrap_or_else(|| "{}".to_owned());
                if let Err(e) = db.save_window(0, ws_key.as_deref(), &geometry, epoch_ms()) {
                    tracing::warn!(error = %e, "could not save window state");
                }
            });
            None
        });
    }

    /// Startup with `workbench.startup = restoreSession`: reopens the last
    /// workspace (which restores its session) or the no-workspace session.
    pub fn restore_last_session(&mut self) {
        let last = self
            .state_db
            .as_ref()
            .and_then(|db| db.load_window(0).ok().flatten())
            .and_then(|(key, _)| key);
        let root = last
            .as_deref()
            .and_then(|k| self.load_session_data(k))
            .and_then(|s| s.root);
        match root {
            Some(root) if Path::new(&root).is_dir() => self.open_workspace(Path::new(&root)),
            _ => {
                self.restore_hot_exit();
                if let Some(data) = self.load_session_data(NO_WORKSPACE_KEY) {
                    self.apply_session(data);
                }
            }
        }
    }

    pub fn load_session_data(&self, key: &str) -> Option<SessionData> {
        let json = self.state_db.as_ref()?.load_session(key).ok()??;
        match serde_json::from_str::<SessionData>(&json) {
            Ok(s) if s.v <= SESSION_VERSION => Some(s),
            Ok(_) => None,
            Err(e) => {
                tracing::warn!(error = %e, key, "discarding unreadable session");
                None
            }
        }
    }

    /// Rebuilds groups and tabs from a saved session. Files load in the
    /// background and slot into their saved position as they arrive.
    pub fn apply_session(&mut self, data: SessionData) {
        let Some(groups) = self.workbench.rebuild_layout(&data.layout) else {
            tracing::warn!("saved layout is invalid; keeping the current layout");
            return;
        };
        if let Some(p) = data.panels {
            self.panels = p;
            self.panels
                .ensure_registered(crate::core::REGISTERED_PANELS);
        }
        if let Some(z) = data.zoom {
            self.zoom = z;
        }
        if let Some(ws) = self.workspace.as_mut() {
            ws.pending_expand = data.expanded.iter().map(PathBuf::from).collect();
            let ready: Vec<PathBuf> = ws.tree.loaded_dirs();
            for d in ready {
                self.expand_pending_children(&d);
            }
        }
        let mut st = RestoreState {
            active_group: groups.get(data.active_group).copied(),
            ..Default::default()
        };
        let mut loads = Vec::new();
        for (gi, gdata) in data.groups.iter().enumerate() {
            let Some(&group) = groups.get(gi) else { break };
            let mut keys = Vec::new();
            for (ti, tab) in gdata.tabs.iter().enumerate() {
                let key = match &tab.source {
                    TabSource::File { path } => {
                        format!(
                            "file:{}",
                            notoir_workspace::fs_io::identity(Path::new(path))
                                .1
                                .as_str()
                        )
                    }
                    TabSource::Scratchpad { id } => format!("scratch:{id}"),
                    TabSource::Untitled { entry } => format!("untitled:{entry}"),
                };
                if gdata.active == Some(ti) {
                    st.active.insert(group, key.clone());
                }
                keys.push(key.clone());
                loads.push((group, key, tab.clone()));
            }
            st.order.insert(group, keys);
        }
        st.pending = loads.len();
        self.session.restore = Some(st);
        for (group, key, tab) in loads {
            let existing = match &tab.source {
                TabSource::File { path } => {
                    let k = notoir_workspace::fs_io::identity(Path::new(path)).1;
                    self.workbench.find_by_key(&k)
                }
                TabSource::Scratchpad { id } => {
                    self.open_scratchpad(id);
                    self.workbench
                        .documents
                        .values()
                        .find(|d| matches!(&d.origin, DocumentOrigin::Scratchpad { scratch_id, .. } if scratch_id == id))
                        .map(|d| d.id)
                }
                TabSource::Untitled { entry } => {
                    self.recovery.restored_untitled.get(entry).copied()
                }
            };
            match (existing, &tab.source) {
                (Some(doc), _) => self.place_restored(doc, group, &key, &tab),
                (None, TabSource::File { path }) => {
                    self.load_for(
                        PathBuf::from(path),
                        LoadPurpose::Restore {
                            group,
                            key,
                            tab: Box::new(tab.clone()),
                        },
                    );
                }
                (None, _) => self.restore_slot_done(None),
            }
        }
        self.finish_restore_if_done();
    }

    /// Puts a restored document's view at its saved place.
    pub fn place_restored(&mut self, doc: DocumentId, group: GroupId, key: &str, tab: &TabData) {
        let Some(st) = self.session.restore.as_ref() else {
            return;
        };
        let order = st.order.get(&group).cloned().unwrap_or_default();
        let my_pos = order.iter().position(|k| k == key).unwrap_or(usize::MAX);
        let group_tabs = self
            .workbench
            .layout
            .group(group)
            .map(|g| g.tabs.clone())
            .unwrap_or_default();
        let index = group_tabs
            .iter()
            .filter(|v| {
                self.workbench
                    .view(**v)
                    .and_then(|v| self.tab_key(v.document_id))
                    .and_then(|k| order.iter().position(|o| *o == k))
                    .is_some_and(|p| p < my_pos)
            })
            .count();
        // Reuse a view of this document (prefer one already in the group),
        // else open one. `move_view` takes a position in the strip as it is
        // before the move.
        let existing: Vec<ViewId> = self
            .workbench
            .views
            .of_document(doc)
            .map(|v| v.id)
            .collect();
        let reuse = existing
            .iter()
            .copied()
            .find(|v| self.workbench.layout.group_of(*v) == Some(group))
            .or_else(|| existing.first().copied());
        let view = match reuse {
            Some(v) => {
                let current = self
                    .workbench
                    .layout
                    .group(group)
                    .and_then(|g| g.index_of(v));
                let pre = match current {
                    Some(p) if p <= index => index + 1,
                    _ => index,
                };
                let _ = self.workbench.move_view(v, group, pre);
                Some(v)
            }
            None => self
                .workbench
                .open_view(doc, group, TabPosition::Index(index))
                .ok(),
        };
        if let Some(v) = view {
            let len = self
                .workbench
                .document(doc)
                .map_or(0, |d| d.buffer.len_chars());
            let ranges: Vec<Range> = tab
                .selection
                .iter()
                .map(|(a, h)| Range::new((*a).min(len), (*h).min(len)))
                .collect();
            if !ranges.is_empty() {
                let primary = tab.primary.min(ranges.len() - 1);
                let _ = self
                    .workbench
                    .set_selection(v, Selection::new(ranges, primary));
            }
            if let Some(view) = self.workbench.views.get_mut(v) {
                view.scroll = ScrollPosition {
                    top_line: tab.top_line,
                    ..Default::default()
                };
                view.wrap_override = tab.wrap;
                view.zoom = tab.zoom;
                view.preview = tab.preview;
                view.folds = tab
                    .folds
                    .iter()
                    .filter(|(h, e)| h < e && *e <= len)
                    .map(|&(header, end)| notoir_core::view::Fold { header, end })
                    .collect();
            }
            if tab.pinned {
                let _ = self.workbench.set_pinned(v, true);
            }
            let is_active = self
                .session
                .restore
                .as_ref()
                .is_some_and(|s| s.active.get(&group).map(String::as_str) == Some(key));
            if is_active && let Some(g) = self.workbench.layout.group_mut(group) {
                g.activate(v);
            }
        }
        self.restore_slot_done(None);
    }

    /// One saved tab is settled (`missing` = its file name when it failed).
    pub fn restore_slot_done(&mut self, missing: Option<String>) {
        if let Some(st) = self.session.restore.as_mut() {
            st.pending = st.pending.saturating_sub(1);
            if let Some(m) = missing {
                st.missing.push(m);
            }
        }
        self.finish_restore_if_done();
    }

    fn finish_restore_if_done(&mut self) {
        if self.session.restore.as_ref().is_none_or(|s| s.pending > 0) {
            return;
        }
        let st = self.session.restore.take().expect("checked");
        // Groups whose tabs all went missing are dropped.
        for g in self.workbench.layout.groups_in_order() {
            if self.workbench.layout.groups.len() > 1
                && self
                    .workbench
                    .layout
                    .group(g)
                    .is_some_and(|g| g.tabs.is_empty())
            {
                self.workbench.remove_group_if_empty(g);
            }
        }
        if let Some(g) = st
            .active_group
            .filter(|g| self.workbench.layout.group(*g).is_some())
        {
            self.workbench.layout.active_group = g;
            if let Some(v) = self.workbench.layout.group(g).and_then(|g| g.active) {
                let _ = self.workbench.activate_view(v);
            }
        }
        if !st.missing.is_empty() {
            let now = self.now_ms;
            self.notifications.notify(
                Notification::new(
                    Severity::Warning,
                    Translatable::new("notifications:sessionMissing")
                        .arg("count", st.missing.len()),
                )
                .detail(st.missing.join("\n")),
                now,
            );
        }
        tracing::info!("session restored");
    }
}
