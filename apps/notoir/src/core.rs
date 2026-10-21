//! `AppCore`: all mutable application state, owned by the UI thread
//! (docs/specs/01-architecture.md §Application State Ownership). Command
//! handlers mutate it; anything that needs the egui context is requested as an
//! [`Effect`] and applied by `NotoirApp` at the end of the frame.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use notoir_core::command::{CommandInvocation, CommandOutcome, CommandQueue, CommandRegistry};
use notoir_core::keymap::{KeyBinding, Keymap, Platform, UserBinding};
use notoir_core::notifications::{
    Notification, NotificationCenter, NotificationSource, Severity, Translatable,
};
use notoir_core::panels::{PanelId, PanelLayout};
use notoir_core::settings::{ResolvedSettings, Schema, SettingValue, SettingsLayer};
use notoir_core::when::ContextSnapshot;
use notoir_core::workbench::Workbench;
use notoir_platform::AppPaths;
use notoir_storage::settings_file;
use notoir_storage::state_db::StateDb;

use crate::i18n::Language;
use crate::io::{IoQueue, IoResult};
use crate::theme::ThemeId;

/// Panels that exist in this build; others are registered by their milestones.
pub const REGISTERED_PANELS: &[PanelId] = &[
    PanelId::Explorer,
    PanelId::Search,
    PanelId::SourceControl,
    PanelId::Outline,
    PanelId::Backlinks,
    PanelId::Tags,
    PanelId::Plugins,
    PanelId::Output,
    PanelId::GitOutput,
    PanelId::McpActivity,
    PanelId::PluginOutput,
];

/// Theme setting: a built-in theme, a custom theme (by id, from
/// `<config>/themes/`) or "auto" (follow the OS).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ThemeChoice {
    Fixed(ThemeId),
    Custom(String),
    Auto,
}

impl ThemeChoice {
    pub fn from_setting(value: &str) -> Option<ThemeChoice> {
        let value = value.trim();
        if value == "auto" {
            Some(ThemeChoice::Auto)
        } else if let Some(t) = ThemeId::from_id(value) {
            Some(ThemeChoice::Fixed(t))
        } else {
            (!value.is_empty()).then(|| ThemeChoice::Custom(value.to_owned()))
        }
    }

    pub fn setting_value(&self) -> String {
        match self {
            ThemeChoice::Fixed(t) => t.id().to_owned(),
            ThemeChoice::Custom(id) => id.clone(),
            ThemeChoice::Auto => "auto".to_owned(),
        }
    }

    /// The theme id to show.
    pub fn resolve(&self, system_dark: bool) -> String {
        match self {
            ThemeChoice::Fixed(t) => t.id().to_owned(),
            ThemeChoice::Custom(id) => id.clone(),
            ThemeChoice::Auto => ThemeId::for_system(system_dark).id().to_owned(),
        }
    }
}

/// Requests that need the egui context.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    ApplyTheme,
    ApplyLanguage,
    ToggleFullScreen,
    Quit,
    OpenPicker(crate::shell::overlays::PickerKind),
    ShowAbout,
    ToggleNotificationCentre,
    FocusWindow,
    /// Put keyboard focus in this view's editor.
    FocusEditor(notoir_core::ids::ViewId),
    CopyText(String),
    /// Enter (true) or leave full screen (Zen Mode).
    SetFullScreen(bool),
}

/// How the app starts (resolved by `main`, or by tests).
#[derive(Clone)]
pub struct StartOptions {
    pub paths: AppPaths,
    pub os_locale: Option<String>,
    pub platform: Platform,
    /// Env overrides (`NOTOIR_LANG`, `NOTOIR_THEME`): not persisted.
    pub language_override: Option<Language>,
    pub theme_override: Option<ThemeId>,
    /// Tests: block on the CJK system font lookup instead of loading it in the
    /// background.
    pub wait_for_fonts: bool,
    /// File dialogs (native unless a test scripts them).
    pub dialogs: Option<std::sync::Arc<dyn crate::files::Dialogs>>,
    /// Git backend (git2 unless a test replaces it).
    pub git_backend: Option<std::sync::Arc<dyn notoir_git::GitBackend>>,
    /// Network Git (system git unless a test replaces it).
    pub git_remote: Option<std::sync::Arc<dyn notoir_git::GitRemote>>,
    /// Administrator authorization (native unless a test scripts it).
    pub elevation: Option<std::sync::Arc<dyn notoir_platform::elevation::Elevation>>,
    /// Files from the command line, with optional `--goto` line/column.
    pub open_paths: Vec<(std::path::PathBuf, Option<(usize, usize)>)>,
    /// Restore the last session (`workbench.startup = restoreSession`, no
    /// command-line paths, not `--new-window`).
    pub restore_session: bool,
    /// Show the first-run page when no settings file exists.
    pub first_run: bool,
    /// Update manifest URL and key (built in for release builds; tests set
    /// their own).
    pub update: Option<notoir_platform::update::UpdateConfig>,
}

impl StartOptions {
    pub fn new(paths: AppPaths) -> Self {
        Self {
            paths,
            os_locale: None,
            platform: Platform::current(),
            language_override: None,
            theme_override: None,
            wait_for_fonts: false,
            dialogs: None,
            elevation: None,
            git_backend: None,
            git_remote: None,
            open_paths: Vec::new(),
            restore_session: false,
            first_run: false,
            update: notoir_platform::update::UpdateConfig::built_in(),
        }
    }
}

