//! `plugin.toml` (docs/specs/30 §Manifest): parsing, validation, the
//! permission set and its stable hash (what an approval covers).

use std::path::{Path, PathBuf};

use notoir_core::permissions::Capability;
use serde::Deserialize;

/// The plugin API major version this Notoir supports.
pub const API_VERSION: &str = "1";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ManifestError {
    #[error("plugin.toml line {line}: {message}")]
    Syntax { line: usize, message: String },
    #[error("{0}")]
    Invalid(String),
    /// `api` major differs from [`API_VERSION`].
    #[error("needs plugin API {0}; this Notoir supports API {API_VERSION}")]
    Incompatible(String),
    #[error("cannot read plugin.toml: {0}")]
    Io(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub api: String,
    #[serde(default = "default_main")]
    pub main: String,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub license: String,
}

fn default_main() -> String {
    "main.lua".into()
}

/// Requested permissions; everything is off unless set.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Permissions {
    pub workspace_read: bool,
    pub workspace_write: bool,
    pub documents_read: bool,
    pub documents_write: bool,
    pub network: bool,
    pub network_hosts: Vec<String>,
    pub process: bool,
    pub clipboard: bool,
    pub mcp_tools: bool,
    pub settings_write: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandContrib {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub category: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeybindingContrib {
    pub command: String,
    pub key: String,
    #[serde(default)]
    pub when: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MenuContrib {
    pub command: String,
    /// `editor/context`, `plugins` (the Plugins menu).
    pub location: String,
    #[serde(default)]
    pub group: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatusItemContrib {
    pub id: String,
    #[serde(default = "default_alignment")]
    pub alignment: String,
    #[serde(default)]
    pub priority: i64,
}

fn default_alignment() -> String {
    "right".into()
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingContrib {
    pub key: String,
    #[serde(rename = "type")]
    pub ty: String,
    pub default: toml::Value,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub values: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LanguageContrib {
    pub id: String,
    #[serde(default)]
    pub extensions: Vec<String>,
    #[serde(default)]
    pub syntax: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PanelContrib {
    pub id: String,
    pub title: String,
    #[serde(default = "default_area")]
    pub area: String,
}

fn default_area() -> String {
    "secondary".into()
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Contributes {
    pub commands: Vec<CommandContrib>,
    pub keybindings: Vec<KeybindingContrib>,
    pub menus: Vec<MenuContrib>,
    pub status_items: Vec<StatusItemContrib>,
    pub settings: Vec<SettingContrib>,
    pub languages: Vec<LanguageContrib>,
    pub panels: Vec<PanelContrib>,
    /// `onStartup`, `onCommand:<id>`, `onLanguage:<id>`, `onEvent:<name>`.
    pub activation: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub plugin: PluginInfo,
    #[serde(default)]
    pub permissions: Permissions,
    #[serde(default)]
    pub contributes: Contributes,
}

/// 1-based line of a byte offset.
fn line_of(text: &str, offset: usize) -> usize {
    text[..offset.min(text.len())].matches('\n').count() + 1
}

fn valid_id(id: &str) -> bool {
    (2..=64).contains(&id.len())
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// `MAJOR.MINOR.PATCH` with an optional `-pre` / `+build`.
fn valid_semver(v: &str) -> bool {
    let core = v.split(['-', '+']).next().unwrap_or("");
    let parts: Vec<&str> = core.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

/// Compares two semver strings by their numeric core.
pub fn version_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let nums = |v: &str| -> Vec<u64> {
        v.split(['-', '+'])
            .next()
            .unwrap_or("")
            .split('.')
            .map(|p| p.parse().unwrap_or(0))
            .collect()
    };
    nums(a).cmp(&nums(b))
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Manifest, ManifestError> {
        let m: Manifest = toml::from_str(text).map_err(|e| ManifestError::Syntax {
            line: e.span().map_or(1, |s| line_of(text, s.start)),
            message: e.message().to_owned(),
        })?;
        m.validate()?;
        Ok(m)
    }

    /// Reads and validates `<dir>/plugin.toml`; also checks `main` exists.
    pub fn load(dir: &Path) -> Result<Manifest, ManifestError> {
        let text = std::fs::read_to_string(dir.join("plugin.toml"))
            .map_err(|e| ManifestError::Io(e.to_string()))?;
        let m = Manifest::parse(&text)?;
        if !dir.join(&m.plugin.main).is_file() {
            return Err(ManifestError::Invalid(format!(
                "main file {} is missing",
                m.plugin.main
            )));
        }
        Ok(m)
    }

    fn validate(&self) -> Result<(), ManifestError> {
        let p = &self.plugin;
        let bad = |m: String| Err(ManifestError::Invalid(m));
        if !valid_id(&p.id) {
            return bad(format!(
                "invalid id \"{}\" (use a-z, 0-9 and -, 2–64 characters)",
                p.id
            ));
        }
        if p.name.trim().is_empty() {
            return bad("name is empty".into());
        }
        if !valid_semver(&p.version) {
            return bad(format!("version \"{}\" is not semver", p.version));
        }
        if p.api.split('.').next() != Some(API_VERSION) {
            return Err(ManifestError::Incompatible(p.api.clone()));
        }
        if p.main.contains("..") || Path::new(&p.main).is_absolute() || !p.main.ends_with(".lua") {
            return bad(format!(
                "main \"{}\" must be a .lua file inside the plugin",
                p.main
            ));
        }
        let mut seen = std::collections::HashSet::new();
        for c in &self.contributes.commands {
            if !valid_id(&c.id.replace('.', "-")) || c.id.is_empty() {
                return bad(format!("invalid command id \"{}\"", c.id));
            }
            if !seen.insert(c.id.as_str()) {
                return bad(format!("command \"{}\" is declared twice", c.id));
            }
        }
        for k in &self.contributes.keybindings {
            if !seen.contains(k.command.as_str()) {
                return bad(format!(
                    "keybinding for undeclared command \"{}\"",
                    k.command
                ));
            }
        }
        for m in &self.contributes.menus {
            if !seen.contains(m.command.as_str()) {
                return bad(format!(
                    "menu entry for undeclared command \"{}\"",
                    m.command
                ));
            }
        }
        for a in &self.contributes.activation {
            let ok = a == "onStartup"
                || a == "*"
                || a.strip_prefix("onCommand:")
                    .is_some_and(|c| seen.contains(c))
                || a.strip_prefix("onLanguage:").is_some_and(|l| !l.is_empty())
                || a.strip_prefix("onEvent:").is_some_and(|e| !e.is_empty());
            if !ok {
                return bad(format!("unknown activation event \"{a}\""));
            }
        }
        Ok(())
    }

    /// Activation on `event` (`onCommand:count`…).
    pub fn activates_on(&self, event: &str) -> bool {
        self.contributes
            .activation
            .iter()
            .any(|a| a == event || a == "*")
    }
}

impl Permissions {
    /// Plain-language rows for the approval prompt: (translation key, granted).
    pub fn rows(&self) -> Vec<(&'static str, bool)> {
        vec![
            ("plugins:perm.documentsRead", self.documents_read),
            ("plugins:perm.documentsWrite", self.documents_write),
            ("plugins:perm.workspaceRead", self.workspace_read),
            ("plugins:perm.workspaceWrite", self.workspace_write),
            ("plugins:perm.network", self.network),
            ("plugins:perm.process", self.process),
            ("plugins:perm.clipboard", self.clipboard),
            ("plugins:perm.settingsWrite", self.settings_write),
            ("plugins:perm.mcpTools", self.mcp_tools),
        ]
    }

    /// Capabilities granted by these permissions (spec 32: requested and
    /// approved → Allow; `process` asks per call).
    pub fn capabilities(&self) -> Vec<Capability> {
        let mut caps = Vec::new();
        let mut add = |on: bool, list: &[Capability]| {
            if on {
                caps.extend_from_slice(list);
            }
        };
        add(
            self.documents_read,
            &[Capability::DocumentRead, Capability::EditorRead],
        );
        add(
            self.documents_write,
            &[Capability::DocumentModify, Capability::EditorControl],
        );
        add(
            self.workspace_read,
            &[Capability::WorkspaceRead, Capability::FilesystemRead],
        );
        add(
            self.workspace_write,
            &[
                Capability::FilesystemWrite,
                Capability::FileCreate,
                Capability::FileRename,
            ],
        );
        add(self.network, &[Capability::Network]);
        add(self.clipboard, &[Capability::Clipboard]);
        add(self.settings_write, &[Capability::SettingsModify]);
        add(self.mcp_tools, &[Capability::ExposeMcpTools]);
        caps
    }

    /// Canonical form (what the hash covers).
    fn canonical(&self) -> String {
        let mut hosts = self.network_hosts.clone();
        hosts.sort();
        format!(
            "wr={};ww={};dr={};dw={};net={};hosts={};proc={};clip={};mcp={};set={}",
            self.workspace_read,
            self.workspace_write,
            self.documents_read,
            self.documents_write,
            self.network,
            hosts.join(","),
            self.process,
            self.clipboard,
            self.mcp_tools,
            self.settings_write
        )
    }

    /// Stable hash (FNV-1a 64, hex) of the permission set.
    pub fn hash(&self) -> String {
        let mut h: u64 = 0xcbf29ce484222325;
        for b in self.canonical().bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x100000001b3);
        }
        format!("{h:016x}")
    }

    /// Rows requested here but not in `old` (shown on re-approval).
    pub fn added_since(&self, old: &Permissions) -> Vec<&'static str> {
        let before = old.rows();
        self.rows()
            .into_iter()
            .zip(before)
            .filter(|((_, now), (_, was))| *now && !*was)
            .map(|((k, _), _)| k)
            .collect()
    }

    /// Whether `host` may be reached (`network_hosts` empty = any).
    pub fn host_allowed(&self, host: &str) -> bool {
        self.network
            && (self.network_hosts.is_empty()
                || self
                    .network_hosts
                    .iter()
                    .any(|h| h.eq_ignore_ascii_case(host)))
    }
}

/// A plugin directory found on disk.
#[derive(Debug, Clone)]
pub struct Discovered {
    pub dir: PathBuf,
    pub manifest: Result<Manifest, ManifestError>,
}

/// Every plugin folder (with a `plugin.toml`) directly inside `dir`, by name.
pub fn scan(dir: &Path) -> Vec<Discovered> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p.join("plugin.toml").is_file())
        .collect();
    dirs.sort();
    dirs.into_iter()
        .map(|dir| Discovered {
            manifest: Manifest::load(&dir),
            dir,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORD_COUNT: &str = r#"
[plugin]
id = "word-count"
name = "Word Count"
version = "1.0.0"
api = "1"
main = "main.lua"

[permissions]
documents_read = true

[contributes]
commands = [{ id = "count", title = "%commands.count%" }]
keybindings = [{ command = "count", key = "Mod+Alt+W", when = "editorFocus" }]
activation = ["onCommand:count"]
"#;

    #[test]
    fn parses_and_validates() {
        let m = Manifest::parse(WORD_COUNT).unwrap();
        assert_eq!(m.plugin.id, "word-count");
        assert!(m.permissions.documents_read && !m.permissions.network);
        assert!(m.activates_on("onCommand:count"));
        assert_eq!(
            m.permissions.capabilities(),
            [Capability::DocumentRead, Capability::EditorRead]
        );
    }

    #[test]
    fn errors_name_the_line() {
        let e = Manifest::parse("[plugin]\nid = \"x\nname = 1\n").unwrap_err();
        assert!(matches!(e, ManifestError::Syntax { line: 2, .. }), "{e:?}");
        let bad = WORD_COUNT.replace("\"word-count\"", "\"Word Count\"");
        assert!(matches!(
            Manifest::parse(&bad),
            Err(ManifestError::Invalid(_))
        ));
        let api = WORD_COUNT.replace("api = \"1\"", "api = \"2\"");
        assert!(matches!(
            Manifest::parse(&api),
            Err(ManifestError::Incompatible(_))
        ));
        let undeclared = WORD_COUNT.replace("command = \"count\"", "command = \"other\"");
        assert!(Manifest::parse(&undeclared).is_err());
    }

    #[test]
    fn permission_hash_and_additions() {
        let a = Permissions {
            documents_read: true,
            ..Default::default()
        };
        let mut b = a.clone();
        assert_eq!(a.hash(), b.hash());
        b.network = true;
        assert_ne!(a.hash(), b.hash());
        assert_eq!(b.added_since(&a), ["plugins:perm.network"]);
        assert!(b.host_allowed("x.example"));
        b.network_hosts = vec!["api.example.com".into()];
        assert!(b.host_allowed("API.example.com") && !b.host_allowed("evil.example"));
        assert_eq!(version_cmp("1.10.0", "1.9.3"), std::cmp::Ordering::Greater);
    }
}
