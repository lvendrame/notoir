//! `settings.toml` (user and workspace) and `keybindings.toml`
//! (docs/specs/20-settings.md §Storage).
//!
//! Settings are dotted keys grouped in tables (`[editor] tabSize = 2` →
//! `editor.tabSize`); language-specific tables use a bracketed name
//! (`["[markdown]".editor]`). Writes go through `toml_edit` so comments and
//! ordering survive; unknown keys are never removed.

use std::path::Path;

use notoir_core::keymap::UserBinding;
use notoir_core::settings::{SettingValue, SettingsLayer};
use serde::Deserialize;
use toml_edit::{DocumentMut, Item, Table, TableLike, Value};

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("line {line}, column {column}: {message}")]
pub struct TomlFileError {
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl TomlFileError {
    fn at(text: &str, offset: usize, message: impl Into<String>) -> Self {
        let before = &text[..offset.min(text.len())];
        let line = before.matches('\n').count() + 1;
        let column = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
        Self {
            line,
            column,
            message: message.into(),
        }
    }

    fn from_toml_edit(text: &str, e: &toml_edit::TomlError) -> Self {
        let offset = e.span().map_or(0, |s| s.start);
        Self::at(text, offset, e.message().trim().to_owned())
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParsedSettings {
    pub layer: SettingsLayer,
    /// Keys whose TOML value has no setting equivalent (dates, mixed arrays…).
    pub unsupported: Vec<String>,
}

fn language_of(key: &str) -> Option<&str> {
    key.strip_prefix('[')
        .and_then(|k| k.strip_suffix(']'))
        .filter(|k| !k.is_empty())
}

fn convert(v: &Value) -> Option<SettingValue> {
    match v {
        Value::Boolean(b) => Some(SettingValue::Bool(*b.value())),
        Value::Integer(i) => Some(SettingValue::Int(*i.value())),
        Value::Float(f) => Some(SettingValue::Float(*f.value())),
        Value::String(s) => Some(SettingValue::Str(s.value().clone())),
        Value::Array(a) => a
            .iter()
            .map(|x| x.as_str().map(str::to_owned))
            .collect::<Option<Vec<_>>>()
            .map(SettingValue::List),
        _ => None,
    }
}

fn flatten(
    prefix: &str,
    table: &dyn TableLike,
    out: &mut Vec<(String, SettingValue)>,
    unsupported: &mut Vec<String>,
) {
    for (k, item) in table.iter() {
        let key = if prefix.is_empty() {
            k.to_owned()
        } else {
            format!("{prefix}.{k}")
        };
        if let Some(t) = item.as_table_like() {
            flatten(&key, t, out, unsupported);
        } else if let Some(v) = item.as_value() {
            match convert(v) {
                Some(sv) => out.push((key, sv)),
                None => unsupported.push(key),
            }
        }
    }
}

pub fn parse_settings(text: &str) -> Result<ParsedSettings, TomlFileError> {
    let doc: DocumentMut = text
        .parse()
        .map_err(|e| TomlFileError::from_toml_edit(text, &e))?;
    let mut parsed = ParsedSettings::default();
    for (k, item) in doc.iter() {
        let mut values = Vec::new();
        match (language_of(k), item.as_table_like()) {
            (Some(lang), Some(t)) => {
                flatten("", t, &mut values, &mut parsed.unsupported);
                for (key, v) in values {
                    parsed.layer.set_for_language(lang, key, v);
                }
            }
            (None, Some(t)) => {
                flatten(k, t, &mut values, &mut parsed.unsupported);
                for (key, v) in values {
                    parsed.layer.set(key, v);
                }
            }
            (_, None) => match item.as_value().and_then(convert) {
                Some(v) => {
                    parsed.layer.set(k, v);
                }
                None => parsed.unsupported.push(k.to_owned()),
            },
        }
    }
    Ok(parsed)
}

fn to_toml(v: &SettingValue) -> Value {
    match v {
        SettingValue::Bool(b) => (*b).into(),
        SettingValue::Int(i) => (*i).into(),
        SettingValue::Float(f) => (*f).into(),
        SettingValue::Str(s) => s.as_str().into(),
        SettingValue::List(items) => items
            .iter()
            .map(String::as_str)
            .collect::<toml_edit::Array>()
            .into(),
    }
}

/// Sets (`Some`) or resets (`None`) one setting in a settings file's text,
/// preserving everything else.
pub fn set_setting(
    text: &str,
    key: &str,
    language: Option<&str>,
    value: Option<&SettingValue>,
) -> Result<String, TomlFileError> {
    let mut doc: DocumentMut = text
        .parse()
        .map_err(|e| TomlFileError::from_toml_edit(text, &e))?;
    let mut segments: Vec<String> = key.split('.').map(str::to_owned).collect();
    if let Some(lang) = language {
        segments.insert(0, format!("[{lang}]"));
    }
    let (last, parents) = segments.split_last().expect("key has at least one segment");
    let mut table: &mut dyn TableLike = doc.as_table_mut();
    for seg in parents {
        if !table.contains_key(seg) {
            if value.is_none() {
                return Ok(doc.to_string());
            }
            let mut t = Table::new();
            t.set_implicit(true);
            table.insert(seg, Item::Table(t));
        }
        let item = table.get_mut(seg).expect("inserted above");
        if item.as_table_like().is_none() {
            // A scalar sits where a table is needed: replace it.
            *item = Item::Table(Table::new());
        }
        table = item.as_table_like_mut().expect("table-like");
    }
    match value {
        Some(v) => {
            let mut new = to_toml(v);
            // Update in place so the key's decor (comments above it) survives.
            match table.get_mut(last) {
                Some(Item::Value(old)) => {
                    *new.decor_mut() = old.decor().clone();
                    *old = new;
                }
                Some(item) => *item = Item::Value(new),
                None => {
                    table.insert(last, Item::Value(new));
                }
            }
        }
        None => {
            table.remove(last);
        }
    }
    prune_empty(doc.as_table_mut());
    Ok(doc.to_string())
}

fn prune_empty(table: &mut dyn TableLike) {
    let keys: Vec<String> = table.iter().map(|(k, _)| k.to_owned()).collect();
    for k in keys {
        if let Some(t) = table.get_mut(&k).and_then(Item::as_table_like_mut) {
            prune_empty(t);
            if t.is_empty() {
                table.remove(&k);
            }
        }
    }
}

/// Result of (re)loading a settings file.
#[derive(Clone, Debug, PartialEq)]
pub struct LoadedLayer {
    pub layer: SettingsLayer,
    pub unsupported: Vec<String>,
    /// Parse error: `layer` holds the previous good values (spec 20).
    pub error: Option<TomlFileError>,
}

#[derive(Debug, thiserror::Error)]
pub enum SettingsFileError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Toml(#[from] TomlFileError),
}

/// Loads a settings file. A missing file is an empty layer; a parse error
/// keeps `previous` (or empty) and reports the error.
pub fn load_settings_file(
    path: &Path,
    previous: Option<&SettingsLayer>,
) -> Result<LoadedLayer, std::io::Error> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };
    Ok(match parse_settings(&text) {
        Ok(p) => LoadedLayer {
            layer: p.layer,
            unsupported: p.unsupported,
            error: None,
        },
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "settings file has errors; keeping previous values");
            LoadedLayer {
                layer: previous.cloned().unwrap_or_default(),
                unsupported: Vec::new(),
                error: Some(e),
            }
        }
    })
}

