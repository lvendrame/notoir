//! Lua plugins on the UI thread (docs/specs/30-lua-plugins.md, 32): discovery
//! and approval, declarative contributions (commands, keybindings, menus)
//! registered before any Lua runs, lazy activation, event delivery, the API
//! calls plugins make (each decided by the permission gate), errors and
//! automatic disabling. Plugin VMs run on their own threads
//! (`notoir-plugins`).

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use notoir_core::command::{
    Category, CommandId, CommandInvocation, CommandOutcome, CommandSpec, InvocationSource, Risk,
};
use notoir_core::document::DocumentOrigin;
use notoir_core::events::{AppEvent, ChangeOrigin};
use notoir_core::ids::{DocumentId, ViewId};
use notoir_core::keymap::{BindingSource, KeyBinding, KeyChord};
use notoir_core::notifications::{Notification, NotificationSource, Severity, Translatable};
use notoir_core::permissions::{Capability, Decision, Principal};
use notoir_core::when::When;
use notoir_editor::{ChangeSet, Edit, EditKind, Selection};
use notoir_plugins::{
    ApiError, ApiResult, Discovered, ErrorKind, FromPlugin, Manifest, ManifestError, PluginRequest,
    PluginRuntime, RuntimeConfig, ToPlugin,
};
use notoir_storage::state_db::PluginApprovalRow;
use serde_json::{Value, json};

use crate::core::AppCore;
use crate::files::OpenAt;
use crate::io::IoResult;

/// Plugin Output lines kept.
const OUTPUT_KEEP: usize = 2000;
/// Script errors within this window that disable a plugin.
const ERROR_WINDOW_MS: u64 = 60_000;
const ERRORS_TO_DISABLE: usize = 3;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PluginStatus {
    /// `plugin.toml` could not be read or is invalid.
    Invalid(String),
    /// Needs another plugin API version.
    Incompatible(String),
    /// Not approved (copied by hand) or an update asks for more.
    NeedsReview {
        update: bool,
    },
    Disabled,
    /// Approved and enabled; the VM starts on an activation event.
    Enabled,
    /// The VM is running.
    Active,
}

pub struct PluginEntry {
    pub id: String,
    pub dir: PathBuf,
    pub manifest: Option<Manifest>,
    pub status: PluginStatus,
    runtime: Option<PluginRuntime>,
    /// Script error times (ms) within the window.
    errors: VecDeque<u64>,
    pub last_error: Option<String>,
    /// Events the plugin's code subscribed to.
    pub subscribed: HashSet<String>,
    /// Commands its code registered (must be declared in the manifest).
    registered: HashSet<String>,
    /// Command ids contributed to the registry.
    contributed: Vec<CommandId>,
    /// API calls by name (Developer Tools).
    pub api_calls: std::collections::BTreeMap<String, u64>,
    /// What activated the VM (`onCommand:count`…).
    pub activated_by: Option<String>,
    /// Translation keys `notoir.i18n.t` did not find.
    pub missing: std::collections::BTreeSet<String>,
    /// Setting keys registered for the plugin (`plugins.<id>.<key>`).
    pub(crate) settings: Vec<String>,
    /// Languages it registered.
    pub(crate) languages: Vec<String>,
}

impl PluginEntry {
    pub fn name(&self) -> String {
        self.manifest
            .as_ref()
            .map_or_else(|| self.id.clone(), |m| m.plugin.name.clone())
    }

    pub fn active(&self) -> bool {
        self.status == PluginStatus::Active
    }
}

/// The approval prompt (install or review).
#[derive(Clone, Debug, PartialEq)]
pub struct PluginReview {
    pub source: PathBuf,
    pub manifest: Manifest,
    /// Copy the folder into the plugins directory on approval.
    pub install: bool,
    /// An installed version is replaced.
    pub update: bool,
}

/// A `notoir.process.run` waiting for the user.
pub struct ProcessPrompt {
    pub plugin: String,
    pub command_line: String,
    request: PluginRequest,
    cwd: PathBuf,
}

/// An MCP-style menu entry contributed by a plugin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginMenuItem {
    pub command: String,
    pub title: String,
    pub location: String,
}

pub struct PluginsState {
    pub entries: Vec<PluginEntry>,
    tx: notoir_plugins::PluginEvents,
    rx: Receiver<(u64, FromPlugin)>,
    /// Instance number of the next VM started.
    next_instance: u64,
    wake: Arc<dyn Fn() + Send + Sync>,
    /// Plugin Output: (plugin, level, message).
    pub output: VecDeque<(String, String, String)>,
    pub review: Option<PluginReview>,
    pub process_prompts: VecDeque<ProcessPrompt>,
    /// Plugin keybindings (part of the keymap).
    pub bindings: Vec<KeyBinding>,
    pub menu_items: Vec<PluginMenuItem>,
    /// A folder pick for Install from Folder is pending.
    install_pick: bool,
    /// Debounced `on_file_change`: document → due (ms).
    change_due: HashMap<DocumentId, u64>,
    /// The plugins directory scan finished (tests wait for it).
    pub scanned: bool,
    /// Contributions handled in `plugin_contrib.rs`.
    pub status_items: Vec<crate::plugin_contrib::PluginStatusItem>,
    pub panels: Vec<crate::plugin_contrib::PluginPanel>,
    /// (plugin, panel) → declarative UI tree.
    pub panel_trees: HashMap<(String, String), Value>,
    /// language → plugin.
    pub formatters: HashMap<String, String>,
    pub outlines: HashMap<String, String>,
    pub(crate) next_call: u64,
    pub(crate) calls: HashMap<u64, crate::plugin_contrib::ProvideCall>,
    /// Plugin outline symbols per document: (revision, symbols).
    pub outline_results: HashMap<DocumentId, (u64, Vec<notoir_editor::outline::Symbol>)>,
    /// `text_input` node values (panel UI state).
    pub input_buffers: HashMap<String, String>,
    pub manager: crate::plugin_contrib::ManagerUi,
    /// Developer Tools: the selected plugin.
    pub dev_plugin: Option<String>,
    /// `.notoir/plugins.toml` recommendations (id, version).
    pub recommended: Vec<(String, String)>,
    /// Uninstall waiting for confirmation.
    pub uninstall_confirm: Option<String>,
    pub(crate) zip_pick: bool,
}

