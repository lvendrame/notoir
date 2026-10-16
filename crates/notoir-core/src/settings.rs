//! Settings schema and scope resolution (docs/specs/20-settings.md).
//!
//! Resolution order for `get(key, language)`:
//! workspace language-specific → workspace → user language-specific → user →
//! built-in language default → schema default. Restricted settings skip the
//! workspace levels unless the workspace is trusted; invalid values are skipped
//! (falling through) and reported as warnings. File IO lives in storage.

use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq)]
pub enum SettingValue {
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    List(Vec<String>),
}

impl SettingValue {
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            SettingValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_int(&self) -> Option<i64> {
        match self {
            SettingValue::Int(i) => Some(*i),
            _ => None,
        }
    }

    pub fn as_float(&self) -> Option<f64> {
        match self {
            SettingValue::Float(f) => Some(*f),
            SettingValue::Int(i) => Some(*i as f64),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            SettingValue::Str(s) => Some(s),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SettingType {
    Bool,
    Int { min: i64, max: i64 },
    Float { min: f64, max: f64 },
    Enum(&'static [&'static str]),
    Str,
    StrList,
}

impl SettingType {
    pub fn accepts(&self, v: &SettingValue) -> bool {
        match (self, v) {
            (SettingType::Bool, SettingValue::Bool(_)) => true,
            (SettingType::Int { min, max }, SettingValue::Int(i)) => (min..=max).contains(&i),
            (SettingType::Float { min, max }, v) => {
                v.as_float().is_some_and(|f| (*min..=*max).contains(&f))
            }
            (SettingType::Enum(options), SettingValue::Str(s)) => options.contains(&s.as_str()),
            (SettingType::Str, SettingValue::Str(_)) => true,
            (SettingType::StrList, SettingValue::List(_)) => true,
            _ => false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SettingCategory {
    General,
    Editor,
    Appearance,
    Files,
    Workspace,
    Git,
    Mcp,
    Plugins,
    Keyboard,
    Advanced,
}

/// Scopes a setting may be written at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scopes {
    pub workspace: bool,
    pub language: bool,
}

impl Scopes {
    pub const USER_ONLY: Scopes = Scopes {
        workspace: false,
        language: false,
    };
    pub const ALL: Scopes = Scopes {
        workspace: true,
        language: true,
    };
    pub const NO_LANGUAGE: Scopes = Scopes {
        workspace: true,
        language: false,
    };
}

#[derive(Clone, Debug)]
pub struct SettingDef {
    pub key: &'static str,
    pub ty: SettingType,
    pub default: SettingValue,
    pub scopes: Scopes,
    pub category: SettingCategory,
    /// Ignored at workspace scope unless the workspace is trusted (spec 07).
    pub restricted: bool,
    /// Extra search terms.
    pub tags: &'static [&'static str],
}

impl SettingDef {
    pub fn title_key(&self) -> String {
        format!("settings:{}.title", self.key)
    }

    pub fn description_key(&self) -> String {
        format!("settings:{}.description", self.key)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Schema {
    defs: BTreeMap<&'static str, SettingDef>,
    /// Built-in language defaults, e.g. markdown → editor.wordWrap = "on".
    language_defaults: BTreeMap<&'static str, BTreeMap<&'static str, SettingValue>>,
}

impl Schema {
    pub fn define(&mut self, def: SettingDef) {
        debug_assert!(
            def.ty.accepts(&def.default),
            "default of {} has the wrong type",
            def.key
        );
        self.defs.insert(def.key, def);
    }

    /// Removes a run-time definition (a plugin's settings).
    pub fn remove(&mut self, key: &str) {
        self.defs.remove(key);
    }

    pub fn language_default(
        &mut self,
        language: &'static str,
        key: &'static str,
        value: SettingValue,
    ) {
        self.language_defaults
            .entry(language)
            .or_default()
            .insert(key, value);
    }

    pub fn get(&self, key: &str) -> Option<&SettingDef> {
        self.defs.get(key)
    }

    pub fn iter(&self) -> impl Iterator<Item = &SettingDef> {
        self.defs.values()
    }

    /// The settings Notoir ships with (grows with each milestone).
    pub fn builtin() -> Schema {
        use SettingCategory as C;
        use SettingType as T;
        use SettingValue as V;
        let mut s = Schema::default();
        let mut def = |key, ty, default, scopes, category| {
            s.define(SettingDef {
                key,
                ty,
                default,
                scopes,
                category,
                restricted: false,
                tags: &[],
            });
        };
        let s_ = |x: &str| V::Str(x.to_owned());
        // General
        def("language", T::Str, s_(""), Scopes::USER_ONLY, C::General);
        def(
            "workbench.startup",
            T::Enum(&["emptyState", "restoreSession", "welcome"]),
            s_("restoreSession"),
            Scopes::USER_ONLY,
            C::General,
        );
        def(
            "app.hotExit",
            T::Bool,
            V::Bool(true),
            Scopes::USER_ONLY,
            C::General,
        );
        // Editor
        def(
            "editor.fontFamily",
            T::Str,
            s_("JetBrains Mono"),
            Scopes::ALL,
            C::Editor,
        );
        def(
            "editor.fontSize",
            T::Int { min: 6, max: 72 },
            V::Int(14),
            Scopes::ALL,
            C::Editor,
        );
        def(
            "editor.lineHeight",
            T::Float { min: 1.0, max: 3.0 },
            V::Float(1.5),
            Scopes::ALL,
            C::Editor,
        );
        def(
            "editor.tabSize",
            T::Int { min: 1, max: 16 },
            V::Int(4),
            Scopes::ALL,
            C::Editor,
        );
        def(
            "editor.insertSpaces",
            T::Bool,
            V::Bool(true),
            Scopes::ALL,
            C::Editor,
        );
        def(
            "editor.detectIndentation",
            T::Bool,
            V::Bool(true),
            Scopes::ALL,
            C::Editor,
        );
        def(
            "editor.wordWrap",
            T::Enum(&["off", "on", "column"]),
            s_("off"),
            Scopes::ALL,
            C::Editor,
        );
        def(
            "editor.wordWrapColumn",
            T::Int { min: 20, max: 500 },
            V::Int(100),
            Scopes::ALL,
            C::Editor,
        );
        def(
            "editor.lineNumbers",
            T::Enum(&["on", "off", "relative"]),
            s_("on"),
            Scopes::ALL,
            C::Editor,
        );
        def(
            "editor.renderWhitespace",
            T::Enum(&["none", "boundary", "selection", "trailing", "all"]),
            s_("selection"),
            Scopes::ALL,
            C::Editor,
        );
        def(
            "editor.highlightCurrentLine",
            T::Enum(&["none", "gutter", "line", "all"]),
            s_("all"),
            Scopes::ALL,
            C::Editor,
        );
        def(
            "editor.cursorStyle",
            T::Enum(&["line", "block", "underline"]),
            s_("line"),
            Scopes::NO_LANGUAGE,
            C::Editor,
        );
        def(
            "editor.cursorBlinking",
            T::Enum(&["blink", "solid"]),
            s_("blink"),
            Scopes::NO_LANGUAGE,
            C::Editor,
        );
        def(
            "editor.zoomScope",
            T::Enum(&["global", "view"]),
            s_("global"),
            Scopes::USER_ONLY,
            C::Editor,
        );
        def(
            "editor.undoLimit",
            T::Int {
                min: 10,
                max: 1_000_000,
            },
            V::Int(10_000),
            Scopes::USER_ONLY,
            C::Editor,
        );
        def(
            "editor.emptySelectionClipboard",
            T::Bool,
            V::Bool(true),
            Scopes::ALL,
            C::Editor,
        );
        def(
            "editor.rulers",
            T::StrList,
            V::List(vec![]),
            Scopes::ALL,
            C::Editor,
        );
        def(
            "editor.minimap.enabled",
            T::Bool,
            V::Bool(true),
            Scopes::ALL,
            C::Editor,
        );
        def(
            "editor.minimap.side",
            T::Enum(&["right", "left"]),
            s_("right"),
            Scopes::NO_LANGUAGE,
            C::Editor,
        );
        def(
            "editor.minimap.width",
            T::Int { min: 40, max: 200 },
            V::Int(80),
            Scopes::NO_LANGUAGE,
            C::Editor,
        );
        def(
            "editor.minimap.scale",
            T::Int { min: 1, max: 3 },
            V::Int(2),
            Scopes::NO_LANGUAGE,
            C::Editor,
        );
        def(
            "editor.minimap.opacity",
            T::Float { min: 0.2, max: 1.0 },
            V::Float(0.6),
            Scopes::NO_LANGUAGE,
            C::Editor,
        );
        def(
            "editor.minimap.showViewport",
            T::Bool,
            V::Bool(true),
            Scopes::NO_LANGUAGE,
            C::Editor,
        );
        def(
            "editor.minimap.showSearchMarks",
            T::Bool,
            V::Bool(true),
            Scopes::NO_LANGUAGE,
            C::Editor,
        );
        def(
            "editor.minimap.showSelection",
            T::Bool,
            V::Bool(true),
            Scopes::NO_LANGUAGE,
            C::Editor,
        );
        // Appearance
        def(
            "workbench.theme",
            T::Str,
            s_("notoir-midnight"),
            Scopes::USER_ONLY,
            C::Appearance,
        );
        def(
            "workbench.uiScale",
            T::Float { min: 0.8, max: 1.5 },
            V::Float(1.0),
            Scopes::USER_ONLY,
            C::Appearance,
        );
        def(
            "workbench.tabs.density",
            T::Enum(&["default", "compact"]),
            s_("default"),
            Scopes::USER_ONLY,
            C::Appearance,
        );
        def(
            "workbench.toolbar.visible",
            T::Bool,
            V::Bool(true),
            Scopes::USER_ONLY,
            C::Appearance,
        );
        def(
            "workbench.statusBar.visible",
            T::Bool,
            V::Bool(true),
            Scopes::USER_ONLY,
            C::Appearance,
        );
        def(
            "workbench.breadcrumbs.enabled",
            T::Bool,
            V::Bool(true),
            Scopes::NO_LANGUAGE,
            C::Appearance,
        );
        // Files
        def(
            "files.autoSave",
            T::Enum(&["off", "afterDelay", "onFocusChange", "onWindowFocusLoss"]),
            s_("off"),
            Scopes::NO_LANGUAGE,
            C::Files,
        );
        def(
            "files.autoSaveDelay",
            T::Int {
                min: 100,
                max: 600_000,
            },
            V::Int(1000),
            Scopes::NO_LANGUAGE,
            C::Files,
        );
        def("files.encoding", T::Str, s_("utf-8"), Scopes::ALL, C::Files);
        def(
            "files.eol",
            T::Enum(&["auto", "lf", "crlf"]),
            s_("auto"),
            Scopes::ALL,
            C::Files,
        );
        def(
            "files.atomicSave",
            T::Bool,
            V::Bool(true),
            Scopes::USER_ONLY,
            C::Files,
        );
        def(
            "files.trimTrailingWhitespaceOnSave",
            T::Bool,
            V::Bool(false),
            Scopes::ALL,
            C::Files,
        );
        def(
            "files.insertFinalNewline",
            T::Bool,
            V::Bool(false),
            Scopes::ALL,
            C::Files,
        );
        def(
            "files.reloadCleanOnExternalChange",
            T::Bool,
            V::Bool(true),
            Scopes::NO_LANGUAGE,
            C::Files,
        );
        def(
            "files.exclude",
            T::StrList,
            V::List(vec![
                "**/.git".into(),
                "**/.DS_Store".into(),
                "**/Thumbs.db".into(),
            ]),
            Scopes::NO_LANGUAGE,
            C::Files,
        );
        def(
            "scratchpad.defaultLanguage",
            T::Str,
            s_("markdown"),
            Scopes::USER_ONLY,
            C::Files,
        );
        def(
            "workbench.tabs.closeAllKeepsPinned",
            T::Bool,
            V::Bool(false),
            Scopes::USER_ONLY,
            C::Appearance,
        );
        // Markdown
        def(
            "markdown.defaultMode",
            T::Enum(&["edit", "split", "preview"]),
            s_("edit"),
            Scopes::ALL,
            C::Editor,
        );
        def(
            "markdown.splitOrientation",
            T::Enum(&["auto", "horizontal", "vertical"]),
            s_("auto"),
            Scopes::NO_LANGUAGE,
            C::Editor,
        );
        def(
            "markdown.preview.maxWidth",
            T::Int {
                min: 300,
                max: 2000,
            },
            V::Int(760),
            Scopes::NO_LANGUAGE,
            C::Editor,
        );
        def(
            "markdown.preview.interactiveCheckboxes",
            T::Bool,
            V::Bool(true),
            Scopes::NO_LANGUAGE,
            C::Editor,
        );
        def(
            "markdown.preview.scrollSync",
            T::Bool,
            V::Bool(true),
            Scopes::NO_LANGUAGE,
            C::Editor,
        );
        def(
            "markdown.preview.loadRemoteImages",
            T::Enum(&["never", "ask", "always"]),
            s_("ask"),
            Scopes::NO_LANGUAGE,
            C::Editor,
        );
        // Folding, bookmarks, outline, breadcrumbs
        def(
            "editor.folding",
            T::Bool,
            V::Bool(true),
            Scopes::ALL,
            C::Editor,
        );
        def(
            "editor.minimap.showGitMarks",
            T::Bool,
            V::Bool(true),
            Scopes::NO_LANGUAGE,
            C::Editor,
        );
        def(
            "editor.minimap.showBookmarks",
            T::Bool,
            V::Bool(true),
            Scopes::NO_LANGUAGE,
            C::Editor,
        );
        def(
            "bookmarks.navigateAcrossFiles",
            T::Bool,
            V::Bool(false),
            Scopes::NO_LANGUAGE,
            C::Editor,
        );
        def(
            "outline.followCursor",
            T::Bool,
            V::Bool(true),
            Scopes::NO_LANGUAGE,
            C::Editor,
        );
        def(
            "outline.depth",
            T::Int { min: 1, max: 10 },
            V::Int(2),
            Scopes::NO_LANGUAGE,
            C::Editor,
        );
        def(
            "workbench.breadcrumbs.enabled",
            T::Bool,
            V::Bool(true),
            Scopes::NO_LANGUAGE,
            C::Appearance,
        );
        // Search, Quick Open, navigation
        def(
            "search.maxResults",
            T::Int {
                min: 100,
                max: 1_000_000,
            },
            V::Int(20_000),
            Scopes::NO_LANGUAGE,
            C::General,
        );
        def(
            "search.maxFileSize",
            T::Int {
                min: 1024,
                max: 1_073_741_824,
            },
            V::Int(5 * 1024 * 1024),
            Scopes::NO_LANGUAGE,
            C::General,
        );
        def(
            "search.exclude",
            T::StrList,
            V::List(vec!["**/node_modules".into(), "**/target".into()]),
            Scopes::NO_LANGUAGE,
            C::General,
        );
        def(
            "search.quickOpenMaxFiles",
            T::Int {
                min: 1000,
                max: 1_000_000,
            },
            V::Int(200_000),
            Scopes::NO_LANGUAGE,
            C::General,
        );
        def(
            "navigation.minLineDistance",
            T::Int { min: 0, max: 1000 },
            V::Int(10),
            Scopes::NO_LANGUAGE,
            C::General,
        );
        // JSON
        def(
            "json.syncSelection",
            T::Bool,
            V::Bool(true),
            Scopes::NO_LANGUAGE,
            C::Editor,
        );
        def(
            "json.tree.maxSize",
            T::Int {
                min: 1024,
                max: 1_073_741_824,
            },
            V::Int(20 * 1024 * 1024),
            Scopes::NO_LANGUAGE,
            C::Editor,
        );
        // Compare
        def(
            "diff.maxSize",
            T::Int {
                min: 1024,
                max: 1_073_741_824,
            },
            V::Int(50 * 1024 * 1024),
            Scopes::NO_LANGUAGE,
            C::Editor,
        );
        def(
            "diff.ignoreTrimWhitespace",
            T::Bool,
            V::Bool(false),
            Scopes::NO_LANGUAGE,
            C::Editor,
        );
        def(
            "diff.algorithm",
            T::Enum(&["patience", "myers"]),
            s_("patience"),
            Scopes::NO_LANGUAGE,
            C::Editor,
        );
        // Zen Mode
        def(
            "zenMode.hideTabs",
            T::Bool,
            V::Bool(true),
            Scopes::USER_ONLY,
            C::Appearance,
        );
        def(
            "zenMode.hideLineNumbers",
            T::Bool,
            V::Bool(true),
            Scopes::USER_ONLY,
            C::Appearance,
        );
        def(
            "zenMode.fullScreen",
            T::Bool,
            V::Bool(true),
            Scopes::USER_ONLY,
            C::Appearance,
        );
        def(
            "zenMode.maxWidth",
            T::Int { min: 40, max: 400 },
            V::Int(80),
            Scopes::USER_ONLY,
            C::Appearance,
        );
        // Workspace
        def(
            "explorer.showHidden",
            T::Bool,
            V::Bool(false),
            Scopes::NO_LANGUAGE,
            C::Workspace,
        );
        def(
            "explorer.showGitIgnored",
            T::Bool,
            V::Bool(true),
            Scopes::NO_LANGUAGE,
            C::Workspace,
        );
        def(
            "explorer.dimGitIgnored",
            T::Bool,
            V::Bool(true),
            Scopes::NO_LANGUAGE,
            C::Workspace,
        );
        def(
            "explorer.sortOrder",
            T::Enum(&["default", "mixed", "modified"]),
            s_("default"),
            Scopes::NO_LANGUAGE,
            C::Workspace,
        );
        def(
            "explorer.autoReveal",
            T::Bool,
            V::Bool(true),
            Scopes::NO_LANGUAGE,
            C::Workspace,
        );
        def(
            "explorer.confirmDragAndDrop",
            T::Bool,
            V::Bool(true),
            Scopes::USER_ONLY,
            C::Workspace,
        );
        // Git (spec 28). Nothing runs on the network unless enabled.
        for (key, default) in [
            ("git.enabled", true),
            ("git.autoDetect", true),
            ("git.autoFetch", false),
            ("git.pullOnOpen", false),
            ("git.autoCommit", false),
            ("git.confirmSync", true),
            ("git.decorateTabs", true),
            ("git.scanNestedRepos", false),
        ] {
            def(key, T::Bool, V::Bool(default), Scopes::NO_LANGUAGE, C::Git);
        }
        def(
            "git.autoFetchPeriod",
            T::Int {
                min: 30,
                max: 86_400,
            },
            V::Int(300),
            Scopes::NO_LANGUAGE,
            C::Git,
        );
        def(
            "git.pullMode",
            T::Enum(&["ffOnly", "rebase", "merge"]),
            s_("ffOnly"),
            Scopes::NO_LANGUAGE,
            C::Git,
        );
        def(
            "git.gutterBase",
            T::Enum(&["index", "head"]),
            s_("index"),
            Scopes::NO_LANGUAGE,
            C::Git,
        );
        def("git.path", T::Str, s_(""), Scopes::USER_ONLY, C::Git);
        def(
            "git.smartCommit",
            T::Enum(&["ask", "always", "never"]),
            s_("ask"),
            Scopes::NO_LANGUAGE,
            C::Git,
        );
        def(
            "notes.sync.commitMessage",
            T::Str,
            s_("Notes sync {{date}} {{time}} from {{hostname}}"),
            Scopes::NO_LANGUAGE,
            C::Git,
        );
        // Sync Notes commits and pushes: restricted, off by default.
        s.define(SettingDef {
            key: "notes.sync.enabled",
            ty: T::Bool,
            default: V::Bool(false),
            scopes: Scopes::NO_LANGUAGE,
            category: C::Git,
            restricted: true,
            tags: &["sync", "notes"],
        });
        // Restricted: a cloned repo must not push on its own.
        s.define(SettingDef {
            key: "git.autoPush",
            ty: T::Bool,
            default: V::Bool(false),
            scopes: Scopes::NO_LANGUAGE,
            category: C::Git,
            restricted: true,
            tags: &["push"],
        });
        // MCP (restricted: a cloned repo must not start servers)
        s.define(SettingDef {
            key: "mcp.autoStart",
            ty: T::Bool,
            default: V::Bool(false),
            scopes: Scopes::NO_LANGUAGE,
            category: C::Mcp,
            restricted: true,
            tags: &["server"],
        });
        // Machine-specific: one port per machine.
        s.define(SettingDef {
            key: "mcp.port",
            ty: T::Int {
                min: 1024,
                max: 65535,
            },
            default: V::Int(47823),
            scopes: Scopes::USER_ONLY,
            category: C::Mcp,
            restricted: false,
            tags: &["server", "port"],
        });
        s.define(SettingDef {
            key: "mcp.requestTimeoutMs",
            ty: T::Int {
                min: 1000,
                max: 600_000,
            },
            default: V::Int(30_000),
            scopes: Scopes::USER_ONLY,
            category: C::Mcp,
            restricted: false,
            tags: &["timeout"],
        });
        s.define(SettingDef {
            key: "mcp.maxReadBytes",
            ty: T::Int {
                min: 1024,
                max: 64 * 1024 * 1024,
            },
            default: V::Int(2 * 1024 * 1024),
            scopes: Scopes::USER_ONLY,
            category: C::Mcp,
            restricted: false,
            tags: &["read", "limit"],
        });
        s.define(SettingDef {
            key: "mcp.highlightEdits",
            ty: T::Bool,
            default: V::Bool(true),
            scopes: Scopes::NO_LANGUAGE,
            category: C::Mcp,
            restricted: false,
            tags: &["highlight", "edit"],
        });
        s.define(SettingDef {
            key: "mcp.activityRetentionDays",
            ty: T::Int { min: 1, max: 365 },
            default: V::Int(14),
            scopes: Scopes::USER_ONLY,
            category: C::Mcp,
            restricted: false,
            tags: &["activity", "log"],
        });
        // Plugins (machine-specific / resource limits: User scope)
        s.define(SettingDef {
            key: "plugins.directory",
            ty: T::Str,
            default: s_(""),
            scopes: Scopes::USER_ONLY,
            category: C::Plugins,
            restricted: false,
            tags: &["folder"],
        });
        s.define(SettingDef {
            key: "plugins.memoryLimitMb",
            ty: T::Int { min: 8, max: 1024 },
            default: V::Int(64),
            scopes: Scopes::USER_ONLY,
            category: C::Plugins,
            restricted: false,
            tags: &["memory", "limit"],
        });
        s.define(SettingDef {
            key: "plugins.commandTimeoutMs",
            ty: T::Int {
                min: 100,
                max: 600_000,
            },
            default: V::Int(5000),
            scopes: Scopes::USER_ONLY,
            category: C::Plugins,
            restricted: false,
            tags: &["time", "limit"],
        });
        s.define(SettingDef {
            key: "plugins.eventTimeoutMs",
            ty: T::Int {
                min: 10,
                max: 60_000,
            },
            default: V::Int(200),
            scopes: Scopes::USER_ONLY,
            category: C::Plugins,
            restricted: false,
            tags: &["time", "limit"],
        });
        s.define(SettingDef {
            key: "plugins.changeEventDebounceMs",
            ty: T::Int {
                min: 0,
                max: 10_000,
            },
            default: V::Int(150),
            scopes: Scopes::USER_ONLY,
            category: C::Plugins,
            restricted: false,
            tags: &["events"],
        });
        // Advanced
        s.define(SettingDef {
            key: "logging.level",
            ty: T::Enum(&["error", "warn", "info", "debug", "trace"]),
            default: s_("info"),
            scopes: Scopes::USER_ONLY,
            category: C::Advanced,
            restricted: false,
            tags: &["log"],
        });

        s.language_default("markdown", "editor.wordWrap", s_("on"));
        s.language_default("plaintext", "editor.wordWrap", s_("on"));
        s
    }
}

/// Values of one scope (user or workspace) as read from its file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SettingsLayer {
    pub values: BTreeMap<String, SettingValue>,
    /// `["[markdown]"]` tables: language → key → value.
    pub languages: BTreeMap<String, BTreeMap<String, SettingValue>>,
}

impl SettingsLayer {
    pub fn set(&mut self, key: impl Into<String>, value: SettingValue) -> &mut Self {
        self.values.insert(key.into(), value);
        self
    }

    pub fn set_for_language(
        &mut self,
        language: impl Into<String>,
        key: impl Into<String>,
        value: SettingValue,
    ) -> &mut Self {
        self.languages
            .entry(language.into())
            .or_default()
            .insert(key.into(), value);
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingScope {
    User,
    Workspace,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingsWarning {
    /// Wrong type or out of range at that scope; the next scope was used.
    InvalidValue { scope: SettingScope, key: String },
    /// Set at a scope the schema does not allow (e.g. machine setting in a
    /// workspace file); ignored.
    WrongScope { scope: SettingScope, key: String },
    /// Not in the schema: preserved in the file, listed under "Unknown settings".
    Unknown { scope: SettingScope, key: String },
}

#[derive(Clone, Debug)]
pub struct ResolvedSettings {
    schema: Arc<Schema>,
    user: SettingsLayer,
    workspace: Option<SettingsLayer>,
    workspace_trusted: bool,
    warnings: Vec<SettingsWarning>,
}

impl ResolvedSettings {
    pub fn new(
        schema: Arc<Schema>,
        user: SettingsLayer,
        workspace: Option<SettingsLayer>,
        workspace_trusted: bool,
    ) -> Self {
        let mut r = Self {
            schema,
            user,
            workspace,
            workspace_trusted,
            warnings: Vec::new(),
        };
        r.warnings = r.validate();
        r
    }

    pub fn defaults(schema: Arc<Schema>) -> Self {
        Self::new(schema, SettingsLayer::default(), None, false)
    }

    pub fn schema(&self) -> &Schema {
        &self.schema
    }

    pub fn warnings(&self) -> &[SettingsWarning] {
        &self.warnings
    }

    fn validate(&self) -> Vec<SettingsWarning> {
        let mut out = Vec::new();
        let layers = [
            (SettingScope::User, Some(&self.user)),
            (SettingScope::Workspace, self.workspace.as_ref()),
        ];
        for (scope, layer) in layers {
            let Some(layer) = layer else { continue };
            let all = layer.values.iter().map(|(k, v)| (k, v, false)).chain(
                layer
                    .languages
                    .values()
                    .flat_map(|m| m.iter().map(|(k, v)| (k, v, true))),
            );
            for (key, value, per_language) in all {
                let Some(def) = self.schema.get(key) else {
                    out.push(SettingsWarning::Unknown {
                        scope,
                        key: key.clone(),
                    });
                    continue;
                };
                let scope_ok = (scope == SettingScope::User || def.scopes.workspace)
                    && (!per_language || def.scopes.language);
                if !scope_ok {
                    out.push(SettingsWarning::WrongScope {
                        scope,
                        key: key.clone(),
                    });
                } else if !def.ty.accepts(value) {
                    out.push(SettingsWarning::InvalidValue {
                        scope,
                        key: key.clone(),
                    });
                }
            }
        }
        out.dedup();
        out
    }

    /// Effective value of `key` for documents of `language` (or language-agnostic).
    pub fn get(&self, key: &str, language: Option<&str>) -> Option<&SettingValue> {
        let def = self.schema.get(key)?;
        let ok = |v: &&SettingValue| def.ty.accepts(v);
        fn lang_value<'a>(
            layer: &'a SettingsLayer,
            def: &SettingDef,
            language: Option<&str>,
        ) -> Option<&'a SettingValue> {
            if !def.scopes.language {
                return None;
            }
            language
                .and_then(|l| layer.languages.get(l))
                .and_then(|m| m.get(def.key))
        }
        let workspace_allowed = def.scopes.workspace && (!def.restricted || self.workspace_trusted);
        if let Some(ws) = self.workspace.as_ref().filter(|_| workspace_allowed) {
            if let Some(v) = lang_value(ws, def, language).filter(ok) {
                return Some(v);
            }
            if let Some(v) = ws.values.get(key).filter(ok) {
                return Some(v);
            }
        }
        if let Some(v) = lang_value(&self.user, def, language).filter(ok) {
            return Some(v);
        }
        if let Some(v) = self.user.values.get(key).filter(ok) {
            return Some(v);
        }
        if let Some(v) = language
            .and_then(|l| self.schema.language_defaults.get(l))
            .and_then(|m| m.get(key))
        {
            return Some(v);
        }
        Some(&def.default)
    }

    pub fn str_list(&self, key: &str, language: Option<&str>) -> Vec<String> {
        match self.get(key, language) {
            Some(SettingValue::List(v)) => v.clone(),
            _ => Vec::new(),
        }
    }

    pub fn bool(&self, key: &str, language: Option<&str>) -> bool {
        self.get(key, language)
            .and_then(SettingValue::as_bool)
            .unwrap_or(false)
    }

    pub fn int(&self, key: &str, language: Option<&str>) -> i64 {
        self.get(key, language)
            .and_then(SettingValue::as_int)
            .unwrap_or(0)
    }

    pub fn float(&self, key: &str, language: Option<&str>) -> f64 {
        self.get(key, language)
            .and_then(SettingValue::as_float)
            .unwrap_or(0.0)
    }

    pub fn str(&self, key: &str, language: Option<&str>) -> &str {
        self.get(key, language)
            .and_then(SettingValue::as_str)
            .unwrap_or("")
    }

    /// Keys whose effective value differs between `self` and `newer`, for the
    /// languages mentioned in either (plus language-agnostic).
    pub fn changed_keys(&self, newer: &ResolvedSettings) -> Vec<String> {
        let mut languages: Vec<Option<&str>> = vec![None];
        for r in [self, newer] {
            for layer in std::iter::once(&r.user).chain(r.workspace.as_ref()) {
                languages.extend(layer.languages.keys().map(|l| Some(l.as_str())));
            }
            languages.extend(r.schema.language_defaults.keys().map(|l| Some(*l)));
        }
        languages.sort();
        languages.dedup();
        newer
            .schema
            .iter()
            .filter(|d| {
                languages
                    .iter()
                    .any(|l| self.get(d.key, *l) != newer.get(d.key, *l))
            })
            .map(|d| d.key.to_owned())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema() -> Arc<Schema> {
        Arc::new(Schema::builtin())
    }

    fn int(i: i64) -> SettingValue {
        SettingValue::Int(i)
    }

    fn s(x: &str) -> SettingValue {
        SettingValue::Str(x.into())
    }

    // spec 20: "Workspace overrides user"
    #[test]
    fn workspace_overrides_user() {
        let mut user = SettingsLayer::default();
        user.set("editor.tabSize", int(4));
        let mut ws = SettingsLayer::default();
        ws.set("editor.tabSize", int(2));
        let r = ResolvedSettings::new(schema(), user, Some(ws), false);
        assert_eq!(r.int("editor.tabSize", Some("rust")), 2);
    }

    // spec 20: "Language-specific beats general"
    #[test]
    fn language_specific_beats_general() {
        let mut user = SettingsLayer::default();
        user.set("editor.wordWrap", s("off"));
        user.set_for_language("markdown", "editor.wordWrap", s("on"));
        let r = ResolvedSettings::new(schema(), user, None, false);
        assert_eq!(r.str("editor.wordWrap", Some("markdown")), "on");
        assert_eq!(r.str("editor.wordWrap", Some("json")), "off");
    }

    #[test]
    fn built_in_language_defaults() {
        let r = ResolvedSettings::defaults(schema());
        assert_eq!(r.str("editor.wordWrap", Some("markdown")), "on");
        assert_eq!(r.str("editor.wordWrap", Some("rust")), "off");
        assert_eq!(r.str("editor.wordWrap", None), "off");
    }

    // spec 20: "Invalid value falls through"
    #[test]
    fn invalid_value_falls_through_with_warning() {
        let mut user = SettingsLayer::default();
        user.set("editor.fontSize", int(16));
        let mut ws = SettingsLayer::default();
        ws.set("editor.fontSize", s("huge"));
        let r = ResolvedSettings::new(schema(), user, Some(ws), false);
        assert_eq!(r.int("editor.fontSize", None), 16);
        assert_eq!(
            r.warnings(),
            &[SettingsWarning::InvalidValue {
                scope: SettingScope::Workspace,
                key: "editor.fontSize".into()
            }]
        );
    }

    #[test]
    fn out_of_range_is_invalid() {
        let mut user = SettingsLayer::default();
        user.set("editor.tabSize", int(99));
        let r = ResolvedSettings::new(schema(), user, None, false);
        assert_eq!(r.int("editor.tabSize", None), 4);
    }

    // spec 20: "Restricted setting in untrusted workspace"
    #[test]
    fn restricted_setting_needs_trust() {
        let mut ws = SettingsLayer::default();
        ws.set("mcp.autoStart", SettingValue::Bool(true));
        let untrusted =
            ResolvedSettings::new(schema(), SettingsLayer::default(), Some(ws.clone()), false);
        assert!(!untrusted.bool("mcp.autoStart", None));
        let trusted = ResolvedSettings::new(schema(), SettingsLayer::default(), Some(ws), true);
        assert!(trusted.bool("mcp.autoStart", None));
    }

    #[test]
    fn user_only_settings_ignore_workspace() {
        let mut ws = SettingsLayer::default();
        ws.set("workbench.theme", s("dracula"));
        let r = ResolvedSettings::new(schema(), SettingsLayer::default(), Some(ws), true);
        assert_eq!(r.str("workbench.theme", None), "notoir-midnight");
        assert!(matches!(
            r.warnings()[0],
            SettingsWarning::WrongScope { .. }
        ));
    }

    #[test]
    fn unknown_keys_are_reported_not_lost() {
        let mut user = SettingsLayer::default();
        user.set("plugins.gone.option", SettingValue::Bool(true));
        let r = ResolvedSettings::new(schema(), user, None, false);
        assert!(
            matches!(&r.warnings()[0], SettingsWarning::Unknown { key, .. } if key == "plugins.gone.option")
        );
    }

    #[test]
    fn changed_keys_lists_effective_changes() {
        let before = ResolvedSettings::defaults(schema());
        let mut user = SettingsLayer::default();
        user.set("editor.tabSize", int(4))
            .set("editor.fontSize", int(18));
        user.set_for_language("markdown", "editor.tabSize", int(2));
        let after = ResolvedSettings::new(schema(), user, None, false);
        assert_eq!(
            before.changed_keys(&after),
            vec!["editor.fontSize".to_owned(), "editor.tabSize".to_owned()]
        );
    }
}