pub struct AppCore {
    pub paths: AppPaths,
    pub platform: Platform,
    pub workbench: Workbench,
    pub schema: Arc<Schema>,
    pub user_settings: SettingsLayer,
    pub settings: ResolvedSettings,
    pub commands: CommandRegistry<AppCore>,
    pub keymap: Keymap,
    pub queue: CommandQueue,
    pub panels: PanelLayout,
    pub notifications: NotificationCenter,
    pub state_db: Option<StateDb>,
    pub io: IoQueue,
    pub language: Language,
    pub theme_choice: ThemeChoice,
    /// Valid themes from `<config>/themes/` (spec 21).
    pub custom_themes: Vec<crate::theme::Theme>,
    /// Bumped whenever `custom_themes` is reloaded.
    pub themes_generation: u64,
    /// Theme override from the environment (wins over the setting).
    pub theme_override: Option<ThemeId>,
    pub effects: Vec<Effect>,
    /// Milliseconds since app start (monotonic clock for timers).
    pub now_ms: u64,
    /// Global editor zoom in percent (`editor.zoomScope = "global"`).
    pub zoom: u32,
    /// The editor widget had keyboard focus last frame.
    pub editor_focused: bool,
    pub files: crate::files::FileState,
    /// Panels as they were before Maximize Editor Group (restored exactly).
    pub maximize_saved: Option<PanelLayout>,
    /// The open workspace (one per window).
    pub workspace: Option<crate::workspace::Workspace>,
    /// Bumped per opened workspace; stale background results are dropped.
    pub ws_generation: u64,
    /// Workspace listings and file operations (ordered).
    pub fs: IoQueue,
    /// The workspace index walk (long-running, separate from listings).
    pub index_queue: IoQueue,
    /// Git operations, one at a time (spec 28 per-repo queue).
    pub git_queue: IoQueue,
    pub git: crate::git::GitState,
    /// The Explorer tree had keyboard focus last frame.
    pub explorer_focused: bool,
    pub recovery: crate::recovery::RecoveryState,
    pub session: crate::session::SessionState,
    /// Banners above editors (external changes, recovered conflicts).
    pub banners: std::collections::HashMap<notoir_core::ids::DocumentId, crate::recovery::Banner>,
    /// Documents with a save in flight (watcher events are ignored for them).
    pub saving: std::collections::HashSet<notoir_core::ids::DocumentId>,
    pub autosave_due: std::collections::HashMap<notoir_core::ids::DocumentId, u64>,
    pub doc_watch: crate::external::DocWatch,
    pub find: crate::find::FindState,
    /// Commands run with a captured context (the Command Palette runs them
    /// with the context of the moment it opened).
    pub queue_with_context: Vec<(CommandInvocation, ContextSnapshot)>,
    pub settings_ui: crate::settings_tab::SettingsUi,
    pub markdown: crate::markdown::MarkdownState,
    pub json: crate::json::JsonState,
    pub structure: crate::structure::StructureState,
    pub nav: crate::navigation::NavState,
    pub search: crate::search::SearchState,
    pub compare: crate::compare::CompareState,
    pub elevation: std::sync::Arc<dyn notoir_platform::elevation::Elevation>,
    /// Selection → Column Selection Mode.
    pub column_mode: bool,
    /// Expand Selection steps per view (before, after), for Shrink.
    pub expand_history: std::collections::HashMap<
        notoir_core::ids::ViewId,
        Vec<(notoir_editor::Selection, notoir_editor::Selection)>,
    >,
    /// `breadcrumbs.focus`: open the last breadcrumb's dropdown next frame.
    pub breadcrumbs_focus: bool,
    /// Zen Mode is on (with what to restore).
    pub zen: Option<crate::settings_tab::ZenState>,
    /// Show the first-run page at startup (production launches).
    pub first_run_check: bool,
    /// MCP server, clients, permissions and activity (spec 29 / 32).
    pub mcp: crate::mcp::McpState,
    /// Lua plugins (spec 30).
    pub plugins: crate::plugins::PluginsState,
    /// `keybindings.toml` as last loaded (the keymap is rebuilt when plugin
    /// bindings change).
    pub user_bindings: Vec<UserBinding>,
    /// `notoir --wait` invocations waiting for their files to close.
    pub cli_waits: Vec<CliWait>,
    /// Wiki links, backlinks and tags (spec 34).
    pub notes: crate::notes::NotesState,
    pub text_index: crate::text_index::TextIndexState,
    pub templates: crate::templates::TemplatesState,
    pub updates: crate::updates::UpdatesState,
}

/// A `notoir --wait` request (spec 38).
pub struct CliWait {
    keys: Vec<notoir_core::document::PathKey>,
    waiter: Option<notoir_platform::single_instance::Waiter>,
    /// One of the files had a tab.
    seen: bool,
    since: u64,
}

/// A waited request whose files never opened (folders only, load errors)
/// is released after this long.
const CLI_WAIT_UNSEEN_MS: u64 = 30_000;

impl AppCore {
    /// Releases `--wait` terminals whose files no longer have tabs.
    pub fn cli_wait_tick(&mut self) {
        let now = self.now_ms;
        let wb = &self.workbench;
        for w in &mut self.cli_waits {
            let open = w.keys.iter().any(|k| {
                wb.find_by_key(k)
                    .is_some_and(|d| wb.views.of_document(d).any(|v| !v.detached))
            });
            if open {
                w.seen = true;
            } else if (w.seen || now.saturating_sub(w.since) > CLI_WAIT_UNSEEN_MS)
                && let Some(waiter) = w.waiter.take()
            {
                waiter.finish();
            }
        }
        self.cli_waits.retain(|w| w.waiter.is_some());
    }
}

