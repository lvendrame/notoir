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
pub const REGISTERED_PANELS: &[PanelId] = &[PanelId::Explorer, PanelId::Output];

/// Theme setting: a fixed theme or "auto" (follow the OS).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeChoice {
    Fixed(ThemeId),
    Auto,
}

impl ThemeChoice {
    pub fn from_setting(value: &str) -> Option<ThemeChoice> {
        if value == "auto" {
            Some(ThemeChoice::Auto)
        } else {
            ThemeId::from_id(value).map(ThemeChoice::Fixed)
        }
    }

    pub fn setting_value(self) -> &'static str {
        match self {
            ThemeChoice::Fixed(t) => t.id(),
            ThemeChoice::Auto => "auto",
        }
    }

    pub fn resolve(self, system_dark: bool) -> ThemeId {
        match self {
            ThemeChoice::Fixed(t) => t,
            ThemeChoice::Auto => ThemeId::for_system(system_dark),
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
    OpenThemePicker,
    OpenLanguagePicker,
    ShowAbout,
    ToggleNotificationCentre,
}

/// How the app starts (resolved by `main`, or by tests).
#[derive(Clone, Debug)]
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
    /// Theme override from the environment (wins over the setting).
    pub theme_override: Option<ThemeId>,
    pub effects: Vec<Effect>,
    /// Milliseconds since app start (monotonic clock for timers).
    pub now_ms: u64,
}

impl AppCore {
    pub fn new(opts: &StartOptions, wake: impl Fn() + Send + 'static) -> Self {
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

        let mut commands = CommandRegistry::default();
        crate::commands::register_all(&mut commands, opts.platform);
        let keymap = build_keymap(&commands, &user_bindings, opts.platform, &mut notifications);

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
            io: IoQueue::start(wake),
            language,
            theme_choice,
            theme_override: opts.theme_override,
            effects: Vec::new(),
            now_ms: 0,
        }
    }

    /// Theme to show given the OS preference.
    pub fn theme(&self, system_dark: bool) -> ThemeId {
        self.theme_override
            .unwrap_or_else(|| self.theme_choice.resolve(system_dark))
    }

    pub fn toolbar_visible(&self) -> bool {
        self.settings.bool("workbench.toolbar.visible", None)
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
            .set_bool("hasWorkspace", false)
            .set_bool("editorFocus", false)
            .set_bool("textInputFocus", false);
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
        while let Some(inv) = self.queue.pop() {
            let outcome = match self.commands.resolve(&inv, ctx) {
                Ok(handler) => handler(self, &inv),
                Err(outcome) => outcome,
            };
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
        let newer =
            ResolvedSettings::new(self.schema.clone(), self.user_settings.clone(), None, false);
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
        for result in self.io.drain() {
            match result {
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

fn build_keymap(
    commands: &CommandRegistry<AppCore>,
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
    let (keymap, errors) = Keymap::build(defaults, Vec::new(), user, platform);
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
