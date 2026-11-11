//! Theme presets, editable drafts and serialized background persistence.
use std::collections::BTreeMap;
use std::path::PathBuf;

use notoir_core::settings::{ResolvedSettings, Schema, SettingValue, SettingsLayer};
use serde::Deserialize;
use serde_json::{Value, json};
use toml_edit::{DocumentMut, Item, Table};

use crate::core::{AppCore, Effect};
use crate::io::IoResult;
use crate::theme::{Style, Theme, ThemeError, ThemeId, Tok};

/// Only appearance preferences belong in a portable theme.
pub const SETTINGS: &[&str] = &[
    "workbench.fontFamily",
    "workbench.fontSize",
    "workbench.uiScale",
    "workbench.tabs.density",
    "editor.fontFamily",
    "editor.fontSize",
    "editor.lineHeight",
    "editor.wordWrap",
    "editor.wordWrapColumn",
    "editor.lineNumbers",
    "editor.renderWhitespace",
    "editor.highlightCurrentLine",
    "editor.cursorStyle",
    "editor.cursorBlinking",
    "editor.rulers",
    "editor.indentGuides",
    "editor.minimap.enabled",
    "editor.minimap.side",
    "editor.minimap.width",
    "editor.minimap.scale",
    "editor.minimap.opacity",
];

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Rule {
    #[serde(default)]
    pub settings: BTreeMap<String, toml::Value>,
    #[serde(default)]
    pub colors: BTreeMap<String, toml::Value>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Appearance {
    #[serde(default)]
    pub settings: BTreeMap<String, toml::Value>,
    #[serde(default)]
    pub languages: BTreeMap<String, Rule>,
    #[serde(default)]
    pub extensions: BTreeMap<String, Rule>,
}

fn invalid(message: impl Into<String>) -> ThemeError {
    ThemeError::Toml(message.into())
}

pub fn setting_value(v: &toml::Value) -> Option<SettingValue> {
    Some(match v {
        toml::Value::Boolean(v) => SettingValue::Bool(*v),
        toml::Value::Integer(v) => SettingValue::Int(*v),
        toml::Value::Float(v) => SettingValue::Float(*v),
        toml::Value::String(v) => SettingValue::Str(v.clone()),
        toml::Value::Array(values) => SettingValue::List(
            values
                .iter()
                .map(|v| v.as_str().map(str::to_owned))
                .collect::<Option<Vec<_>>>()?,
        ),
        _ => return None,
    })
}

pub fn normalize_extension(extension: &str) -> Result<String, ThemeError> {
    let key = extension.trim().trim_start_matches('.').to_lowercase();
    if key.is_empty()
        || key.split('.').any(str::is_empty)
        || key
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, '/' | '\\' | '*' | '?' | '[' | ']'))
    {
        return Err(invalid("invalid file extension"));
    }
    Ok(key)
}

impl Appearance {
    pub fn parse(source: &str) -> Result<Self, ThemeError> {
        #[derive(Deserialize)]
        struct File {
            #[serde(default)]
            appearance: Appearance,
        }
        let mut appearance = toml::from_str::<File>(source)
            .map_err(|e| invalid(e.to_string()))?
            .appearance;
        let mut normalized = BTreeMap::new();
        for (extension, rule) in std::mem::take(&mut appearance.extensions) {
            let extension = normalize_extension(&extension)?;
            if normalized.insert(extension, rule).is_some() {
                return Err(invalid("duplicate file extension"));
            }
        }
        appearance.extensions = normalized;
        let schema = Schema::builtin();
        validate_settings(&appearance.settings, &schema, false)?;
        for rule in appearance
            .languages
            .values()
            .chain(appearance.extensions.values())
        {
            validate_settings(&rule.settings, &schema, true)?;
            for (key, value) in &rule.colors {
                if !Tok::ALL.iter().any(|t| {
                    t.name() == key && (key.starts_with("editor.") || key.starts_with("syntax."))
                }) {
                    return Err(invalid(format!("invalid editor color token: {key}")));
                }
                // Validate style syntax without recursively parsing appearance rules.
                let mut source = ThemeId::Midnight.load();
                source.apply_colors(&BTreeMap::from([(key.clone(), value.clone())]))?;
            }
        }
        Ok(appearance)
    }

