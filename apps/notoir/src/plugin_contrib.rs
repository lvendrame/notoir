//! Plugin contributions beyond commands (docs/specs/30 §Manifest, §API, §Plugin
//! Manager): status items, declarative panels, formatters, outline
//! providers, languages, plugin settings, and the Plugin Manager actions
//! (uninstall, install from .zip, workspace recommendations, reload one).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use notoir_core::command::{CommandId, CommandInvocation, InvocationSource};
use notoir_core::events::ChangeOrigin;
use notoir_core::ids::{DocumentId, ViewId};
use notoir_core::notifications::{Notification, NotificationSource, Severity, Translatable};
use notoir_core::settings::{Scopes, SettingCategory, SettingDef, SettingType, SettingValue};
use notoir_editor::outline::{Symbol, SymbolKind};
use notoir_editor::{ChangeSet, Edit, EditKind, Selection};
use notoir_plugins::{ApiError, ApiResult, Manifest, ToPlugin};
use serde_json::{Value, json};

use crate::core::AppCore;
use crate::io::IoResult;
use crate::plugins::{PluginStatus, command_id};

#[derive(Clone, Debug, PartialEq)]
pub struct PluginStatusItem {
    pub plugin: String,
    pub id: String,
    /// `left` or `right`.
    pub alignment: String,
    pub priority: i64,
    /// Hidden until the plugin sets text.
    pub text: Option<String>,
    pub tooltip: Option<String>,
    /// Plugin command run on click (`count` → `plugin.<id>.count`).
    pub command: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginPanel {
    pub plugin: String,
    pub id: String,
    pub title: String,
}

/// A formatter / outline call waiting for its plugin.
#[derive(Clone, Debug)]
pub struct ProvideCall {
    pub plugin: String,
    pub kind: String,
    pub doc: DocumentId,
    pub revision: u64,
    pub view: Option<ViewId>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ManagerTab {
    #[default]
    Installed,
    Recommended,
}

#[derive(Clone, Debug, Default)]
pub struct ManagerUi {
    pub search: String,
    pub tab: ManagerTab,
    /// The plugin whose permissions are expanded.
    pub expanded: Option<String>,
}

pub const MANAGER_TITLE: &str = "tabs:pluginManager";

/// Interns a string for `&'static` registries (each value leaks once).
fn intern(s: &str) -> &'static str {
    static SEEN: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<&'static str>>> =
        std::sync::OnceLock::new();
    let mut seen = SEEN
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if let Some(found) = seen.get(s) {
        return found;
    }
    let leaked: &'static str = Box::leak(s.to_owned().into_boxed_str());
    seen.insert(leaked);
    leaked
}

/// A manifest setting as a schema definition (`plugins.<id>.<key>`).
fn setting_def(plugin: &str, s: &notoir_plugins::manifest::SettingContrib) -> Option<SettingDef> {
    let key = intern(&format!("plugins.{plugin}.{}", s.key));
    let (ty, default) = match s.ty.as_str() {
        "bool" => (SettingType::Bool, SettingValue::Bool(s.default.as_bool()?)),
        "int" => (
            SettingType::Int {
                min: i64::MIN,
                max: i64::MAX,
            },
            SettingValue::Int(s.default.as_integer()?),
        ),
        "float" => (
            SettingType::Float {
                min: f64::MIN,
                max: f64::MAX,
            },
            SettingValue::Float(s.default.as_float()?),
        ),
        "string" => (
            SettingType::Str,
            SettingValue::Str(s.default.as_str()?.to_owned()),
        ),
        "enum" => {
            let values: Vec<&'static str> = s.values.iter().map(|v| intern(v)).collect();
            let default = s.default.as_str()?.to_owned();
            if !values.contains(&default.as_str()) {
                return None;
            }
            (
                SettingType::Enum(Box::leak(values.into_boxed_slice())),
                SettingValue::Str(default),
            )
        }
        "list" => (
            SettingType::StrList,
            SettingValue::List(
                s.default
                    .as_array()?
                    .iter()
                    .map(|v| v.as_str().map(str::to_owned))
                    .collect::<Option<Vec<_>>>()?,
            ),
        ),
        _ => return None,
    };
    Some(SettingDef {
        key,
        ty,
        default,
        scopes: Scopes::NO_LANGUAGE,
        category: SettingCategory::Plugins,
        restricted: false,
        tags: &[],
    })
}

/// Plugin outline entries (`{ name, line, end_line?, children? }`) as symbols.
fn symbols_from(rope: &notoir_editor::Rope, v: &Value) -> Vec<Symbol> {
    let lines = rope.len_lines().max(1);
    let line_start = |l: u64| rope.line_to_char((l.max(1) as usize - 1).min(lines - 1));
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| {
            let name = e.get("name")?.as_str()?.to_owned();
            let line = e.get("line").and_then(Value::as_u64).unwrap_or(1);
            let end_line = e.get("end_line").and_then(Value::as_u64).unwrap_or(line);
            let start = line_start(line);
            let head_end = line_start(line + 1).max(start);
            let end = if (end_line as usize) < lines {
                line_start(end_line + 1)
            } else {
                rope.len_chars()
            };
            Some(Symbol {
                name,
                kind: SymbolKind::Key,
                range: (start, end.max(head_end)),
                selection_range: (start, head_end),
                children: e
                    .get("children")
                    .map_or_else(Vec::new, |c| symbols_from(rope, c)),
            })
        })
        .collect()
}

