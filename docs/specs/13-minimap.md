# 13 — Minimap

## Purpose

Define the optional structural minimap: rendering, interaction, markers, settings and performance limits.

## Rendering

Structural, not legible text. Each display row maps to a horizontal block whose x-extent spans leading indentation to
last non-whitespace column, scaled to the minimap width:

```text
████████████████
  ██████████
  ███████████████

████████
██████████████████
```

- Row height: 2 physical px per buffer line by default (`editor.minimap.scale = 1 | 2 | 3`).
- Colour: blocks use the line's dominant syntax token colour at reduced opacity when syntax highlighting is active
  (Phase 2), otherwise `--text-tertiary`. Markdown headings render as stronger blocks.
- When the document is taller than the minimap at the chosen scale, the minimap scrolls proportionally with the editor
  (like VS Code) so the viewport indicator stays visible.
- Rendering is cached as an egui texture built off-thread from a rope snapshot (rows → RGBA bitmap), invalidated per
  changed line range; only the visible minimap window is uploaded. Target: rebuild ≤ 4 ms for 100k lines incremental.

## Overlays (drawn on top of the cached texture)

| Overlay | Setting | Default |
|---|---|---|
| Viewport highlight (translucent rect, stronger on hover/drag) | `editor.minimap.showViewport` | on |
| Search matches (accent ticks, full width) | `editor.minimap.showSearchMarks` | on |
| Git changes (left-edge bars: added / modified / deleted) | `editor.minimap.showGitMarks` | on |
| Bookmarks (small right-edge squares) | `editor.minimap.showBookmarks` | on |
| Selections / multi-cursor (accent outline) | `editor.minimap.showSelection` | on |

## Interaction

- Click: centres the editor viewport on that line (with `editor.minimap.clickBehavior = jump | scroll-page`; default
  `jump`).
- Drag the viewport rect: scrolls continuously; dragging elsewhere starts a jump-then-drag.
- Mouse wheel over the minimap scrolls the editor normally.
- Hover shows a tooltip with the line number range under the pointer (no text preview in v1).
- Context menu: Hide Minimap, Position › Left/Right, Scale ›, toggles for each overlay.

## Settings

```toml
[editor.minimap]
enabled = true            # View → Appearance → Minimap
side = "right"            # "right" | "left"
width = 80                # logical px, 40–200
scale = 2
opacity = 0.6             # block opacity 0.2–1.0
showViewport = true
showSearchMarks = true
showGitMarks = true
showBookmarks = true
showSelection = true
```

Disabled automatically in large-file mode (spec 02) and for views narrower than 400 px.

## Test Cases

```gherkin
Feature: Minimap

  Scenario: Click to jump
    Given a 2,000-line document with the minimap visible
    When the user clicks the minimap at the position for line 1,500
    Then the editor viewport is centred on line 1,500

  Scenario: Drag viewport
    When the user drags the minimap viewport rectangle downwards
    Then the editor scrolls continuously and the rectangle follows the pointer

  Scenario: Search marks
    Given the find widget shows 3 matches
    Then 3 search ticks are drawn in the minimap at their line positions

  Scenario: Position left
    When the user sets the minimap side to "left"
    Then the minimap is drawn at the left edge of the view, before the gutter

  Scenario: Hidden in large-file mode
    Given a document in large-file mode
    Then no minimap is drawn for its views
```
