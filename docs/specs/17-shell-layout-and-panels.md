# 17 — Shell Layout, Panels, Empty State & First Run

**Status:** Partial — panel model, activity bar, sidebars, bottom panel, empty state (M0, M3); Scratchpad-on-type (M5), first run (M10), panel drag-docking (later)

## Purpose

Define the window layout, the generic panel system (primary sidebar, secondary sidebar, bottom panel), the empty state
and the first-run experience.

## Window Layout

```text
┌────────────────────────────────────────────────────────────────────────────┐
│ File  Edit  Selection  View  Navigate  Git  Tools  Plugins  Help           │  menu bar (spec 18)
├────────────────────────────────────────────────────────────────────────────┤
│ ☰  +  Open  Save    ↶ ↷    Find    [ contextual controls ]    Compare      │  toolbar (spec 18)
├──┬───────────┬──────────────────────────────────────────────┬──────────────┤
│▣ │ EXPLORER  │ README.md   ● notes.md    config.json         │ OUTLINE      │
│⌕ │           ├──────────────────────────────────────────────┤              │
│⎇ │ ▾ notes   │ docs > README.md > Architecture              │              │  breadcrumbs (spec 26)
│≡ │   ▾ docs  ├─────┬───────────────────────────────┬────────┤              │
│⧉ │     …     │  42 │  EDITOR GROUPS (spec 03)      │ mini   │              │
│  │           │     │                               │ map    │              │
├──┴───────────┴─────┴───────────────────────────────┴────────┴──────────────┤
│ OUTPUT | MCP ACTIVITY | GIT                                    (bottom, hidden by default)
├────────────────────────────────────────────────────────────────────────────┤
│ ⎇ main ↑1 ↓0 │ Ln 44, Col 18 │ Spaces:4 │ UTF-8 │ LF │ Markdown │ 100% │  status bar (spec 19)
└────────────────────────────────────────────────────────────────────────────┘
```

Default for a fresh install: menu bar, toolbar, activity bar + Explorer (empty, "No folder opened"), editor area with
empty state, status bar. Secondary sidebar and bottom panel hidden.

## Panel Model

```rust
pub enum PanelId { Explorer, Search, SourceControl, Outline, Plugins, MarkdownPreview, JsonTree, McpActivity, Output, Problems, GitOutput, PluginOutput, Plugin(PluginPanelId) }

pub enum PanelArea { Primary, Secondary, Bottom }

pub struct AreaState {
    pub panels: Vec<PanelId>,      // order in the area's switcher
    pub active: Option<PanelId>,
    pub visible: bool,
    pub size: f32,                 // width for sidebars, height for bottom (logical px)
    pub maximized: bool,           // bottom panel only: takes the editor area
}

pub struct PanelLayout {
    pub primary_side: Side,        // Left (default) | Right
    pub areas: EnumMap<PanelArea, AreaState>,
    pub activity_bar_visible: bool,
}
```

- Each panel has a `PanelDescriptor { id, title_key, icon, default_area, allowed_areas, when }`.
- One panel visible per area at a time; areas switch panels via the activity bar (primary), a compact icon strip at the
  top of the secondary sidebar, or tabs (bottom).
- Default areas: Explorer, Search, Source Control, Plugins → Primary; Outline, Markdown Preview, JSON Tree → Secondary;
  Output, MCP Activity, Git Output, Plugin Output, Problems → Bottom. Outline and MCP Activity may live in any area.
- The domain model is independent of the rendering technique. Phase 1 renders with `egui::SidePanel` /
  `TopBottomPanel` (resizable); drag-and-drop docking of panels is a later enhancement that only changes the renderer
  and adds `MovePanel` commands that already exist.

## Panel Header

```text
EXPLORER                                        [actions…]  ⋯  ×
```

- Title: 11 px uppercase section-label style (spec 21).
- Panel-specific actions appear on hover/focus (icon buttons).
- `⋯` menu: panel-specific items, then `Move to Primary Sidebar`, `Move to Secondary Sidebar`, `Move to Bottom Panel`
  (only allowed areas), `Maximize Panel` (bottom only), `Hide`.
- `×` hides the panel's area (not the panel).

## Activity Bar