impl AppCore {
    /// Status items, panels, settings and languages of an enabled plugin.
    pub(crate) fn plugin_contribute_more(
        &mut self,
        id: &str,
        m: &Manifest,
        dir: &Path,
        table: &HashMap<String, String>,
    ) {
        let title = |t: &str| match t.strip_prefix('%').and_then(|t| t.strip_suffix('%')) {
            Some(k) => table.get(k).cloned().unwrap_or_else(|| k.to_owned()),
            None => t.to_owned(),
        };
        for s in &m.contributes.status_items {
            self.plugins.status_items.push(PluginStatusItem {
                plugin: id.to_owned(),
                id: s.id.clone(),
                alignment: s.alignment.clone(),
                priority: s.priority,
                text: None,
                tooltip: None,
                command: None,
            });
        }
        for p in &m.contributes.panels {
            self.plugins.panels.push(PluginPanel {
                plugin: id.to_owned(),
                id: p.id.clone(),
                title: title(&p.title),
            });
        }
        // Settings under plugins.<id>.*
        if !m.contributes.settings.is_empty() {
            let mut schema = (*self.schema).clone();
            let mut keys = Vec::new();
            for s in &m.contributes.settings {
                match setting_def(id, s) {
                    Some(def) => {
                        let label = if s.title.is_empty() {
                            s.key.clone()
                        } else {
                            title(&s.title)
                        };
                        crate::i18n::set_dynamic(&format!("settings:{}", def.key), &label);
                        keys.push(def.key.to_owned());
                        schema.define(def);
                    }
                    None => self.plugins.log(
                        id,
                        "error",
                        format!("Setting \"{}\" has an invalid type or default", s.key),
                    ),
                }
            }
            self.schema = Arc::new(schema);
            self.apply_resolved_settings();
            if let Some(e) = self.plugins.entry_mut(id) {
                e.settings = keys;
            }
        }
        // Languages (grammars compile off the UI thread).
        let mut langs = Vec::new();
        for l in &m.contributes.languages {
            let name = l.name.clone().unwrap_or_else(|| l.id.clone());
            let lang = notoir_editor::language::register(&l.id, &name, &l.extensions);
            langs.push(l.id.clone());
            if let Some(syntax) = &l.syntax {
                let path = dir.join(syntax);
                let plugin = id.to_owned();
                let inside = path
                    .canonicalize()
                    .ok()
                    .zip(dir.canonicalize().ok())
                    .is_some_and(|(p, d)| p.starts_with(d));
                if !inside {
                    self.plugins.log(
                        id,
                        "error",
                        format!("Syntax {syntax} is outside the plugin"),
                    );
                    continue;
                }
                self.io.submit(move || {
                    let result = std::fs::read_to_string(&path)
                        .map_err(|e| e.to_string())
                        .and_then(|yaml| notoir_editor::syntax::register_syntax(lang, &yaml));
                    Some(IoResult::PluginSyntax {
                        plugin,
                        language: lang.as_str().to_owned(),
                        error: result.err(),
                    })
                });
            }
        }
        if let Some(e) = self.plugins.entry_mut(id) {
            e.languages = langs;
        }
    }

