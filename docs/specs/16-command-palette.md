# 16 — Command Palette

**Status:** Done — second-step pickers with live theme preview (M3); Command Palette with translated and English fuzzy matching, recently used (persisted), shortcuts per row, show-all toggle with dimmed disabled commands, commands run with the context captured at open (M9). Uses Notoir's own fuzzy matcher instead of nucleo-matcher; plugin badges arrive with plugins (M22)

## Purpose

Define the Command Palette UI: the central entry point that keeps advanced features out of permanent chrome.

## Behaviour

`workbench.commandPalette` (`Mod+Shift+P`, `F1`), or `>` in Quick Open. Overlay at the top centre, width
min(640 px, 60 % window), max 12 visible rows.

```text
┌───────────────────────────────────────────────────────────┐
│ > toggle min                                              │
├───────────────────────────────────────────────────────────┤
│ recently used                                             │
│ View: Toggle Minimap                              ⌥⌘M     │
│ ───────────────────────────────────────────────────────── │
│ View: Toggle Line Numbers                                 │
│ Markdown: Toggle Preview                          ⌘K V    │
└───────────────────────────────────────────────────────────┘
```

- Lists commands with `palette = true` whose `when` evaluates true **at the moment the palette opened** (context captured
  from the previously focused editor, so editor commands remain available).
- Label "Category: Title", both translated; fuzzy matching (`nucleo-matcher`) runs on the translated label **and** the
  English label so users of any language can type English command names.
- Empty query: "recently used" section (last 10 executed via palette, persisted) then all commands alphabetically.
- Each row shows the current keybinding (platform-formatted) and plugin badge for plugin commands.
- Commands with arguments open a second step (e.g. "Settings: Change Theme" → theme list with live preview while
  navigating; Esc reverts). Second-step pickers are generic `PickerProvider`s reused by Change Language, Change
  Language Mode, Change Encoding, Change Line Ending, Switch Branch.
- Enter executes with source `Palette`; Esc closes and restores focus.
- Disabled commands (when false) are not shown; a "Show all commands" toggle (`Mod+Shift+P` again) shows them dimmed with
  the reason tooltip where available.

## Initial Command Set (Phase 1)

File: New File, New Scratchpad, New Window, Open File, Open Workspace, Open Recent, Save, Save As, Save All, Revert,
Close, Close Others, Close All, Close Workspace, Auto Save (picker).
View: Toggle Primary Sidebar, Toggle Secondary Sidebar, Toggle Bottom Panel, Toggle Toolbar, Toggle Status Bar, Toggle
Minimap, Toggle Line Numbers, Toggle Word Wrap, Toggle Whitespace, Split Left/Right/Up/Down, Single Editor, Maximize
Editor Group, Zoom In/Out/Reset, Full Screen, Zen Mode.
Edit, Selection, Navigate: every menu item.
Settings: Open Settings (User / Workspace), Change Theme, Change Language, Keyboard Shortcuts.
Help: Welcome, Keyboard Shortcut Reference, View Logs, About.

Later phases add: Markdown, JSON, Compare, Git, MCP, Plugins commands.

## Test Cases

```gherkin
Feature: Command Palette

  Scenario: Fuzzy match
    When the user opens the Command Palette and types "togmin"
    Then "View: Toggle Minimap" is the first result

  Scenario: English matching in another language
    Given the UI language is Portuguese
    When the user types "toggle minimap"
    Then the Portuguese-labelled minimap toggle command is listed

  Scenario: Editor commands available from palette
    Given the editor had focus before opening the palette
    When the user runs "Edit: Duplicate Line"
    Then the line at the cursor of the previously focused view is duplicated

  Scenario: Theme picker previews live
    When the user runs "Settings: Change Theme" and moves the selection to "Notoir Paper"
    Then the UI previews Notoir Paper
    When the user presses Esc
    Then the previous theme is restored

  Scenario: Recently used first
    Given the user ran "View: Toggle Word Wrap" from the palette
    When the palette is opened with an empty query
    Then "View: Toggle Word Wrap" is listed under "recently used"
```
