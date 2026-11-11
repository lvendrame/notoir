//! One process owns the event loop; each editor owns an independent window core (spec 40).
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use eframe::App;
use notoir_core::command::{CommandInvocation, InvocationSource};
use notoir_platform::single_instance::{InstanceRequest, Waiter};
use serde::{Deserialize, Serialize};

use crate::app::{Launch, NotoirApp};
use crate::core::StartOptions;
use crate::io::IoQueue;
use crate::session::{SessionData, WindowGeometry};

pub const MANIFEST_KEY: &str = "::application::windows::v1";

#[derive(Clone, Debug, PartialEq)]
pub enum ApplicationRequest {
    NewWindow(Option<PathBuf>),
    OpenWorkspace(PathBuf),
    Activate(String),
    Close,
    Minimize,
    BringAllToFront,
    Quit,
    StartMcp,
    StopMcp,
    RefreshMcp,
    RefreshPlugins,
    RefreshThemes,
    UserSetting {
        key: String,
        value: Option<notoir_core::settings::SettingValue>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApplicationEvent {
    WindowCreated(String),
    WindowActivated(String),
    WindowClosed(String),
    QuitStarted,
    QuitCancelled,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WindowInfo {
    pub id: String,
    pub title: String,
    pub workspace: Option<String>,
    pub active: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WindowSnapshot {
    pub id: String,
    pub session: SessionData,
    pub geometry: Option<WindowGeometry>,
    pub recovery: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApplicationSession {
    pub version: u32,
    pub active: Option<String>,
    pub windows: Vec<WindowSnapshot>,
}

pub struct EditorWindow {
    pub id: String,
    pub app: NotoirApp,
    pub builder: egui::ViewportBuilder,
    closing: bool,
}

impl EditorWindow {
    pub fn viewport(&self) -> egui::ViewportId {
        egui::ViewportId::from_hash_of((&self.id, "notoir-editor"))
    }
}

struct LaunchWait {
    waiter: Waiter,
    targets: Vec<(String, notoir_core::document::PathKey)>,
    seen: bool,
    started: Instant,
}

/// The only eframe application in production. Editor controllers are also usable
/// independently by existing UI tests, without creating native windows.
pub struct ApplicationHost {
    pub windows: Vec<EditorWindow>,
    pub active: Option<String>,
    pending_focus: Option<(String, Instant)>,
    events: std::collections::VecDeque<ApplicationEvent>,
    accepting_launches: std::sync::Arc<std::sync::atomic::AtomicBool>,
    opts: StartOptions,
    instance_rx: Option<Receiver<(InstanceRequest, Option<Waiter>)>>,
    io: IoQueue,
    started: Instant,
    next_write: Instant,
    last_manifest: Option<String>,
    quitting: bool,
    exit_committed: bool,
    shutdown_started: bool,
    manifest_pending: bool,
    manifest_tx: std::sync::mpsc::Sender<Result<(), String>>,
    manifest_rx: std::sync::mpsc::Receiver<Result<(), String>>,
    waits: Vec<LaunchWait>,
    pub(crate) mcp: crate::mcp::McpState,
    pending_mcp: Vec<(String, notoir_mcp::ReplyHandle)>,
    updates: crate::updates::UpdatesState,
    update_results: Vec<crate::io::IoResult>,
    fonts_installed: Option<Vec<notoir_platform::system_fonts::Script>>,
    family_generation: u64,
    config_stamp: Option<(std::time::SystemTime, u64)>,
    next_config_check: Instant,
    config_rx: std::sync::mpsc::Receiver<Option<(std::time::SystemTime, u64)>>,
    config_tx: std::sync::mpsc::Sender<Option<(std::time::SystemTime, u64)>>,
    #[cfg(target_os = "macos")]
    native_menu: Option<crate::native_menu::NativeMenu>,
}

impl ApplicationHost {
    pub fn new(cc: &eframe::CreationContext<'_>, launch: Launch) -> Self {
        cc.egui_ctx.add_plugin(crate::input::WindowInput::default());
        let paths = notoir_platform::AppPaths::resolve().expect("application paths");
        let mut opts = StartOptions::new(paths);
        opts.os_locale = notoir_platform::os_locale();
        opts.language_override = std::env::var(crate::app::LANG_ENV)
            .ok()
            .as_deref()
            .and_then(crate::i18n::Language::from_code);
        opts.theme_override = std::env::var(crate::app::THEME_ENV)
            .ok()
            .as_deref()
            .and_then(crate::theme::ThemeId::from_id);
        opts.first_run = true;
        let mut host = Self::with_options(&cc.egui_ctx, opts, launch.restore_session);
        host.instance_rx = launch.instance_rx;
        if let Some(gate) = launch.accepting_launches {
            host.accepting_launches = gate;
        }
        if !launch.open_paths.is_empty() {
            host.open_paths(&cc.egui_ctx, &launch.open_paths, false);
        }
        host
    }

    pub fn with_options(ctx: &egui::Context, opts: StartOptions, restore: bool) -> Self {
        if let Err(e) = opts.paths.ensure_dirs() {
            tracing::error!(error = %e, "application paths unavailable");
        }
        let repaint = ctx.clone();
        let io = IoQueue::start(move || repaint.request_repaint());
        let repaint = ctx.clone();
        let (config_tx, config_rx) = std::sync::mpsc::channel();
        let (manifest_tx, manifest_rx) = std::sync::mpsc::channel();
        let updates = crate::updates::UpdatesState::new(opts.update.clone());
        let mut host = Self {
            pending_mcp: Vec::new(),
            updates,
            update_results: Vec::new(),
            accepting_launches: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
            windows: Vec::new(),
            active: None,
            pending_focus: None,
            events: Default::default(),
            opts,
            instance_rx: None,
            io,
            started: Instant::now(),
            next_write: Instant::now(),
            last_manifest: None,
            quitting: false,
            exit_committed: false,
            shutdown_started: false,
            manifest_pending: false,
            manifest_tx,
            manifest_rx,
            waits: Vec::new(),
            mcp: crate::mcp::McpState::new(std::sync::Arc::new(move || repaint.request_repaint())),
            fonts_installed: None,
            family_generation: u64::MAX,
            config_stamp: None,
            next_config_check: Instant::now(),
            config_tx,
            config_rx,
            #[cfg(target_os = "macos")]
            native_menu: None,
        };
        let stamp_path = host.opts.paths.cache.join("update").join("last-check");
        host.io.submit(move || {
            Some(crate::io::IoResult::UpdateStamp(
                std::fs::read_to_string(stamp_path)
                    .ok()
                    .and_then(|s| s.trim().parse().ok()),
            ))
        });
        let db = notoir_storage::state_db::StateDb::open(&host.opts.paths.data.join("state.db"))
            .ok()
            .map(|(db, _)| db);
        let restore = restore
            && notoir_storage::settings_file::load_settings_file(
                &host.opts.paths.user_settings_file(),
                None,
            )
            .ok()
            .and_then(|loaded| loaded.layer.values.get("workbench.startup").cloned())
            .is_none_or(|value| {
                value == notoir_core::settings::SettingValue::Str("restoreSession".into())
            });
        let prior = restore
            .then(|| {
                db.as_ref()?
                    .load_session(MANIFEST_KEY)
                    .ok()?
                    .and_then(|s| serde_json::from_str::<ApplicationSession>(&s).ok())
            })
            .flatten()
            .filter(|s| s.version == 1);
        if let Some(prior) = prior {
            let active = prior.active;
            for window in prior.windows {
                if uuid::Uuid::parse_str(&window.id).is_err()
                    || host.windows.iter().any(|w| w.id == window.id)
                {
                    continue;
                }
                if let Some(root) = window.session.root.as_deref()
                    && host.workspace_owner(Path::new(root)).is_some()
                {
                    continue;
                }
                host.create_window(ctx, Some(window), None);
            }
            if let Some(active) = active {
                host.activate(ctx, &active);
            }
        } else if restore {
            // Import the legacy singleton without changing or deleting its rows.
            let legacy = db.as_ref().and_then(|db| {
                let key = db
                    .load_window(0)
                    .ok()
                    .flatten()
                    .and_then(|(k, _)| k)
                    .unwrap_or_else(|| crate::session::NO_WORKSPACE_KEY.to_owned());
                let session = db.load_session(&key).ok().flatten()?;
                Some(WindowSnapshot {
                    id: uuid::Uuid::new_v4().simple().to_string(),
                    session: serde_json::from_str(&session).ok()?,
                    geometry: crate::session::load_geometry(&host.opts.paths.data),
                    recovery: None,
                })
            });
            if let Some(window) = legacy {
                host.create_window(ctx, Some(window), None);
            }
        }
        if host.windows.is_empty() {
            host.create_window(ctx, None, None);
        }
        host.mcp.registry = host.windows[0].app.core.mcp.registry.clone();
        if host
            .windows
            .iter()
            .any(|w| w.app.core.settings.bool("mcp.autoStart", None))
        {
            host.start_mcp();
        }
        host
    }

    fn emit(&mut self, event: ApplicationEvent) {
        self.events.push_back(event);
        while self.events.len() > 128 {
            self.events.pop_front();
        }
    }

    pub fn drain_events(&mut self) -> impl Iterator<Item = ApplicationEvent> + '_ {
        self.events.drain(..)
    }

    pub fn create_window(
        &mut self,
        ctx: &egui::Context,
        restore: Option<WindowSnapshot>,
        root: Option<&Path>,
    ) -> String {
        if let Some(root) = root
            && let Some(id) = self.workspace_owner(root)
        {
            self.activate(ctx, &id);
            return id;
        }
        let id = restore
            .as_ref()
            .map(|s| s.id.clone())
            .unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string());
        let mut opts = self.opts.clone();
        opts.window_id = Some(id.clone());
        opts.update_poster = Some(self.io.poster());
        opts.recovery_source = restore.as_ref().and_then(|s| s.recovery.clone());
        if opts.recovery_source.is_none() && self.windows.is_empty() {
            opts.recovery_source = Some("legacy".into());
        }
        if restore.is_none()
            && let Some(root) = root
        {
            let workspace = notoir_workspace::fs_io::identity(root).1;
            opts.recovery_source =
                notoir_storage::recovery::scan(&opts.paths.data.join("recovery"), None)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|s| {
                        s.manifest
                            .as_ref()
                            .is_some_and(|m| m.workspace.as_deref() == Some(workspace.as_str()))
                    })
                    .max_by_key(|s| s.manifest.as_ref().map(|m| m.updated_at_ms))
                    .and_then(|s| s.manifest.map(|m| m.session_id));
        }
        opts.first_run &= self.windows.is_empty();
        opts.restore_session = false;
        opts.open_paths.clear();
        let mut app = NotoirApp::with_options(ctx, opts);
        app.core.native_menu = cfg!(target_os = "macos");
        if self.windows.is_empty() {
            self.mcp.registry = app.core.mcp.registry.clone();
        } else {
            app.core.mcp.registry = self.mcp.registry.clone();
        }
        let geometry = restore.as_ref().and_then(|s| s.geometry);
        if let Some(snapshot) = restore {
            if let Some(root) = snapshot
                .session
                .root
                .as_deref()
                .filter(|p| Path::new(p).is_dir())
            {
                // Prevent the historical workspace session from being applied twice.
                app.core.session_override = Some(snapshot.session.clone());
                app.core.open_workspace_local(Path::new(root));
            } else {
                app.core.apply_session(snapshot.session);
            }
        } else if let Some(root) = root {
            app.core.open_workspace_local(root);
        }
        let mut builder = egui::ViewportBuilder::default()
            .with_title("Notoir")
            .with_app_id(notoir_platform::paths::BUNDLE_ID)
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([640.0, 400.0])
            .with_icon(crate::icons::app_icon());
        if let Some(g) = geometry {
            builder = builder
                .with_inner_size([g.width.max(640.0), g.height.max(400.0)])
                .with_position([g.x, g.y])
                .with_maximized(g.maximized)
                .with_fullscreen(g.fullscreen);
        }
        self.windows.push(EditorWindow {
            id: id.clone(),
            app,
            builder,
            closing: false,
        });
        self.active = Some(id.clone());
        self.pending_focus = Some((id.clone(), Instant::now()));
        self.emit(ApplicationEvent::WindowCreated(id.clone()));
        self.emit(ApplicationEvent::WindowActivated(id.clone()));
        tracing::info!(window_id = %id, "editor window created");
        id
    }

    pub fn workspace_owner(&self, path: &Path) -> Option<String> {
        let key = notoir_workspace::fs_io::identity(path).1;
        self.windows
            .iter()
            .find(|w| {
                w.app
                    .core
                    .workspace
                    .as_ref()
                    .is_some_and(|ws| ws.key == key.as_str())
            })
            .map(|w| w.id.clone())
    }

    pub fn activate(&mut self, ctx: &egui::Context, id: &str) {
        let changed = self.active.as_deref() != Some(id);
        if let Some(w) = self.windows.iter().find(|w| w.id == id) {
            self.active = Some(id.to_owned());
            self.pending_focus = Some((id.to_owned(), Instant::now()));
            if ctx.input(|i| {
                i.raw
                    .viewports
                    .get(&w.viewport())
                    .is_some_and(|v| v.native_pixels_per_point.is_some())
            }) {
                ctx.send_viewport_cmd_to(w.viewport(), egui::ViewportCommand::Minimized(false));
                ctx.send_viewport_cmd_to(w.viewport(), egui::ViewportCommand::Focus);
            }
            if changed {
                self.emit(ApplicationEvent::WindowActivated(id.to_owned()));
            }
        }
    }

    pub fn window_info(&self) -> Vec<WindowInfo> {
        self.windows
            .iter()
            .map(|w| WindowInfo {
                id: w.id.clone(),
                title: w
                    .app
                    .core
                    .workspace
                    .as_ref()
                    .map(|ws| ws.root.display().to_string())
                    .or_else(|| {
                        w.app
                            .core
                            .active_file_path()
                            .map(|p| p.display().to_string())
                    })
                    .unwrap_or_else(|| {
                        format!("Notoir · {}", w.id.chars().take(8).collect::<String>())
                    }),
                workspace: w
                    .app
                    .core
                    .workspace
                    .as_ref()
                    .map(|ws| ws.root.display().to_string()),
                active: self.active.as_ref() == Some(&w.id),
            })
            .collect()
    }

    pub fn request(&mut self, ctx: &egui::Context, source: &str, request: ApplicationRequest) {
        match request {
            ApplicationRequest::NewWindow(root) => {
                self.create_window(ctx, None, root.as_deref());
            }
            ApplicationRequest::OpenWorkspace(root) => {
                if let Some(owner) = self.workspace_owner(&root) {
                    self.activate(ctx, &owner);
                } else if let Some(w) = self.windows.iter_mut().find(|w| w.id == source) {
                    w.app.core.open_workspace_local(&root);
                }
            }
            ApplicationRequest::Activate(id) => self.activate(ctx, &id),
            ApplicationRequest::Close => {
                if let Some(w) = self.windows.iter_mut().find(|w| w.id == source) {
                    w.closing = true;
                    w.app.core.is_closing = true;
                }
            }
            ApplicationRequest::Quit => {
                self.accepting_launches
                    .store(false, std::sync::atomic::Ordering::Release);
                if !self.quitting {
                    self.emit(ApplicationEvent::QuitStarted);
                }
                self.quitting = true;
                for w in &mut self.windows {
                    w.closing = true;
                    w.app.core.is_closing = true;
                }
            }
            ApplicationRequest::Minimize => {
                if let Some(w) = self.windows.iter().find(|w| w.id == source) {
                    ctx.send_viewport_cmd_to(w.viewport(), egui::ViewportCommand::Minimized(true));
                }
            }
            ApplicationRequest::BringAllToFront => {
                for w in &self.windows {
                    ctx.send_viewport_cmd_to(w.viewport(), egui::ViewportCommand::Minimized(false));
                    ctx.send_viewport_cmd_to(w.viewport(), egui::ViewportCommand::Focus);
                }
                if let Some(id) = self.active.clone() {
                    self.activate(ctx, &id);
                }
            }
            ApplicationRequest::StartMcp => self.start_mcp(),
            ApplicationRequest::StopMcp => self.stop_mcp(),
            ApplicationRequest::UserSetting { key, value } => {
                for window in self.windows.iter_mut().filter(|w| w.id != source) {
                    let core = &mut window.app.core;
                    if let Some(value) = &value {
                        core.user_settings.set(&key, value.clone());
                    } else {
                        core.user_settings.values.remove(&key);
                    }
                    core.apply_resolved_settings();
                    if key == "workbench.theme" {
                        if let Some(choice) =
                            crate::core::ThemeChoice::from_setting(core.settings.str(&key, None))
                        {
                            core.theme_choice = choice;
                            core.effects.push(crate::core::Effect::ApplyTheme);
                        }
                    } else if key == "language" {
                        core.language = crate::i18n::Language::detect(
                            self.opts.os_locale.as_deref(),
                            Some(core.settings.str(&key, None)),
                        );
                        core.effects.push(crate::core::Effect::ApplyLanguage);
                    }
                }
            }
            ApplicationRequest::RefreshThemes => {
                for window in self.windows.iter_mut().filter(|w| w.id != source) {
                    window.app.core.reload_custom_themes();
                }
            }
            ApplicationRequest::RefreshPlugins => {
                for window in self.windows.iter_mut().filter(|w| w.id != source) {
                    window.app.core.plugins_scan();
                }
            }
            ApplicationRequest::RefreshMcp => {
                if let Some(owner) = self.windows.iter().find(|w| w.id == source) {
                    let clients = owner.app.core.mcp.clients.clone();
                    let grants: Vec<_> = clients
                        .iter()
                        .map(|c| {
                            let p = notoir_core::permissions::Principal::McpClient(c.id.clone());
                            (p.clone(), owner.app.core.mcp.gate.configured(&p), c.revoked)
                        })
                        .collect();
                    for window in self.windows.iter_mut().filter(|w| w.id != source) {
                        window.app.core.mcp.clients.clone_from(&clients);
                        for (principal, values, revoked) in &grants {
                            window.app.core.mcp.gate.load(principal, values.clone());
                            if *revoked {
                                window.app.core.mcp.gate.revoke(principal);
                            }
                        }
                    }
                }
            }
        }
    }

    fn open_paths(
        &mut self,
        ctx: &egui::Context,
        paths: &[(PathBuf, Option<(usize, usize)>)],
        new_window: bool,
    ) -> Vec<(String, notoir_core::document::PathKey)> {
        let folders: Vec<_> = paths.iter().filter(|(p, _)| p.is_dir()).collect();
        let mut target = self.active.clone();
        if let Some((root, _)) = folders.first() {
            if let Some(owner) = self.workspace_owner(root) {
                target = Some(owner);
            } else if new_window || self.windows.is_empty() {
                target = Some(self.create_window(ctx, None, Some(root)));
            } else {
                let id = self
                    .windows
                    .iter()
                    .find(|w| {
                        w.app.core.workspace.is_none() && w.app.core.workbench.documents.is_empty()
                    })
                    .map(|w| w.id.clone())
                    .or_else(|| self.active.clone())
                    .unwrap();
                self.request(ctx, &id, ApplicationRequest::OpenWorkspace(root.clone()));
                target = Some(id);
            }
            for (root, _) in folders.iter().skip(1) {
                self.create_window(ctx, None, Some(root));
            }
        } else if new_window || target.is_none() {
            target = Some(self.create_window(ctx, None, None));
        }
        let id = target.unwrap();
        let w = self.windows.iter_mut().find(|w| w.id == id).unwrap();
        let mut targets = Vec::new();
        for (path, at) in paths.iter().filter(|(p, _)| !p.is_dir()) {
            w.app.core.open_path(
                path,
                crate::files::OpenAt {
                    line_col: *at,
                    ..Default::default()
                },
            );
            targets.push((id.clone(), notoir_workspace::fs_io::identity(path).1));
        }
        self.activate(ctx, &id);
        targets
    }

    fn launch(&mut self, ctx: &egui::Context, req: InstanceRequest, waiter: Option<Waiter>) {
        let Ok(cli) = crate::cli::parse(&req.args, &req.cwd) else {
            return;
        };
        let targets = self.open_paths(ctx, &cli.paths, cli.new_window);
        if let Some(waiter) = waiter {
            self.waits.push(LaunchWait {
                waiter,
                targets,
                seen: false,
                started: Instant::now(),
            });
        }
        if std::env::var("NOTOIR_TEST_COMMANDS").as_deref() == Ok("1")
            && let Some(w) = self
                .windows
                .iter_mut()
                .find(|w| Some(&w.id) == self.active.as_ref())
        {
            for (id, args) in cli.commands {
                w.app.core.enqueue(
                    CommandInvocation::new(id, InvocationSource::Internal)
                        .with_args(serde_json::from_str(&args).unwrap_or_default()),
                );
            }
        }
    }

    pub fn snapshot(&self) -> ApplicationSession {
        ApplicationSession {
            version: 1,
            active: self.active.clone(),
            windows: self
                .windows
                .iter()
                .map(|w| WindowSnapshot {
                    id: w.id.clone(),
                    session: w.app.core.capture_session(),
                    geometry: w.app.core.session.geometry,
                    recovery: w.app.core.own_session_id(),
                })
                .collect(),
        }
    }

    fn pump_persistence(&mut self) {
        for result in self.manifest_rx.try_iter() {
            self.manifest_pending = false;
            if let Err(error) = result {
                self.last_manifest = None;
                self.quitting = false;
                self.accepting_launches
                    .store(true, std::sync::atomic::Ordering::Release);
                self.shutdown_started = false;
                for w in &mut self.windows {
                    w.closing = false;
                    w.app.core.is_closing = false;
                    w.app.core.close_ready = false;
                    w.app.core.files.quit_confirmed = false;
                }
                if let Some(w) = self.windows.first_mut() {
                    w.app.core.notifications.notify(
                        notoir_core::notifications::Notification::new(
                            notoir_core::notifications::Severity::Error,
                            notoir_core::notifications::Translatable::new(
                                "notifications:sessionWriteFailed",
                            ),
                        )
                        .detail(error),
                        w.app.core.now_ms,
                    );
                }
            }
        }
    }

    pub fn persist(&mut self, force: bool) {
        self.pump_persistence();
        if self.manifest_pending {
            return;
        }
        if !force && Instant::now() < self.next_write {
            return;
        }
        self.next_write = Instant::now() + Duration::from_secs(2);
        if self
            .windows
            .iter()
            .any(|w| w.app.core.session.restore.is_some())
        {
            return;
        }
        for w in &mut self.windows {
            w.app.core.flush_recovery();
        }
        let Ok(json) = serde_json::to_string(&self.snapshot()) else {
            return;
        };
        if self.last_manifest.as_ref() == Some(&json) {
            return;
        }
        self.last_manifest = Some(json.clone());
        let path = self.opts.paths.data.join("state.db");
        self.manifest_pending = true;
        let tx = self.manifest_tx.clone();
        self.io.submit(move || {
            let result = notoir_storage::state_db::StateDb::open(&path)
                .and_then(|(db, _)| {
                    db.save_session(MANIFEST_KEY, &json, crate::session::epoch_ms())
                })
                .map_err(|e| e.to_string());
            let _ = tx.send(result);
            None
        });
    }

    pub fn lifecycle(&mut self, ctx: &egui::Context) {
        self.pump_persistence();
        if self.exit_committed {
            return;
        }
        if self
            .windows
            .iter()
            .any(|w| w.closing && w.app.core.close_cancelled)
        {
            self.quitting = false;
            self.emit(ApplicationEvent::QuitCancelled);
            self.accepting_launches
                .store(true, std::sync::atomic::Ordering::Release);
            self.updates.install_on_quit = false;
            self.shutdown_started = false;
            for w in &mut self.windows {
                w.closing = false;
                w.app.core.is_closing = false;
                w.app.core.close_cancelled = false;
                w.app.core.close_ready = false;
                w.app.core.files.quit_confirmed = false;
            }
        }
        let mut order: Vec<_> = (0..self.windows.len()).collect();
        order.sort_by_key(|&index| Some(&self.windows[index].id) != self.active.as_ref());
        for index in order {
            let w = &mut self.windows[index];
            if w.closing && !w.app.core.close_ready {
                w.app.core.close_ready = w.app.core.request_quit();
                if self.quitting && !w.app.core.close_ready {
                    break;
                }
            }
        }
        if self.quitting {
            if self
                .windows
                .iter()
                .all(|w| w.app.core.close_ready && w.app.core.session.restore.is_none())
            {
                if !self.shutdown_started {
                    for w in &mut self.windows {
                        w.app.core.flush_recovery();
                        w.app.core.write_session(false);
                    }
                    self.shutdown_started = true;
                    return;
                }
                if self.windows.iter().any(|w| {
                    !w.app.core.io.is_idle()
                        || !w.app.core.fs.is_idle()
                        || !w.app.core.git_queue.is_idle()
                }) {
                    return;
                }
                self.persist(true);
                if !self.quitting || self.manifest_pending || !self.io.is_idle() {
                    return;
                }
                self.exit_committed = true;
                self.stop_mcp();
                for w in &mut self.windows {
                    w.app.core.shutdown();
                }
                if let Some(w) = self
                    .windows
                    .iter_mut()
                    .find(|w| w.app.core.updates.install_on_quit)
                {
                    w.app.core.updates_hand_off();
                }
                ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Close);
            }
        } else {
            let mut closed = false;
            for window in &self.windows {
                if window.closing && window.app.core.close_ready {
                    for (_, reply) in self.pending_mcp.iter().filter(|(id, _)| id == &window.id) {
                        reply.respond(notoir_mcp::ToolReply::error(
                            "window_closed",
                            "The target Notoir window closed",
                        ));
                    }
                }
            }
            let closing_ids: Vec<_> = self
                .windows
                .iter()
                .filter(|w| w.closing && w.app.core.close_ready)
                .map(|w| w.id.clone())
                .collect();
            for id in closing_ids {
                self.emit(ApplicationEvent::WindowClosed(id));
            }
            self.windows.retain_mut(|w| {
                if w.closing && w.app.core.close_ready {
                    w.app.core.shutdown();
                    tracing::info!(window_id = %w.id, "editor window closed");
                    closed = true;
                    false
                } else {
                    true
                }
            });
            if closed {
                if !self
                    .windows
                    .iter()
                    .any(|w| Some(&w.id) == self.active.as_ref())
                {
                    self.active = self.windows.last().map(|w| w.id.clone());
                }
                self.persist(true);
                if self.windows.is_empty() && !cfg!(target_os = "macos") {
                    self.io.wait_idle(Duration::from_secs(30));
                    self.exit_committed = true;
                    self.stop_mcp();
                    ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Close);
                }
            }
        }
    }