    pub fn plugins_syntax_loaded(&mut self, plugin: &str, language: &str, error: Option<String>) {
        match error {
            Some(e) => self
                .plugins
                .log(plugin, "error", format!("Syntax for {language}: {e}")),
            None => self
                .plugins
                .log(plugin, "info", format!("Syntax for {language} loaded")),
        }
    }

    pub(crate) fn plugin_unload_more(&mut self, id: &str) {
        self.plugins.status_items.retain(|s| s.plugin != id);
        self.plugins.panels.retain(|p| p.plugin != id);
        self.plugins.panel_trees.retain(|(p, _), _| p != id);
        self.plugins.formatters.retain(|_, p| p != id);
        let had_outline = self.plugins.outlines.values().any(|p| p == id);
        self.plugins.outlines.retain(|_, p| p != id);
        if had_outline {
            self.plugins.outline_results.clear();
            self.structure.docs.clear();
        }
        self.plugins.calls.retain(|_, c| c.plugin != id);
        let (settings, langs) = match self.plugins.entry_mut(id) {
            Some(e) => (
                std::mem::take(&mut e.settings),
                std::mem::take(&mut e.languages),
            ),
            None => (Vec::new(), Vec::new()),
        };
        if !settings.is_empty() {
            let mut schema = (*self.schema).clone();
            for k in &settings {
                schema.remove(k);
            }
            self.schema = Arc::new(schema);
            self.apply_resolved_settings();
        }
        for l in langs {
            if let Some(lang) = notoir_editor::LanguageId::from_id(&l) {
                notoir_editor::syntax::unregister_syntax(lang);
            }
            notoir_editor::language::unregister(&l);
        }
    }

    /// A plugin's own setting key (`includeCode` → `plugins.<id>.includeCode`);
    /// other keys are returned as given.
    pub(crate) fn plugin_setting_key(&self, plugin: &str, key: &str) -> String {
        let own = self.plugins.entry(plugin).is_some_and(|e| {
            e.manifest
                .as_ref()
                .is_some_and(|m| m.contributes.settings.iter().any(|s| s.key == key))
        });
        if own {
            format!("plugins.{plugin}.{key}")
        } else {
            key.to_owned()
        }
    }

    pub(crate) fn plugin_contrib_api(
        &mut self,
        plugin: &str,
        name: &str,
        args: &Value,
    ) -> ApiResult {
        let s = |k: &str| args.get(k).and_then(Value::as_str).map(str::to_owned);
        match name {
            "status.set" => {
                let id = s("id").unwrap_or_default();
                let item = args.get("item").cloned().unwrap_or(Value::Null);
                let Some(slot) = self
                    .plugins
                    .status_items
                    .iter_mut()
                    .find(|i| i.plugin == plugin && i.id == id)
                else {
                    return Err(ApiError::new(
                        "not_declared",
                        format!("status item \"{id}\" is not declared in plugin.toml"),
                    ));
                };
                if item.is_null() {
                    slot.text = None;
                } else {
                    slot.text = item.get("text").and_then(Value::as_str).map(str::to_owned);
                    slot.tooltip = item
                        .get("tooltip")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                    slot.command = item
                        .get("command")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                }
                Ok(Value::Null)
            }
            "panels.render" => {
                let id = s("id").unwrap_or_default();
                if !self
                    .plugins
                    .panels
                    .iter()
                    .any(|p| p.plugin == plugin && p.id == id)
                {
                    return Err(ApiError::new(
                        "not_declared",
                        format!("panel \"{id}\" is not declared in plugin.toml"),
                    ));
                }
                let tree = args.get("tree").cloned().unwrap_or(Value::Null);
                self.plugins
                    .panel_trees
                    .insert((plugin.to_owned(), id), tree);
                Ok(Value::Null)
            }
            "formatters.register" | "outline.register" => {
                let language = s("language").unwrap_or_default();
                if notoir_editor::LanguageId::from_id(&language).is_none() {
                    return Err(ApiError::new(
                        "unknown_language",
                        format!("No language {language}"),
                    ));
                }
                if name == "formatters.register" {
                    self.plugins.formatters.insert(language, plugin.to_owned());
                } else {
                    self.plugins.outlines.insert(language, plugin.to_owned());
                    self.structure.docs.clear();
                }
                Ok(Value::Null)
            }
            other => Err(ApiError::new("unknown_api", format!("Unknown API {other}"))),
        }
    }

