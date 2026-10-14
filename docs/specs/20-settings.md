# 20 — Settings

## Purpose

Define the settings schema, storage, scope resolution (Default → User → Workspace → language-specific), the
searchable Settings tab, and keybinding files.

## Storage

| Scope | File | Location |
|---|---|---|
| User | `settings.toml` | `<config>/settings.toml` (spec 31 paths) |
| User keybindings | `keybindings.toml` | `<config>/keybindings.toml` |
| Workspace | `settings.toml` | `<workspace>/.notoir/settings.toml` (created on first workspace-scope write) |
| Workspace plugins | `plugins.toml` | `<workspace>/.notoir/plugins.toml` (spec 30) |

- TOML with dotted keys grouped in tables; written with `toml_edit` so user comments and ordering survive.
- Files are watched; external edits apply live. Parse errors keep the last good values and show a warning
  notification with "Open settings.toml" (line/column of the error).
- Unknown keys are preserved and reported in the Settings UI under "Unknown settings" (helps after downgrades/plugin
  removal), never deleted.
- Writes are atomic (temp + rename).

Example user file:

```toml
language = "pt-BR"

[workbench]
theme = "notoir-midnight"
uiScale = 1.0

[editor]
fontFamily = "JetBrains Mono"
fontSize = 15
wordWrap = "on"
lineNumbers = "on"

[editor.minimap]
enabled = true

[files]
autoSave = "off"

["[markdown]".editor]
wordWrap = "on"
tabSize = 2
```

Workspace file:

```toml
[editor]
tabSize = 2

[files]
autoSave = "afterDelay"
```

## Schema

Each setting is declared once in `notoir-core::settings::schema`:

```rust
pub struct SettingDef {
    pub key: &'static str,                 // "editor.tabSize"
    pub ty: SettingType,                   // Bool | Int{min,max} | Float{..} | Enum(&[..]) | String | StringList | Color | Font
    pub default: SettingValue,
    pub scopes: ScopeMask,                 // USER | WORKSPACE | LANGUAGE
    pub category: SettingCategory,         // General, Editor, Appearance, Files, Workspace, Git, Mcp, Plugins, Keyboard, Advanced
    pub restricted: bool,                  // ignored at Workspace scope unless workspace is trusted (spec 07)
    pub title_key: &'static str,           // "settings:editor.tabSize.title"
    pub description_key: &'static str,
    pub tags: &'static [&'static str],     // extra search terms
}
```

Machine-specific settings (e.g. `mcp.port`, `plugins.directory`, `git.path`) have `scopes = USER` only. Plugins register
settings under `plugins.<id>.*` with the same structure via their manifest (spec 30).

## Resolution

`ResolvedSettings::get(key, language)`:

1. Workspace language-specific (`["[markdown]"]` table in workspace file) if allowed
2. Workspace
3. User language-specific
4. User
5. Built-in language default (e.g. Markdown wordWrap on)
6. Schema default

Restricted settings skip workspace levels when untrusted. Invalid values (wrong type/out of range) are ignored at that
level with a warning, falling through to the next level. `SettingsChanged` events list the keys whose **effective**
value changed.

## Settings UI

Opens as an editor tab ("Settings", `settings` icon), never a blocking modal. `app.settings` (`Mod+,`).

```text
┌────────────────────────────────────────────────────────────────────────────┐
│ [ Search settings                     ]                  ( User | Workspace ) │
├──────────────────┬─────────────────────────────────────────────────────────┤
│ General          │ Editor › Font                                            │
│ Editor           │ Font Family                                              │
│ Appearance       │ [ JetBrains Mono                                   ▾ ]   │
│ Files            │                                                          │
│ Workspace        │ Font Size                         ● modified in User     │
│ Git              │ [ 15 ]                                                   │
│ MCP              │                                                          │
│ Plugins          │ ☑ Line Numbers                                           │
│ Keyboard         │ ☑ Minimap                                                │
│ Advanced         │ ☑ Word Wrap      ⚠ overridden in Workspace               │
└──────────────────┴─────────────────────────────────────────────────────────┘
```

- Scope selector `User | Workspace` (Workspace disabled without an open workspace). Editing writes to the selected
  scope's file.
