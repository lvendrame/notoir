# 05 — Command System & Keybindings

**Status:** Done — command registry, when-expressions, keymap (M1); keyboard dispatch, menu/toolbar wiring (M3); Command Palette, keybindings.toml applied on save or external change, Preferences: Open Keyboard Shortcuts File (M9); shortcuts editor (M10); plugin commands (`plugin.<id>.<command>`, titles from the manifest and plugin locales) and plugin keybindings between user and default bindings, invalid plugin when-expressions rejected into Plugin Output (M22)

## Purpose

Define the single command infrastructure used by menus, toolbar, keyboard shortcuts, the Command Palette, context menus,
status bar items, Lua plugins and MCP. Business logic exists once, in command handlers and the domain services they
call.

```text
Toolbar Save ─┐
Menu Save ────┤
Ctrl/Cmd+S ───┼──► CommandInvocation { id: "file.save", .. } ──► handler ──► DocumentRegistry / save pipeline
Palette ──────┤
Lua notoir.document.save() ─┤
MCP document_save ──────────┘
```

## Command Identity and Metadata

```rust
pub struct CommandId(Cow<'static, str>);   // "file.save", "view.toggleMinimap", "plugin.word-count.count"

pub struct CommandSpec {
    pub id: CommandId,
    pub title_key: TitleSource,            // I18nKey("commands:file.save") | Plugin { plugin, key, fallback }
    pub category: CommandCategory,         // File, Edit, Selection, View, Navigate, Git, Tools, Plugins, Help, Markdown, Json, Compare, Mcp, Settings
    pub icon: Option<Icon>,
    pub args: ArgsSchema,                  // JSON-schema-like description; None for nullary
    pub enablement: When,                  // context expression, e.g. "editorFocus && !readOnly"
    pub palette: bool,                     // show in Command Palette (false for arg-only internal commands)
    pub default_keys: &'static [KeyBindingDef],
    pub handler: Handler,                  // Builtin(fn) | Plugin { plugin_id, callback_ref }
    pub risk: Risk,                        // Safe | Sensitive | Dangerous — consulted by the permission gate (spec 32)
}

pub struct CommandInvocation {
    pub id: CommandId,
    pub args: serde_json::Value,
    pub source: InvocationSource,          // Keyboard | Menu | Toolbar | Palette | ContextMenu | StatusBar | Plugin(PluginId) | Mcp(ClientId) | Internal
}

pub enum CommandOutcome {
    Done,
    Pending(JobId),                        // async work started
    NeedsConfirmation(ConfirmRequest),     // e.g. dirty prompt, permission prompt
    Rejected(UserError),                   // translated, shown as notification
}
```

Builtin handlers have signature `fn(&mut AppCore, &mut HandlerCtx, Args) -> CommandOutcome`, registered per module
(`commands/file.rs`, `commands/view.rs`, …) via `registry.register(spec)`. There is no single giant `match`.

## Naming

`<category>.<camelCaseAction>`, lower-case category. Plugins: `plugin.<plugin-id>.<id>`. MCP tools map onto commands or
domain services but have their own wire names (spec 29). Command ids are stable API: renaming requires an alias entry.

## Context ("When") Expressions

A small boolean language evaluated against a `ContextSnapshot` built once per frame:

Keys: `editorFocus`, `textInputFocus`, `paletteOpen`, `readOnly`, `dirty`, `languageId == 'markdown'`, `viewKind`,
`hasWorkspace`, `isGitRepo`, `groupCount`, `hasSelection`, `multiCursor`, `diffViewFocus`, `explorerFocus`,
`mcpRunning`, `zenMode`, plus `plugin.<id>.<key>` keys set by plugins.

Grammar: identifiers, `!`, `&&`, `||`, `==`, `!=`, string literals, parentheses. Parsed once at registration;
invalid expressions in plugin manifests reject that contribution with an error in Plugin Output.

## Keybindings

```rust
pub struct KeyBinding { pub chord: KeyChord /* 1 or 2 keystrokes */, pub command: CommandId, pub args: Value, pub when: When }
```

- Defaults are declared per command with platform variants. `Mod` = Cmd on macOS, Ctrl elsewhere.
- User overrides: `keybindings.toml` in the config dir (spec 20); entries add, replace, or remove (`command = "-file.save"`).
- Resolution: user bindings (last wins) → plugin bindings → defaults. Among matches, first whose `when` is true wins.
- Two-stroke chords (e.g. `Mod+K Mod+Z` for Zen Mode) show "(Mod+K) was pressed. Waiting for second key…" in the status bar.
- Conflicts are surfaced in the Keyboard Shortcuts editor (Tools → Keyboard Shortcuts) with a warning icon.
- Keys are matched on logical key (`egui::Key`) with modifiers; `Mod++` matches both `=` and numpad `+`.
- When a text field other than the editor has focus (palette, find box, settings), only bindings whose `when` allows
  `textInputFocus` fire; standard text-editing keys are delivered to the field. (Until the first such binding exists,
  the dispatcher simply skips the keymap while a text field has focus; overlays handle their own Enter/Esc/arrows.)

### Initial Default Shortcuts