    /// Runs a plugin command given relative to the plugin (`count`) or as a
    /// full id.
    pub fn plugin_action(&mut self, plugin: &str, command: &str, args: Value) {
        let full = if command.starts_with("plugin.") {
            command.to_owned()
        } else {
            command_id(plugin, command)
        };
        if self.commands.get(&CommandId::from(full.clone())).is_none() {
            self.plugins.log(
                plugin,
                "error",
                format!("Unknown command \"{command}\" in a status item or panel"),
            );
            return;
        }
        self.enqueue(CommandInvocation::new(full, InvocationSource::Internal).with_args(args));
    }

    // ── formatters & outline ─────────────────────────────────────────────

    fn provider_for(&self, kind: &str, doc: DocumentId) -> Option<(String, String)> {
        let d = self.workbench.document(doc)?;
        let lang = d.language.as_str().to_owned();
        let map = if kind == "format" {
            &self.plugins.formatters
        } else {
            &self.plugins.outlines
        };
        let plugin = map.get(&lang)?.clone();
        self.plugins.runtime(&plugin)?;
        Some((plugin, lang))
    }

    /// Whether a plugin formats the active document's language.
    pub fn plugin_formatter_active(&self) -> bool {
        self.workbench
            .active_editor()
            .and_then(|v| self.workbench.view(v))
            .is_some_and(|v| self.provider_for("format", v.document_id).is_some())
    }

    fn send_provide(&mut self, kind: &str, doc: DocumentId, view: Option<ViewId>) -> bool {
        let Some((plugin, language)) = self.provider_for(kind, doc) else {
            return false;
        };
        let Some(d) = self.workbench.document(doc) else {
            return false;
        };
        let call = self.plugins.next_call;
        self.plugins.next_call += 1;
        let style = self.indent_style(doc);
        let msg = ToPlugin::Provide {
            call,
            kind: kind.to_owned(),
            language,
            text: d.buffer.text(),
            options: json!({ "tab_size": style.tab_size, "insert_spaces": style.insert_spaces }),
        };
        self.plugins.calls.insert(
            call,
            ProvideCall {
                plugin: plugin.clone(),
                kind: kind.to_owned(),
                doc,
                revision: d.revision,
                view,
            },
        );
        self.plugins.runtime(&plugin).is_some_and(|rt| rt.send(msg))
    }

    /// Format Document through a plugin formatter (true when one ran).
    pub fn plugin_format(&mut self, view: ViewId) -> bool {
        let Some(doc) = self.workbench.view(view).map(|v| v.document_id) else {
            return false;
        };
        self.send_provide("format", doc, Some(view))
    }

    /// Plugin outline symbols for the document's current revision; a new
    /// request starts when they are stale (the last result is kept).
    pub fn plugin_outline(&mut self, doc: DocumentId) -> Option<Vec<Symbol>> {
        self.provider_for("outline", doc)?;
        let revision = self.workbench.document(doc)?.revision;
        let cached = self.plugins.outline_results.get(&doc).cloned();
        let fresh = cached.as_ref().is_some_and(|(r, _)| *r == revision);
        let inflight = self
            .plugins
            .calls
            .values()
            .any(|c| c.kind == "outline" && c.doc == doc);
        if !fresh && !inflight {
            self.send_provide("outline", doc, None);
        }
        cached.map(|(_, s)| s)
    }

