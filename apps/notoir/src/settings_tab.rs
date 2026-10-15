//! Settings tab, Welcome page and Zen Mode state (docs/specs/14, 17, 20).
//! The Settings and Welcome tabs are virtual documents rendered by their own
//! views; settings changes apply immediately and are written to the file of
//! the selected scope.

use std::path::PathBuf;

use notoir_core::document::{DocumentOrigin, LoadedText};
use notoir_core::ids::{DocumentId, ViewId};
use notoir_core::keymap::{KeyChord, Keystroke};
use notoir_core::panels::PanelLayout;
use notoir_core::settings::{SettingCategory, SettingValue};
use notoir_core::view::ViewKind;
use notoir_core::workbench::TabPosition;
use notoir_editor::LanguageId;
use notoir_storage::settings_file;

use crate::core::{AppCore, Effect};
use crate::io::IoResult;

/// i18n keys used as virtual document titles.
pub const SETTINGS_TITLE: &str = "tabs:settings";
pub const WELCOME_TITLE: &str = "tabs:welcome";
/// State DB notice recording that the first-run page was completed.
pub const FIRST_RUN_DONE: &str = "firstRun.completed";

/// Settings UI state (search, scope, category, shortcut recording).
#[derive(Default)]
pub struct SettingsUi {
    pub search: String,
    /// Edit the Workspace file instead of the User file.
    pub workspace_scope: bool,
    pub category: Option<SettingCategory>,
    pub kb_search: String,
    /// Recording a new chord for this command.
    pub recording: Option<(String, Vec<Keystroke>)>,
    pub focus_search: bool,
}

/// Panels, maximize and full-screen state before Zen Mode.
#[derive(Clone, Debug)]
pub struct ZenState {
    pub panels: PanelLayout,
    pub was_maximized: bool,
    pub was_fullscreen: bool,
}

pub const CATEGORIES: &[SettingCategory] = &[
    SettingCategory::General,
    SettingCategory::Editor,
    SettingCategory::Appearance,
    SettingCategory::Files,
    SettingCategory::Workspace,
    SettingCategory::Git,
    SettingCategory::Mcp,
    SettingCategory::Plugins,
    SettingCategory::Keyboard,
    SettingCategory::Advanced,
];

pub fn category_key(c: SettingCategory) -> &'static str {
    match c {
        SettingCategory::General => "settings:category.general",
        SettingCategory::Editor => "settings:category.editor",
        SettingCategory::Appearance => "settings:category.appearance",
        SettingCategory::Files => "settings:category.files",
        SettingCategory::Workspace => "settings:category.workspace",
        SettingCategory::Git => "settings:category.git",
        SettingCategory::Mcp => "settings:category.mcp",
        SettingCategory::Plugins => "settings:category.plugins",
        SettingCategory::Keyboard => "settings:category.keyboard",
        SettingCategory::Advanced => "settings:category.advanced",
    }
}

impl AppCore {
    fn virtual_doc(&self, title: &str) -> Option<DocumentId> {
        self.workbench
            .documents
            .values()
            .find(|d| matches!(&d.origin, DocumentOrigin::Virtual { title: t } if t == title))
            .map(|d| d.id)
    }

    /// Opens (or focuses) a virtual tab rendered by `kind`.
    fn open_virtual(&mut self, title: &str, kind: ViewKind) -> Option<ViewId> {
        let existing = self
            .virtual_doc(title)
            .and_then(|doc| self.workbench.views.of_document(doc).next().map(|v| v.id));
        if let Some(view) = existing {
            let _ = self.workbench.activate_view(view);
            return Some(view);
        }
        let doc = self.workbench.open_document(
            DocumentOrigin::Virtual {
                title: title.to_owned(),
            },
            LoadedText::empty(LanguageId::PLAINTEXT),
        );
        let group = self.workbench.layout.active_group;
        let view = self
            .workbench
            .open_view(doc, group, TabPosition::AfterActive)
            .ok()?;
        if let Some(v) = self.workbench.views.get_mut(view) {
            v.kind = kind;
        }
        let _ = self.workbench.activate_view(view);
        Some(view)
    }

    /// `app.settings` (Mod+,): the Settings tab, optionally at a category.
    pub fn open_settings(&mut self, category: Option<SettingCategory>) {
        self.exit_zen();
        self.open_virtual(SETTINGS_TITLE, ViewKind::Settings);
        if category.is_some() {
            self.settings_ui.category = category;
            self.settings_ui.search.clear();
        }
        self.settings_ui.focus_search = true;
    }

    /// Help → Welcome, and the first launch.
    pub fn open_welcome(&mut self) {
        self.open_virtual(WELCOME_TITLE, ViewKind::Welcome);
    }

    /// First launch: no settings file and the page was never completed.
    pub fn maybe_first_run(&mut self) {
        let done = self
            .state_db
            .as_ref()
            .is_some_and(|db| db.is_notice_dismissed(FIRST_RUN_DONE).unwrap_or(false));
        if !done && !self.paths.user_settings_file().exists() {
            self.open_welcome();
        }
    }