| Command | Win/Linux | macOS |
|---|---|---|
| `file.newUntitled` | Ctrl+N | Cmd+N |
| `file.newScratchpad` | Ctrl+Alt+N | Cmd+Option+N |
| `file.newWindow` | Ctrl+Shift+N | Cmd+Shift+N |
| `file.open` | Ctrl+O | Cmd+O |
| `file.openWorkspace` | Ctrl+K Ctrl+O | Cmd+K Cmd+O |
| `file.save` | Ctrl+S | Cmd+S |
| `file.saveAs` | Ctrl+Shift+S | Cmd+Shift+S |
| `file.saveAll` | Ctrl+Alt+S | Cmd+Option+S |
| `view.closeTab` | Ctrl+W | Cmd+W |
| `view.reopenClosedTab` | Ctrl+Shift+T | Cmd+Shift+T |
| `edit.undo` / `edit.redo` | Ctrl+Z / Ctrl+Y, Ctrl+Shift+Z | Cmd+Z / Cmd+Shift+Z |
| `edit.find` / `edit.replace` | Ctrl+F / Ctrl+H | Cmd+F / Cmd+Option+F |
| `edit.findNext` / `edit.findPrevious` | F3 / Shift+F3 | Cmd+G / Cmd+Shift+G |
| `search.workspace` | Ctrl+Shift+F | Cmd+Shift+F |
| `navigate.quickOpen` | Ctrl+P | Cmd+P |
| `workbench.commandPalette` | Ctrl+Shift+P | Cmd+Shift+P |
| `help.contextHelp` | F1 | F1 |
| `navigate.goToLine` | Ctrl+G | Ctrl+G |
| `navigate.back` / `forward` | Alt+← / Alt+→ | Ctrl+- / Ctrl+Shift+- |
| `view.zoomIn` / `zoomOut` / `zoomReset` | Ctrl++ / Ctrl+- / Ctrl+0 | Cmd++ / Cmd+- / Cmd+0 |
| `view.toggleWordWrap` | Alt+Z | Option+Z |
| `view.togglePrimarySidebar` | Ctrl+B (not in Markdown editor, see spec 24), Ctrl+K B | Cmd+B (same rule), Cmd+K B |
| `markdown.toggleBold` / `toggleItalic` | Ctrl+B / Ctrl+I (Markdown editor focus) | Cmd+B / Cmd+I |
| `markdown.togglePreview` / `openPreviewToSide` | Ctrl+Shift+V / Ctrl+K V | Cmd+Shift+V / Cmd+K V |
| `view.toggleBottomPanel` | Ctrl+J | Cmd+J |
| `view.toggleSecondarySidebar` | Ctrl+Alt+B | Cmd+Option+B |
| `view.showExplorer` / `view.showOutput` | Ctrl+Shift+E / Ctrl+Shift+U | Cmd+Shift+E / Cmd+Shift+U |
| `view.toggleFullScreen` | F11 | Ctrl+Cmd+F |
| `view.zenMode` | Ctrl+K Z | Cmd+K Z |
| `view.splitRight` | Ctrl+\ | Cmd+\ |
| `edit.duplicateLine` | Ctrl+Shift+D | Cmd+Shift+D |
| `edit.deleteLine` | Ctrl+Shift+K | Cmd+Shift+K |
| `edit.moveLineUp/Down` | Alt+↑ / Alt+↓ | Option+↑ / Option+↓ |
| `bookmarks.toggle` | Ctrl+F2 | Cmd+F2 |
| `bookmarks.next/previous` | F2 / Shift+F2 | F2 / Shift+F2 |
| `edit.toggleComment` (Phase 2) | Ctrl+/ | Cmd+/ |
| `app.settings` | Ctrl+, | Cmd+, |
| `app.quit` | Ctrl+Q | Cmd+Q |

Conflicts with OS-reserved shortcuts (e.g. Cmd+H hide on macOS, Alt+F4) are avoided; macOS Replace uses Cmd+Option+F.

## Command Palette Integration

Every command with `palette = true` and a satisfied `when` appears in the palette as "Category: Title" (translated),
with its current keybinding rendered on the right. See spec 16.

## Toolbar Integration

The toolbar is a list of `ToolbarItem { command, icon, tooltip_key }` read from settings (`workbench.toolbar.items`),
defaulting to the fixed minimal set (spec 18). This enables later customization without new architecture.

## Test Cases

```gherkin
Feature: Single command path

  Scenario: Menu, shortcut, and palette all save through the same handler
    Given a dirty document "notes.md"
    When the user saves via the File menu
    Then the "file.save" handler runs once with source Menu
    When the user edits and presses Ctrl/Cmd+S
    Then the "file.save" handler runs once with source Keyboard

  Scenario: Disabled command is not executed
    Given the active document is read-only
    When the user presses the shortcut for "edit.deleteLine"
    Then the document is unchanged

Feature: Keybinding resolution

  Scenario: User binding overrides default
    Given keybindings.toml binds "Mod+Shift+S" to "file.saveAll"
    When the user presses Ctrl/Cmd+Shift+S in the editor
    Then "file.saveAll" runs instead of "file.saveAs"

  Scenario: Removing a default binding
    Given keybindings.toml contains command "-edit.deleteLine" for "Mod+Shift+K"
    When the user presses Ctrl/Cmd+Shift+K
    Then no command runs

  Scenario: Chord binding
    When the user presses Ctrl/Cmd+K then Z
    Then Zen Mode toggles
    And the status bar showed the pending-chord hint between the keystrokes

  Scenario: Platform modifiers
    Given the app runs on macOS
    Then "file.save" is shown as "⌘S" in menus and the palette
    Given the app runs on Windows
    Then "file.save" is shown as "Ctrl+S"

Feature: When expressions

  Scenario: Invalid plugin when-expression is rejected
    Given a plugin contributes a keybinding with when "editorFocus &&"
    When the plugin loads
    Then that keybinding is not registered
    And Plugin Output shows a parse error naming the plugin
```