- Each modified setting shows a left accent bar and "Modified in User/Workspace"; gear menu per row: Reset Setting, Copy
  Setting ID, Copy Setting as TOML, "Apply to Language…".
- When a value is overridden by a higher scope, the row shows "Overridden in Workspace" linking there.
- Search matches title, description, key, tags, both translated and English; filters `@modified`, `@lang:markdown`,
  `@plugin:<id>`.
- Controls by type: checkbox, number with stepper, dropdown, text, font picker (system fonts via `fontdb`, monospace
  first), colour swatch, list editor.
- "Open settings.toml" button (top right) opens the raw file of the selected scope in an editor tab.
- Keyboard category embeds the Keyboard Shortcuts editor: searchable table Command | Keybinding | When | Source, with
  record-key-chord editing and conflict warnings; writes to `keybindings.toml`.
- The Settings tab is a `Virtual` document (spec 02); its state is not dirty-tracked (changes apply immediately).

## Categories and Key Settings (initial)

| Category | Settings |
|---|---|
| General | language, `workbench.startup` (emptyState / restoreSession / welcome), `app.hotExit`, `window.restoreWindows`, `update.mode` |
| Editor | fontFamily, fontSize, lineHeight, tabSize, insertSpaces, detectIndentation, wordWrap, wordWrapColumn, lineNumbers, renderWhitespace, highlightCurrentLine, cursorStyle, cursorBlinking, minimap.*, rulers, autoClosingBrackets, zoomScope, undoLimit, emptySelectionClipboard |
| Appearance | workbench.theme, uiScale, tabs.density, toolbar visibility, statusBar visibility, breadcrumbs visibility, activityBar visibility, editor.padding, selection style |
| Files | autoSave, autoSaveDelay, encoding, eol (default for new files), atomicSave, trimTrailingWhitespaceOnSave, insertFinalNewline, exclude, watcherExclude, associations, reloadCleanOnExternalChange, largeFile.* |
| Workspace | explorer.showHidden, explorer.showGitIgnored, explorer.sortOrder, explorer.autoReveal, search.exclude, search.maxResults |
| Git | git.enabled, git.path, git.autoDetect, git.autoFetch (default off), git.pullOnOpen (off), git.autoCommit (off), git.autoPush (off), git.confirmSync, notes.sync.* |
| MCP | mcp.enabled, mcp.autoStart (off, restricted), mcp.port, mcp.permissions defaults, mcp.activityRetentionDays |
| Plugins | plugins.enabled, plugins.directory, plugins.changeEventDebounceMs, per-plugin settings |
| Keyboard | shortcuts editor |
| Advanced | logging.level, recovery.maxSizeMb, performance flags, `workbench.experimental.*` |

## Test Cases

```gherkin
Feature: Settings resolution

  Scenario: Workspace overrides user
    Given User editor.tabSize = 4 and Workspace editor.tabSize = 2
    When a document in the workspace is open
    Then the effective tab size is 2

  Scenario: Language-specific beats general
    Given User editor.wordWrap = "off" and User [markdown] editor.wordWrap = "on"
    Then Markdown documents wrap and JSON documents do not

  Scenario: Invalid value falls through
    Given Workspace editor.fontSize = "huge"
    Then the effective font size is the User value
    And a warning names the invalid setting

  Scenario: Restricted setting in untrusted workspace
    Given Workspace mcp.autoStart = true and the workspace is not trusted
    Then the effective mcp.autoStart is false

Feature: Settings files

  Scenario: Comments are preserved
    Given settings.toml contains a comment line above editor.fontSize
    When the user changes the font size in the Settings UI
    Then the comment is still in settings.toml

  Scenario: Parse error keeps last good values
    Given the user writes invalid TOML into settings.toml externally
    Then the previous settings stay in effect
    And a warning offers to open settings.toml at the error line

  Scenario: Workspace settings file created lazily
    Given an open workspace without ".notoir"
    When the user changes editor.tabSize with scope "Workspace"
    Then ".notoir/settings.toml" is created containing only that setting

Feature: Settings UI

  Scenario: Searchable
    When the user types "minimap" in the settings search
    Then only minimap-related settings are listed

  Scenario: Settings opens as a tab
    When the user presses Ctrl/Cmd+,
    Then a "Settings" tab opens in the active group
```
