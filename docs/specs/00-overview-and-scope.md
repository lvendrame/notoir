# 00 — Overview & Scope

**Status:** Partial — product scope fixed; scenarios need the app shell (M3) and file opening (M5)

## Purpose

Define what Notoir is, the product principles every other spec must respect, the phase boundaries, and the UX
acceptance criteria. This document is the entry point for all other specs in `docs/specs/`.

## Product Overview

Notoir is a fast, polished, cross-platform native desktop text editor and workspace application written in Rust
(`egui` / `eframe`). It runs on Windows 10/11, macOS (Apple Silicon and Intel) and mainstream Linux desktops.

It is a **hybrid editor**, combining:

- a fast Notepad-like editor,
- a lightweight workspace editor,
- a Markdown-oriented notes application,
- a Git-backed document workspace,
- an MCP-accessible text/document environment.

Conceptual position:

> Plain files + Git + MCP + native editor.

It is **not** an IDE, a database-driven notes app, an AI chat app, or a proprietary note format. Plain files on disk are
always the source of truth.

## Product Principle

> Features should increase capability without increasing permanent visual complexity.

Consequences, normative for every spec:

1. The centre of the window always belongs to the editor. Everything else is collapsible, movable, hideable,
   keyboard-accessible, and reachable from the Command Palette.
2. A fresh install shows only the editor and (optionally) the Explorer sidebar.
3. Advanced features (Git, MCP, plugins, compare, structured views) never add permanent chrome by default; they are
   reached through menus, context menus, the Command Palette, contextual toolbar controls, and the status bar.
4. Opening a workspace is optional. A single file opens and is editable immediately.

## Priorities

When requirements conflict, resolve in this order:

1. Preventing data loss
2. Usability
3. Predictable behaviour
4. Performance (native responsiveness, low startup overhead)
5. Extensibility
6. Implementation convenience

## Related Specs

| Area | Spec |
|---|---|
| Architecture, crates, ownership, async | [01-architecture.md](./01-architecture.md) |
| Document model, encodings, large files | [02-document-model.md](./02-document-model.md) |
| Editor views, groups, splits | [03-editor-views-groups-and-splits.md](./03-editor-views-groups-and-splits.md) |
| Tabs | [04-tabs.md](./04-tabs.md) |
| Command system & keybindings | [05-command-system-and-keybindings.md](./05-command-system-and-keybindings.md) |
| Event system | [06-event-system.md](./06-event-system.md) |
| Workspace & Explorer | [07-workspace-and-explorer.md](./07-workspace-and-explorer.md) |
| Opening files, Scratchpads, Recent | [08-opening-files-scratchpads-and-recent.md](./08-opening-files-scratchpads-and-recent.md) |
| Save, autosave, recovery | [09-save-autosave-and-recovery.md](./09-save-autosave-and-recovery.md) |
| External changes & file watching | [10-external-changes-and-file-watching.md](./10-external-changes-and-file-watching.md) |
| Text editing core | [11-text-editing.md](./11-text-editing.md) |
| Gutter, bookmarks, folding | [12-gutter-bookmarks-and-folding.md](./12-gutter-bookmarks-and-folding.md) |
| Minimap | [13-minimap.md](./13-minimap.md) |
| View options: zoom, wrap, whitespace, Zen | [14-view-options-and-zen-mode.md](./14-view-options-and-zen-mode.md) |
| Search, Quick Open, navigation | [15-search-quick-open-and-navigation.md](./15-search-quick-open-and-navigation.md) |
| Command Palette | [16-command-palette.md](./16-command-palette.md) |
| Shell layout, panels, empty state, first run | [17-shell-layout-and-panels.md](./17-shell-layout-and-panels.md) |
| Menus, toolbar, context menus | [18-menus-toolbar-and-context-menus.md](./18-menus-toolbar-and-context-menus.md) |
| Status bar & notifications | [19-status-bar-and-notifications.md](./19-status-bar-and-notifications.md) |
| Settings | [20-settings.md](./20-settings.md) |
| Themes, design system, accessibility | [21-themes-and-design-system.md](./21-themes-and-design-system.md) |
| Internationalization | [22-internationalization.md](./22-internationalization.md) |
| Session & state persistence | [23-session-persistence.md](./23-session-persistence.md) |
| Markdown | [24-markdown.md](./24-markdown.md) |
| JSON | [25-json.md](./25-json.md) |
| Syntax, outline, breadcrumbs | [26-syntax-outline-and-breadcrumbs.md](./26-syntax-outline-and-breadcrumbs.md) |
| Compare / diff | [27-compare.md](./27-compare.md) |
| Git & Source Control | [28-git.md](./28-git.md) |
| MCP server | [29-mcp.md](./29-mcp.md) |
| Lua plugins | [30-lua-plugins.md](./30-lua-plugins.md) |
| Platform, packaging, updates | [31-platform-packaging-and-updates.md](./31-platform-packaging-and-updates.md) |
| Permissions & security | [32-permissions-and-security.md](./32-permissions-and-security.md) |
| Testing & CI | [33-testing-and-ci.md](./33-testing-and-ci.md) |
| Future note features | [34-future-note-features.md](./34-future-note-features.md) |
| Logging & error handling | [35-logging-and-errors.md](./35-logging-and-errors.md) |