    pub fn extension_rule(&self, filename: Option<&str>) -> Option<&Rule> {
        let filename = filename?.to_lowercase();
        self.extensions
            .iter()
            .filter(|(ext, _)| filename.ends_with(&format!(".{ext}")))
            .max_by_key(|(ext, _)| ext.len())
            .map(|(_, rule)| rule)
    }

    pub fn settings_layer(&self, language: Option<&str>, filename: Option<&str>) -> SettingsLayer {
        let mut layer = SettingsLayer::default();
        let mut add = |values: &BTreeMap<String, toml::Value>| {
            for (key, value) in values {
                if let Some(value) = setting_value(value) {
                    layer.set(key, value);
                }
            }
        };
        add(&self.settings);
        if let Some(rule) = language.and_then(|l| self.languages.get(l)) {
            add(&rule.settings);
        }
        if let Some(rule) = self.extension_rule(filename) {
            add(&rule.settings);
        }
        layer
    }

    pub fn defaults_layer(&self) -> SettingsLayer {
        let mut layer = self.settings_layer(None, None);
        for language in self.languages.keys() {
            let values = self.settings_layer(Some(language), None);
            layer.languages.insert(language.clone(), values.values);
        }
        layer
    }
}

fn validate_settings(
    values: &BTreeMap<String, toml::Value>,
    schema: &Schema,
    editor_only: bool,
) -> Result<(), ThemeError> {
    for (key, value) in values {
        let Some(def) = schema.get(key) else {
            return Err(invalid(format!("unknown appearance setting: {key}")));
        };
        if !SETTINGS.contains(&key.as_str())
            || (editor_only && !key.starts_with("editor."))
            || !setting_value(value).is_some_and(|v| def.ty.accepts(&v))
        {
            return Err(invalid(format!("invalid appearance setting: {key}")));
        }
    }
    Ok(())
}

impl Theme {
    pub fn apply_colors(
        &mut self,
        colors: &BTreeMap<String, toml::Value>,
    ) -> Result<(), ThemeError> {
        if colors.is_empty() {
            return Ok(());
        }
        let mut doc = DocumentMut::new();
        doc["meta"]["id"] = toml_edit::value("rule-preview");
        doc["meta"]["name"] = toml_edit::value("rule-preview");
        doc["meta"]["kind"] = toml_edit::value(if self.is_dark() { "dark" } else { "light" });
        for (key, value) in colors {
            let tok = Tok::ALL
                .iter()
                .find(|t| t.name() == key)
                .ok_or_else(|| invalid(key))?;
            let (section, name) = token_path(*tok);
            let encoded = toml::to_string(&BTreeMap::from([("value", value)]))
                .map_err(|e| invalid(e.to_string()))?;
            let parsed = encoded
                .parse::<DocumentMut>()
                .map_err(|e| invalid(e.to_string()))?;
            doc[section][name] = parsed["value"].clone();
        }
        let resolved = Theme::parse_over(&doc.to_string(), Some(self))?;
        for key in colors.keys() {
            if let Some(tok) = Tok::ALL.iter().find(|t| t.name() == key) {
                self.set_style(*tok, resolved.style(*tok));
            }
        }
        Ok(())
    }

    pub fn for_document(&self, language: Option<&str>, filename: Option<&str>) -> Self {
        let mut result = self.clone();
        if let Some(rule) = language.and_then(|l| self.appearance.languages.get(l)) {
            let _ = result.apply_colors(&rule.colors);
        }
        if let Some(rule) = self.appearance.extension_rule(filename) {
            let _ = result.apply_colors(&rule.colors);
        }
        result
    }
}

