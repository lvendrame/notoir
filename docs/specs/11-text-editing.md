# 11 — Text Editing

**Status:** Partial — transactions, undo coalescing, selections (M1); editor widget: typing, grapheme/word editing, Enter with indent and Markdown lists, Tab indent/outdent, clipboard, select all, mouse selection, IME, read-only (M4); line operations (duplicate, delete, move, sort, trim, case, insert line, join, toggle read-only), find/replace widget (case, whole word, regex with captures, find in selection, wrap hint, Replace All as one step), Go to Line, editor context menu (M9). multi-cursor (Add Cursor Above/Below, Select Next/All Occurrences, Alt+click, Esc collapses, per-cursor paste), column selection (Shift+Alt drag, column mode), Expand/Shrink Selection (M17). Later: toggle comment and case-preserving replace (Phase 2), Paste as Plain Text, worker-thread search for documents over 5 MiB

## Purpose

Define the editor widget's editing behaviour: input, cursor movement, selection, clipboard, undo/redo, in-document
find/replace, line operations, case conversion, indentation, and the editor context menu. Rendering structure (gutter,
minimap) is in specs 12–13; view options in spec 14.

## Editor Widget Architecture (AD-1)

```text
EditorWidget (apps/notoir/editor_widget)
 ├── DisplayMap        (notoir-editor) buffer lines ↔ display rows (wrap + folds)
 ├── LineLayoutCache   per-line egui Galley keyed by (line hash, wrap width, font id, zoom, highlight revision)
 ├── Painter           background, current line, selections, text, whitespace, carets, IME preedit, search marks
 ├── InputHandler      egui events → EditIntent → EditOps / commands
 └── AccessKit node    exposes visible text, caret and selection for screen readers
```

- Only visible rows (+ 1 screen overscan) are laid out each frame. Scrolling uses `DisplayMap` row counts; wrapped
  heights outside the viewport are estimated and corrected as they become visible (scroll anchoring keeps the top
  visible line stable).
- Monospace default font (JetBrains Mono, configurable; proportional fonts supported but column-based features assume
  monospace). Word wrap is computed by `notoir-editor::wrap` on the column grid (tabs to tab stops, East-Asian wide
  characters as two columns) and each display row is painted on that grid, so row math and pixels always agree.
- Caret blinks (configurable `editor.cursorBlinking = blink | solid`), styles `line | block | underline`.
- IME: preedit text rendered inline with underline; candidate window positioned via `egui::output.ime` cursor rect.
- Tabs render at `editor.tabSize` columns; grapheme clusters (emoji, combining marks) move and delete as one unit;
  double-width CJK characters respected for column computations.
- Bidirectional text and complex shaping beyond egui's capabilities are documented limitations in v1.

## Transactions and Undo

- Every edit is a `Transaction { changes: ChangeSet, selections_before, selections_after, origin, timestamp }`.
- Undo/redo per document (AD-3). Typing coalesces into one undo step until: 1 s pause, cursor jump, a non-whitespace
  character typed after whitespace (word boundary), a newline, or a non-typing command. Paste, line ops, replace-all, format and reload are single steps.
- Undo restores selections into the view that performs the undo, scrolling to reveal the change.
- History limit: `editor.undoLimit` (default 10,000 steps or 256 MiB of change data, whichever first).
- Undo history is not persisted across restarts in Phase 1 (recovered documents start a fresh history).

## Cursor and Selection

- Standard platform movement: arrows, Home/End (smart Home toggles first non-whitespace/column 0), word-wise
  (Ctrl/Alt), document start/end, PageUp/PageDown, with Shift to extend.
- Mouse: click, drag, double-click word, triple-click line, Shift-click extend, Alt/Option-drag column selection
  (Phase 2), drag-and-drop of selected text (move; Alt/Option copy), auto-scroll while dragging near edges.
- Multi-cursor (Phase 2 per Selection menu): Add Cursor Above/Below, Select Next Occurrence (`Mod+D`), Select All
  Occurrences (`Mod+Shift+L`), Alt/Option+click to add caret, `Esc` collapses to primary. The `Selections` model supports
  N ranges from Phase 1 so this does not require a refactor.
- Expand/Shrink Selection (Phase 2): word → string/bracket contents → line → paragraph/block → document.

## Clipboard

- Cut/Copy with no selection operates on the whole line (`editor.emptySelectionClipboard = true`).
- Paste normalizes line endings to `\n`. Multi-cursor paste distributes lines if line count equals cursor count.
- Paste as Plain Text (`edit.pastePlain`): identical for plain text in Phase 1; reserved for rich clipboard handling
  (e.g. HTML → Markdown conversion in Markdown documents is a later, optional plugin-able behaviour).

## Commands (Edit menu & palette)

