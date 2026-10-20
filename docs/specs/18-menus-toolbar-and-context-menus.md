# 18 — Menus, Toolbar & Context Menus

**Status:** Partial — data-driven menu bar, minimal toolbar (M3); items appear with their commands; contextual controls (M13, M14, M17); Help › Documentation, Keyboard Shortcut Reference and context help on F1

## Purpose

Define the main menu structure, the minimal toolbar with contextual controls, and context-menu conventions. Every item
is a command (spec 05); menus are declarative data (`MenuItem { command, args, when, group, order }`) so plugins can
contribute.

## Menu Bar Conventions

- In-window menu bar on all platforms (AD-11). On macOS the first menu is **Notoir** (About Notoir, Settings… `Cmd+,`,
  Hide/Quit are OS-level where available) and "Exit" is "Quit Notoir"; Settings lives under Notoir instead of Tools.
- Items show translated labels, platform-formatted shortcuts, checkmarks for toggles, `›` for submenus, `…` when the
  item asks for more input.
- Disabled items are shown disabled (not hidden) unless the whole feature is unavailable (e.g. Git menu items in a
  non-repo show "Initialize Repository" and "Clone Repository…" enabled, the rest disabled).
- Menus are declared in full (this spec) but an item whose command is not registered in the build is hidden;
  separators are normalized around hidden items. A top-level menu with no implemented items shows one disabled
  "Coming soon" entry, so the top-level set never changes between releases.
- Separators group logically; no more than ~7 items per group.

## File

```text
New File                          Mod+N
New Scratchpad                    Mod+Alt+N
New Window                        Mod+Shift+N
─────
Open File…                        Mod+O
Open Workspace…                   Mod+K Mod+O
Open Recent ›                     (spec 08)
─────
Save                              Mod+S
Save As…                          Mod+Shift+S
Save All                          Mod+Alt+S
Auto Save ›   ◉ Off  ○ After Delay  ○ On Focus Change  ○ On Window Focus Loss
Revert File
─────
Close                             Mod+W
Close Others
Close All
Close Workspace
─────
Properties…                       (path, size, encoding, line endings, modified time, language)
─────
Exit                              (Quit Notoir on macOS, in app menu)
```

## Edit

```text
Undo / Redo
─────
Cut / Copy / Paste / Paste as Plain Text
─────
Find / Replace / Find Next / Find Previous
Find in Workspace                  Mod+Shift+F
─────
Select All
─────
Duplicate Line / Delete Line / Move Line Up / Move Line Down
Sort Lines
Trim Trailing Whitespace
Convert Case ›   UPPERCASE · lowercase · Title Case
─────
Toggle Read-Only
```

## Selection

```text
Select All
Expand Selection / Shrink Selection            (Phase 2)
─────
Add Cursor Above / Add Cursor Below            (Phase 2)
Select Next Occurrence / Select All Occurrences (Phase 2)
Column Selection Mode                          (Phase 2)
```

Phase-2 items appear once their commands exist (see the rule below).

## View

```text
Command Palette…                   Mod+Shift+P
Quick Open…                        Mod+P
─────
Panels ›        Explorer · Search · Outline · Source Control · Markdown Preview · JSON Tree · MCP · Plugins · Output
Appearance ›    Toolbar · Status Bar · Activity Bar · Breadcrumbs · Line Numbers · Minimap · Whitespace ·
                Current Line Highlight · Primary Sidebar Position (Left/Right)
Editor Layout › Split Left · Split Right · Split Up · Split Down · Single Editor · Maximize Editor Group
─────
Word Wrap                          Alt+Z
Fold / Unfold / Fold All / Unfold All
─────
Zoom In / Zoom Out / Reset Zoom
─────
Full Screen
Zen Mode                           Mod+K Z
```

## Navigate

Quick Open…, Go to Line…, Go to Symbol…, ───, Next Tab, Previous Tab, Next Editor Group, Previous Editor Group, ───,
Back, Forward, ───, Next Change, Previous Change, Next Bookmark, Previous Bookmark, Next Search Result, Previous Search
Result.

## Git (spec 28)

Source Control, Status, ───, Commit…, Commit All…, ───, Pull, Push, Sync, Fetch, ───, Branches › (Current Branch: main,
Switch Branch…, Create Branch…), ───, View File Changes, Compare with HEAD, File History, ───, Initialize Repository,
Clone Repository…, ───, Sync Notes (when enabled in settings).

## Tools

```text
Compare Files…
Format Document                    Shift+Alt+F
─────
MCP ›   Start Server · Stop Server · Connections · Copy Setup Command › · Permissions · Activity · Settings
─────
Reload Window
Reload Plugins
─────
Settings                           Mod+,   (in app menu on macOS)
Keyboard Shortcuts
Change Theme…
Change Language…
```

There is no top-level MCP menu.

## Plugins

Plugin Manager, Installed Plugins, Reload Plugins, ───, Open Plugins Folder, Plugin Developer Tools, ───,
[plugin-contributed items, grouped by plugin].