Narrow vertical icon strip (40 px) on the primary side: Explorer, Search, Source Control (only when a repo is detected
or always via `workbench.activityBar.showScm`), any panel the user moved into the primary area (e.g. Outline), Plugins (only when ≥ 1 plugin is
installed or `plugins.showInActivityBar`). Clicking the active icon collapses the primary sidebar. Badges: Source
Control change count, Search result count when active. Hideable (`workbench.activityBar.visible`); when hidden, panels
are reached via View → Panels and the palette.

## Contextual Panels

- Markdown Preview panel in the secondary sidebar is an alternative to in-editor split (spec 24) — it follows the
  active Markdown document.
- JSON Tree panel similarly follows the active JSON document (spec 25).
- When the active document type has no content for a contextual panel, the panel shows a short neutral message ("Open a
  Markdown file to see its preview").

## Bottom Panel

Tabs: Output (channel selector: Notoir, Git, Plugins, MCP), MCP Activity, Git Output, Plugin Output, Problems (reserved).
Hidden by default; never auto-opens except on explicit errors the user asks to see ("Show Details" in a notification).
Output lines: `HH:MM:SS  message`, monospace, virtualized, max 10,000 lines per channel, copyable.

## Sizes & Persistence

Sidebar min 170 px / default 260 px; bottom min 100 px / default 220 px; editor area keeps min 320 px width. All sizes,
visibility, panel placement and active panels persist per workspace (spec 23); a global default applies to new
workspaces.

## Empty State

When the editor area (last group) has no tabs:

```text
                       NOTOIR

                   Start writing.

                + New Scratchpad          Ctrl+Alt+N
                  Open File…              Ctrl+O
                  Open Workspace…         Ctrl+K Ctrl+O

                Recent
                📌 Personal Notes
                   StringArtIt
                   Documentation

          Ctrl+Shift+P — Command Palette
```

- Wordmark in `--text-tertiary`, letter-spaced; no logo art, no cards, no dashboard tiles.
- Recent shows up to 5 pinned + recent workspaces and files, pin icons (lucide `pin`), click to open, right-click to
  pin/unpin/remove.
- Shortcuts shown with platform formatting.
- Clicking anywhere in blank space and typing creates a new Scratchpad and inserts the typed text
  (`workbench.emptyState.typeToScratch = true`).
- Secondary groups that are empty show only a faint "Drop a file here" hint.

## First-Run Experience

Shown once on first launch (no settings file exists), as a single centred page in the editor area (not a modal), fully
skippable:

```text
Welcome to Notoir                                        [Skip]

Language     [ English ▾ ]          (pre-selected from OS locale, spec 22)

Appearance
  ○ Notoir Paper      ● Notoir Midnight      ○ Dracula     (live preview swatches)

Git          [Configure]      — opens Settings › Git
MCP          [Learn More]     — opens documentation page
Plugins      [Explore]        — opens Plugin Manager

                                                    [Start writing]
```

Choices apply immediately; Skip/Start writing records `firstRun.completed = true` and opens the empty state. Help →
Welcome reopens it.

## Test Cases

```gherkin
Feature: Panel system

  Scenario: Move Outline to the bottom panel
    Given the Outline panel is in the secondary sidebar
    When the user chooses "Move to Bottom Panel" from its ⋯ menu
    Then Outline is a tab in the bottom panel
    And the secondary sidebar hides if it has no other panels

  Scenario: Primary sidebar on the right
    When the user sets the primary sidebar side to Right
    Then the activity bar and Explorer are drawn on the right edge
    And the secondary sidebar moves to the left edge

  Scenario: Clicking the active activity icon collapses the sidebar
    Given the Explorer is visible
    When the user clicks the Explorer activity icon
    Then the primary sidebar collapses

  Scenario: Bottom panel hidden by default
    Given a fresh install
    Then the bottom panel is not visible

Feature: Empty state

  Scenario: Type to create a scratchpad
    Given no tabs are open
    When the user clicks the empty editor area and types "hello"
    Then a new Scratchpad opens containing "hello"

  Scenario: Pinned recent shown
    Given "Personal Notes" is a pinned recent workspace
    Then the empty state's Recent list starts with "Personal Notes" with a pin icon

Feature: First run

  Scenario: Skip immediately
    Given Notoir launches for the first time
    When the user clicks "Skip"
    Then the empty state is shown with Notoir Midnight
    And the welcome page is not shown on the next launch

  Scenario: Theme choice previews and applies
    When the user selects "Notoir Paper" on the welcome page
    Then the whole UI switches to the light theme immediately
```
