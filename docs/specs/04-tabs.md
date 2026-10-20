# 04 — Tabs

**Status:** Partial — tab strip with states, disambiguation, pin, close/close others/to the right/saved/all, context menu, reopen closed tab, overflow list (M5); drag reorder and split/move-group items (M6), compare items (M17); preview tabs (Explorer single-click, `workbench.tabs.previewMode`, default on)

## Purpose

Define tab presentation, state indicators, pinning, ordering, closing, the tab context menu, and reopening closed tabs.

## Tab Anatomy

```text
┌─────────────────────────────┐
│ [file-type icon] name  [●|×] │
└─────────────────────────────┘
```

- Label: file name; disambiguated with the shortest distinguishing parent path suffix when two open tabs share a name
  (`README.md — docs`, `README.md — api`). Scratchpads show their title ("Scratch 1"). Untitled: "Untitled-1".
- Duplicate views of one document in the same group show a subtle `⧉2` suffix.
- Trailing slot: close button (`x` icon) on hover/active; when dirty and not hovered, a filled dot instead.
- Tooltip: full path (or "Scratchpad — not saved to a file"), encoding, and state text.

State indicators use vector icons (lucide), colour **and** shape (spec 21 accessibility):

| State | Indicator |
|---|---|
| Modified (dirty) | Filled dot in trailing slot |
| Externally modified | `refresh-cw` icon before name, label in warning colour |
| Deleted on disk | `file-x` icon, name struck through |
| Read-only | `lock` icon before name |
| Pinned | `pin` icon replaces close button; tab shows icon + name compactly |
| Saving | small spinner in trailing slot |
| Save failed | `triangle-alert` icon in danger colour |

Active tab: `--bg-editor` background, accent top border (2 px). Inactive tabs: `--bg-panel`. Focused group's active tab
uses full accent; unfocused group's active tab uses muted accent.

## Behaviour

- Click activates; middle-click closes; double-click on empty strip area creates a new Untitled file.
- Overflow: strip scrolls horizontally (wheel / trackpad); a `chevron-down` overflow button lists all tabs of the group.
- Pinned tabs stay at the left, are not closed by Close Others / Close to the Right / Close Saved, and survive
  `Close All` only when `workbench.tabs.closeAllKeepsPinned = true` (default false — Close All closes everything after
  confirming dirty ones).
- New tabs open to the right of the active tab (`workbench.tabs.openPosition`: `right` default | `end`).
- Tab density: `workbench.tabs.density = compact | default` (spec 21).
- Opening a file from Explorer single-click opens a **preview tab** (italic label) when
  `workbench.tabs.previewMode = true` (default). Each group has at most one; the next single-click in the same group
  replaces it in its slot. A file already open in any tab is just activated. A preview tab becomes a normal tab when
  the file is double-clicked (or opened with Enter) in the Explorer, its tab is double-clicked, its document is edited
  (undo/redo included, reloads from disk excluded), it is pinned, or "Keep Open" is chosen in the tab context menu.
  Other ways of opening (Quick Open, search, Source Control, breadcrumbs, MCP) open normal tabs. Preview state is kept
  in the session.

## Context Menu

```text
Save
Save As...
──────────────
Close
Close Others
Close Tabs to the Right
Close Saved Tabs
Close All
──────────────
Duplicate View
──────────────
Compare With...
Select for Compare           (becomes "Compare with Selected" when another tab is selected)
──────────────
Split Left
Split Right
Split Up
Split Down
──────────────
Move to Previous Group
Move to Next Group
──────────────
Pin Tab / Unpin Tab
Keep Open                    (only on a preview tab)
──────────────
Copy Path
Copy Relative Path           (only when inside a workspace)
Reveal in File Explorer / Reveal in Finder / Open Containing Folder   (platform label, spec 31)
```

Items not applicable are disabled, not hidden (Copy Path for Scratchpads, Reveal for Untitled). Every item is a command
(spec 05); the menu passes the clicked view id as argument, so it acts on the right-clicked tab, not the active one.

## Reopen Closed Tab

`Ctrl/Cmd+Shift+T` reopens the most recently closed file-backed view (stack of 20 per window) in its original group (or
the active group if gone), restoring cursor and scroll. Closed Scratchpads that were discarded are not reopenable.

## Keyboard

| Action | Windows/Linux | macOS |
|---|---|---|
| Close tab | Ctrl+W | Cmd+W |
| Next/previous tab (MRU) | Ctrl+Tab / Ctrl+Shift+Tab | Ctrl+Tab / Ctrl+Shift+Tab |
| Next/previous tab (strip order) | Ctrl+PageDown / Ctrl+PageUp | Cmd+Option+→ / Cmd+Option+← |
| Go to tab N | Alt+1…9 | Ctrl+1…9 |
| Reopen closed | Ctrl+Shift+T | Cmd+Shift+T |

## Test Cases

```gherkin
Feature: Tab states

  Scenario: Dirty indicator
    Given "notes.md" is open and saved
    When the user types a character
    Then the tab shows the modified dot
    When the user saves
    Then the modified dot disappears

  Scenario: Same-named files are disambiguated
    Given "docs/README.md" and "api/README.md" are open
    Then the tabs read "README.md — docs" and "README.md — api"

Feature: Preview tabs

  Scenario: Single-click replaces the preview tab
    Given a workspace with "a.md" and "b.md" and no open tabs
    When the user clicks "a.md" in the Explorer
    Then "a.md" opens in a preview tab
    When the user clicks "b.md" in the Explorer
    Then "b.md" replaces "a.md" in the preview tab

  Scenario: Double-click keeps the tab open
    Given a workspace with "a.md" and "b.md" and no open tabs
    When the user double-clicks "a.md" in the Explorer
    Then "a.md" opens in a normal tab
    When the user clicks "b.md" in the Explorer
    Then "a.md" stays open and "b.md" opens in a preview tab

  Scenario: Double-clicking a preview tab keeps it open
    Given "a.md" is open in a preview tab
    When the user double-clicks its tab
    And the user clicks "b.md" in the Explorer
    Then "a.md" stays open as a normal tab and "b.md" opens in a preview tab

  Scenario: Editing a preview tab keeps it open
    Given "a.md" is open in a preview tab
    When the user types a character
    And the user clicks "b.md" in the Explorer
    Then "a.md" stays open as a normal tab and "b.md" opens in a preview tab

Feature: Tab context menu

  Scenario: Close Others keeps pinned tabs
    Given tabs "a.md" (pinned), "b.md", "c.md" with "b.md" right-clicked
    When the user chooses "Close Others"
    Then "a.md" and "b.md" remain open

  Scenario: Context menu acts on the clicked tab
    Given "a.md" is active and the user right-clicks "c.md"
    When the user chooses "Close"
    Then "c.md" is closed and "a.md" remains active

  Scenario: Close Saved Tabs skips dirty tabs
    Given tabs "a.md" (dirty) and "b.md" (saved)
    When the user chooses "Close Saved Tabs"
    Then only "a.md" remains

  Scenario: Select for compare
    Given the user chose "Select for Compare" on "old.md"
    When the user right-clicks "new.md" and chooses "Compare with Selected"
    Then a diff view opens comparing "old.md" (left) with "new.md" (right)

Feature: Reopen closed tab

  Scenario: Reopen restores position
    Given "notes.md" is open at line 120 and the user closes it
    When the user presses Ctrl/Cmd+Shift+T
    Then "notes.md" reopens in the same group with the cursor on line 120
```
