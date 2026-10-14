# 12 — Gutter, Bookmarks & Folding

**Status:** Partial — line-number gutter lane incl. relative mode (M4); bookmarks, folding and other lanes (M15)

## Purpose

Define the reusable `EditorGutter` (line numbers, fold controls, bookmarks, Git change indicators, future diagnostics),
bookmarks, and folding.

## Gutter Architecture

The gutter is a horizontal stack of **lanes**, each implementing a small trait in the app crate fed by domain data:

```rust
pub trait GutterLane {
    fn id(&self) -> LaneId;                                // Bookmarks, LineNumbers, GitChanges, Folding, Diagnostics, Plugin(..)
    fn width(&self, ctx: &GutterCtx) -> f32;               // may depend on digits / zoom
    fn paint_row(&self, ctx: &GutterCtx, row: DisplayRow, rect: Rect, painter: &Painter);
    fn on_click(&mut self, ctx: &GutterCtx, row: DisplayRow, button: PointerButton) -> Option<CommandInvocation>;
}
```

Default lane order (left → right):

```text
┌─┬──────┬─┬─┐
│●│  128 │▌│▾│ text…
└─┴──────┴─┴─┘
 │    │    │ └ fold control (shown on gutter hover, always for folded regions)
 │    │    └── Git change bar (added / modified / deleted wedge)
 │    └─────── line number (right-aligned, tabular figures)
 └──────────── bookmark marker (also hosts future diagnostics icons)
```

- Line numbers: `editor.lineNumbers = on | off | relative`. Current line number uses `--text-primary`; others
  `--text-tertiary`. Width fits max digits (min 3) and adapts without jitter while scrolling (computed from total lines).
- Wrapped continuation rows show no number.
- Clicking a line number selects the line; dragging selects lines.
- Gutter context menu: Toggle Bookmark, Copy Line Number, Line Numbers ›, Fold/Unfold here, Show Git change (Phase 3).
- Lanes are hidden entirely when their feature is off (no empty space).
- Plugins (Phase 5) may contribute a lane with simple markers via the plugin API (icon + tooltip per line).

## Bookmarks

- Per document, anchored to buffer positions (move with edits; deleted when their line is deleted and no line remains).
- Commands: `bookmarks.toggle`, `bookmarks.next`, `bookmarks.previous` (wrap across the document, then optionally
  across open documents with `bookmarks.navigateAcrossFiles = true`), `bookmarks.clear` (document), `bookmarks.clearAll`.
- Click in bookmark lane toggles.
- Optional label (Phase 2: "Edit Bookmark Label…").
- Persisted in the session store per file path (spec 23), restored when the file opens; reconciled by line number
  (and line content hash where available) if the file changed externally.
- Shown in the minimap when `editor.minimap.showBookmarks` (default true).
- Accessible: bookmark marker is a filled `bookmark` icon shape, not colour alone.

## Folding

Fold ranges are produced per language by a `FoldingProvider` in `notoir-editor`:

| Language | Strategy |
|---|---|
| Markdown | Heading sections (a heading folds until the next heading of same or higher level), fenced code blocks, lists with nested items, front matter |
| JSON | Objects and arrays spanning multiple lines |
| Brace languages (Rust, JS, TS, CSS, …) | Bracket pairs spanning lines, skipping strings/comments using the syntax tokenizer |
| YAML, Python, TOML, others | Indentation-based |
| `#region` / `// region` markers | Explicit regions in any language |
| Large-file mode | Indentation-based only, computed lazily for visible area |

- Fold ranges recompute on a debounce (150 ms) after edits, off-thread for large documents; fold state is per view
  (spec 03) and anchored so it survives edits.
- Folded region rendered as the first line plus an inline `⋯` chip; clicking the chip unfolds.
- Commands: `fold.fold`, `fold.unfold`, `fold.toggle`, `fold.foldAll`, `fold.unfoldAll`, `fold.foldLevel1`…`foldLevel7`,
  `fold.foldAllExceptCurrent` (palette).
- Cursor never rests inside a folded region: moving into it unfolds it; search matches in folds unfold on navigation.
- Fold state persisted per view in the session.

## Test Cases

```gherkin
Feature: Gutter

  Scenario: Line numbers width does not change while scrolling
    Given a document with 12,000 lines
    When the user scrolls from line 1 to line 11,000
    Then the gutter width stays constant

  Scenario: Relative line numbers
    Given line numbers are "relative" and the cursor is on line 50
    Then line 47 shows "3" and line 50 shows "50"

Feature: Bookmarks

  Scenario: Bookmarks move with edits
    Given a bookmark on line 10
    When the user inserts 5 lines above line 10
    Then the bookmark is on line 15

  Scenario: Next bookmark wraps
    Given bookmarks on lines 5 and 40 and the cursor on line 41
    When the user runs "Next Bookmark"
    Then the cursor moves to line 5

  Scenario: Bookmarks persist
    Given a bookmark on line 12 of "notes.md"
    When Notoir restarts and "notes.md" is reopened
    Then line 12 is bookmarked

Feature: Folding

  Scenario: Fold a Markdown section
    Given the Markdown document
      """
      # Architecture
      text
      ## Editor
      more
      # Roadmap
      """
    When the user folds line 1
    Then lines 2–4 are hidden and line 5 is visible

  Scenario: Fold level 1 in JSON
    Given a JSON document with nested objects
    When the user runs "Fold Level 1"
    Then every top-level property's object value is folded

  Scenario: Navigating into a fold unfolds it
    Given lines 10–20 are folded
    When "Go to Line 15" runs
    Then the region is unfolded and the cursor is on line 15
```
