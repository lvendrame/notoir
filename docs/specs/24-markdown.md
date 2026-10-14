# 24 — Markdown

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