impl PluginsState {
    pub fn new(wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        let (tx, rx) = channel();
        Self {
            entries: Vec::new(),
            tx,
            rx,
            next_instance: 1,
            wake,
            output: VecDeque::new(),
            review: None,
            process_prompts: VecDeque::new(),
            bindings: Vec::new(),
            menu_items: Vec::new(),
            install_pick: false,
            change_due: HashMap::new(),
            scanned: false,
            status_items: Vec::new(),
            panels: Vec::new(),
            panel_trees: HashMap::new(),
            formatters: HashMap::new(),
            outlines: HashMap::new(),
            next_call: 1,
            calls: HashMap::new(),
            outline_results: HashMap::new(),
            input_buffers: HashMap::new(),
            manager: Default::default(),
            dev_plugin: None,
            recommended: Vec::new(),
            uninstall_confirm: None,
            zip_pick: false,
        }
    }

    pub(crate) fn runtime(&self, id: &str) -> Option<&PluginRuntime> {
        self.entry(id).and_then(|e| e.runtime.as_ref())
    }

    pub fn entry(&self, id: &str) -> Option<&PluginEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    pub(crate) fn entry_mut(&mut self, id: &str) -> Option<&mut PluginEntry> {
        self.entries.iter_mut().find(|e| e.id == id)
    }

    pub(crate) fn log(&mut self, plugin: &str, level: &str, message: impl Into<String>) {
        let message = message.into();
        tracing::info!(plugin, level, message = message.as_str(), "plugin output");
        self.output
            .push_back((plugin.to_owned(), level.to_owned(), message));
        while self.output.len() > OUTPUT_KEEP {
            self.output.pop_front();
        }
    }
}

fn epoch_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

fn principal(id: &str) -> Principal {
    Principal::Plugin(id.to_owned())
}

fn denied(what: &str) -> ApiError {
    ApiError::new(
        "permission_denied",
        format!("{what} is not allowed for this plugin"),
    )
}

/// The plugin an event comes from.
fn plugin_of(e: &FromPlugin) -> &str {
    match e {
        FromPlugin::Request(r) => &r.plugin,
        FromPlugin::Activated { plugin }
        | FromPlugin::CommandDone { plugin, .. }
        | FromPlugin::Log { plugin, .. }
        | FromPlugin::Error { plugin, .. }
        | FromPlugin::Exited { plugin }
        | FromPlugin::Provided { plugin, .. }
        | FromPlugin::MissingTranslation { plugin, .. } => plugin,
    }
}

/// `plugin.<plugin-id>.<command>`.
pub fn command_id(plugin: &str, command: &str) -> String {
    format!("plugin.{plugin}.{command}")
}

/// The plugin and command of a `plugin.<id>.<command>` id.
fn split_command(id: &str) -> Option<(&str, &str)> {
    id.strip_prefix("plugin.")?.split_once('.')
}

/// `%key%` → the plugin's translation (in `table`), else the text itself.
fn resolve_title(title: &str, table: &HashMap<String, String>) -> String {
    match title.strip_prefix('%').and_then(|t| t.strip_suffix('%')) {
        Some(key) => table.get(key).cloned().unwrap_or_else(|| key.to_owned()),
        None => title.to_owned(),
    }
}

/// The handler of every plugin command.
fn run_plugin_command(core: &mut AppCore, inv: &CommandInvocation) -> CommandOutcome {
    let id = inv.id.as_str().to_owned();
    if let Some((plugin, command)) = split_command(&id) {
        core.plugin_run_command(plugin, command, inv.args.clone());
    }
    CommandOutcome::Done
}

impl AppCore {
    pub fn plugins_dir(&self) -> PathBuf {
        let custom = self.settings.str("plugins.directory", None);
        if custom.is_empty() {
            self.paths.plugins.clone()
        } else {
            PathBuf::from(custom)
        }
    }

    /// Scans the plugins directory (off the UI thread).
    pub fn plugins_scan(&mut self) {
        let dir = self.plugins_dir();
        self.io
            .submit(move || Some(IoResult::PluginsScanned(notoir_plugins::scan(&dir))));
    }

    /// Plugins → Reload Plugins: stops every VM, drops contributions and
    /// scans again.
    pub fn plugins_reload(&mut self) {
        let ids: Vec<String> = self.plugins.entries.iter().map(|e| e.id.clone()).collect();
        for id in ids {
            self.plugin_unload(&id);
        }
        self.plugins.entries.clear();
        self.plugins_rebuild_keymap();
        self.plugins_scan();
    }