| Command | Behaviour |
|---|---|
| `edit.undo` / `edit.redo` | as above |
| `edit.cut/copy/paste/pastePlain/selectAll` | standard |
| `edit.duplicateLine` | duplicates current line(s) or selection below |
| `edit.deleteLine` | deletes current line(s) |
| `edit.moveLineUp/Down` | moves line(s), keeps selection |
| `edit.sortLines` / `sortLinesDescending` | sorts selected lines (natural, case-insensitive by default; Unicode collation-agnostic) |
| `edit.trimTrailingWhitespace` | whole document or selection |
| `edit.transformToUppercase/Lowercase/TitleCase` | selection or word at cursor |
| `edit.indent` / `edit.outdent` | Tab / Shift+Tab with selection |
| `edit.toggleComment` (Phase 2) | uses language comment tokens |
| `edit.insertLineAfter/Before` | Mod+Enter / Mod+Shift+Enter |
| `edit.joinLines` | joins selected lines with a single space; palette only (no default binding) |
| `edit.toggleReadOnly` | per document, user read-only |

## Indentation

- `editor.insertSpaces` (default true), `editor.tabSize` (default 4), `editor.detectIndentation` (default true): on load,
  guess from the first 1,000 lines; status bar shows "Spaces: 4" / "Tab Size: 4", clickable to change per document or
  convert indentation.
- Auto-indent on Enter copies the previous line's indentation; adds one level after an opening bracket or a Markdown
  list item (continues `- `, `1. `, `- [ ] `; Enter on an empty list item removes the marker).
- Auto-closing brackets and quotes: `editor.autoClosingBrackets = languageDefined` (off for plaintext/Markdown prose
  except backticks in Markdown).

## Find / Replace (current document)

Find widget docks at the top-right of the editor view (not a modal):

```text
┌──────────────────────────────────────────────────────────────────┐
│ ▸ [ find…                       ] Aa  ab  .*   3 of 17  ↑ ↓  ≡  × │
│   [ replace…                    ]  [Replace] [Replace All]        │
└──────────────────────────────────────────────────────────────────┘
```

- `Aa` case-sensitive, `ab` whole word, `.*` regex (Rust `regex` syntax, multiline mode with `\n` supported), `≡` find in
  selection. Toggle states persist globally.
- Opening seeds the query with the selection (single-line) or the word at cursor.
- Search runs incrementally as you type; for documents > 5 MiB, search runs on a worker and results stream in.
- All matches are highlighted (`editor.findMatchHighlight`), current match emphasized; match positions feed the minimap
  and scrollbar marks.
- Enter / Shift+Enter next/previous; wraps around with a brief "Search wrapped" hint.
- Replace supports regex capture groups (`$1`, `${name}`) and case-preserving replace (toggle, Phase 2).
- Replace All is one undo step and reports "Replaced 17 occurrences".
- Invalid regex shows the error inline in danger colour; no search runs.
- `Esc` closes the widget and returns focus to the text.

## Go to Line

`navigate.goToLine` (Ctrl+G): small input in the palette area accepting `line`, `line:col`, `+n`/`-n` relative; previews
the target line while typing; Esc restores the previous position.

## Read-Only

Read-only views reject edits with a brief tooltip "Read-only" near the caret; selection, copy, find still work.

## Editor Context Menu

```text
Cut
Copy
Paste
Paste as Plain Text
──────────────
Select All
──────────────
Find…
Replace…
──────────────
Go to Line…
Toggle Bookmark
──────────────
Change Case ›          UPPERCASE · lowercase · Title Case
Sort Lines ›
──────────────
Format Document        (when a formatter exists)
Compare Current Changes
──────────────
[plugin contributions]
```

## Test Cases

```gherkin
Feature: Undo coalescing

  Scenario: Typing a word is one undo step
    Given an empty document
    When the user types "hello world"
    And presses undo once
    Then the document contains "hello "
    And pressing undo again leaves the document empty

  Scenario: Paste is one undo step
    When the user pastes three lines
    And presses undo
    Then all three lines are removed

  Scenario: Undo across duplicate views
    Given two views of one document
    When the user types "a" in view A and "b" in view B
    And presses undo in view B
    Then "b" is removed
    And pressing undo in view B again removes "a" and moves view B's caret to where "a" was

Feature: Line operations

  Scenario: Move line down
    Given lines "1", "2", "3" with the cursor on "1"
    When the user runs "Move Line Down"
    Then the lines are "2", "1", "3" and the cursor is on "1"

  Scenario: Sort lines
    Given selected lines "b", "a10", "a2"
    When the user runs "Sort Lines"
    Then the lines are "a2", "a10", "b"

Feature: Find and replace

  Scenario: Regex replace with capture groups
    Given the document "2026-10-14"
    When the user replaces regex "(\d+)-(\d+)-(\d+)" with "$3/$2/$1"
    Then the document is "14/10/2026"

  Scenario: Whole word
    Given the document "cat concat cat"
    When the user searches "cat" with whole word on
    Then 2 matches are found

  Scenario: Replace all is one undo step
    Given 17 matches of "foo"
    When the user chooses Replace All with "bar"
    And presses undo
    Then all 17 occurrences of "foo" are restored

  Scenario: Invalid regex
    When the user enters regex "(" with regex mode on
    Then an inline error is shown and no matches are highlighted

Feature: Markdown list continuation

  Scenario: Enter continues a task list
    Given the line "- [ ] buy milk" with the cursor at its end
    When the user presses Enter
    Then a new line "- [ ] " is inserted with the cursor after it

Feature: Grapheme handling

  Scenario: Backspace deletes a whole emoji
    Given the text "a👍🏽" with the cursor at the end
    When the user presses Backspace
    Then the text is "a"
```