impl AppCore {
    pub fn new(opts: &StartOptions, wake: impl Fn() + Send + Sync + 'static) -> Self {
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(wake);
        let paths = opts.paths.clone();
        if let Err(e) = paths.ensure_dirs() {
            tracing::error!(error = %e, "could not create app directories");
        }
        let mut notifications = NotificationCenter::default();

        let schema = Arc::new(Schema::builtin());
        let user_settings =
            match settings_file::load_settings_file(&paths.user_settings_file(), None) {
                Ok(loaded) => {
                    if let Some(e) = &loaded.error {
                        notifications.notify(
                            Notification::new(
                                Severity::Warning,
                                Translatable::new("notifications:settingsParse")
                                    .arg("line", e.line),
                            )
                            .detail(e.to_string())
                            .source(NotificationSource::Settings),
                            0,
                        );
                    }
                    loaded.layer
                }
                Err(e) => {
                    tracing::error!(error = %e, "could not read settings file");
                    SettingsLayer::default()
                }
            };
        let settings = ResolvedSettings::new(schema.clone(), user_settings.clone(), None, false);

        let user_bindings = load_keybindings(&paths.keybindings_file(), &mut notifications);

        let state_db = match StateDb::open(&paths.data.join("state.db")) {
            Ok((db, report)) => {
                if report.corrupt_backup.is_some() {
                    notifications.notify(
                        Notification::new(
                            Severity::Warning,
                            Translatable::new("notifications:stateDbCorrupt"),
                        )
                        .source(NotificationSource::Storage),
                        0,
                    );
                }
                if report.newer_version.is_some() {
                    notifications.notify(
                        Notification::new(
                            Severity::Info,
                            Translatable::new("notifications:stateDbNewer"),
                        )
                        .source(NotificationSource::Storage),
                        0,
                    );
                }
                Some(db)
            }
            Err(e) => {
                tracing::error!(error = %e, "state database unavailable");
                None
            }
        };

        let language = opts.language_override.unwrap_or_else(|| {
            let stored = settings.str("language", None);
            Language::detect(
                opts.os_locale.as_deref(),
                (!stored.is_empty()).then_some(stored),
            )
        });
        let theme_choice = ThemeChoice::from_setting(settings.str("workbench.theme", None))
            .unwrap_or(ThemeChoice::Fixed(ThemeId::Midnight));

        let mcp = crate::mcp::McpState::new(wake.clone());
        let plugins = crate::plugins::PluginsState::new(wake.clone());
        let mut commands = CommandRegistry::default();
        crate::commands::register_all(&mut commands, opts.platform);
        let keymap = build_keymap(
            &commands,
            &[],
            &user_bindings,
            opts.platform,
            &mut notifications,
        );

        Self {
            paths,
            platform: opts.platform,
            workbench: Workbench::new(),
            schema,
            user_settings,
            settings,
            commands,
            keymap,
            queue: CommandQueue::default(),
            panels: PanelLayout::new(REGISTERED_PANELS),
            notifications,
            state_db,
            io: IoQueue::start({
                let w = wake.clone();
                move || w()
            }),
            fs: IoQueue::start({
                let w = wake.clone();
                move || w()
            }),
            git_queue: IoQueue::start({
                let w = wake.clone();
                move || w()
            }),
            git: {
                let mut g = crate::git::GitState::new(
                    opts.git_backend
                        .clone()
                        .unwrap_or_else(|| std::sync::Arc::new(notoir_git::Git2Backend)),
                );
                g.remote = opts.git_remote.clone();
                g
            },
            index_queue: IoQueue::start(move || wake()),
            workspace: None,
            ws_generation: 0,
            explorer_focused: false,
            recovery: crate::recovery::RecoveryState::new(opts.paths.recovery.clone()),
            session: crate::session::SessionState::default(),
            banners: Default::default(),
            saving: Default::default(),
            autosave_due: Default::default(),
            doc_watch: Default::default(),
            find: Default::default(),
            queue_with_context: Vec::new(),
            settings_ui: Default::default(),
            markdown: Default::default(),
            json: Default::default(),
            structure: Default::default(),
            nav: Default::default(),
            search: Default::default(),
            compare: Default::default(),
            elevation: opts.elevation.clone().unwrap_or_else(|| {
                std::sync::Arc::new(notoir_platform::elevation::NativeElevation)
            }),
            column_mode: false,
            expand_history: Default::default(),
            breadcrumbs_focus: false,
            zen: None,
            first_run_check: opts.first_run,
            mcp,
            plugins,
            user_bindings: user_bindings.clone(),
            cli_waits: Vec::new(),
            notes: Default::default(),
            text_index: Default::default(),
            templates: Default::default(),
            updates: crate::updates::UpdatesState::new(opts.update.clone()),
            language,
            theme_choice,
            custom_themes: Vec::new(),
            themes_generation: 0,
            theme_override: opts.theme_override,
            effects: Vec::new(),
            now_ms: 0,
            zoom: 100,
            editor_focused: false,
            maximize_saved: None,
            files: crate::files::FileState::new(
                opts.dialogs
                    .clone()
                    .unwrap_or_else(|| std::sync::Arc::new(crate::files::NativeDialogs)),
                opts.paths.scratchpads.clone(),
            ),
        }
    }