/// Writes one setting into the file at `path`, creating the file and its
/// directory (e.g. `.notoir/`) only now — never on read.
pub fn write_setting_file(
    path: &Path,
    key: &str,
    language: Option<&str>,
    value: Option<&SettingValue>,
) -> Result<(), SettingsFileError> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if value.is_none() {
                return Ok(());
            }
            String::new()
        }
        Err(e) => return Err(e.into()),
    };
    let out = set_setting(&text, key, language, value)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    crate::atomic::write(path, out.as_bytes(), crate::atomic::WriteOptions::default())?;
    Ok(())
}

#[derive(Deserialize)]
struct KeybindingsFile {
    #[serde(default)]
    binding: Vec<RawBinding>,
}

#[derive(Deserialize)]
struct RawBinding {
    key: Option<String>,
    command: String,
    when: Option<String>,
    args: Option<toml::Value>,
}

pub fn parse_keybindings(text: &str) -> Result<Vec<UserBinding>, TomlFileError> {
    let file: KeybindingsFile = toml::from_str(text).map_err(|e| {
        let offset = e.span().map_or(0, |s| s.start);
        TomlFileError::at(text, offset, e.message().trim().to_owned())
    })?;
    Ok(file
        .binding
        .into_iter()
        .map(|b| UserBinding {
            key: b.key,
            command: b.command,
            when: b.when,
            args: b.args.and_then(|a| serde_json::to_value(a).ok()),
        })
        .collect())
}

