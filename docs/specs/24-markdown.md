# 24 — Markdown

**Status:** Done — Markdown display preferences (page/table width, responsive table cells, code wrapping and soft breaks);  render tree with source ranges (`notoir-editor::markdown`: CommonMark + GFM tables, task lists, strikethrough, footnotes, heading attributes, front matter, slugs), Edit / Split / Preview per view (toolbar control, Mod+Shift+V, Mod+K V, `markdown.defaultMode`, draggable split with auto orientation), preview renderer (headings, inline styles, links, local images incl. SVG decoded off the UI thread, quotes, nested lists with clickable task boxes as undoable edits, tables with alignment, highlighted code blocks, rules, footnotes, collapsible front matter, escaped HTML), 100/250 ms debounced re-parse, two-way scroll sync, double-click to source, links (web/mail to the OS, relative files at `#anchor`, in-document anchors, other schemes confirmed), Mod+B / Mod+I / code / insert link, paste URL over selection (M13); Markdown Insert context submenu with selection-aware blocks, link/image dialogs and workspace file completion. Deviations: remote images show the placeholder but cannot be loaded yet (no HTTP client); no per-block layout cache; no Markdown Preview panel; parse runs on the UI thread for all sizes; editor Ctrl/Cmd+click on links later. M26: File → Export as HTML… (`markdown.exportHtml`): a standalone page (`notoir-editor::markdown::to_html` + `html_document`: the preview's extensions, front matter left out, raw HTML escaped, headings with the preview's slugs as ids, wiki links as links to `<Target>.html#heading`, title from the first heading or the file name, embedded light/dark/print stylesheet) written through the save writer on the IO thread; local images stay relative to the export location. Deviation: no PDF export (no PDF renderer bundled — the page prints to PDF from a browser); Mermaid and math not rendered

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