    fn ensure_fonts(&mut self, ctx: &egui::Context) {
        use notoir_platform::system_fonts;
        let mut scripts: Vec<_> = self
            .windows
            .iter()
            .flat_map(|w| w.app.required_scripts())
            .collect();
        scripts.sort();
        scripts.dedup();
        let mut faces = Vec::new();
        for script in scripts {
            if let Some(Some(face)) = system_fonts::ready(script) {
                faces.push(face);
            } else {
                let repaint = ctx.clone();
                system_fonts::request(script, move || repaint.request_repaint());
            }
        }
        let installed = faces.iter().map(|f| f.script).collect();
        if self.fonts_installed.as_ref() != Some(&installed)
            || self.family_generation != system_fonts::family_generation()
        {
            crate::fonts::install(ctx, &faces);
            self.fonts_installed = Some(installed);
            self.family_generation = system_fonts::family_generation();
        }
    }

    fn pump_configuration(&mut self) {
        if Instant::now() >= self.next_config_check {
            self.next_config_check = Instant::now() + Duration::from_secs(1);
            let paths = [
                self.opts.paths.user_settings_file(),
                self.opts.paths.keybindings_file(),
            ];
            let tx = self.config_tx.clone();
            self.io.submit(move || {
                let stamp = paths
                    .iter()
                    .filter_map(|p| std::fs::metadata(p).ok())
                    .filter_map(|m| Some((m.modified().ok()?, m.len())))
                    .max();
                let _ = tx.send(stamp);
                None
            });
        }
        for stamp in self.config_rx.try_iter() {
            if stamp != self.config_stamp {
                self.config_stamp = stamp;
                for window in &mut self.windows {
                    window.app.core.reload_config();
                }
            }
        }
    }