    pub(crate) fn plugin_provided(
        &mut self,
        plugin: &str,
        call: u64,
        result: Result<Value, String>,
    ) {
        let Some(c) = self.plugins.calls.remove(&call) else {
            return;
        };
        let current = self.workbench.document(c.doc).map(|d| d.revision);
        match (c.kind.as_str(), result) {
            ("format", Ok(Value::String(text))) => {
                if current != Some(c.revision) {
                    return;
                }
                self.plugin_apply_format(plugin, &c, &text);
            }
            ("format", r) => {
                let name = self
                    .plugins
                    .entry(plugin)
                    .map(|e| e.name())
                    .unwrap_or_default();
                let mut n = Notification::new(
                    Severity::Warning,
                    Translatable::new("plugins:formatFailed").arg("name", name),
                )
                .source(NotificationSource::Plugin);
                if let Err(e) = r {
                    n = n.detail(e);
                }
                self.notify(n);
            }
            ("outline", Ok(v)) => {
                let Some(rope) = self
                    .workbench
                    .document(c.doc)
                    .map(|d| d.buffer.rope().clone())
                else {
                    return;
                };
                let symbols = symbols_from(&rope, &v);
                self.plugins
                    .outline_results
                    .insert(c.doc, (c.revision, symbols));
                self.structure.docs.remove(&c.doc);
            }
            (_, _) => {}
        }
    }

    fn plugin_apply_format(&mut self, plugin: &str, c: &ProvideCall, text: &str) {
        let Some(d) = self.workbench.document(c.doc) else {
            return;
        };
        let old: Vec<char> = d.buffer.rope().chars().collect();
        let new: Vec<char> = text.chars().collect();
        let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
        let suffix = old[prefix..]
            .iter()
            .rev()
            .zip(new[prefix..].iter().rev())
            .take_while(|(a, b)| a == b)
            .count();
        if prefix == old.len() && prefix == new.len() {
            return;
        }
        let insert: String = new[prefix..new.len() - suffix].iter().collect();
        let Ok(changes) = ChangeSet::new(vec![Edit::new(prefix, old.len() - suffix, insert)])
        else {
            return;
        };
        let caret = c
            .view
            .and_then(|v| self.workbench.view(v))
            .map_or(0, |v| v.selection.primary().head)
            .min(new.len());
        let _ = self.workbench.apply_edit(
            c.doc,
            c.view,
            changes,
            Selection::caret(caret),
            EditKind::Other,
            ChangeOrigin::Plugin(plugin.to_owned()),
            self.now_ms,
        );
    }

    // ── Plugin Manager ───────────────────────────────────────────────────

    pub fn plugins_open_manager(&mut self) {
        self.exit_zen();
        self.open_virtual(MANAGER_TITLE, notoir_core::view::ViewKind::PluginManager);
        self.plugins_load_recommendations();
    }

    /// `.notoir/plugins.toml`: `recommend = [{ id = "x", version = "1.0.0" }]`.
    pub fn plugins_load_recommendations(&mut self) {
        let Some(ws) = &self.workspace else {
            self.plugins.recommended.clear();
            return;
        };
        let path = ws.root.join(".notoir").join("plugins.toml");
        self.io.submit(move || {
            let list = std::fs::read_to_string(&path)
                .ok()
                .and_then(|t| t.parse::<toml::Table>().ok())
                .and_then(|t| t.get("recommend").and_then(|r| r.as_array()).cloned())
                .unwrap_or_default()
                .iter()
                .filter_map(|r| {
                    Some((
                        r.get("id")?.as_str()?.to_owned(),
                        r.get("version")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_owned(),
                    ))
                })
                .collect();
            Some(IoResult::PluginRecommendations(list))
        });
    }

