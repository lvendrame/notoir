# 24 — Markdown

**Status:** Done — Linked table worksheets with live two-way source synchronization, inline formatting, structural editing, fixed row counters and column headers, nonadjacent row/column selection, row drag/drop and structural context menus, resizable rows/columns with wrapping and auto-fit, rectangular clipboard operations, reversible source bindings and session-only presentation; Markdown display preferences (page/table width, responsive table cells, code wrapping and soft breaks);  render tree with source ranges (`notoir-editor::markdown`: CommonMark + GFM tables, task lists, strikethrough, footnotes, heading attributes, front matter, slugs), Edit / Split / Preview per view (toolbar control, Mod+Shift+V, Mod+K V, `markdown.defaultMode`, draggable split with auto orientation), preview renderer (headings, inline styles, links, local images incl. SVG decoded off the UI thread, quotes, nested lists with clickable task boxes as undoable edits, tables with alignment, highlighted code blocks, rules, footnotes, collapsible front matter, escaped HTML), 100/250 ms debounced re-parse, two-way scroll sync, double-click to source, links (web/mail to the OS, relative files at `#anchor`, in-document anchors, other schemes confirmed), Mod+B / Mod+I / code / insert link, paste URL over selection (M13); Markdown Insert context submenu with selection-aware blocks, link/image dialogs and workspace file completion. Deviations: remote images show the placeholder but cannot be loaded yet (no HTTP client); no per-block layout cache; no Markdown Preview panel; parse runs on the UI thread for all sizes; editor Ctrl/Cmd+click on links later. M26: File → Export as HTML… (`markdown.exportHtml`): a standalone page (`notoir-editor::markdown::to_html` + `html_document`: the preview's extensions, front matter left out, raw HTML escaped, headings with the preview's slugs as ids, wiki links as links to `<Target>.html#heading`, title from the first heading or the file name, embedded light/dark/print stylesheet) written through the save writer on the IO thread; local images stay relative to the export location. Deviation: no PDF export (no PDF renderer bundled — the page prints to PDF from a browser); Mermaid and math not rendered

## Purpose

Markdown is a first-class file type. Define editing aids, the three display modes (Editor / Preview / Editor|Preview),
the preview renderer, scroll synchronization, links and images.

## Display Modes

Per view (`EditorView.kind`): `Text` (Editor), `MarkdownPreview`, `MarkdownSplit`. Toolbar contextual control:

```text
[ Edit │ Split │ Preview ]
```

- Shortcuts: `markdown.togglePreview` (`Mod+Shift+V`) toggles Edit ↔ Preview; `markdown.openPreviewToSide`
  (`Mod+K V`) sets Split.
- Split ratio adjustable (splitter), persisted per view. Split orientation horizontal by default; vertical when the view
  is narrower than 700 px (`markdown.splitOrientation = auto | horizontal | vertical`).
- Alternatively the Markdown Preview **panel** (spec 17) shows the active Markdown document's preview in a sidebar.
- `markdown.defaultMode = edit | split | preview` (default `edit`) for newly opened Markdown files.

## Preview Rendering

- Parse with `pulldown-cmark` (CommonMark + GFM: tables, task lists, strikethrough, footnotes, heading attributes)
  into a `MdDoc` render tree with **source byte ranges** for every block (notoir-editor; no egui).
- Render tree → egui widgets in the app crate: custom renderer for full theme control and source mapping.
- Re-parse debounced: 100 ms for documents < 200 KB on the UI thread; larger ones on a worker (250 ms debounce); live
  preview disabled in large-file mode with a "Refresh Preview" button.
- Layout caching: blocks are cached by (source range hash, width, zoom, theme) so typing re-lays-out only changed
  blocks.

### Supported elements