    /// Skip / Start writing on the Welcome page.
    pub fn finish_welcome(&mut self) {
        if let Some(db) = self.state_db.as_ref() {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis() as i64);
            let _ = db.dismiss_notice(FIRST_RUN_DONE, now);
        }
        if let Some(doc) = self.virtual_doc(WELCOME_TITLE) {
            let views: Vec<ViewId> = self
                .workbench
                .views
                .of_document(doc)
                .map(|v| v.id)
                .collect();
            for v in views {
                self.close_view_now(v);
            }
        }
    }

    /// Writes a setting at Workspace scope (`.notoir/settings.toml` is created
    /// on the first write).
    pub fn set_workspace_setting(&mut self, key: &str, value: Option<SettingValue>) {
        let Some(ws) = self.workspace.as_mut() else {
            return;
        };
        let layer = ws.settings.get_or_insert_with(Default::default);
        match &value {
            Some(v) => {
                layer.set(key, v.clone());
            }
            None => {
                layer.values.remove(key);
            }
        }
        let path: PathBuf = ws.root.join(".notoir").join("settings.toml");
        self.apply_resolved_settings();
        let key = key.to_owned();
        self.io.submit(move || {
            if value.is_some()
                && let Some(dir) = path.parent()
            {
                let _ = std::fs::create_dir_all(dir);
            }
            settings_file::write_setting_file(&path, &key, None, value.as_ref())
                .err()
                .map(|e| IoResult::SettingsWriteFailed {
                    path: path.display().to_string(),
                    error: e.to_string(),
                })
        });
    }

    /// Writes a setting at the scope selected in the Settings tab.
    pub fn set_setting_in_scope(&mut self, key: &str, value: Option<SettingValue>) {
        if self.settings_ui.workspace_scope && self.workspace.is_some() {
            self.set_workspace_setting(key, value);
        } else {
            self.set_user_setting(key, value);
        }
    }

    /// Value stored at the selected scope (not the effective one).
    pub fn scope_value(&self, key: &str) -> Option<SettingValue> {
        if self.settings_ui.workspace_scope {
            self.workspace
                .as_ref()?
                .settings
                .as_ref()?
                .values
                .get(key)
                .cloned()
        } else {
            self.user_settings.values.get(key).cloned()
        }
    }

    // ── keyboard shortcuts ───────────────────────────────────────────────

    /// Appends a binding (or a `-command` removal) to `keybindings.toml`; the
    /// file reload rebuilds the keymap.
    pub fn write_keybinding(&mut self, chord: Option<&KeyChord>, command: &str) {
        let path = self.paths.keybindings_file();
        let key = chord.map(|c| c.to_config(self.platform));
        let command = command.to_owned();
        let fs_poster = self.fs.poster();
        self.io.submit(move || {
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            let result = settings_file::append_keybinding(&text, key.as_deref(), &command, None)
                .map_err(|e| e.to_string())
                .and_then(|t| {
                    if let Some(dir) = path.parent() {
                        let _ = std::fs::create_dir_all(dir);
                    }
                    notoir_storage::atomic::write(&path, t.as_bytes(), Default::default())
                        .map_err(|e| e.to_string())
                });
            match result {
                Ok(_) => {
                    // Picked up even when the folder watch is unavailable.
                    fs_poster.post(IoResult::DocsChanged(vec![path]));
                    None
                }
                Err(e) => Some(IoResult::SettingsWriteFailed {
                    path: path.display().to_string(),
                    error: e,
                }),
            }
        });
    }

    // ── Zen Mode ─────────────────────────────────────────────────────────

    /// `view.zenMode` (Mod+K Z): hides everything but the active document.
    pub fn toggle_zen(&mut self, fullscreen_now: bool) {
        if self.zen.is_some() {
            self.exit_zen();
            return;
        }
        let was_maximized = self.workbench.layout.maximized.is_some();
        self.zen = Some(ZenState {
            panels: self
                .maximize_saved
                .clone()
                .unwrap_or_else(|| self.panels.clone()),
            was_maximized,
            was_fullscreen: fullscreen_now,
        });
        self.panels.hide_all();
        if !was_maximized {
            let g = self.workbench.layout.active_group;
            self.workbench.toggle_maximize(g);
        }
        if self.settings.bool("zenMode.fullScreen", None) && !fullscreen_now {
            self.effects.push(Effect::SetFullScreen(true));
        }
        if let Some(v) = self.workbench.active_view() {
            self.effects.push(Effect::FocusEditor(v));
        }
    }

    /// Leaves Zen Mode; panels, maximize and full screen return exactly.
    pub fn exit_zen(&mut self) {
        let Some(z) = self.zen.take() else { return };
        if !z.was_maximized
            && let Some(g) = self.workbench.layout.maximized
        {
            self.workbench.toggle_maximize(g);
        }
        if !z.was_maximized {
            self.panels = z.panels;
        }
        if !z.was_fullscreen && self.settings.bool("zenMode.fullScreen", None) {
            self.effects.push(Effect::SetFullScreen(false));
        }
    }
}