    /// Scan results: statuses from approvals; enabled plugins contribute.
    pub fn plugins_scanned(&mut self, found: Vec<Discovered>) {
        let approvals: HashMap<String, PluginApprovalRow> = self
            .state_db
            .as_ref()
            .and_then(|db| db.plugin_approvals().ok())
            .unwrap_or_default()
            .into_iter()
            .map(|a| (a.plugin_id.clone(), a))
            .collect();
        let mut review = Vec::new();
        let mut seen = HashSet::new();
        for d in found {
            let folder = d
                .dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let (id, manifest, status) = match d.manifest {
                Err(ManifestError::Incompatible(api)) => {
                    (folder, None, PluginStatus::Incompatible(api))
                }
                Err(e) => (folder, None, PluginStatus::Invalid(e.to_string())),
                Ok(m) => {
                    let id = m.plugin.id.clone();
                    let status = match approvals.get(&id) {
                        Some(a) if a.permissions_hash == m.permissions.hash() => {
                            if a.version != m.plugin.version
                                && let Some(db) = &self.state_db
                            {
                                // Same permissions: the approval carries over.
                                let _ = db.set_plugin_approval(&PluginApprovalRow {
                                    version: m.plugin.version.clone(),
                                    ..a.clone()
                                });
                            }
                            if a.enabled {
                                PluginStatus::Enabled
                            } else {
                                PluginStatus::Disabled
                            }
                        }
                        Some(_) => PluginStatus::NeedsReview { update: true },
                        None => PluginStatus::NeedsReview { update: false },
                    };
                    (id, Some(m), status)
                }
            };
            if !seen.insert(id.clone()) {
                self.plugins
                    .log(&id, "error", "Another plugin has the same id; ignored");
                continue;
            }
            // Reload of a single plugin: the old entry goes.
            if self.plugins.entry(&id).is_some() {
                self.plugin_unload(&id);
                self.plugins.entries.retain(|e| e.id != id);
            }
            if let PluginStatus::Invalid(e) = &status {
                self.plugins
                    .log(&id, "error", format!("Invalid plugin: {e}"));
            }
            if matches!(status, PluginStatus::NeedsReview { .. }) {
                review.push((id.clone(), manifest.as_ref().map(|m| m.plugin.name.clone())));
            }
            let enabled = status == PluginStatus::Enabled;
            self.plugins.entries.push(PluginEntry {
                id: id.clone(),
                dir: d.dir,
                manifest,
                status,
                runtime: None,
                errors: VecDeque::new(),
                last_error: None,
                subscribed: HashSet::new(),
                registered: HashSet::new(),
                contributed: Vec::new(),
                api_calls: Default::default(),
                activated_by: None,
                missing: Default::default(),
                settings: Vec::new(),
                languages: Vec::new(),
            });
            if enabled {
                self.plugin_contribute(&id);
            }
        }
        self.plugins_rebuild_keymap();
        for (id, name) in review {
            self.notify(
                Notification::new(
                    Severity::Info,
                    Translatable::new("plugins:needsReview")
                        .arg("name", name.unwrap_or(id.clone())),
                )
                .action(
                    "plugins:reviewAction",
                    CommandInvocation::new("plugins.review", InvocationSource::Internal)
                        .with_args(json!({ "plugin": id })),
                )
                .source(NotificationSource::Plugin),
            );
        }
        self.plugins.scanned = true;
        // onStartup activation.
        let startup: Vec<String> = self
            .plugins
            .entries
            .iter()
            .filter(|e| {
                e.status == PluginStatus::Enabled
                    && e.manifest
                        .as_ref()
                        .is_some_and(|m| m.activates_on("onStartup"))
            })
            .map(|e| e.id.clone())
            .collect();
        for id in startup {
            self.plugin_activate_by(&id, "onStartup");
        }
    }

    /// Registers an enabled plugin's commands, keybindings and menus, and its
    /// approved permissions with the gate.
    fn plugin_contribute(&mut self, id: &str) {
        let Some(entry) = self.plugins.entry(id) else {
            return;
        };
        let Some(m) = entry.manifest.clone() else {
            return;
        };
        let dir = entry.dir.clone();
        let p = principal(id);
        self.mcp.gate.reset(&p);
        for cap in m.permissions.capabilities() {
            self.mcp.gate.set(&p, cap, Decision::Allow);
        }
        if m.permissions.process {
            let always = self
                .state_db
                .as_ref()
                .and_then(|db| db.plugin_grants(id).ok())
                .unwrap_or_default()
                .iter()
                .any(|(c, d)| c == "process" && d == "allow");
            let d = if always {
                Decision::Allow
            } else {
                Decision::Ask
            };
            self.mcp.gate.set(&p, Capability::Process, d);
        }
        let table = notoir_plugins::runtime::load_locales(&dir, crate::i18n::language().code());
        let mut contributed = Vec::new();
        for c in &m.contributes.commands {
            let full = command_id(id, &c.id);
            let title_key = format!("@plugin/{id}/{}", c.id);
            crate::i18n::set_dynamic(&title_key, &resolve_title(&c.title, &table));
            let spec = CommandSpec {
                id: CommandId::from(full.clone()),
                title_key,
                category: Category::Plugins,
                icon: None,
                enablement: When::ALWAYS,
                palette: true,
                default_keys: Vec::new(),
                risk: Risk::Safe,
                handler: run_plugin_command,
            };
            if self.commands.register(spec).is_ok() {
                contributed.push(CommandId::from(full));
            }
        }
        for k in &m.contributes.keybindings {
            let when = match k.when.as_deref().map(When::parse) {
                None => When::ALWAYS,
                Some(Ok(w)) => w,
                Some(Err(e)) => {
                    self.plugins.log(
                        id,
                        "error",
                        format!(
                            "Keybinding for \"{}\" not registered: invalid when-expression \"{}\": {e}",
                            k.command,
                            k.when.as_deref().unwrap_or_default()
                        ),
                    );
                    continue;
                }
            };
            match KeyChord::parse(&k.key, self.platform) {
                Ok(chord) => self.plugins.bindings.push(KeyBinding {
                    chord,
                    command: CommandId::from(command_id(id, &k.command)),
                    args: Value::Null,
                    when,
                    source: BindingSource::Plugin,
                }),
                Err(e) => self.plugins.log(
                    id,
                    "error",
                    format!("Keybinding \"{}\" not registered: {e}", k.key),
                ),
            }
        }
        for menu in &m.contributes.menus {
            let title = m
                .contributes
                .commands
                .iter()
                .find(|c| c.id == menu.command)
                .map(|c| resolve_title(&c.title, &table))
                .unwrap_or_default();
            self.plugins.menu_items.push(PluginMenuItem {
                command: command_id(id, &menu.command),
                title,
                location: menu.location.clone(),
            });
        }
        if let Some(e) = self.plugins.entry_mut(id) {
            e.contributed = contributed;
        }
        self.plugin_contribute_more(id, &m, &dir, &table);
    }

    /// Stops the VM and removes the plugin's contributions.
    pub(crate) fn plugin_unload(&mut self, id: &str) {
        let Some(e) = self.plugins.entry_mut(id) else {
            return;
        };
        e.runtime = None;
        e.registered.clear();
        e.subscribed.clear();
        let contributed = std::mem::take(&mut e.contributed);
        if e.status == PluginStatus::Active {
            e.status = PluginStatus::Enabled;
        }
        for c in contributed {
            self.commands.unregister(&c);
        }
        let prefix = format!("plugin.{id}.");
        self.plugins
            .bindings
            .retain(|b| !b.command.as_str().starts_with(&prefix));
        self.plugins
            .menu_items
            .retain(|m| !m.command.starts_with(&prefix));
        crate::i18n::clear_dynamic(&format!("@plugin/{id}/"));
        self.mcp.gate.reset(&principal(id));
        self.plugin_unload_more(id);
    }

    pub(crate) fn plugins_rebuild_keymap(&mut self) {
        self.keymap = crate::core::build_keymap(
            &self.commands,
            &self.plugins.bindings,
            &self.user_bindings,
            self.platform,
            &mut self.notifications,
        );
    }