## Help

Welcome, Documentation, Keyboard Shortcut Reference, ───, Report Issue, View Logs, Recover Unsaved Documents…, ───,
Check for Updates (only when updater is enabled, spec 31), ───, About Notoir (in app menu on macOS).

**Documentation** (`help.documentation`) opens the published user guide (https://lvendrame.github.io/notoir/) in the
browser; **Keyboard Shortcut Reference** (`help.keyboardReference`) opens its shortcut reference page.
**F1** (`help.contextHelp`, "Help for Current Context") opens the guide page for what the user is working on, first
match wins: the focused Explorer (Workspaces); the active tab's kind (Settings, or Keyboard Shortcuts on that
category; Welcome → First steps; Compare; Plugin Manager; Markdown preview/split; JSON tree/split); the active text
editor (find widget open → Find and search; Markdown; JSON; a focused scratchpad or untitled file → Files; any other
focused editor → Editing); the visible primary sidebar panel (Explorer, Search, Source Control, Outline, Plugins,
Backlinks/Tags, MCP Activity). Otherwise the guide's start page opens.

## Toolbar

Fixed minimal set (customizable later via `workbench.toolbar.items`, spec 05):

```text
[☰] [+] [Open] [Save]   [↶] [↷]   [Find]      [ contextual controls ]      [Compare]
```

| Item | Command | Icon (lucide) |
|---|---|---|
| ☰ | `view.togglePrimarySidebar` | `panel-left` |
| + | `file.newMenu` — click: New Scratchpad (`toolbar.newAction = scratchpad` default, or `file`); dropdown arrow: New File / New Scratchpad / New Window | `plus` |
| Open | `file.openMenu` — click: Open File; dropdown: Open File / Open Workspace / Recent | `folder-open` |
| Save | `file.save` | `save` |
| Undo / Redo | `edit.undo` / `edit.redo` | `undo-2` / `redo-2` |
| Find | `edit.find` (in editor) / `search.workspace` (Shift-click) | `search` |
| Compare | `compare.open` | `git-compare` |

- Icon-only buttons with tooltips (label + shortcut), 28 px height, 16 px icons.
- Disabled state when not applicable (e.g. Save is disabled when the active document is clean and file-backed; Undo
  when there is nothing to undo).
- `View → Appearance → Toolbar` hides the toolbar entirely; contextual controls then move to the editor group header's
  right side.
- No permanent Git, MCP, plugin, theme, wrap, line-number or formatting buttons.

### Contextual Controls

Rendered centred in the toolbar based on the active view:

| Active view | Controls |
|---|---|
| Markdown | Segmented `Edit │ Split │ Preview` |
| JSON | Segmented `Text │ Split │ Tree` + `Format` icon button |
| Diff view | `Previous Change`, `Next Change`, `Ignore Whitespace` toggle, `Swap Sides` |
| Plain text / code | none |
| Plugin-provided | Plugins may register contextual controls for a language id (Phase 5) |

## Context Menus (general rules)

Context menus are the main way to reach secondary actions without permanent buttons. Required in: tabs (spec 04),
Explorer files/folders (spec 07), editor text (spec 11), gutter (spec 12), minimap (spec 13), Source Control rows
(spec 28), panels (spec 17), status bar items (spec 19), search results (spec 15).

Rules: logically grouped with separators; destructive items last in their group and in danger colour; act on the
right-clicked target; keyboard-accessible (`Shift+F10` / Menu key opens the context menu for the focused element);
plugin items appended in a separate group.

## Test Cases

```gherkin
Feature: Menus

  Scenario: No MCP top-level menu
    Then the menu bar has exactly File, Edit, Selection, View, Navigate, Git, Tools, Plugins, Help (plus Notoir on macOS)

  Scenario: macOS app menu
    Given the app runs on macOS
    Then "Settings…" is in the Notoir menu with shortcut ⌘,
    And the File menu has no "Exit" item

  Scenario: Git menu in non-repository
    Given the workspace is not a Git repository
    Then "Initialize Repository" and "Clone Repository…" are enabled
    And "Commit…" is disabled

Feature: Help

  Scenario: Documentation opens the user guide
    When the user chooses Help › Documentation
    Then the browser opens "https://lvendrame.github.io/notoir/"

  Scenario: F1 opens the guide page for the focused feature
    Given the Explorer has keyboard focus
    When the user presses F1
    Then the browser opens "https://lvendrame.github.io/notoir/workspaces.html"
    When a Markdown document is active and the editor has focus
    And the user presses F1
    Then the browser opens "https://lvendrame.github.io/notoir/markdown.html"

Feature: Toolbar

  Scenario: Contextual controls for Markdown
    When a Markdown document becomes active
    Then the toolbar shows the Edit | Split | Preview control
    When a plain text document becomes active
    Then no contextual controls are shown

  Scenario: Hidden toolbar keeps contextual controls reachable
    Given the toolbar is hidden
    And a JSON document is active
    Then the Text | Split | Tree control is shown in the editor group header
```