| Phase | Elements |
|---|---|
| 2 (initial) | Headings H1–H6, paragraphs, bold, italic, strikethrough, inline code, links, autolinks, images (local), blockquotes (nested), ordered/unordered lists (nested), task lists, tables (with alignment), fenced code blocks with syntax highlighting (syntect, spec 26), horizontal rules, hard/soft breaks, footnotes |
| 2 (later) | Heading anchors (`#heading-slug` links scroll within preview), front matter (YAML/TOML) shown as a collapsible table, HTML blocks shown as escaped source (no HTML rendering engine) |
| 6 | Mermaid diagrams, math (KaTeX-like), export to HTML, export to PDF, wiki links (spec 34) |

Typography in preview: UI font (Manrope) for prose at `editor.fontSize × 1.0`, JetBrains Mono for code; max content
width `markdown.preview.maxWidth` (default 760 px), centred.

### Display preferences

Settings → Markdown groups all `markdown.*` preferences. Layout preferences apply live in Preview and Split,
without modifying source text or HTML export. They support User and Workspace scopes.

| Key (`markdown.preview.*`) | Default | Behavior |
|---|---|---|
| `maxWidth` | 760 | Maximum centered page width, 300–2000 logical pixels; shrinks to fit the pane |
| `tableMaxWidth` | 0 | Maximum table width, 0–2000 logical pixels; 0 follows the page, always capped by local available width |
| `tableWrap` | true | Measure headers and cells, share column widths across rows, and wrap cell content |
| `codeWrap` | false | Wrap code lines while preserving whitespace and syntax highlighting |
| `breaks` | false | Render soft breaks as explicit newlines instead of spaces; hard breaks always remain newlines |

Tables keep short columns at natural width. Under pressure each column starts at its natural width or 64 logical
pixels, whichever is smaller; remaining space is distributed proportionally to unmet natural widths, excluding
18-pixel column gaps. Text wraps inside cells, including long words and URLs, with Markdown column alignment.
Tables that still cannot fit scroll horizontally within their own bounded area. Disabling table wrapping uses
natural widths and local scrolling. Code without wrapping also scrolls locally; scrollbars appear only when needed.
Reflow updates source-navigation and scroll-synchronization geometry.