pub fn token_path(token: Tok) -> (&'static str, &'static str) {
    let name = token.name();
    for section in ["editor", "syntax", "git", "diff"] {
        if let Some(key) = name.strip_prefix(section).and_then(|s| s.strip_prefix('.')) {
            return (section, key);
        }
    }
    ("ui", name)
}

pub fn hex(style: Style) -> String {
    let [r, g, b, a] = style.color.to_srgba_unmultiplied();
    if a == 255 {
        format!("#{r:02x}{g:02x}{b:02x}")
    } else {
        format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
    }
}

/// Fully materialize colors and defaults so a copy/export is independent of its source.
pub fn standalone(theme: &Theme, name: &str) -> Result<String, ThemeError> {
    let mut doc = theme
        .source
        .parse::<DocumentMut>()
        .map_err(|e| invalid(e.to_string()))?;
    doc["meta"]["id"] = toml_edit::value(format!("theme-{}", uuid::Uuid::new_v4()));
    doc["meta"]["name"] = toml_edit::value(name);
    for token in Tok::ALL {
        let (section, key) = token_path(*token);
        let style = theme.style(*token);
        let mut value = toml_edit::InlineTable::new();
        value.insert("color", hex(style).into());
        value.insert("bold", style.bold.into());
        value.insert("italic", style.italic.into());
        doc[section][key] = toml_edit::value(value);
    }
    Ok(doc.to_string())
}

#[derive(Clone, Debug)]
pub struct Draft {
    pub id: String,
    pub original: String,
    pub source: String,
    pub path: Option<PathBuf>,
    pub builtin: bool,
    pub new: bool,
}
impl Draft {
    pub fn dirty(&self) -> bool {
        self.new || self.source != self.original
    }
    pub fn theme(&self) -> Result<Theme, ThemeError> {
        if let Some(id) = ThemeId::from_id(&self.id) {
            Theme::parse_over(&self.source, Some(&id.load()))
        } else {
            Theme::parse_custom(&self.source)
        }
    }
}

#[derive(Default)]
pub struct Manager {
    pub draft: Option<Draft>,
    pub search: String,
    pub filter: String,
    pub color_inputs: BTreeMap<String, String>,
    pub page: usize,
    /// None = theme-wide, otherwise (languages | extensions, key).
    pub rule: Option<(String, String)>,
    pub rule_input: String,
    pub rule_language: bool,
    pub preview_extension: String,
    pub error: Option<String>,
    pub busy: bool,
    pub pending: Option<Value>,
    pub confirmation: Option<Value>,
    pub conflict: bool,
    pub fonts: Vec<String>,
    pub fonts_requested: bool,
    pub preview: Option<Box<Preview>>,
}

pub struct Preview {
    pub workbench: notoir_core::Workbench,
    pub caches: crate::editor_widget::EditorCaches,
    pub view: notoir_core::ids::ViewId,
    pub extension: String,
}

#[derive(Debug)]
pub enum Done {
    Saved {
        id: String,
        source: String,
        path: PathBuf,
    },
    Removed {
        id: String,
        reset: bool,
    },
    Imported(Theme),
    Exported,
    Fonts(Vec<String>),
    Cancelled,
}

impl AppCore {
    pub fn select_theme_draft(&mut self, id: &str) {
        let theme = self.load_theme(id);
        self.theme_manager.draft = Some(Draft {
            id: theme.id.clone(),
            original: theme.source.clone(),
            source: theme.source,
            path: theme.path,
            builtin: ThemeId::from_id(id).is_some(),
            new: false,
        });
        self.theme_manager.rule = None;
        self.theme_manager.color_inputs.clear();
        self.theme_manager.error = None;
        self.theme_manager.conflict = false;
    }

    pub fn request_theme_fonts(&mut self) {
        if self.theme_manager.fonts_requested {
            return;
        }
        self.theme_manager.fonts_requested = true;
        self.io.submit(|| {
            Some(IoResult::ThemeManager(Box::new(Ok(Done::Fonts(
                notoir_platform::system_fonts::families(),
            )))))
        });
    }

