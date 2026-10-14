# 14 — View Options & Zen Mode

## Purpose

Define zoom, word wrap, whitespace rendering, current-line highlight, full screen and Zen Mode.

## Zoom

Decision AD-12: zoom applies to **all editor views** by default; a per-view mode is available.

- `editor.zoomScope = "global" | "view"` (default `global`).
- Zoom level is a multiplier on `editor.fontSize`, steps: 50, 67, 75, 80, 90, 100, 110, 125, 150, 175, 200, 250, 300 %.
- Commands `view.zoomIn` (`Mod++` / `Mod+=` / `Mod+wheel up`), `view.zoomOut` (`Mod+-` / `Mod+wheel down`),
  `view.zoomReset` (`Mod+0`).
- Affects editor text, gutter and preview panes (Markdown preview / JSON tree follow the editor zoom). UI chrome uses the
  separate `workbench.uiScale` setting (spec 21), changed in Settings, not by Mod+/-.
- Status bar shows the zoom percentage when ≠ 100 % (always when `statusBar.showZoom = always`); clicking opens a
  small popover with −, %, +, Reset.
- Global zoom persists in the session; per-view zoom persists per view.
- Zoom keeps the line at the top of the viewport anchored.

## Word Wrap

- `editor.wordWrap = "off" | "on" | "column"` (default `on` for Markdown/plaintext via language defaults, `off` for
  code and JSON). `editor.wordWrapColumn` default 100 for `column` mode.
- `view.toggleWordWrap` (`Alt+Z` / `Option+Z`) toggles for the active view (per-view override, spec 03).
- Discoverable in: View menu (checkbox), Command Palette, Settings, and the status bar language-mode popover
  ("Word Wrap" toggle). The toolbar never shows it.
- Wrapped lines break at word boundaries (Unicode line-breaking rules via egui), fall back to character breaks for long
  tokens; continuation rows indent to the original line's indentation (`editor.wrappingIndent = same`).

## Whitespace & Other Rendering Toggles

| Setting | Values | Default | Menu |
|---|---|---|---|
| `editor.renderWhitespace` | none, boundary, selection, trailing, all | selection | View → Appearance → Whitespace |
| `editor.renderControlCharacters` | bool | true | — |
| `editor.highlightCurrentLine` | none, gutter, line, all | all | View → Appearance → Current Line Highlight |
| `editor.lineNumbers` | on, off, relative | on | View → Appearance → Line Numbers |
| `editor.rulers` | list of columns | [] | Settings |
| `editor.indentGuides` | bool | true | Settings |
| `editor.padding` | top/bottom px | 8 | Settings |

Whitespace glyphs: `·` space, `→` tab, `¬` end-of-line (only in `all`), drawn in `--text-whitespace` colour.

## Full Screen

`view.toggleFullScreen` (`F11` Win/Linux, `Ctrl+Cmd+F` macOS) via `ViewportCommand::Fullscreen`.

## Zen Mode

`view.zenMode` (`Mod+K Z`). Distraction-free:

```text
              current document, centred column

          (no sidebars, no panels, no toolbar, no status bar)
```

- Hides: menu bar (revealed on Alt / hover at top on Win/Linux; macOS menu stays OS-managed), toolbar, sidebars,
  bottom panel, status bar, minimap, breadcrumbs; optionally tabs (`zenMode.hideTabs = true`), line numbers
  (`zenMode.hideLineNumbers = true`); enters full screen (`zenMode.fullScreen = true`); centres the text at
  `zenMode.maxWidth` (default 80 ch) for prose.
- Only the active group is shown (others hidden like Maximize).
- `Esc Esc` (twice) or `Mod+K Z` exits; previous layout and panel visibility restored exactly.
- Notifications still appear (toasts) but are quieter (no success toasts).

## Test Cases

```gherkin
Feature: Zoom

  Scenario: Global zoom affects every view
    Given two groups with different documents
    When the user presses Ctrl/Cmd+= twice
    Then both views render at 125 %
    And the status bar shows "125%"

  Scenario: Reset zoom
    When the user presses Ctrl/Cmd+0
    Then all views render at 100 % and the zoom item hides from the status bar

  Scenario: Per-view zoom
    Given editor.zoomScope is "view"
    When the user zooms in on the left view only
    Then the right view stays at 100 %

Feature: Word wrap

  Scenario: Toggle word wrap for one view
    Given a long line and word wrap off
    When the user presses Alt/Option+Z
    Then the line wraps in the active view only

Feature: Zen Mode

  Scenario: Enter and exit restores layout
    Given the Explorer and Outline panels are visible and the window is not full screen
    When the user enters Zen Mode
    Then only the active document's text is visible, centred, in full screen
    When the user presses Esc twice
    Then the Explorer and Outline panels are visible again and the window leaves full screen
```