    pub fn flush_storage(&self) -> bool {
        self.io.wait_idle(Duration::from_secs(10))
    }

    fn start_mcp(&mut self) {
        let port = self.windows.first().map_or(notoir_mcp::DEFAULT_PORT, |w| {
            w.app.core.settings.int("mcp.port", None).clamp(0, 65535) as u16
        });
        self.mcp.start_transport(port);
    }

    fn stop_mcp(&mut self) {
        self.mcp.stop_transport();
        for (_, reply) in self.pending_mcp.drain(..) {
            reply.respond(notoir_mcp::ToolReply::error(
                "server_stopped",
                "The Notoir MCP server stopped",
            ));
        }
        for w in &mut self.windows {
            w.app.core.mcp_stop_local();
        }
    }

    fn pump_mcp(&mut self) {
        self.pending_mcp.retain(|(_, reply)| !reply.finished());
        use notoir_mcp::{ServerEvent, ToolReply};
        for event in self.mcp.take_events() {
            match event {
                ServerEvent::Call(call) => {
                    if self.quitting {
                        call.respond(ToolReply::error(
                            "application_quitting",
                            "Notoir is quitting",
                        ));
                        continue;
                    }
                    if call.tool == "window_list" {
                        call.respond(ToolReply::Ok {
                            structured: serde_json::json!({ "windows": self.window_info() }),
                            summary: "Notoir windows".into(),
                        });
                        continue;
                    }
                    let explicit = call.args.get("window_id").and_then(|v| v.as_str());
                    let target = explicit.or(self.active.as_deref());
                    if let Some(w) = self
                        .windows
                        .iter_mut()
                        .find(|w| Some(w.id.as_str()) == target)
                    {
                        self.pending_mcp.push((w.id.clone(), call.reply_handle()));
                        w.app.core.mcp.inject(ServerEvent::Call(call));
                    } else {
                        let code = if explicit.is_some() {
                            "window_not_found"
                        } else {
                            "no_active_window"
                        };
                        call.respond(ToolReply::error(code, "No matching Notoir window"));
                    }
                }
                event => self.mcp.receive_status(event),
            }
        }
        let url = self.mcp.url();
        for w in &mut self.windows {
            w.app.core.mcp.set_transport_status(url.clone(), &self.mcp);
        }
    }
}