Research: [Obsidian settings](https://obsidian.md/help/settings) expose readable line length and strict line breaks;
[VS Code Markdown preferences](https://github.com/microsoft/vscode/blob/main/extensions/markdown-language-features/package.json)
include soft-break and typography controls. This feature covers layout and wrapping; fonts, colors and line height
remain outside its scope.

### Task lists

Clicking a checkbox in the preview toggles `[ ]` ↔ `[x]` in the source as an undoable transaction
(`markdown.preview.interactiveCheckboxes = true`).

### Links

- `http(s)` / `mailto` links: Ctrl/Cmd+click (or click in preview) opens via the OS (`open` crate); hovering shows the
  URL in a tooltip.
- Relative links to files (`./api.md`, `../notes/x.md#section`) open in Notoir (anchor scrolls to heading).
- Other schemes (`file:`, custom) ask for confirmation before handing to the OS.
- In the editor, links are underlined on Ctrl/Cmd-hover and followable with Ctrl/Cmd+click.

### Images

- Local images (relative to the document, or absolute within the workspace) decode via `image` on a worker, cached by
  (path, mtime), downscaled to display width; formats PNG, JPEG, GIF (first frame), WebP, SVG (resvg).
- Remote images are **not** loaded by default (privacy): placeholder with "Load remote images" button;
  `markdown.preview.loadRemoteImages = never | ask | always` (default `ask`, decision remembered per workspace).
- Missing images show a broken-image placeholder with the path.

## Scroll Synchronization

Split mode syncs scroll both ways using the block source ranges: the editor's top visible line maps to the preview
block containing it (interpolated within the block); scrolling the preview maps back. Sync can be disabled
(`markdown.preview.scrollSync`). Double-click in preview moves the editor cursor to that block's source.

## Editing Aids (Text mode)

- Syntax highlighting with heading/emphasis/link/code tokens (spec 26); headings slightly bolder.
- List continuation on Enter, indent/outdent list items with Tab/Shift+Tab (spec 11).
- `markdown.toggleBold` (`Mod+B`) and `markdown.toggleItalic` (`Mod+I`), active only when
  `editorFocus && languageId == 'markdown'`. In that context `Mod+B` formats instead of toggling the sidebar; the sidebar
  stays reachable via the chord `Mod+K B` (bound everywhere), the ☰ toolbar button and the View menu.
- `markdown.toggleCode`, `markdown.insertLink`: no default shortcut (`Mod+K` is a chord prefix and `Mod+Shift+K` is
  Delete Line); available from the palette and the editor context menu.
- Paste of a URL over a selection creates `[selection](url)`.
- Outline and breadcrumbs from headings (spec 26); folding by heading (spec 12).

### Insert context menu

Markdown editor text (Edit mode or the editor side of Split) offers Insert: Link…, Heading (H1–H6),
Code (General, JavaScript, TypeScript, Shell, Python, Rust, JSON, HTML, CSS, SQL, YAML, More…), Quote,
Formatting (Bold, Italic, Strikethrough, Inline Code), List (Bulleted, Numbered, Task), Image…, Table,
and Horizontal Rule. More… searches registered languages. General omits the fence language; Shell uses `sh`.
Other languages and preview surfaces do not offer Insert. Read-only Markdown disables insertion commands.

Actions target the clicked view and its primary selection. Right-click inside a selection preserves it;
outside it, Markdown moves the caret to the clicked position. Inline formatting uses the selection or word
at the caret. Headings replace an existing ATX prefix; quotes/lists prefix affected lines, excluding a final
line selected only at its start. Empty actions create editable templates. Fenced code preserves selected
content and uses a fence longer than embedded backtick runs. Block templates have separating blank lines;
Table opens a dimensions dialog (1–100 columns and 1–100 data rows; defaults: two columns and one data row). A header row is added automatically; Cancel leaves the document unchanged. Each insertion is one undoable transaction.

Link… opens Caption and URL or file fields, with selected text as the caption. Image… uses Alternative text
and permits empty text. No buffer changes occur until confirmation. A destination is required. Empty link
captions default to the chosen filename or destination. Escape/Cancel leave the buffer unchanged.
A leading `@` in the destination searches indexed workspace files using fuzzy matching on their relative paths;
exact filename and path matches rank first, followed by exact filename stems, filename prefixes, substrings and fuzzy matches. Mouse or arrows/Enter select a file without submitting the dialog. Image completion lists only supported image extensions (PNG, JPG/JPEG, GIF, WebP, SVG), case-insensitively. File destinations are relative to the source
document's directory (workspace root for unsaved documents), with forward slashes and Markdown-safe encoding.
Show no-workspace, indexing and no-results states. The captured view/document/revision must remain valid,
Markdown, and editable at confirmation; otherwise reject insertion with a localized explanation.

## Linked table worksheets

Preview tables expose a localized edit icon on hover or keyboard focus, without
changing layout. `markdown.editTable` opens a session-local `MarkdownTable` view
of the same document, or focuses the worksheet already linked to that table.
The command is also available from the palette at a table's source position.

The compact worksheet renders inactive cells and exposes raw inline Markdown in
the active cell. An icon-only toolbar with localized tooltips and accessible labels offers bold, italic, strikethrough, inline code, links,
row/column insertion and deletion, and Markdown column alignment. One header and
one column are mandatory; header-only tables are allowed. Column labels and
cell positions are accessible, theme-aware and localized. No formulas, merged
cells, arbitrary fonts/colors, or block-level formatting are provided.

The worksheet link icon runs `markdown.insertLink` and reuses the Insert → Link
dialog: Caption, URL or file, `@` workspace-file completion, Cancel/Escape and
stale-target validation. Opening or cancelling the dialog does not edit the
document. Confirmation replaces the captured cell text or selected text within
an active cell with a safely escaped Markdown link, as one undo step. For a
rectangular selection, the entered caption and destination apply to all selected
cells. Cell focus and selection are captured when the dialog opens.
If a single selected cell contains one link, or the active text selection/caret
identifies one link, prefill its caption and destination and replace that whole
link on confirmation. Preserve surrounding cell text and any existing title;
changing only the URL preserves the caption's Markdown styling and escapes.
Reference-style links resolve against the document and are edited as inline links.

Clicks select cells; Shift-click extends a rectangular selection. Arrows navigate;
Enter/F2/double-click edit, Enter finishes, and Tab/Shift-Tab navigate cells.
Escape leaves cell editing without reverting already synchronized text. Typing
updates the shared Markdown immediately and uses the document's undo history.
Formatting acts on selected text/word in an active cell or whole selected cells.
Rectangular clipboard operations use tabs/newlines, grow the table on paste, and
are atomic undo steps. Physical line breaks pasted inside one cell become spaces.

### Worksheet row and column selectors

- A fixed left gutter numbers worksheet rows from 1, including the Markdown header.
  The gutter is presentation only and is absent from source, clipboard data and exports.
  The header remains visible during vertical scrolling; row counters remain visible during horizontal scrolling.
- Clicking a row counter selects the whole row. Shift-click selects a range;
  Ctrl/Command-click adds or removes individual rows, permitting nonadjacent groups.
  Clicking a column header selects the whole column with the same range/additive modifiers.
  Double-click or Enter/F2 still edits header text. Changing selection does not change the document.
- Right-clicking a selected row counter preserves its selected group and offers only
  Insert row above, Insert row below and Delete selected rows. A counter outside the group
  selects that row before opening the menu. Column headers behave likewise with
  Insert column before, Insert column after and Delete selected columns.
  Insertions add one row or column before the first or after the last selected index.
- Pressing and dragging a selected row counter or a cell in a selected row moves
  the selected rows as a block, retaining their original order. A line marks the drop
  position, and the view scrolls at the vertical edges. Drop commits one undoable source
  edit; dragging alone never writes. Escape, a drop outside the worksheet, read-only
  transitions or source changes cancel the operation. Undo/redo restore row order and selection.
- The Markdown header stays fixed and cannot be moved/deleted or have a row inserted
  above it; data rows may be reordered below it. The last column cannot be deleted.
  Delete, formatting, alignment and clipboard operations honor nonadjacent selections
  without applying to the gaps. Row moves preserve all cell Markdown and column alignment.

### Worksheet dimensions and wrapping

Every column has a draggable divider on the right edge of its header, and every row
has a height handle at the bottom of its numbered counter. These handles have
localized tooltips, accessible names, resize cursors and keyboard arrow adjustment.
Escape restores the size from before a drag. Double-click a column divider to fit
the widest rendered cell, or a row divider to fit its wrapped content.

Inactive cells wrap Markdown within their column and clip to their row's height;
expanding the row reveals additional lines. Active cells retain inline Markdown
editing with soft wrapping and an internal scrollbar. The header's displayed height
is bounded by the pane so data rows remain reachable. Variable row heights are
virtualized, while the fixed counter gutter and header remain aligned during scrolling.

Sizes belong to each worksheet view, including read-only views, and never modify
Markdown, dirty state, save output or the document undo stack. Structural worksheet
edits carry sizes with their rows/columns; structural undo/redo restores the matching
layout. Undoing ordinary text edits preserves subsequent view-only size changes.

Table bindings map source character ranges through edits and retain history-state
checkpoints for undo/redo. Unavailable or ambiguous targets display the last grid
with edits disabled; source repair or undo restoring the target resumes editing.
Stale actions never overwrite newer revisions. Read-only state, save, autosave,
recovery, and dirty indicators belong to the shared source document. Closing a
source tab does not close its worksheet. Worksheets are omitted from session
restore; a document with only worksheets retains one ordinary source-tab entry.

```gherkin
Feature: Linked table worksheets

  Scenario: Resize worksheet rows and columns without changing Markdown
    Given a worksheet with long cell content
    When the user drags a column divider and row height handles
    Then column widths and row heights change independently
    And header cells remain aligned with their data cells
    And the source, dirty state and document undo history remain unchanged

  Scenario: Auto-fit worksheet content
    Given a worksheet with long content in a narrow column
    When the user double-clicks the row height handle
    Then the row grows to fit the wrapped content
    When the user double-clicks a column divider
    Then the column grows to fit its widest rendered cell
    And short rows retain the minimum height
    And the source remains unchanged

  Scenario: Select and delete worksheet rows from their counters
    Given a worksheet with four data rows
    When the user clicks a row counter and uses Shift-click and Control-click to adjust the selection
    Then whole rows are selected and nonadjacent rows can be selected together
    When the user right-clicks a selected counter
    Then the context menu offers row insertion above, insertion below and deletion
    And the existing row selection is preserved
    When the user deletes the selected rows
    Then only those rows are deleted and the gaps are preserved
    And one undo restores the source and row selection

  Scenario: Select worksheet columns and use their context menu
    Given a worksheet with two columns
    When the user clicks column headers to select columns
    Then each selected column includes its header and all data cells
    When the user right-clicks a selected column header
    Then the context menu offers insertion before, insertion after and deletion
    And deleting all columns is disabled
    When the user inserts columns before and after the selection
    Then blank columns are inserted at the selected boundaries
    When the user deletes one selected column
    Then the remaining column and its data are preserved
    And undo restores the source

  Scenario: Drag selected worksheet rows together
    Given two nonadjacent worksheet rows selected through their counters
    When the user drags a cell in a selected row below the last data row
    Then the selected rows move together in their original order
    And the moved rows remain selected
    When the user undoes the move
    Then the original source and row selection are restored
    And redo restores the moved row order and selection

  Scenario: Edit an existing worksheet link
    Given a worksheet cell containing a Markdown link
    When the user opens its link dialog
    Then Caption and URL or file contain the existing caption and destination
    When the user cancels
    Then the cell remains unchanged
    When the user reopens the dialog and changes both fields
    And confirms the change
    Then the original link is replaced without nesting another link
    And one undo restores the original cell

  Scenario: Open a linked table worksheet
    Given a Markdown document containing a table
    When the user requests its worksheet
    Then a worksheet tab shares the source document
    And opening the same table again focuses the existing worksheet
    And worksheet changes mark and save the source document
    And session capture retains only an ordinary source entry

  Scenario: Synchronize worksheet and Markdown edits
    Given an open table worksheet
    When source edits move the table and change a cell
    Then the worksheet follows its original table and displays the changed cell
    When the worksheet changes a cell
    Then the source changes immediately
    And shared undo and redo restore the cell values

  Scenario: Pause and recover an unavailable worksheet
    Given a worksheet and another identical table in the same document
    When source edits invalidate the linked table
    Then the worksheet becomes unavailable and rejects cell writes
    When source repair restores the table
    Then the worksheet resumes editing
    When the linked table is deleted
    Then the worksheet does not attach to the other table
    And undo restores the original binding

  Scenario: Edit worksheet structure and paste cell ranges
    Given an open table worksheet
    When a rectangular paste extends beyond its dimensions
    Then rows and columns grow to contain the pasted cells
    And one undo restores the original table
    And redo restores the pasted range
    When the user formats a selected range and changes its alignment
    Then selected cells contain Markdown formatting and selected columns have that alignment
    And the header cannot be deleted
    And deleting a selected column preserves the remaining table
```

## Test Cases

```gherkin
Feature: Markdown modes

  Scenario: Switch to split mode
    Given "README.md" is open in Edit mode
    When the user clicks "Split" in the toolbar
    Then the view shows the editor and the live preview side by side

  Scenario: Live preview
    Given split mode
    When the user types "# Title" on a new line
    Then the preview shows a level-1 heading "Title" within 200 ms

Feature: Preview rendering

  Scenario: Task list toggle writes to source
    Given the source line "- [ ] buy milk"
    When the user clicks the checkbox in the preview
    Then the source line becomes "- [x] buy milk"
    And undo restores "- [ ] buy milk"

  Scenario: Remote images not loaded by default
    Given the document contains "![x](https://example.com/a.png)"
    Then the preview shows a placeholder with "Load remote images"
    And no network request was made

  Scenario: Relative link opens file at heading
    Given "a.md" links to "./b.md#setup"
    When the user clicks the link in the preview
    Then "b.md" opens scrolled to the "Setup" heading

  Scenario: Code block highlighting
    Given a fenced block with language "rust"
    Then the preview renders it with Rust syntax colours

Feature: Scroll sync

  Scenario: Editor scroll moves preview
    Given split mode on a long document
    When the editor scrolls to the "Roadmap" heading
    Then the preview shows the "Roadmap" heading at its top
```

```gherkin
Feature: Markdown insertion

  Scenario: Insert Markdown blocks as undoable edits
    Given a Markdown document with selected lines
    When a heading, quote, list or fenced code insertion is requested
    Then the selected content is formatted with the requested Markdown syntax
    And one undo restores the original text
    And redo restores the formatted text

  Scenario: Insert link through a dialog
    Given selected text in a Markdown document
    When the user chooses Insert Link
    Then a dialog opens with the selected text as its caption
    And the document remains unchanged until confirmation
    When the user cancels
    Then the document remains unchanged
    When the user opens the dialog again and confirms a destination
    Then the selection becomes a Markdown link
    And one undo restores the selected text

  Scenario: Complete workspace files in link destinations
    Given a Markdown file in a workspace subdirectory
    When the user types an @ query in the link destination
    Then matching workspace file paths are shown
    When the user selects a file with Enter
    Then a relative destination replaces the query without submitting
    When the user confirms the dialog
    Then the inserted link uses the caption and safely encoded relative file path

  Scenario: Reject a stale Markdown insertion dialog
    Given an open link insertion dialog
    When the source document changes before confirmation
    Then confirmation leaves the changed document untouched
    And the dialog explains that the original target changed
```

```gherkin
Feature: Markdown insertion refinements

  Scenario: Filename matches rank ahead of fuzzy path matches
    Given a workspace containing "apps/notoir/src/app.rs" and many fuzzy matches
    When the user types "@app.rs" in the link destination
    Then the source file appears in the results
    And Enter selects the exact filename match first
    When the user searches for "@app" in the image destination
    Then matching image files are listed
    And non-image files are absent
    And Enter selects the closest image filename match first

  Scenario: Choose table dimensions before insertion
    When the user chooses Insert Table
    Then a dialog offers columns and data rows without changing the document
    When the user cancels
    Then the document remains unchanged
    When the user opens the dialog again and chooses 3 columns and 4 data rows
    And confirms insertion
    Then the document contains a table with 3 columns, a header and 4 data rows
    And one undo removes the table
```

```gherkin
Feature: Markdown display preferences

  Scenario: Narrow Markdown pages fit the pane
    Given a Markdown document in Split mode with a narrow preview pane
    Then the prose wraps within the pane
    And the source text remains unchanged

  Scenario: Markdown tables wrap within the configured width
    Given a Markdown table with long descriptions in Preview mode
    When the table maximum width is set to 240 logical pixels
    Then cell text wraps within that width
    When the maximum width increases to 600 logical pixels
    Then the cells reflow into fewer lines without reopening the document
    And the source text remains unchanged

  Scenario: Markdown code wrapping and line breaks apply live
    Given a Markdown preview with a soft break, a hard break and a long code line
    Then the soft break displays as a space and the hard break displays as a newline
    When code wrapping and single-newline display are enabled
    Then the code wraps into multiple lines and the soft break displays as a newline
    And the source text remains unchanged
```