    /// Uninstall (after confirmation): the folder goes to the trash.
    pub fn plugin_uninstall(&mut self, id: &str) {
        let Some(dir) = self.plugins.entry(id).map(|e| e.dir.clone()) else {
            return;
        };
        self.plugin_unload(id);
        let dialogs = self.files.dialogs.clone();
        let id = id.to_owned();
        self.io.submit(move || {
            let error = dialogs.trash(std::slice::from_ref(&dir)).err();
            Some(IoResult::PluginUninstalled { id, error })
        });
    }

    pub fn plugins_uninstalled(&mut self, id: String, error: Option<String>) {
        if let Some(e) = error {
            self.notify(
                Notification::new(
                    Severity::Error,
                    Translatable::new("plugins:uninstallFailed").arg("name", id),
                )
                .detail(e)
                .source(NotificationSource::Plugin),
            );
            return;
        }
        if let Some(db) = &self.state_db {
            let _ = db.remove_plugin(&id);
        }
        self.plugins.entries.retain(|e| e.id != id);
        self.plugins_rebuild_keymap();
        self.plugins.log(&id, "info", "Uninstalled");
    }

    /// Install from .zip…: pick an archive, extract it, then review.
    pub fn plugins_install_from_zip(&mut self) {
        self.plugins.zip_pick = true;
        let opts = notoir_platform::dialogs::FileDialogOptions {
            title: Some(crate::t!("plugins:installZipTitle")),
            directory: None,
            file_name: None,
        };
        self.files.dialogs.pick_files(opts, self.io.poster());
    }

    /// Files picked for Install from .zip (true when it was ours).
    pub fn plugins_files_picked(&mut self, files: &[PathBuf]) -> bool {
        if !std::mem::take(&mut self.plugins.zip_pick) {
            return false;
        }
        if let Some(zip) = files.first() {
            self.plugins_install_zip(zip.clone());
        }
        true
    }

    /// Extracts an archive into the cache and reviews the plugin inside.
    pub fn plugins_install_zip(&mut self, zip: PathBuf) {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let target = self
            .paths
            .cache
            .join("plugin-install")
            .join(stamp.to_string());
        self.io.submit(move || {
            let result = extract_plugin(&zip, &target);
            let (dir, result) = match result {
                Ok(dir) => {
                    let m = Manifest::load(&dir).map_err(|e| e.to_string());
                    (dir, m)
                }
                Err(e) => (target, Err(e)),
            };
            Some(IoResult::PluginManifest {
                dir,
                result: Box::new(result),
            })
        });
    }

    /// Developer Tools → Reload this plugin.
    pub fn plugin_reload_one(&mut self, id: &str) {
        let Some(dir) = self.plugins.entry(id).map(|e| e.dir.clone()) else {
            return;
        };
        self.plugin_unload(id);
        self.io.submit(move || {
            Some(IoResult::PluginsScanned(vec![notoir_plugins::Discovered {
                manifest: Manifest::load(&dir),
                dir,
            }]))
        });
    }

    /// Statuses for the manager list, newest problems first.
    pub fn plugin_status_label(status: &PluginStatus) -> &'static str {
        match status {
            PluginStatus::Invalid(_) => "plugins:status.invalid",
            PluginStatus::Incompatible(_) => "plugins:status.incompatible",
            PluginStatus::NeedsReview { .. } => "plugins:status.needsReview",
            PluginStatus::Disabled => "plugins:status.disabled",
            PluginStatus::Enabled => "plugins:status.enabled",
            PluginStatus::Active => "plugins:status.active",
        }
    }
}

/// Extracts `zip` into `target` and returns the folder holding plugin.toml
/// (the archive root or its single top folder).
fn extract_plugin(zip: &Path, target: &Path) -> Result<PathBuf, String> {
    let file = std::fs::File::open(zip).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(target).map_err(|e| e.to_string())?;
    // `extract` refuses entries that would land outside `target`.
    archive.extract(target).map_err(|e| e.to_string())?;
    if target.join("plugin.toml").is_file() {
        return Ok(target.to_path_buf());
    }
    let dirs: Vec<PathBuf> = std::fs::read_dir(target)
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    match dirs.as_slice() {
        [one] if one.join("plugin.toml").is_file() => Ok(one.clone()),
        _ => Err("The archive has no plugin.toml".into()),
    }
}