impl App for ApplicationHost {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self
            .pending_focus
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed() > Duration::from_secs(2))
        {
            self.pending_focus = None;
        }
        if !self.exit_committed && ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.request(ctx, "", ApplicationRequest::Quit);
        }
        if !self.quitting {
            let requests: Vec<_> = self
                .instance_rx
                .as_ref()
                .map(|rx| rx.try_iter().collect())
                .unwrap_or_default();
            for (req, waiter) in requests {
                self.launch(ctx, req, waiter);
            }
        }
        self.update_results.extend(self.io.drain());
        if let Some(window) = self
            .windows
            .iter_mut()
            .find(|w| Some(&w.id) == self.active.as_ref())
        {
            let open = window.app.core.updates.open;
            window.app.core.updates = self.updates.clone();
            window.app.core.updates.open = open;
            for result in std::mem::take(&mut self.update_results) {
                match result {
                    crate::io::IoResult::UpdateStamp(ms) => window.app.core.updates_stamp(ms),
                    crate::io::IoResult::UpdateChecked(result) => {
                        let (result, manual) = *result;
                        window.app.core.updates_checked(result, manual);
                    }
                    crate::io::IoResult::UpdateDownloaded(result) => {
                        window.app.core.updates_downloaded(*result)
                    }
                    _ => {}
                }
            }
            self.updates = window.app.core.updates.clone();
        }
        self.pump_configuration();
        self.pump_mcp();
        #[cfg(target_os = "macos")]
        let info = self.window_info();
        #[cfg(target_os = "macos")]
        let mut requests: Vec<(String, ApplicationRequest)> = Vec::new();
        #[cfg(not(target_os = "macos"))]
        let requests: Vec<(String, ApplicationRequest)> = Vec::new();
        #[cfg(target_os = "macos")]
        {
            let core = self
                .windows
                .iter()
                .find(|w| Some(&w.id) == self.active.as_ref())
                .map(|w| &w.app.core);
            for (target, invocation) in self
                .native_menu
                .get_or_insert_with(|| crate::native_menu::NativeMenu::new(ctx))
                .update(core, &info)
            {
                if let Some(w) = self
                    .windows
                    .iter_mut()
                    .find(|w| Some(&w.id) == target.as_ref())
                {
                    w.app.core.enqueue(invocation);
                } else if invocation.id.as_str() == "file.newWindow" {
                    requests.push((String::new(), ApplicationRequest::NewWindow(None)));
                } else if invocation.id.as_str() == "window.bringAllToFront" {
                    requests.push((String::new(), ApplicationRequest::BringAllToFront));
                } else if invocation.id.as_str() == "app.quit" {
                    requests.push((String::new(), ApplicationRequest::Quit));
                } else {
                    let id = self.create_window(ctx, None, None);
                    self.windows
                        .iter_mut()
                        .find(|w| w.id == id)
                        .unwrap()
                        .app
                        .core
                        .enqueue(invocation);
                }
            }
        }
        for (source, request) in requests {
            self.request(ctx, &source, request);
        }
        let mut host_actions = Vec::new();
        for window in &mut self.windows {
            let core = &mut window.app.core;
            let open = core.updates.open;
            core.updates = self.updates.clone();
            core.updates.open = open;
            core.run_queued(&core.context(false));
            self.updates = core.updates.clone();
            let mut local = Vec::new();
            for effect in std::mem::take(&mut core.effects) {
                match effect {
                    crate::core::Effect::Application(request) => {
                        host_actions.push((window.id.clone(), request))
                    }
                    crate::core::Effect::Quit => {
                        host_actions.push((window.id.clone(), ApplicationRequest::Quit))
                    }
                    crate::core::Effect::CloseWindow => {
                        host_actions.push((window.id.clone(), ApplicationRequest::Close))
                    }
                    other => local.push(other),
                }
            }
            core.effects = local;
        }
        for (id, request) in host_actions {
            self.request(ctx, &id, request);
        }
        if let Some(plugin) = ctx.plugin_opt::<crate::input::WindowInput>() {
            let mut input = plugin.lock();
            input.blocked = self
                .windows
                .iter()
                .filter(|w| w.closing && w.app.core.close_ready)
                .map(EditorWindow::viewport)
                .collect();
        }
        let child_visible = ctx.input(|i| {
            i.raw
                .viewports
                .iter()
                .any(|(id, v)| *id != egui::ViewportId::ROOT && v.visible().unwrap_or(true))
        });
        if !child_visible {
            for w in &mut self.windows {
                w.app.core.now_ms = self.started.elapsed().as_millis() as u64;
                w.app.core.pump_io();
                w.app.core.mcp_tick();
                w.app.core.plugins_tick();
                w.app.core.recovery_tick();
                w.app.core.autosave_tick();
                w.app.core.flush_scratchpads(false);
                w.app.core.write_session(true);
            }
            self.lifecycle(ctx);
            self.persist(false);
            if self
                .windows
                .iter()
                .any(|w| ctx.input(|i| !i.raw.viewports.contains_key(&w.viewport())))
            {
                // eframe skips ui when the hidden root has no visible children.
                // Briefly enable the offscreen root to declare the editor viewports.
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            }
        }
        ctx.request_repaint_after(Duration::from_secs(1));
    }
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.ensure_fonts(&ctx);
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        let info = self.window_info();
        let mut requests: Vec<(String, ApplicationRequest)> = Vec::new();
        for window in &mut self.windows {
            window.app.core.window_list = info.clone();
            let open = window.app.core.updates.open;
            window.app.core.updates = self.updates.clone();
            window.app.core.updates.open = open;
            let viewport = window.viewport();
            let id = window.id.clone();
            if !ctx.input(|i| {
                i.raw
                    .viewports
                    .get(&viewport)
                    .is_some_and(|v| v.native_pixels_per_point.is_some())
            }) {
                // Let eframe materialize the native window from viewport output first.
                // On macOS some redraw paths have no current event-loop handle;
                // creating a brand-new immediate viewport there skips its callback.
                ctx.show_viewport_deferred(viewport, window.builder.clone(), |ui, _| {
                    ui.ctx().request_repaint_of(egui::ViewportId::ROOT);
                });
                ctx.request_repaint();
                continue;
            }
            ctx.show_viewport_immediate(viewport, window.builder.clone(), |ui, _| {
                if ui.input(|i| i.viewport().focused == Some(true))
                    && self
                        .pending_focus
                        .as_ref()
                        .is_none_or(|(target, _)| target == &id)
                {
                    self.pending_focus = None;
                    self.active = Some(id.clone());
                }
                ui.push_id(&id, |ui| {
                    window.app.ui(ui, frame);
                });
            });
            self.updates = window.app.core.updates.clone();
            requests.extend(
                std::mem::take(&mut window.app.core.application_requests)
                    .into_iter()
                    .map(|r| (id.clone(), r)),
            );
        }
        for (source, request) in requests {
            self.request(&ctx, &source, request);
        }
        self.lifecycle(&ctx);
        self.persist(false);
        let mut waiting = Vec::new();
        for mut wait in std::mem::take(&mut self.waits) {
            let open = wait.targets.iter().any(|(id, key)| {
                self.windows.iter().any(|w| {
                    &w.id == id
                        && w.app.core.workbench.find_by_key(key).is_some_and(|d| {
                            w.app
                                .core
                                .workbench
                                .views
                                .of_document(d)
                                .any(|v| !v.detached)
                        })
                })
            });
            if open {
                wait.seen = true;
            }
            if open || (!wait.seen && wait.started.elapsed() < Duration::from_secs(2)) {
                waiting.push(wait);
            } else {
                wait.waiter.finish();
            }
        }
        self.waits = waiting;
        // Periodic wake also services zero-window IPC, background completion and recovery.
        ctx.request_repaint_after(Duration::from_secs(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use notoir_core::permissions::{Capability, ClientId, Decision, Principal};
    use notoir_mcp::{McpCall, ServerEvent, ToolReply};
    use serde_json::json;

    fn host(home: &Path) -> (egui::Context, ApplicationHost) {
        let ctx = egui::Context::default();
        let mut opts = StartOptions::new(notoir_platform::AppPaths::under(home));
        opts.language_override = Some(crate::i18n::Language::En);
        opts.update = None;
        opts.dialogs = Some(std::sync::Arc::new(crate::files::ScriptedDialogs::default()));
        let host = ApplicationHost::with_options(&ctx, opts, false);
        (ctx, host)
    }

    // spec 40: "MCP requests retain their target"
    #[test]
    fn permission_prompt_and_execution_stay_in_original_window() {
        let home = tempfile::tempdir().unwrap();
        let root = home.path().join("notes");
        std::fs::create_dir(&root).unwrap();
        let file = root.join("a.txt");
        std::fs::write(&file, "before").unwrap();
        let (ctx, mut host) = host(home.path());
        let first = host.windows[0].id.clone();
        host.request(&ctx, &first, ApplicationRequest::OpenWorkspace(root));
        host.windows[0]
            .app
            .core
            .open_path(&file, Default::default());
        host.windows[0].app.core.flush_io();
        host.windows[0].app.core.pump_io();
        let second = host.create_window(&ctx, None, None);
        let client = ClientId("test-window-client".into());
        host.windows[0].app.core.mcp.gate.set(
            &Principal::McpClient(client.clone()),
            Capability::DocumentModify,
            Decision::Ask,
        );
        let (call, mut response) = McpCall::new(
            client.clone(),
            "document_write",
            json!({"window_id":first,"path":"a.txt","content":"after"}),
        );
        host.mcp.inject(ServerEvent::Call(call));
        host.pump_mcp();
        host.windows[0].app.core.mcp_tick();
        assert_eq!(host.windows[0].app.core.mcp.prompts.len(), 1);
        assert!(host.windows[1].app.core.mcp.prompts.is_empty());
        host.activate(&ctx, &second);
        host.windows[0]
            .app
            .core
            .mcp_answer(crate::mcp::PromptAnswer::Once);
        assert!(matches!(response.try_recv().unwrap(), ToolReply::Ok { .. }));
        let core = &host.windows[0].app.core;
        let doc = core.workbench.active_document().unwrap();
        assert_eq!(core.workbench.document(doc).unwrap().buffer.text(), "after");
        assert!(host.windows[1].app.core.workbench.documents.is_empty());
        // A pending permission request receives an explicit close error, never a sibling mutation.
        let (call, mut response) = McpCall::new(
            client,
            "document_write",
            json!({"window_id":first,"path":"a.txt","content":"never"}),
        );
        host.mcp.inject(ServerEvent::Call(call));
        host.pump_mcp();
        host.windows[0].app.core.mcp_tick();
        host.request(&ctx, &first, ApplicationRequest::Close);
        host.lifecycle(&ctx);
        assert!(
            matches!(response.try_recv().unwrap(), ToolReply::Err { code, .. } if code == "window_closed")
        );
        assert_eq!(host.active, Some(second));
        assert!(host.windows[0].app.core.workbench.documents.is_empty());
    }

    #[test]
    fn persistence_failure_aborts_quit_without_destroying_windows() {
        let home = tempfile::tempdir().unwrap();
        let (ctx, mut host) = host(home.path());
        let id = host.windows[0].id.clone();
        let blocked = home.path().join("not-a-directory");
        std::fs::write(&blocked, "blocked").unwrap();
        host.opts.paths.data = blocked;
        host.request(&ctx, &id, ApplicationRequest::Quit);
        host.lifecycle(&ctx);
        host.windows[0].app.core.flush_io();
        host.lifecycle(&ctx);
        assert!(host.flush_storage());
        host.lifecycle(&ctx);
        assert!(!host.quitting);
        assert!(!host.exit_committed);
        assert_eq!(host.windows.len(), 1);
        assert!(!host.windows[0].app.core.is_closing);
    }

    #[test]
    fn mcp_discovery_and_invalid_targets_are_explicit() {
        let home = tempfile::tempdir().unwrap();
        let (ctx, mut host) = host(home.path());
        host.create_window(&ctx, None, None);
        let client = ClientId("test".into());
        let (call, mut response) = McpCall::new(client.clone(), "window_list", json!({}));
        host.mcp.inject(ServerEvent::Call(call));
        host.pump_mcp();
        let ToolReply::Ok { structured, .. } = response.try_recv().unwrap() else {
            panic!("discovery failed")
        };
        assert_eq!(structured["windows"].as_array().unwrap().len(), 2);
        let (call, mut response) = McpCall::new(
            client.clone(),
            "workspace_get",
            json!({"window_id":"missing"}),
        );
        host.mcp.inject(ServerEvent::Call(call));
        host.pump_mcp();
        assert!(
            matches!(response.try_recv().unwrap(), ToolReply::Err { code, .. } if code == "window_not_found")
        );
        host.windows.clear();
        host.active = None;
        let (call, mut response) = McpCall::new(client, "workspace_get", json!({}));
        host.mcp.inject(ServerEvent::Call(call));
        host.pump_mcp();
        assert!(
            matches!(response.try_recv().unwrap(), ToolReply::Err { code, .. } if code == "no_active_window")
        );
    }
}