    /// Id of the theme to show given the OS preference.
    pub fn theme(&self, system_dark: bool) -> String {
        match self.theme_override {
            Some(t) => t.id().to_owned(),
            None => self.theme_choice.resolve(system_dark),
        }
    }

    /// A theme by id: built-in, or a loaded custom theme; an unknown id
    /// falls back to Notoir Midnight.
    pub fn load_theme(&self, id: &str) -> crate::theme::Theme {
        ThemeId::from_id(id)
            .map(ThemeId::load)
            .or_else(|| self.custom_themes.iter().find(|t| t.id == id).cloned())
            .unwrap_or_else(|| ThemeId::Midnight.load())
    }

    pub fn toolbar_visible(&self) -> bool {
        self.settings.bool("workbench.toolbar.visible", None)
    }

    /// Maximize Editor Group: hides other groups and every panel; toggling
    /// again restores the panels exactly.
    pub fn toggle_maximize(&mut self) {
        if self.workbench.layout.maximized.is_some() {
            self.exit_maximize();
            return;
        }
        let group = self.workbench.layout.active_group;
        self.maximize_saved = Some(self.panels.clone());
        self.panels.hide_all();
        self.workbench.toggle_maximize(group);
    }

    /// Leaves maximized mode (when a panel or another group is requested).
    pub fn exit_maximize(&mut self) {
        // Showing a panel or another group also ends Zen Mode.
        self.exit_zen();
        if let Some(g) = self.workbench.layout.maximized {
            self.workbench.toggle_maximize(g);
        }
        if let Some(saved) = self.maximize_saved.take() {
            self.panels = saved;
        }
    }

    /// Word wrap for a view: its override, else the language setting (off in
    /// large-file mode unless overridden).
    pub fn wrap_enabled(&self, view: notoir_core::ids::ViewId) -> bool {
        let Some(v) = self.workbench.view(view) else {
            return false;
        };
        if let Some(on) = v.wrap_override {
            return on;
        }
        let doc = self.workbench.document(v.document_id);
        if doc.is_some_and(|d| d.large_file) {
            return false;
        }
        let lang = doc.map(|d| d.language.as_str());
        self.settings.str("editor.wordWrap", lang) != "off"
    }

    /// Effective zoom factor for a view.
    pub fn zoom_for(&self, view: notoir_core::ids::ViewId) -> f32 {
        let per_view = self.settings.str("editor.zoomScope", None) == "view";
        match self.workbench.view(view).and_then(|v| v.zoom) {
            Some(z) if per_view => z,
            _ => self.zoom as f32 / 100.0,
        }
    }

    /// Indentation for a document: detected (when `editor.detectIndentation`)
    /// or from settings.
    pub fn indent_style(
        &self,
        doc: notoir_core::ids::DocumentId,
    ) -> notoir_editor::edit_ops::IndentStyle {
        let d = self.workbench.document(doc);
        if let Some(style) = d.and_then(|d| d.indent_override) {
            return style;
        }
        let lang = d.map(|d| d.language.as_str());
        if self.settings.bool("editor.detectIndentation", lang)
            && let Some(style) = d.and_then(|d| d.indentation)
        {
            return style;
        }
        notoir_editor::edit_ops::IndentStyle {
            tab_size: self.settings.int("editor.tabSize", lang).max(1) as usize,
            insert_spaces: self.settings.bool("editor.insertSpaces", lang),
        }
    }