    /// Starts the VM (runs `main.lua`) if it is not running.
    pub fn plugin_activate(&mut self, id: &str) {
        self.plugin_activate_by(id, "manual");
    }

    pub fn plugin_activate_by(&mut self, id: &str, reason: &str) {
        let language = crate::i18n::language().code().to_owned();
        let memory = self.settings.int("plugins.memoryLimitMb", None).max(8) as usize;
        let command_ms = self.settings.int("plugins.commandTimeoutMs", None).max(100) as u64;
        let event_ms = self.settings.int("plugins.eventTimeoutMs", None).max(10) as u64;
        let tx = self.plugins.tx.clone();
        let wake = self.plugins.wake.clone();
        let Some(e) = self.plugins.entry_mut(id) else {
            return;
        };
        if e.status != PluginStatus::Enabled || e.runtime.is_some() || e.manifest.is_none() {
            return;
        }
        let instance = self.plugins.next_instance;
        self.plugins.next_instance += 1;
        let Some(e) = self.plugins.entry_mut(id) else {
            return;
        };
        let Some(m) = &e.manifest else {
            return;
        };
        let cfg = RuntimeConfig {
            id: id.to_owned(),
            instance,
            dir: e.dir.clone(),
            main: m.plugin.main.clone(),
            permissions: m.permissions.clone(),
            language,
            memory_limit_mb: memory,
            command_budget: Duration::from_millis(command_ms),
            event_budget: Duration::from_millis(event_ms),
        };
        e.runtime = Some(PluginRuntime::start(cfg, tx, wake));
        e.status = PluginStatus::Active;
        e.activated_by = Some(reason.to_owned());
        self.plugins.log(id, "info", "Activating");
    }

    /// Runs a plugin command, activating the plugin first.
    pub fn plugin_run_command(&mut self, plugin: &str, command: &str, args: Value) {
        self.plugin_activate_by(plugin, &format!("onCommand:{command}"));
        if let Some(rt) = self.plugins.entry(plugin).and_then(|e| e.runtime.as_ref()) {
            rt.send(ToPlugin::Command {
                command: command.to_owned(),
                args,
            });
        }
    }

    // ── per frame ────────────────────────────────────────────────────────

    pub fn plugins_tick(&mut self) {
        while let Ok((instance, e)) = self.plugins.rx.try_recv() {
            // Late events of a VM that was replaced (reload, disable +
            // enable) must not touch the current one.
            let current = self.plugins.runtime(plugin_of(&e)).map(|rt| rt.instance);
            if current != Some(instance) {
                match e {
                    FromPlugin::Request(r) => r.respond(Err(ApiError::new(
                        "stopped",
                        "This plugin instance was stopped",
                    ))),
                    FromPlugin::Log {
                        plugin,
                        level,
                        message,
                    } => self.plugins.log(&plugin, &level, message),
                    _ => {}
                }
                continue;
            }
            match e {
                FromPlugin::Request(r) => self.plugin_request(r),
                FromPlugin::Activated { plugin } => self.plugins.log(&plugin, "info", "Activated"),
                FromPlugin::CommandDone { .. } => {}
                FromPlugin::Log {
                    plugin,
                    level,
                    message,
                } => self.plugins.log(&plugin, &level, message),
                FromPlugin::Error {
                    plugin,
                    during,
                    message,
                    kind,
                } => self.plugin_error(&plugin, &during, &message, kind),
                FromPlugin::Provided {
                    plugin,
                    call,
                    result,
                } => self.plugin_provided(&plugin, call, result),
                FromPlugin::MissingTranslation { plugin, key } => {
                    if let Some(e) = self.plugins.entry_mut(&plugin) {
                        e.missing.insert(key);
                    }
                }
                FromPlugin::Exited { plugin } => {
                    if let Some(e) = self.plugins.entry_mut(&plugin) {
                        e.runtime = None;
                        e.registered.clear();
                        e.subscribed.clear();
                        if e.status == PluginStatus::Active {
                            e.status = PluginStatus::Enabled;
                        }
                    }
                }
            }
        }
        // Debounced change events.
        let now = self.now_ms;
        let due: Vec<DocumentId> = self
            .plugins
            .change_due
            .iter()
            .filter(|(_, t)| now >= **t)
            .map(|(d, _)| *d)
            .collect();
        for doc in due {
            self.plugins.change_due.remove(&doc);
            let payload = self.plugin_doc_payload(doc);
            self.plugins_emit("on_file_change", payload);
        }
    }

    /// Next debounce deadline (repaint scheduling).
    pub fn plugins_next_due(&self) -> Option<u64> {
        self.plugins.change_due.values().copied().min()
    }

    fn plugin_error(&mut self, plugin: &str, during: &str, message: &str, kind: ErrorKind) {
        self.plugins
            .log(plugin, "error", format!("Error in {during}: {message}"));
        let now = self.now_ms;
        let Some(e) = self.plugins.entry_mut(plugin) else {
            return;
        };
        e.last_error = Some(format!("{during}: {message}"));
        e.errors.push_back(now);
        while e
            .errors
            .front()
            .is_some_and(|t| now.saturating_sub(*t) > ERROR_WINDOW_MS)
        {
            e.errors.pop_front();
        }
        let disable = kind == ErrorKind::Limit || e.errors.len() >= ERRORS_TO_DISABLE;
        let name = e.name();
        let view = CommandInvocation::new("plugins.viewError", InvocationSource::Internal)
            .with_args(json!({ "plugin": plugin }));
        if disable {
            self.plugin_set_enabled(plugin, false);
            self.notify(
                Notification::new(
                    Severity::Error,
                    Translatable::new("plugins:errorDisabled").arg("name", name),
                )
                .detail(message.to_owned())
                .action("plugins:viewError", view)
                .action(
                    "plugins:reenable",
                    CommandInvocation::new("plugins.enable", InvocationSource::Internal)
                        .with_args(json!({ "plugin": plugin })),
                )
                .source(NotificationSource::Plugin),
            );
        } else {
            self.notify(
                Notification::new(
                    Severity::Warning,
                    Translatable::new("plugins:error").arg("name", name),
                )
                .detail(message.to_owned())
                .action("plugins:viewError", view)
                .source(NotificationSource::Plugin),
            );
        }
    }