/// Appends a `[[binding]]` entry, preserving the rest of the file.
pub fn append_keybinding(
    text: &str,
    key: Option<&str>,
    command: &str,
    when: Option<&str>,
) -> Result<String, TomlFileError> {
    let mut doc: DocumentMut = text
        .parse()
        .map_err(|e| TomlFileError::from_toml_edit(text, &e))?;
    let mut t = Table::new();
    if let Some(k) = key {
        t.insert("key", toml_edit::value(k));
    }
    t.insert("command", toml_edit::value(command));
    if let Some(w) = when {
        t.insert("when", toml_edit::value(w));
    }
    match doc
        .get_mut("binding")
        .and_then(Item::as_array_of_tables_mut)
    {
        Some(arr) => arr.push(t),
        None => {
            let mut arr = toml_edit::ArrayOfTables::new();
            arr.push(t);
            doc.insert("binding", Item::ArrayOfTables(arr));
        }
    }
    Ok(doc.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"# My settings
language = "pt-BR"

[editor]
# bigger text
fontSize = 15
wordWrap = "on"

[editor.minimap]
enabled = false

["[markdown]".editor]
tabSize = 2

[files]
exclude = ["**/.git", "**/node_modules"]
"#;

    #[test]
    fn parses_dotted_and_language_tables() {
        let p = parse_settings(SAMPLE).unwrap();
        let v = &p.layer.values;
        assert_eq!(v["language"], SettingValue::Str("pt-BR".into()));
        assert_eq!(v["editor.fontSize"], SettingValue::Int(15));
        assert_eq!(v["editor.minimap.enabled"], SettingValue::Bool(false));
        assert_eq!(
            p.layer.languages["markdown"]["editor.tabSize"],
            SettingValue::Int(2)
        );
        assert!(matches!(&v["files.exclude"], SettingValue::List(l) if l.len() == 2));
    }

    #[test]
    fn writing_preserves_comments_and_order() {
        let out = set_setting(
            SAMPLE,
            "editor.fontSize",
            None,
            Some(&SettingValue::Int(18)),
        )
        .unwrap();
        assert!(out.contains("# bigger text\nfontSize = 18"), "{out}");
        assert!(out.starts_with("# My settings"));
        let p = parse_settings(&out).unwrap();
        assert_eq!(p.layer.values["editor.fontSize"], SettingValue::Int(18));
        assert_eq!(
            p.layer.values["editor.wordWrap"],
            SettingValue::Str("on".into())
        );
    }

    #[test]
    fn writing_new_nested_and_language_keys() {
        let out = set_setting("", "editor.tabSize", None, Some(&SettingValue::Int(2))).unwrap();
        assert_eq!(out.trim(), "[editor]\ntabSize = 2");
        let out = set_setting(
            &out,
            "editor.wordWrap",
            Some("markdown"),
            Some(&SettingValue::Str("on".into())),
        )
        .unwrap();
        let p = parse_settings(&out).unwrap();
        assert_eq!(
            p.layer.languages["markdown"]["editor.wordWrap"],
            SettingValue::Str("on".into())
        );
    }

    #[test]
    fn resetting_removes_key_and_empty_tables() {
        let out = set_setting(SAMPLE, "editor.minimap.enabled", None, None).unwrap();
        assert!(!out.contains("[editor.minimap]"), "{out}");
        assert!(out.contains("fontSize = 15"));
    }

    #[test]
    fn unknown_keys_survive_writes() {
        let text = "[plugins.gone]\noption = true\n";
        let out = set_setting(text, "editor.tabSize", None, Some(&SettingValue::Int(3))).unwrap();
        assert!(out.contains("[plugins.gone]\noption = true"));
    }

    #[test]
    fn parse_error_reports_line_and_column() {
        let err = parse_settings("[editor]\nfontSize = = 3\n").unwrap_err();
        assert_eq!(err.line, 2);
        assert!(err.column > 1);
    }

    #[test]
    fn unsupported_values_are_listed() {
        let p = parse_settings("when = 1979-05-27\nmixed = [1, \"a\"]\n").unwrap();
        assert_eq!(p.unsupported, vec!["when".to_owned(), "mixed".to_owned()]);
    }

    #[test]
    fn workspace_file_is_created_lazily_with_only_that_setting() {
        let ws = tempfile::tempdir().unwrap();
        let path = ws.path().join(".notoir/settings.toml");
        let loaded = load_settings_file(&path, None).unwrap();
        assert_eq!(loaded.layer, SettingsLayer::default());
        assert!(
            !ws.path().join(".notoir").exists(),
            "reading creates nothing"
        );
        write_setting_file(&path, "editor.tabSize", None, Some(&SettingValue::Int(2))).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap().trim(),
            "[editor]\ntabSize = 2"
        );
    }

    #[test]
    fn parse_error_keeps_previous_values() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        std::fs::write(&path, "[editor]\nfontSize = 16\n").unwrap();
        let good = load_settings_file(&path, None).unwrap();
        std::fs::write(&path, "[editor\nfontSize = 20\n").unwrap();
        let bad = load_settings_file(&path, Some(&good.layer)).unwrap();
        assert_eq!(bad.error.as_ref().unwrap().line, 1);
        assert_eq!(bad.layer.values["editor.fontSize"], SettingValue::Int(16));
    }

    #[test]
    fn keybindings_parse_and_append() {
        let text = "[[binding]]\nkey = \"Mod+Shift+S\"\ncommand = \"file.saveAll\"\n\n[[binding]]\nkey = \"Mod+Shift+K\"\ncommand = \"-edit.deleteLine\"\nargs = { lines = 2 }\n";
        let b = parse_keybindings(text).unwrap();
        assert_eq!(b.len(), 2);
        assert_eq!(b[1].command, "-edit.deleteLine");
        assert_eq!(b[1].args, Some(serde_json::json!({ "lines": 2 })));
        let out = append_keybinding(
            text,
            Some("Mod+B"),
            "markdown.toggleBold",
            Some("editorFocus"),
        )
        .unwrap();
        let b = parse_keybindings(&out).unwrap();
        assert_eq!(b.len(), 3);
        assert_eq!(b[2].when.as_deref(), Some("editorFocus"));
        assert_eq!(parse_keybindings("").unwrap(), vec![]);
    }
}