    /// Editor widget options for a view from the resolved settings.
    pub fn editor_options(
        &self,
        view: notoir_core::ids::ViewId,
    ) -> crate::editor_widget::EditorOptions {
        use crate::editor_widget::{CursorStyle, EditorOptions, LineNumbers, Whitespace, WrapMode};
        let doc = self
            .workbench
            .view(view)
            .and_then(|v| self.workbench.document(v.document_id));
        let lang = doc.map(|d| d.language.as_str());
        let s = &self.settings;
        let wrap = if !self.wrap_enabled(view) {
            WrapMode::Off
        } else if s.str("editor.wordWrap", lang) == "column" {
            WrapMode::Column(s.int("editor.wordWrapColumn", lang).max(10) as usize)
        } else {
            WrapMode::On
        };
        let highlight = s.str("editor.highlightCurrentLine", lang);
        EditorOptions {
            font_size: s.int("editor.fontSize", lang) as f32,
            line_height: s.float("editor.lineHeight", lang) as f32,
            zoom: self.zoom_for(view),
            indent: doc.map_or_else(
                || notoir_editor::edit_ops::IndentStyle {
                    tab_size: s.int("editor.tabSize", lang).max(1) as usize,
                    insert_spaces: s.bool("editor.insertSpaces", lang),
                },
                |d| self.indent_style(d.id),
            ),
            wrap,
            line_numbers: match s.str("editor.lineNumbers", lang) {
                _ if self.zen.is_some() && s.bool("zenMode.hideLineNumbers", None) => {
                    LineNumbers::Off
                }
                "off" => LineNumbers::Off,
                "relative" => LineNumbers::Relative,
                _ => LineNumbers::On,
            },
            whitespace: match s.str("editor.renderWhitespace", lang) {
                "none" => Whitespace::None,
                "boundary" => Whitespace::Boundary,
                "trailing" => Whitespace::Trailing,
                "all" => Whitespace::All,
                _ => Whitespace::Selection,
            },
            highlight_line: matches!(highlight, "line" | "all"),
            highlight_gutter: matches!(highlight, "gutter" | "all"),
            cursor: match s.str("editor.cursorStyle", lang) {
                "block" => CursorStyle::Block,
                "underline" => CursorStyle::Underline,
                _ => CursorStyle::Line,
            },
            blink: s.str("editor.cursorBlinking", lang) != "solid",
            markdown_lists: lang == Some("markdown"),
            empty_selection_clipboard: s.bool("editor.emptySelectionClipboard", lang),
            padding: 8.0,
            find_matches: self.find_highlights(view).0,
            find_current: self.find_highlights(view).1,
            center: self.find.center.contains(&view),
            set_scroll: self.find.scroll.get(&view).copied(),
            highlight: !doc.is_some_and(|d| d.large_file),
            diagnostics: doc.map(|d| self.json_diagnostics(d.id)).unwrap_or_default(),
            mcp_edits: doc.map(|d| self.mcp_highlights(d.id)).unwrap_or_default(),
            wiki_links: Default::default(),
            bookmarks: doc.map(|d| d.bookmark_lines()).unwrap_or_default(),
            fold_ranges: doc
                .filter(|_| self.settings.bool("editor.folding", lang))
                .and_then(|d| self.structure.docs.get(&d.id))
                .map(|s| s.folds.clone()),
            max_width_cols: self
                .zen
                .as_ref()
                .map(|_| s.int("zenMode.maxWidth", None).max(20) as usize),
            minimap: (s.bool("editor.minimap.enabled", lang)
                && self.zen.is_none()
                && !doc.is_some_and(|d| d.large_file))
            .then(|| crate::editor_widget::minimap::MinimapOptions {
                left: s.str("editor.minimap.side", None) == "left",
                width: s.int("editor.minimap.width", None) as f32,
                line_h: s.int("editor.minimap.scale", None).clamp(1, 3) as f32,
                opacity: s.float("editor.minimap.opacity", None) as f32,
                show_viewport: s.bool("editor.minimap.showViewport", None),
                show_search: s.bool("editor.minimap.showSearchMarks", None),
                show_selection: s.bool("editor.minimap.showSelection", None),
                show_bookmarks: s.bool("editor.minimap.showBookmarks", None),
                show_git: s.bool("editor.minimap.showGitMarks", None),
            }),
            diff: None,
            git_changes: doc.and_then(|d| self.git_changes(d.id)),
            set_scroll_px: None,
            column_mode: self.column_mode,
        }
    }

    pub fn status_bar_visible(&self) -> bool {
        self.settings.bool("workbench.statusBar.visible", None)
    }

    /// Context keys for when-expressions and menu check marks.
    pub fn context(&self, fullscreen: bool) -> ContextSnapshot {
        let mut c = ContextSnapshot::default();
        c.set_bool("isMac", self.platform == Platform::Mac)
            .set_bool("primarySidebarVisible", self.panels.primary.visible)
            .set_bool("secondarySidebarVisible", self.panels.secondary.visible)
            .set_bool("bottomPanelVisible", self.panels.bottom.visible)
            .set_bool(
                "secondarySidebarHasPanels",
                !self.panels.secondary.panels.is_empty(),
            )
            .set_bool("activityBarVisible", self.panels.activity_bar_visible)
            .set_bool(
                "primarySidebarRight",
                self.panels.primary_side == notoir_core::panels::Side::Right,
            )
            .set_bool("toolbarVisible", self.toolbar_visible())
            .set_bool("statusBarVisible", self.status_bar_visible())
            .set_bool("fullScreen", fullscreen)
            .set_bool("hasWorkspace", self.workspace.is_some())
            .set_bool("explorerFocus", self.explorer_focused)
            .set_bool("zenMode", self.zen.is_some())
            .set_bool("editorFocus", self.editor_focused)
            .set_bool("textInputFocus", false)
            .set_bool(
                "inDiff",
                self.workbench
                    .active_view()
                    .and_then(|v| self.workbench.view(v))
                    .is_some_and(|v| v.kind == notoir_core::view::ViewKind::Diff),
            )
            .set_bool("compareSelected", self.compare.selected.is_some())
            .set_bool("columnSelection", self.column_mode)
            .set_bool("autoCommit", self.settings.bool("git.autoCommit", None))
            .set_bool(
                "autoCommit.manual",
                self.settings.bool("git.autoCommit", None),
            )
            .set_bool(
                "autoCommit.off",
                !self.settings.bool("git.autoCommit", None),
            )
            .set_bool("autoPush", self.settings.bool("git.autoPush", None))
            .set_bool("gitRepo", self.git.repo.is_some())
            .set_bool("gitBusy", self.git_busy())
            .set_bool("gitNetwork", self.git.net.is_some())
            .set_bool("gitPeek", self.git.peek.is_some())
            .set_bool("mcpRunning", self.mcp.running())
            .set_bool("pluginFormatter", self.plugin_formatter_active())
            .set_bool("mcpPrompt", !self.mcp.prompts.is_empty())
            .set_bool(
                "gitPeekStageable",
                self.git.peek.as_ref().is_some_and(|p| p.stageable),
            )
            .set_bool(
                "notesSync",
                self.settings.bool("notes.sync.enabled", None)
                    && self.workspace.as_ref().is_some_and(|w| w.trusted),
            );
        if let Some(view) = self
            .workbench
            .active_editor()
            .and_then(|v| self.workbench.view(v))
        {
            let doc = self.workbench.document(view.document_id);
            let language = doc.map_or("plaintext", |d| d.language.as_str());
            let path = doc.and_then(|d| d.path());
            c.set_bool("hasFilePath", path.is_some()).set_bool(
                "fileInWorkspace",
                path.is_some_and(|p| {
                    self.workspace
                        .as_ref()
                        .is_some_and(|w| w.relative(p).is_some())
                }),
            );
            c.set_bool("hasActiveEditor", true)
                .set_str("languageId", language)
                .set_bool("readOnly", doc.is_some_and(|d| d.is_read_only()))
                .set_bool("hasSelection", !view.selection.is_empty())
                .set_bool("canUndo", doc.is_some_and(|d| d.buffer.can_undo()))
                .set_bool("canRedo", doc.is_some_and(|d| d.buffer.can_redo()))
                .set_bool("wordWrap", self.wrap_enabled(view.id));
        }
        c.set_bool(
            "lineNumbersVisible",
            self.settings.str("editor.lineNumbers", None) != "off",
        )
        .set_bool(
            "breadcrumbsVisible",
            self.settings.bool("workbench.breadcrumbs.enabled", None),
        )
        .set_bool(
            "lineNumbersRelative",
            self.settings.str("editor.lineNumbers", None) == "relative",
        )
        .set_bool(
            "whitespaceVisible",
            self.settings.str("editor.renderWhitespace", None) == "all",
        )
        .set_bool(
            "currentLineHighlight",
            self.settings.str("editor.highlightCurrentLine", None) != "none",
        );
        let sort = self.settings.str("explorer.sortOrder", None).to_owned();
        c.set_bool(
            "explorerShowHidden",
            self.settings.bool("explorer.showHidden", None),
        )
        .set_bool(
            "explorerShowGitIgnored",
            self.settings.bool("explorer.showGitIgnored", None),
        );
        c.set_bool(
            "minimapVisible",
            self.settings.bool("editor.minimap.enabled", None),
        );
        let auto = self.settings.str("files.autoSave", None).to_owned();
        for m in ["off", "afterDelay", "onFocusChange", "onWindowFocusLoss"] {
            c.set_bool(format!("autoSave.{m}"), auto == m);
        }
        for v in ["default", "mixed", "modified"] {
            c.set_bool(format!("explorerSort.{v}"), sort == v);
        }
        for &p in REGISTERED_PANELS {
            c.set_bool(
                format!("panel.{}.visible", panel_key(p)),
                self.panels.is_shown(p),
            );
        }
        c
    }