    /// Enable / Disable (persisted).
    pub fn plugin_set_enabled(&mut self, id: &str, enabled: bool) {
        let Some(e) = self.plugins.entry(id) else {
            return;
        };
        let ok = matches!(
            e.status,
            PluginStatus::Enabled | PluginStatus::Active | PluginStatus::Disabled
        );
        if !ok {
            return;
        }
        if let Some(db) = &self.state_db {
            let _ = db.set_plugin_enabled(id, enabled);
        }
        if enabled {
            if let Some(e) = self.plugins.entry_mut(id)
                && e.status == PluginStatus::Disabled
            {
                e.status = PluginStatus::Enabled;
                e.errors.clear();
                self.plugin_contribute(id);
                self.plugins.log(id, "info", "Enabled");
            }
        } else {
            self.plugin_unload(id);
            if let Some(e) = self.plugins.entry_mut(id) {
                e.status = PluginStatus::Disabled;
            }
            self.plugins.log(id, "info", "Disabled");
        }
        self.plugins_rebuild_keymap();
    }

    // ── approval ─────────────────────────────────────────────────────────

    /// Review an installed plugin (needs review / update).
    pub fn plugin_review(&mut self, id: &str) -> bool {
        let Some(e) = self.plugins.entry(id) else {
            return false;
        };
        let Some(m) = e.manifest.clone() else {
            return false;
        };
        let update = matches!(e.status, PluginStatus::NeedsReview { update: true });
        self.plugins.review = Some(PluginReview {
            source: e.dir.clone(),
            manifest: m,
            install: false,
            update,
        });
        true
    }

    /// Install from Folder…: pick a folder, then review it.
    pub fn plugins_install_from_folder(&mut self) {
        self.plugins.install_pick = true;
        let opts = notoir_platform::dialogs::FileDialogOptions {
            title: Some(crate::t!("plugins:installTitle")),
            directory: None,
            file_name: None,
        };
        self.files.dialogs.pick_folder(opts, self.io.poster());
    }

    /// A folder picked for installation (true when it was ours).
    pub fn plugins_folder_picked(&mut self, path: Option<&Path>) -> bool {
        if !std::mem::take(&mut self.plugins.install_pick) {
            return false;
        }
        if let Some(dir) = path {
            self.plugins_install_from(dir.to_path_buf());
        }
        true
    }

    /// Reads a plugin folder's manifest (off the UI thread), then reviews it.
    pub fn plugins_install_from(&mut self, dir: PathBuf) {
        self.io.submit(move || {
            let result = Manifest::load(&dir).map_err(|e| e.to_string());
            Some(IoResult::PluginManifest {
                dir,
                result: Box::new(result),
            })
        });
    }

    pub fn plugins_manifest_read(&mut self, dir: PathBuf, result: Result<Manifest, String>) {
        match result {
            Ok(m) => {
                let update = self.plugins.entry(&m.plugin.id).is_some();
                self.plugins.review = Some(PluginReview {
                    source: dir,
                    manifest: m,
                    install: true,
                    update,
                });
            }
            Err(e) => self.notify(
                Notification::new(Severity::Error, Translatable::new("plugins:installInvalid"))
                    .detail(e)
                    .source(NotificationSource::Plugin),
            ),
        }
    }

    /// The user answered the approval prompt.
    pub fn plugins_review_answer(&mut self, approve: bool) {
        let Some(review) = self.plugins.review.take() else {
            return;
        };
        if !approve {
            return;
        }
        let m = &review.manifest;
        let id = m.plugin.id.clone();
        let row = PluginApprovalRow {
            plugin_id: id.clone(),
            version: m.plugin.version.clone(),
            permissions_hash: m.permissions.hash(),
            approved_at: epoch_ms(),
            enabled: true,
        };
        if let Some(db) = &self.state_db {
            let _ = db.set_plugin_approval(&row);
        }
        if review.install {
            let target = self.plugins_dir().join(&id);
            let source = review.source.clone();
            // Stop the old version before its files are replaced.
            self.plugin_unload(&id);
            self.io.submit(move || {
                let r = install_copy(&source, &target);
                Some(IoResult::PluginInstalled {
                    id,
                    error: r.err().map(|e| e.to_string()),
                })
            });
        } else {
            if let Some(e) = self.plugins.entry_mut(&id) {
                e.status = PluginStatus::Enabled;
                e.errors.clear();
            }
            self.plugin_contribute(&id);
            self.plugins_rebuild_keymap();
            self.plugins.log(&id, "info", "Approved and enabled");
        }
    }

    pub fn plugins_installed(&mut self, id: String, error: Option<String>) {
        match error {
            None => {
                self.plugins.log(&id, "info", "Installed");
                self.plugins_reload();
            }
            Some(e) => self.notify(
                Notification::new(
                    Severity::Error,
                    Translatable::new("plugins:installFailed").arg("name", id),
                )
                .detail(e)
                .source(NotificationSource::Plugin),
            ),
        }
    }

    /// The process prompt's answer: `once`, `always` or `deny`.
    pub fn plugins_process_answer(&mut self, answer: &str) {
        let Some(p) = self.plugins.process_prompts.pop_front() else {
            return;
        };
        match answer {
            "once" | "always" => {
                if answer == "always" {
                    self.mcp
                        .gate
                        .set(&principal(&p.plugin), Capability::Process, Decision::Allow);
                    if let Some(db) = &self.state_db {
                        let _ = db.set_plugin_grant(&p.plugin, "process", Some("allow"));
                    }
                }
                let cwd = p.cwd.to_string_lossy().into_owned();
                p.request.respond(Ok(json!({ "cwd": cwd })));
            }
            _ => p.request.respond(Err(ApiError::new(
                "permission_denied",
                "The user did not allow running this program",
            ))),
        }
    }

    // ── events ───────────────────────────────────────────────────────────

    pub(crate) fn plugin_doc_payload(&self, doc: DocumentId) -> Value {
        let Some(d) = self.workbench.document(doc) else {
            return json!({ "document": doc.0 });
        };
        json!({
            "document": doc.0,
            "path": d.path().map(|p| p.to_string_lossy().into_owned()),
            "title": self.document_title(doc),
            "language": d.language.as_str(),
            "dirty": d.is_dirty(),
        })
    }