    pub fn appearance_settings(&self, view: notoir_core::ids::ViewId) -> ResolvedSettings {
        let doc = self
            .workbench
            .view(view)
            .and_then(|v| self.workbench.document(v.document_id));
        let language = doc.map(|d| d.language.as_str());
        let filename = doc
            .and_then(|d| d.path())
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy());
        let theme = self.load_theme(&self.theme(self.appearance_dark));
        self.settings.clone().with_theme_defaults(
            theme
                .appearance
                .settings_layer(language, filename.as_deref()),
        )
    }

    pub fn theme_action(&mut self, args: &Value) {
        let action = args["action"].as_str().unwrap_or_default();
        if self.theme_manager.busy && action != "edit" {
            return;
        }
        let dirty = self.theme_manager.draft.as_ref().is_some_and(Draft::dirty);
        if matches!(
            action,
            "select"
                | "create"
                | "duplicate"
                | "import"
                | "export"
                | "reset"
                | "delete"
                | "close"
                | "quit"
        ) && dirty
            && !args["discard"].as_bool().unwrap_or(false)
        {
            self.theme_manager.pending = Some(args.clone());
            return;
        }
        self.theme_manager.error = None;
        if args["discard"].as_bool().unwrap_or(false) {
            if self
                .theme_manager
                .draft
                .as_ref()
                .is_some_and(|draft| draft.original.is_empty())
            {
                let id = self.theme(self.appearance_dark);
                self.select_theme_draft(&id);
            } else if let Some(draft) = &mut self.theme_manager.draft {
                draft.source = draft.original.clone();
                draft.new = false;
            }
        }
        match action {
            "cancelPending" => {
                if self
                    .theme_manager
                    .pending
                    .take()
                    .is_some_and(|pending| pending["action"] == "quit")
                {
                    self.close_cancelled = true;
                }
            }
            "select" => {
                if let Some(id) = args["id"].as_str() {
                    self.select_theme_draft(id);
                }
            }
            "create" | "duplicate" => {
                let base = if action == "create" {
                    ThemeId::Midnight.load()
                } else {
                    self.theme_manager
                        .draft
                        .as_ref()
                        .and_then(|d| d.theme().ok())
                        .unwrap_or_else(|| ThemeId::Midnight.load())
                };
                let name = crate::t!("themes:newName");
                match standalone(&base, &name).and_then(|s| Theme::parse_custom(&s)) {
                    Ok(theme) => {
                        self.theme_manager.draft = Some(Draft {
                            id: theme.id,
                            original: String::new(),
                            source: theme.source,
                            path: None,
                            builtin: false,
                            new: true,
                        })
                    }
                    Err(e) => self.theme_manager.error = Some(e.to_string()),
                }
                self.theme_manager.rule = None;
                self.theme_manager.color_inputs.clear();
            }
            "edit" => {
                if self.theme_manager.busy {
                    return;
                }
                if let Some(draft) = &mut self.theme_manager.draft {
                    let result = edit_source(&draft.source, args);
                    match result {
                        Ok(source) => {
                            let candidate = Draft {
                                source: source.clone(),
                                ..draft.clone()
                            };
                            match candidate.theme() {
                                Ok(theme) if theme.id == draft.id => draft.source = source,
                                Ok(_) => {
                                    self.theme_manager.error = Some(crate::t!("themes:invalidRule"))
                                }
                                Err(error) => self.theme_manager.error = Some(error.to_string()),
                            }
                        }
                        Err(error) => self.theme_manager.error = Some(error),
                    }
                }
            }
            "addRule" => {
                let section = if args["language"].as_bool().unwrap_or(false) {
                    "languages"
                } else {
                    "extensions"
                };
                let input = args["key"].as_str().unwrap_or_default().trim();
                let key = if section == "extensions" {
                    normalize_extension(input).map_err(|e| e.to_string())
                } else if input.is_empty() {
                    Err(crate::t!("themes:invalidRule"))
                } else {
                    Ok(input.to_owned())
                };
                match key {
                    Ok(key) => {
                        if let Some(draft) = &mut self.theme_manager.draft {
                            let existing = match draft.theme() {
                                Ok(theme) => theme,
                                Err(error) => {
                                    self.theme_manager.error = Some(error.to_string());
                                    return;
                                }
                            };
                            let rules = if section == "extensions" {
                                &existing.appearance.extensions
                            } else {
                                &existing.appearance.languages
                            };
                            if rules.contains_key(&key) {
                                self.theme_manager.error = Some(crate::t!("themes:duplicateRule"));
                                return;
                            }
                            let path =
                                json!({"path":["appearance",section,key,"settings"],"value":{}});
                            match edit_source(&draft.source, &path) {
                                Ok(source) => draft.source = source,
                                Err(error) => {
                                    self.theme_manager.error = Some(error);
                                    return;
                                }
                            }
                            self.theme_manager.rule = Some((section.into(), key));
                            self.theme_manager.rule_input.clear();
                        }
                    }
                    Err(e) => self.theme_manager.error = Some(e),
                }
            }
            "removeRule" => {
                if let Some((section, key)) = self.theme_manager.rule.take()
                    && let Some(draft) = &mut self.theme_manager.draft
                {
                    let args = json!({"path":["appearance",section,key],"value":null});
                    match edit_source(&draft.source, &args) {
                        Ok(source) => draft.source = source,
                        Err(error) => self.theme_manager.error = Some(error),
                    }
                }
            }
            "save" => self.save_theme_draft(args["overwrite"].as_bool().unwrap_or(false)),
            "use" => {
                if let Some(d) = self
                    .theme_manager
                    .draft
                    .as_ref()
                    .filter(|d| !d.new && !d.dirty())
                {
                    let id = d.id.clone();
                    self.theme_choice =
                        crate::core::ThemeChoice::from_setting(&id).expect("valid theme");
                    self.theme_override = None;
                    self.set_user_setting("workbench.theme", Some(SettingValue::Str(id)));
                    self.effects.push(Effect::ApplyTheme);
                }
            }
            "reset" | "delete" => {
                if !args["confirmed"].as_bool().unwrap_or(false) {
                    self.theme_manager.confirmation = Some(args.clone());
                    return;
                }
                let Some(draft) = &self.theme_manager.draft else {
                    return;
                };
                if (action == "reset") != draft.builtin {
                    return;
                }
                let id = draft.id.clone();
                let path = draft.path.clone();
                let dialogs = self.files.dialogs.clone();
                let reset = action == "reset";
                self.theme_manager.busy = true;
                self.io.submit(move || {
                    Some(IoResult::ThemeManager(Box::new((|| {
                        if let Some(path) = path {
                            dialogs.trash(&[path])?;
                        }
                        Ok(Done::Removed { id, reset })
                    })())))
                });
            }
            "import" => {
                self.theme_manager.busy = true;
                let poster = self.io.poster();
                self.files.dialogs.import_theme(
                    notoir_platform::dialogs::FileDialogOptions {
                        title: Some(crate::t!("themes:import")),
                        ..Default::default()
                    },
                    poster,
                );
            }
            "export" => {
                let Some(draft) = &self.theme_manager.draft else {
                    return;
                };
                match draft.theme().and_then(|t| standalone(&t, &t.name)) {
                    Ok(source) => {
                        self.theme_manager.busy = true;
                        self.files.dialogs.export_theme(
                            notoir_platform::dialogs::FileDialogOptions {
                                title: Some(crate::t!("themes:export")),
                                file_name: Some("theme.toml".into()),
                                ..Default::default()
                            },
                            source,
                            self.io.poster(),
                        );
                    }
                    Err(e) => self.theme_manager.error = Some(e.to_string()),
                }
            }
            "quit" => {
                if let Some(draft) = &mut self.theme_manager.draft {
                    draft.source = draft.original.clone();
                    draft.new = false;
                }
                if self.request_quit() {
                    self.close_ready = true;
                    self.effects.push(if self.window_id.is_some() {
                        Effect::CloseWindow
                    } else {
                        Effect::Quit
                    });
                }
            }
            "close" => {
                if let Some(view) = args["view"].as_u64().and_then(|id| {
                    self.workbench
                        .views
                        .iter()
                        .find(|v| v.id.0 == id)
                        .map(|v| v.id)
                }) {
                    self.close_view_now(view);
                }
            }
            _ => {}
        }
    }

    fn save_theme_draft(&mut self, overwrite: bool) {
        if self
            .theme_manager
            .color_inputs
            .values()
            .any(|value| crate::theme::parse_hex("color", value).is_err())
        {
            self.theme_manager.error = Some(crate::t!("themes:invalidColor"));
            return;
        }
        let Some(draft) = &self.theme_manager.draft else {
            return;
        };
        let theme = match draft.theme() {
            Ok(t) => t,
            Err(e) => {
                self.theme_manager.error = Some(e.to_string());
                return;
            }
        };
        if theme.name.trim().is_empty() {
            self.theme_manager.error = Some(crate::t!("themes:nameRequired"));
            return;
        }
        let path = draft.path.clone().unwrap_or_else(|| {
            if draft.builtin {
                self.paths
                    .themes
                    .join("defaults")
                    .join(format!("{}.toml", draft.id))
            } else {
                self.paths
                    .themes
                    .join(format!("theme-{}.toml", uuid::Uuid::new_v4()))
            }
        });
        let expected = draft.path.as_ref().map(|_| draft.original.clone());
        let source = draft.source.clone();
        let id = draft.id.clone();
        self.theme_manager.busy = true;
        self.io.submit(move || {
            Some(IoResult::ThemeManager(Box::new((|| {
                let current = match std::fs::read_to_string(&path) {
                    Ok(s) => Some(s),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                    Err(e) => return Err(e.to_string()),
                };
                if !overwrite && current != expected {
                    return Err("THEME_CONFLICT".into());
                }
                std::fs::create_dir_all(path.parent().expect("theme directory"))
                    .map_err(|e| e.to_string())?;
                notoir_storage::atomic::write(&path, source.as_bytes(), Default::default())
                    .map_err(|e| e.to_string())?;
                Ok(Done::Saved { id, source, path })
            })())))
        });
    }

    pub fn theme_manager_done(&mut self, result: Result<Done, String>) {
        if !matches!(result, Ok(Done::Fonts(_))) {
            self.theme_manager.busy = false;
        }
        match result {
            Ok(Done::Saved { id, source, path }) => {
                if let Some(draft) = &mut self.theme_manager.draft
                    && draft.id == id
                {
                    draft.original = source.clone();
                    draft.path = Some(path.clone());
                    draft.new = false;
                }
                if let Ok(mut theme) = if let Some(builtin) = ThemeId::from_id(&id) {
                    Theme::parse_over(&source, Some(&builtin.load()))
                } else {
                    Theme::parse_custom(&source)
                } {
                    theme.path = Some(path);
                    self.custom_themes.retain(|t| t.id != id);
                    self.custom_themes.push(theme);
                }
                self.themes_generation += 1;
                if self.window_id.is_some() {
                    self.effects.push(Effect::Application(
                        crate::application::ApplicationRequest::RefreshThemes,
                    ));
                }
                self.apply_resolved_settings();
                self.effects.push(Effect::ApplyTheme);
                self.resume_theme_action();
            }
            Ok(Done::Removed { id, reset }) => {
                let kind = self.load_theme(&id).kind;
                self.custom_themes.retain(|t| t.id != id);
                if reset {
                    self.select_theme_draft(&id);
                } else {
                    if self.theme_choice.setting_value() == id {
                        let fallback = ThemeId::base_for(kind).id();
                        self.theme_choice =
                            crate::core::ThemeChoice::from_setting(fallback).expect("builtin");
                        self.set_user_setting(
                            "workbench.theme",
                            Some(SettingValue::Str(fallback.into())),
                        );
                    }
                    self.select_theme_draft(ThemeId::base_for(kind).id());
                }
                self.themes_generation += 1;
                if self.window_id.is_some() {
                    self.effects.push(Effect::Application(
                        crate::application::ApplicationRequest::RefreshThemes,
                    ));
                }
                self.apply_resolved_settings();
                self.effects.push(Effect::ApplyTheme);
            }
            Ok(Done::Imported(theme)) => {
                self.theme_manager.rule = None;
                self.theme_manager.color_inputs.clear();
                self.theme_manager.draft = Some(Draft {
                    id: theme.id,
                    original: String::new(),
                    source: theme.source,
                    path: None,
                    builtin: false,
                    new: true,
                });
                self.save_theme_draft(false);
            }
            Ok(Done::Fonts(fonts)) => {
                self.theme_manager.fonts = fonts;
            }
            Ok(Done::Exported | Done::Cancelled) => {}
            Err(error) => {
                self.theme_manager.conflict = error == "THEME_CONFLICT";
                self.theme_manager.error = Some(if self.theme_manager.conflict {
                    crate::t!("themes:conflict")
                } else {
                    error
                });
            }
        }
    }

    pub fn resume_theme_action(&mut self) {
        if let Some(mut action) = self.theme_manager.pending.take() {
            action["discard"] = json!(true);
            self.theme_action(&action);
        }
    }
}