    pub fn enqueue(&mut self, inv: CommandInvocation) {
        self.queue.push(inv);
    }

    /// Runs every queued command (UI thread, end of frame).
    pub fn run_queued(&mut self, ctx: &ContextSnapshot) {
        let mut pending: std::collections::VecDeque<(CommandInvocation, Option<ContextSnapshot>)> =
            std::mem::take(&mut self.queue_with_context)
                .into_iter()
                .map(|(inv, c)| (inv, Some(c)))
                .collect();
        loop {
            let (inv, own) = match pending.pop_front() {
                Some(j) => j,
                None => match self.queue.pop() {
                    Some(inv) => (inv, None),
                    None => break,
                },
            };
            let ctx = own.as_ref().unwrap_or(ctx);
            let before = self.current_location();
            let outcome = match self.commands.resolve(&inv, ctx) {
                Ok(handler) => handler(self, &inv),
                Err(outcome) => outcome,
            };
            self.nav_after_command(before);
            match outcome {
                CommandOutcome::Rejected(err) => {
                    let now = self.now_ms;
                    self.notifications
                        .notify(Notification::from_error(&err), now);
                }
                CommandOutcome::Disabled => {
                    tracing::debug!(command = %inv.id, "command disabled in this context");
                }
                _ => {}
            }
        }
    }

    pub fn run_now(&mut self, inv: CommandInvocation, ctx: &ContextSnapshot) {
        self.enqueue(inv);
        self.run_queued(ctx);
    }

    /// Changes a User-scope setting: applies it now and writes the file in
    /// the background.
    pub fn set_user_setting(&mut self, key: &str, value: Option<SettingValue>) {
        match &value {
            Some(v) => {
                self.user_settings.set(key, v.clone());
            }
            None => {
                self.user_settings.values.remove(key);
            }
        }
        let newer = self.resolve_settings();
        let changed = self.settings.changed_keys(&newer);
        self.settings = newer;
        if !changed.is_empty() {
            self.workbench
                .events
                .emit(notoir_core::AppEvent::SettingsChanged { keys: changed });
        }
        let path: PathBuf = self.paths.user_settings_file();
        let key = key.to_owned();
        self.io.submit(move || {
            settings_file::write_setting_file(&path, &key, None, value.as_ref())
                .err()
                .map(|e| IoResult::SettingsWriteFailed {
                    path: path.display().to_string(),
                    error: e.to_string(),
                })
        });
    }