    /// Workbench events → plugin events (spec 06 §Lua Event Mapping).
    pub fn plugins_event(&mut self, event: &AppEvent) {
        if self.plugins.entries.is_empty() {
            return;
        }
        let (name, payload) = match event {
            AppEvent::DocumentOpened { doc } => {
                let lang = self
                    .workbench
                    .document(*doc)
                    .map(|d| d.language.as_str().to_owned());
                if let Some(lang) = lang {
                    self.plugins_activate_on(&format!("onLanguage:{lang}"));
                }
                ("on_file_open", self.plugin_doc_payload(*doc))
            }
            AppEvent::DocumentSaved { doc } => ("on_file_save", self.plugin_doc_payload(*doc)),
            AppEvent::DocumentClosed { doc } => ("on_file_close", json!({ "document": doc.0 })),
            AppEvent::DocumentChanged { doc, .. } => {
                let ms = self
                    .settings
                    .int("plugins.changeEventDebounceMs", None)
                    .max(0) as u64;
                self.plugins.change_due.insert(*doc, self.now_ms + ms);
                return;
            }
            AppEvent::ActiveDocumentChanged { doc, .. } => {
                if let Some(lang) = doc
                    .and_then(|d| self.workbench.document(d))
                    .map(|d| d.language.as_str().to_owned())
                {
                    self.plugins_activate_on(&format!("onLanguage:{lang}"));
                }
                (
                    "on_active_editor_change",
                    doc.map_or(Value::Null, |d| self.plugin_doc_payload(d)),
                )
            }
            AppEvent::WorkspaceOpened => (
                "on_workspace_open",
                json!({ "root": self.workspace.as_ref().map(|w| w.root.to_string_lossy().into_owned()) }),
            ),
            AppEvent::WorkspaceClosed => ("on_workspace_close", Value::Null),
            _ => return,
        };
        self.plugins_activate_on(&format!("onEvent:{name}"));
        self.plugins_emit(name, payload);
    }

    fn plugins_activate_on(&mut self, event: &str) {
        let ids: Vec<String> = self
            .plugins
            .entries
            .iter()
            .filter(|e| {
                e.status == PluginStatus::Enabled
                    && e.manifest.as_ref().is_some_and(|m| m.activates_on(event))
            })
            .map(|e| e.id.clone())
            .collect();
        for id in ids {
            self.plugin_activate_by(&id, event);
        }
    }

    /// Delivers an event to active plugins. Plugins that were activated but
    /// whose code has not subscribed yet still get it queued (their VM
    /// handles it after `main`).
    fn plugins_emit(&mut self, name: &str, payload: Value) {
        for e in &self.plugins.entries {
            let Some(rt) = &e.runtime else {
                continue;
            };
            let declared = e
                .manifest
                .as_ref()
                .is_some_and(|m| m.activates_on(&format!("onEvent:{name}")));
            if e.subscribed.contains(name) || declared {
                rt.send(ToPlugin::Event {
                    name: name.to_owned(),
                    payload: payload.clone(),
                });
            }
        }
    }

    // ── API requests ─────────────────────────────────────────────────────

    fn plugin_check(&self, plugin: &str, cap: &Capability, what: &str) -> Result<(), ApiError> {
        match self.mcp.gate.check(&principal(plugin), cap, true) {
            Decision::Allow => Ok(()),
            _ => Err(denied(what)),
        }
    }

    fn plugin_request(&mut self, r: PluginRequest) {
        let active = self
            .plugins
            .entry(&r.plugin)
            .is_some_and(PluginEntry::active);
        if !active {
            r.respond(Err(ApiError::new("disabled", "The plugin is not active")));
            return;
        }
        if let Some(e) = self.plugins.entry_mut(&r.plugin) {
            *e.api_calls.entry(r.name.clone()).or_default() += 1;
        }
        if r.name == "process.check" {
            self.plugin_process_check(r);
            return;
        }
        let result = self.plugin_api(&r.plugin, &r.name, &r.args);
        r.respond(result);
    }

    fn plugin_view(&self, args: &Value) -> Result<ViewId, ApiError> {
        let v = args
            .get("view")
            .and_then(Value::as_u64)
            .map(ViewId)
            .or_else(|| self.workbench.active_editor())
            .ok_or_else(|| ApiError::new("no_editor", "No editor is active"))?;
        self.workbench
            .view(v)
            .map(|_| v)
            .ok_or_else(|| ApiError::new("no_editor", "The editor was closed"))
    }