/// Edit one leaf while retaining comments, unknown fields and inheritance.
pub fn edit_source(source: &str, args: &Value) -> Result<String, String> {
    let mut doc = source.parse::<DocumentMut>().map_err(|e| e.to_string())?;
    // Canonicalize extension spellings when editing imported files.
    if let Some(extensions) = doc
        .get_mut("appearance")
        .and_then(|a| a.get_mut("extensions"))
        .and_then(Item::as_table_like_mut)
    {
        let names: Vec<_> = extensions.iter().map(|(key, _)| key.to_owned()).collect();
        for key in names {
            let normalized = normalize_extension(&key).map_err(|e| e.to_string())?;
            if normalized != key
                && let Some(value) = extensions.remove(&key)
            {
                extensions.insert(&normalized, value);
            }
        }
    }
    let path: Vec<&str> = args["path"]
        .as_array()
        .ok_or("missing path")?
        .iter()
        .filter_map(Value::as_str)
        .collect();
    if path.is_empty() {
        return Err("empty path".into());
    }
    fn table<'a>(
        doc: &'a mut dyn toml_edit::TableLike,
        path: &[&str],
        inline: bool,
    ) -> Result<&'a mut dyn toml_edit::TableLike, String> {
        if let Some((key, rest)) = path.split_first() {
            if !doc.contains_key(key) {
                doc.insert(
                    key,
                    if inline {
                        toml_edit::value(toml_edit::InlineTable::new())
                    } else {
                        Item::Table(Table::new())
                    },
                );
            }
            let child = doc.get_mut(key).ok_or("expected table")?;
            let next_inline = child.is_inline_table();
            let next = child.as_table_like_mut().ok_or("expected table")?;
            table(next, rest, next_inline)
        } else {
            Ok(doc)
        }
    }
    let parent = table(doc.as_table_mut(), &path[..path.len() - 1], false)?;
    let key = path[path.len() - 1];
    if args["value"].is_null() {
        parent.remove(key);
    } else {
        let encoded = toml::to_string(&BTreeMap::from([("value", &args["value"])]))
            .map_err(|e| e.to_string())?;
        let value = encoded.parse::<DocumentMut>().map_err(|e| e.to_string())?;
        let mut replacement = value["value"].clone();
        if let Some(old) = parent.get(key).and_then(Item::as_value)
            && let Some(new) = replacement.as_value_mut()
        {
            *new.decor_mut() = old.decor().clone();
        }
        parent.insert(key, replacement);
    }
    Ok(doc.to_string())
}