    /// Handles IO results (UI thread).
    pub fn pump_io(&mut self) {
        let mut results = self.io.drain();
        results.extend(self.fs.drain());
        results.extend(self.index_queue.drain());
        results.extend(self.git_queue.drain());
        for result in results {
            match result {
                IoResult::FilesPicked(paths) if self.plugins_files_picked(&paths) => {}
                IoResult::FilesPicked(paths) => {
                    for p in paths {
                        self.open_path(&p, crate::files::OpenAt::default());
                    }
                }
                IoResult::SaveAsPicked { doc, path } => self.handle_save_as_picked(doc, path),
                IoResult::ExportPicked { doc, path } => self.markdown_export_to(doc, path),
                IoResult::Exported { path, result } => self.markdown_exported(path, result),
                IoResult::Loaded(outcome) => self.handle_loaded(*outcome),
                IoResult::Saved(outcome) => self.handle_saved(*outcome),
                IoResult::ScratchpadWriteFailed { error } => {
                    tracing::error!(%error, "scratchpad write failed");
                    let now = self.now_ms;
                    self.notifications.notify(
                        Notification::new(
                            Severity::Error,
                            Translatable::new("notifications:scratchWriteFailed"),
                        )
                        .detail(error)
                        .dedupe("scratch-write"),
                        now,
                    );
                }
                IoResult::Instance(req) => self.handle_instance_request(req, None),
                IoResult::PluginsScanned(found) => self.plugins_scanned(found),
                IoResult::PluginManifest { dir, result } => {
                    self.plugins_manifest_read(dir, *result)
                }
                IoResult::PluginInstalled { id, error } => self.plugins_installed(id, error),
                IoResult::PluginSyntax {
                    plugin,
                    language,
                    error,
                } => self.plugins_syntax_loaded(&plugin, &language, error),
                IoResult::PluginRecommendations(list) => self.plugins.recommended = list,
                IoResult::PluginUninstalled { id, error } => self.plugins_uninstalled(id, error),
                IoResult::NotesIndexed(generation, index) => self.notes_indexed(generation, *index),
                IoResult::TextIndex(done) => self.text_index_done(*done),
                IoResult::TemplatesListed(list) => self.templates_listed(list),
                IoResult::CustomThemes(list) => self.custom_themes_loaded(list),
                IoResult::UpdateStamp(ms) => self.updates_stamp(ms),
                IoResult::UpdateChecked(r) => {
                    let (result, manual) = *r;
                    self.updates_checked(result, manual);
                }
                IoResult::UpdateDownloaded(r) => self.updates_downloaded(*r),
                IoResult::TemplateLoaded(loaded) => {
                    let (t, text) = *loaded;
                    self.templates_loaded(t, text);
                }
                IoResult::NotesLinksUpdated(failed) => self.notes_links_updated(failed),
                IoResult::NotesUnlinked(rel, found) => self.notes_unlinked_found(rel, found),
                IoResult::ShellCommand { install, result } => {
                    let n = match (install, result) {
                        (true, Ok(p)) => Notification::new(
                            Severity::Info,
                            Translatable::new("notifications:shellCommandInstalled").arg("path", p),
                        ),
                        (false, Ok(p)) => Notification::new(
                            Severity::Info,
                            Translatable::new("notifications:shellCommandRemoved").arg("path", p),
                        ),
                        (_, Err(e)) => Notification::new(
                            Severity::Error,
                            Translatable::new("notifications:shellCommandFailed"),
                        )
                        .detail(e),
                    };
                    self.notify(n);
                }
                IoResult::FolderPicked(path) if self.git_clone_folder(path.as_deref()) => {}
                IoResult::FolderPicked(path) if self.plugins_folder_picked(path.as_deref()) => {}
                IoResult::FolderPicked(Some(path)) => self.open_workspace(&path),
                IoResult::FolderPicked(None) => {}
                IoResult::Workspace(r) => self.handle_workspace(r),
                IoResult::DocsChanged(paths) => self.check_changed_paths(&paths),
                IoResult::Git(r) => self.handle_git(*r),
                IoResult::DiskChecked(r) => self.handle_disk_check(*r),
                IoResult::RecoveryWriteFailed { error } => self.recovery_write_failed(error),
                IoResult::ConfigReloaded(files) => self.apply_config(*files),
                IoResult::ImageDecoded(img) => self.markdown.decoded.push(*img),
                IoResult::Search(b) => {
                    let (generation, batch) = *b;
                    self.handle_search(generation, batch);
                }
                IoResult::ReplaceDone(r) => {
                    let (done, failed) = *r;
                    self.replace_finished(done, failed);
                }
                IoResult::SettingsWriteFailed { path, error } => {
                    tracing::error!(%path, %error, "settings write failed");
                    let now = self.now_ms;
                    self.notifications.notify(
                        Notification::new(
                            Severity::Error,
                            Translatable::new("notifications:settingsWriteFailed"),
                        )
                        .detail(format!("{path}: {error}"))
                        .source(NotificationSource::Settings)
                        .dedupe("settings-write"),
                        now,
                    );
                }
            }
        }
    }

    /// Tests/shutdown: wait for background writes.
    pub fn flush_io(&self) -> bool {
        self.io.wait_idle(Duration::from_secs(5))
            & self.fs.wait_idle(Duration::from_secs(5))
            & self.index_queue.wait_idle(Duration::from_secs(5))
            & self.git_queue.wait_idle(Duration::from_secs(5))
    }
}

/// Stable lowercase key of a panel for args and context keys.
pub fn panel_key(p: PanelId) -> &'static str {
    match p {
        PanelId::Explorer => "explorer",
        PanelId::Search => "search",
        PanelId::SourceControl => "sourceControl",
        PanelId::Outline => "outline",
        PanelId::Plugins => "plugins",
        PanelId::MarkdownPreview => "markdownPreview",
        PanelId::JsonTree => "jsonTree",
        PanelId::McpActivity => "mcpActivity",
        PanelId::Output => "output",
        PanelId::Problems => "problems",
        PanelId::GitOutput => "gitOutput",
        PanelId::PluginOutput => "pluginOutput",
        PanelId::Backlinks => "backlinks",
        PanelId::Tags => "tags",
    }
}