    fn plugin_api(&mut self, plugin: &str, name: &str, args: &Value) -> ApiResult {
        let s = |k: &str| args.get(k).and_then(Value::as_str).map(str::to_owned);
        match name {
            "commands.declared" => {
                let id = s("id").unwrap_or_default();
                let declared = self.plugins.entry(plugin).is_some_and(|e| {
                    e.manifest
                        .as_ref()
                        .is_some_and(|m| m.contributes.commands.iter().any(|c| c.id == id))
                });
                if !declared {
                    return Err(ApiError::new(
                        "not_declared",
                        format!("command \"{id}\" is not declared in plugin.toml"),
                    ));
                }
                if let Some(e) = self.plugins.entry_mut(plugin) {
                    e.registered.insert(id);
                }
                Ok(Value::Null)
            }
            "notify" => {
                let level = match s("level").as_deref() {
                    Some("error") => Severity::Error,
                    Some("warning") => Severity::Warning,
                    _ => Severity::Info,
                };
                let name = self
                    .plugins
                    .entry(plugin)
                    .map(PluginEntry::name)
                    .unwrap_or_default();
                self.notify(
                    Notification::new(
                        level,
                        Translatable::new("plugins:notify")
                            .arg("name", name)
                            .arg("message", s("message").unwrap_or_default()),
                    )
                    .source(NotificationSource::Plugin),
                );
                Ok(Value::Null)
            }
            "commands.execute" => {
                let id = s("id").unwrap_or_default();
                let own = id.starts_with(&format!("plugin.{plugin}."));
                let safe = self
                    .commands
                    .get(&CommandId::from(id.clone()))
                    .is_some_and(|c| c.risk == Risk::Safe);
                if !own
                    && !(safe
                        || self.mcp.gate.check(
                            &principal(plugin),
                            &Capability::ExecuteCommand(id.clone()),
                            true,
                        ) == Decision::Allow)
                {
                    return Err(denied(&format!("running command {id}")));
                }
                if self.commands.get(&CommandId::from(id.clone())).is_none() {
                    return Err(ApiError::new("unknown_command", format!("No command {id}")));
                }
                self.enqueue(
                    CommandInvocation::new(id, InvocationSource::Internal)
                        .with_args(args.get("args").cloned().unwrap_or(Value::Null)),
                );
                Ok(Value::Null)
            }
            "events.subscribe" => {
                if let (Some(e), Some(n)) = (self.plugins.entry_mut(plugin), s("name")) {
                    e.subscribed.insert(n);
                }
                Ok(Value::Null)
            }
            "editor.active" => {
                self.plugin_check(plugin, &Capability::EditorRead, "reading the editor")?;
                // Settings, Welcome and Plugin Manager tabs are not editors.
                let view = self.workbench.active_editor().filter(|v| {
                    self.workbench
                        .view(*v)
                        .and_then(|v| self.workbench.document(v.document_id))
                        .is_some_and(|d| !matches!(d.origin, DocumentOrigin::Virtual { .. }))
                });
                Ok(view.map_or(Value::Null, |v| json!(v.0)))
            }
            "editor.text" | "editor.selection" | "editor.selections" => {
                self.plugin_check(plugin, &Capability::DocumentRead, "reading documents")?;
                let view = self.plugin_view(args)?;
                let v = self.workbench.view(view).expect("checked");
                let d = self
                    .workbench
                    .document(v.document_id)
                    .ok_or_else(|| ApiError::new("no_editor", "The document was closed"))?;
                let rope = d.buffer.rope();
                let range = |r: &notoir_editor::selection::Range| {
                    let mut j = crate::mcp_tools::range_json(rope, r.from(), r.to());
                    j["text"] = json!(rope.slice(r.from()..r.to()).to_string());
                    j
                };
                Ok(match name {
                    "editor.text" => json!(d.buffer.text()),
                    "editor.selection" => range(&v.selection.primary()),
                    _ => Value::Array(v.selection.ranges().iter().map(range).collect()),
                })
            }
            "editor.path" | "editor.language" => {
                self.plugin_check(plugin, &Capability::EditorRead, "reading the editor")?;
                let view = self.plugin_view(args)?;
                let doc = self.workbench.view(view).map(|v| v.document_id);
                let d = doc.and_then(|d| self.workbench.document(d));
                Ok(match name {
                    "editor.path" => d
                        .and_then(|d| d.path())
                        .map_or(Value::Null, |p| json!(p.to_string_lossy())),
                    _ => d.map_or(Value::Null, |d| json!(d.language.as_str())),
                })
            }
            "editor.replace" | "editor.insert" => {
                self.plugin_check(plugin, &Capability::DocumentModify, "editing documents")?;
                let view = self.plugin_view(args)?;
                let text = s("text").unwrap_or_default();
                let (doc, rope, sel) = {
                    let v = self.workbench.view(view).expect("checked");
                    let d = self
                        .workbench
                        .document(v.document_id)
                        .ok_or_else(|| ApiError::new("no_editor", "The document was closed"))?;
                    (v.document_id, d.buffer.rope().clone(), v.selection.clone())
                };
                let (start, end) = if name == "editor.insert" {
                    let h = sel.primary().head;
                    (h, h)
                } else {
                    let range = args.get("range").unwrap_or(&Value::Null);
                    let p = |k| {
                        crate::mcp_tools::position(&rope, range.get(k)).map_err(|_| {
                            ApiError::new("invalid_range", "The range is outside the document")
                        })
                    };
                    let (a, b) = (p("start")?, p("end")?);
                    (a.min(b), a.max(b))
                };
                let caret = start + text.chars().count();
                let changes = ChangeSet::new(vec![Edit::new(start, end, text)])
                    .map_err(|e| ApiError::new("invalid_range", e.to_string()))?;
                self.workbench
                    .apply_edit(
                        doc,
                        Some(view),
                        changes,
                        Selection::caret(caret),
                        EditKind::Other,
                        ChangeOrigin::Plugin(plugin.to_owned()),
                        self.now_ms,
                    )
                    .map_err(|e| ApiError::new("edit_failed", e.to_string()))?;
                Ok(Value::Null)
            }
            "editor.set_selection" => {
                self.plugin_check(plugin, &Capability::EditorControl, "changing the selection")?;
                let view = self.plugin_view(args)?;
                let rope = self
                    .workbench
                    .view(view)
                    .and_then(|v| self.workbench.document(v.document_id))
                    .map(|d| d.buffer.rope().clone())
                    .ok_or_else(|| ApiError::new("no_editor", "The document was closed"))?;
                let range = args.get("range").unwrap_or(&Value::Null);
                let p = |k| {
                    crate::mcp_tools::position(&rope, range.get(k)).map_err(|_| {
                        ApiError::new("invalid_range", "The range is outside the document")
                    })
                };
                let (a, b) = (p("start")?, p("end")?);
                let _ = self.workbench.set_selection(
                    view,
                    Selection::single(notoir_editor::selection::Range::new(a, b)),
                );
                Ok(Value::Null)
            }
            "documents.list" => {
                self.plugin_check(plugin, &Capability::DocumentRead, "reading documents")?;
                let list: Vec<Value> = self
                    .workbench
                    .documents
                    .values()
                    .filter(|d| !matches!(d.origin, DocumentOrigin::Virtual { .. }))
                    .map(|d| self.plugin_doc_payload(d.id))
                    .collect();
                Ok(Value::Array(list))
            }
            "documents.open" => {
                self.plugin_check(plugin, &Capability::DocumentRead, "opening documents")?;
                let path = self.plugin_path(&s("path").unwrap_or_default())?;
                self.open_path(
                    &path,
                    OpenAt {
                        keep_focus: true,
                        ..Default::default()
                    },
                );
                Ok(Value::Null)
            }
            "workspace.root" => {
                self.plugin_check(plugin, &Capability::WorkspaceRead, "reading the workspace")?;
                Ok(self
                    .workspace
                    .as_ref()
                    .map_or(Value::Null, |w| json!(w.root.to_string_lossy())))
            }
            "workspace.find_files" => {
                self.plugin_check(plugin, &Capability::WorkspaceRead, "reading the workspace")?;
                let Some(ws) = &self.workspace else {
                    return Ok(json!([]));
                };
                let glob = s("glob").unwrap_or_else(|| "**/*".into());
                let set = notoir_workspace::search::glob_set(&[glob]);
                let mut files: Vec<&str> = ws
                    .index
                    .iter()
                    .filter(|e| e.kind == notoir_core::explorer::NodeKind::File && !e.ignored)
                    .filter(|e| set.is_match(&e.rel))
                    .map(|e| e.rel.as_str())
                    .collect();
                files.sort_unstable();
                Ok(json!(files))
            }
            "settings.get" => {
                let key = self.plugin_setting_key(plugin, &s("key").unwrap_or_default());
                Ok(self
                    .settings
                    .get(&key, None)
                    .map_or(Value::Null, setting_json))
            }
            "settings.set" => {
                let raw = s("key").unwrap_or_default();
                let key = self.plugin_setting_key(plugin, &raw);
                // Its own settings are always writable.
                if key == raw {
                    self.plugin_check(plugin, &Capability::SettingsModify, "changing settings")?;
                }
                let Some(def) = self.schema.get(&key) else {
                    return Err(ApiError::new(
                        "unknown_setting",
                        format!("No setting {key}"),
                    ));
                };
                if def.restricted {
                    return Err(denied(&format!("changing {key}")));
                }
                let value = match args.get("value") {
                    None | Some(Value::Null) => None,
                    Some(v) => Some(
                        json_setting(&def.ty, v)
                            .filter(|sv| def.ty.accepts(sv))
                            .ok_or_else(|| {
                                ApiError::new("invalid_value", format!("Invalid value for {key}"))
                            })?,
                    ),
                };
                let key = def.key;
                self.set_user_setting(key, value);
                Ok(Value::Null)
            }
            "fs.check" => {
                let op = s("op").unwrap_or_default();
                let cap = if op == "write" {
                    Capability::FilesystemWrite
                } else {
                    Capability::FilesystemRead
                };
                self.plugin_check(plugin, &cap, "file access")?;
                let path = self.plugin_path(&s("path").unwrap_or_default())?;
                Ok(json!({ "path": path.to_string_lossy() }))
            }
            "http.check" => {
                self.plugin_check(plugin, &Capability::Network, "network access")?;
                let url = s("url").unwrap_or_default();
                if !(url.starts_with("http://") || url.starts_with("https://")) {
                    return Err(ApiError::new("invalid_url", "Only http and https URLs"));
                }
                let host = s("host").unwrap_or_default();
                let allowed = self.plugins.entry(plugin).is_some_and(|e| {
                    e.manifest
                        .as_ref()
                        .is_some_and(|m| m.permissions.host_allowed(&host))
                });
                if !allowed {
                    return Err(denied(&format!("access to {host}")));
                }
                Ok(Value::Null)
            }
            "clipboard.check" => {
                self.plugin_check(plugin, &Capability::Clipboard, "the clipboard")?;
                Ok(Value::Null)
            }
            "status.set" | "panels.render" | "formatters.register" | "outline.register" => {
                self.plugin_contrib_api(plugin, name, args)
            }
            other => Err(ApiError::new("unknown_api", format!("Unknown API {other}"))),
        }
    }