## Phases

| Phase | Theme | Content |
|---|---|---|
| 1 | Core editor foundation | Shell, themes, i18n, editor, files, Scratchpads, tabs, splits, duplicate views, Explorer, recent, line numbers, wrap, zoom, undo/redo, find/replace, settings, commands + palette foundation, session, recovery, autosave, external-change detection, status bar, minimap. Must already be a usable daily editor. |
| 2 | Structured documents | Markdown preview/split, JSON tree/split, syntax highlighting, outline, breadcrumbs, folding, bookmarks, Quick Open, workspace search, polished palette, compare, navigation history. |
| 3 | Git | Detection, Source Control panel, status, decorations, diff, commit, fetch/pull/push/sync, branches, compare with HEAD, file history. |
| 4 | MCP | Server, workspace/document/editor/settings tools, read-only Git tools, permissions, connections and activity UI. |
| 5 | Lua plugins | Runtime, discovery, manifest, permissions, lifecycle, commands, menus, events, panels, formatters, manager. |
| 6 | Advanced | Wiki links, backlinks, tags, indexed search, templates, enhanced note sync, custom themes, marketplace, Markdown export, advanced JSON editing, merge UI. |

Concrete milestones live in [`docs/plan/orchestrator.md`](../plan/orchestrator.md).

### Phase discipline

For each milestone: define architecture → data models → commands → UI state → implement → tests → verify on all three
OSes (CI) → refactor → continue. No speculative abstractions without a planned feature, but the following decisions are
fixed from day one because they are expensive to retrofit:

- Document vs EditorView separation (spec 02, 03)
- Command system (spec 05)
- True editor groups (spec 03)
- Workspace model (spec 07)
- Recovery architecture (spec 09)
- Async worker architecture (spec 01)
- Plugin and MCP permission models (spec 32)
- Generic panel system (spec 17)

## Explicitly Out of Scope

Terminal emulator, debugger, build system, project compilation, language-server integration as a core feature, dozens of
permanent tool icons, mandatory accounts, cloud-first storage, proprietary document database as source of truth,
permanent AI chat UI, unrestricted MCP filesystem access, unrestricted plugin system access, hidden automatic Git
pushes, silently overwriting externally modified files.

## Brand

- Application name: **Notoir**. Never translated.
- Built-in themes: **Notoir Midnight** (dark, default), **Notoir Paper** (light), **Dracula**.
- Taglines (marketing/about only): "Write. Connect. Recall." / "Notes that connect."
- Branding inside the app is restrained: the empty state wordmark and About dialog only.

## UX Acceptance Criteria

The product succeeds when:

1. A first-time user can open a file and start typing without explanation.
2. Opening a folder as a workspace remains optional.
3. The interface does not resemble a crowded IDE by default.
4. All advanced features are discoverable through menus or the Command Palette.
5. The centre editor receives the majority of screen space.
6. Side and bottom panels can all be hidden.
7. Users can recover unsaved work after a crash.
8. Split editors behave predictably.
9. Duplicate views share the same underlying document.
10. Git operations never freeze the UI.
11. MCP operations never bypass the permission model.
12. Lua plugins cannot silently obtain dangerous privileges.
13. Settings clearly distinguish User and Workspace scopes.
14. Modifications by external applications are handled safely.
15. Notoir behaves naturally on Windows, macOS and Linux.

## Test Cases

```gherkin
Feature: Minimal first impression

  Scenario: Fresh install shows only the editor surface
    Given Notoir has never been launched on this machine
    When the user skips the first-run screen
    Then the window shows the menu bar, toolbar, empty state, and status bar
    And no side panel other than the (empty) Explorer sidebar is visible
    And the bottom panel is hidden

  Scenario: Open a file and type without a workspace
    Given Notoir is running with no workspace open
    When the user opens "notes.txt" via File → Open File...
    Then the file is shown in a tab in the editor area
    And typing inserts text immediately at the cursor

  Scenario: Every panel can be hidden
    Given the primary sidebar, secondary sidebar, and bottom panel are visible
    When the user toggles each of them off from the View menu
    Then only the editor groups, toolbar, and status bar remain
```