pub fn panel_from_key(key: &str) -> Option<PanelId> {
    REGISTERED_PANELS
        .iter()
        .copied()
        .find(|p| panel_key(*p) == key)
}

fn load_keybindings(
    path: &std::path::Path,
    notifications: &mut NotificationCenter,
) -> Vec<UserBinding> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => {
            tracing::error!(error = %e, "could not read keybindings file");
            return Vec::new();
        }
    };
    match settings_file::parse_keybindings(&text) {
        Ok(b) => b,
        Err(e) => {
            notifications.notify(
                Notification::new(
                    Severity::Warning,
                    Translatable::new("notifications:keybindingsParse").arg("line", e.line),
                )
                .detail(e.to_string())
                .source(NotificationSource::Settings),
                0,
            );
            Vec::new()
        }
    }
}

pub(crate) fn build_keymap(
    commands: &CommandRegistry<AppCore>,
    plugins: &[KeyBinding],
    user: &[UserBinding],
    platform: Platform,
    notifications: &mut NotificationCenter,
) -> Keymap {
    let mut defaults = Vec::new();
    for spec in commands.iter() {
        for def in &spec.default_keys {
            match KeyBinding::from_def(def, spec.id.clone(), platform) {
                Ok(b) => defaults.push(b),
                Err(e) => {
                    tracing::error!(command = %spec.id, error = %e, "invalid default keybinding")
                }
            }
        }
    }
    let (keymap, errors) = Keymap::build(defaults, plugins.to_vec(), user, platform);
    if !errors.is_empty() {
        notifications.notify(
            Notification::new(
                Severity::Warning,
                Translatable::new("notifications:keybindingsInvalid").arg("count", errors.len()),
            )
            .detail(
                errors
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("\n"),
            )
            .source(NotificationSource::Settings),
            0,
        );
    }
    keymap
}

impl AppCore {
    /// After construction: reopen Scratchpads, load recent files, open the
    /// command-line files.
    pub fn startup(
        &mut self,
        open_paths: &[(std::path::PathBuf, Option<(usize, usize)>)],
        restore_session: bool,
    ) {
        self.load_recent();
        self.load_recent_workspaces();
        self.mcp_init();
        self.reload_custom_themes();
        self.updates_init();
        self.recovery_start();
        self.restore_scratchpads();
        let restore = restore_session
            && open_paths.is_empty()
            && self.settings.str("workbench.startup", None) == "restoreSession";
        if restore {
            self.restore_last_session();
        } else {
            self.open_cli_paths(open_paths);
        }
        if self.workspace.is_none() {
            self.restore_hot_exit();
        }
        if self.first_run_check {
            self.maybe_first_run();
        }
        self.mcp_autostart();
        self.plugins_scan();
    }

    /// Exit: Scratchpads, session and recovery are written before the
    /// process ends.
    pub fn shutdown(&mut self) {
        self.mcp_stop();
        // Plugin VMs stop when their handles drop.
        self.plugins.entries.clear();
        self.text_index_shutdown();
        self.flush_scratchpads(true);
        self.write_session(false);
        self.recovery_shutdown();
        self.flush_io();
        self.updates_hand_off();
    }

    /// Command-line paths: files open as tabs; the first folder becomes this
    /// window's workspace and further folders open their own windows.
    pub fn open_cli_paths(&mut self, paths: &[(std::path::PathBuf, Option<(usize, usize)>)]) {
        let mut folder_used = false;
        for (path, at) in paths {
            if path.is_dir() {
                if folder_used {
                    self.open_workspace_in_new_window(path);
                } else {
                    folder_used = true;
                    self.open_workspace(path);
                }
                continue;
            }
            self.open_path(
                path,
                crate::files::OpenAt {
                    line_col: *at,
                    restore: None,
                    keep_focus: false,
                    anchor: None,
                    ..Default::default()
                },
            );
        }
    }

    /// Command line forwarded by a second launch.
    pub fn handle_instance_request(
        &mut self,
        req: notoir_platform::single_instance::InstanceRequest,
        waiter: Option<notoir_platform::single_instance::Waiter>,
    ) {
        let paths = crate::cli::paths(&req.args, &req.cwd);
        self.open_cli_paths(&paths);
        if let Some(waiter) = waiter {
            let keys = paths
                .iter()
                .map(|(p, _)| notoir_workspace::fs_io::identity(p).1)
                .collect();
            self.cli_waits.push(CliWait {
                keys,
                waiter: Some(waiter),
                seen: false,
                since: self.now_ms,
            });
        }
        if std::env::var("NOTOIR_TEST_COMMANDS").as_deref() == Ok("1") {
            for (id, args) in crate::cli::commands(&req.args, &req.cwd) {
                let args = serde_json::from_str(&args).unwrap_or(serde_json::Value::Null);
                tracing::info!(command = %id, "test channel command");
                self.enqueue(
                    CommandInvocation::new(id, notoir_core::command::InvocationSource::Internal)
                        .with_args(args),
                );
            }
        }
        self.effects.push(Effect::FocusWindow);
    }
}