    /// A plugin path: inside the workspace only.
    fn plugin_path(&self, path: &str) -> Result<PathBuf, ApiError> {
        let ws = self
            .workspace
            .as_ref()
            .ok_or_else(|| ApiError::new("permission_denied", "No workspace is open"))?;
        notoir_mcp::paths::resolve(&ws.root, path)
            .map_err(|_| ApiError::new("permission_denied", "The path is outside the workspace"))
    }

    /// `notoir.process.run`: denied, allowed, or asks the user.
    fn plugin_process_check(&mut self, r: PluginRequest) {
        let decision = self
            .mcp
            .gate
            .check(&principal(&r.plugin), &Capability::Process, true);
        let cwd = match r.args.get("cwd").and_then(Value::as_str) {
            Some(c) => match self.plugin_path(c) {
                Ok(p) => p,
                Err(e) => {
                    r.respond(Err(e));
                    return;
                }
            },
            None => self
                .workspace
                .as_ref()
                .map_or_else(std::env::temp_dir, |w| w.root.clone()),
        };
        match decision {
            Decision::Deny => r.respond(Err(denied("running programs"))),
            Decision::Allow => {
                let cwd = cwd.to_string_lossy().into_owned();
                r.respond(Ok(json!({ "cwd": cwd })));
            }
            Decision::Ask => {
                let mut line = r.args["command"].as_str().unwrap_or_default().to_owned();
                for a in r.args["args"].as_array().into_iter().flatten() {
                    line.push(' ');
                    line.push_str(a.as_str().unwrap_or_default());
                }
                self.plugins.process_prompts.push_back(ProcessPrompt {
                    plugin: r.plugin.clone(),
                    command_line: line,
                    request: r,
                    cwd,
                });
            }
        }
    }
}

fn setting_json(v: &notoir_core::settings::SettingValue) -> Value {
    use notoir_core::settings::SettingValue as V;
    match v {
        V::Bool(b) => json!(b),
        V::Int(i) => json!(i),
        V::Float(f) => json!(f),
        V::Str(s) => json!(s),
        V::List(l) => json!(l),
    }
}

fn json_setting(
    ty: &notoir_core::settings::SettingType,
    v: &Value,
) -> Option<notoir_core::settings::SettingValue> {
    use notoir_core::settings::{SettingType as T, SettingValue as V};
    Some(match ty {
        T::Bool => V::Bool(v.as_bool()?),
        T::Int { .. } => V::Int(v.as_i64()?),
        T::Float { .. } => V::Float(v.as_f64()?),
        T::Enum(_) | T::Str => V::Str(v.as_str()?.to_owned()),
        T::StrList => V::List(
            v.as_array()?
                .iter()
                .map(|s| s.as_str().map(str::to_owned))
                .collect::<Option<Vec<_>>>()?,
        ),
    })
}

/// Copies a plugin folder into the plugins directory, replacing an older
/// version.
fn install_copy(source: &Path, target: &Path) -> std::io::Result<()> {
    if source.canonicalize().ok() == target.canonicalize().ok() {
        return Ok(());
    }
    if target.exists() {
        std::fs::remove_dir_all(target)?;
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    notoir_workspace::fs_ops::copy_recursive(source, target)
}
