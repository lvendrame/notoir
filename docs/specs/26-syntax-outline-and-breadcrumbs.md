# 26 — Syntax Highlighting, Outline & Breadcrumbs

**Status:** Partial — language registry and mode picker (M2, M5); syntax highlighting: syntect with pure-Rust regex over two-face's bat grammar set plus `assets/syntaxes/` (PowerShell), compiled into one packdump by `notoir-editor/build.rs`, incremental per-line states with convergence after edits, viewport-first within a per-frame budget, scope → spec 21 token mapping, off in large-file mode (M12). Remaining lines continue on later frames rather than a worker thread. Outline, Go to Symbol and breadcrumbs (M15); Markdown contextual controls for the language-override scenario (M13)

## Purpose

Define language identification, syntax highlighting, the generic Outline panel, Go to Symbol and breadcrumbs. Notoir is
not an IDE: no language servers in the core product.

## Languages

`LanguageId` registry (in `notoir-editor`), each entry: id, display name (translated), extensions, file names,
first-line patterns, comment tokens, bracket pairs, auto-close pairs, word pattern, folding strategy, outline provider,
syntax definition reference, defaults (e.g. Markdown word wrap on).

Initial set: plaintext, markdown, json, jsonc, jsonl, yaml, toml, xml, html, css, javascript, typescript, rust, python,
shellscript (sh/bash/zsh), powershell, sql, ini, dotenv, diff, gitcommit, log.

The status bar language item (spec 19) opens a picker to change the language mode of a document and to configure
associations (`files.associations`).

## Syntax Highlighting

- Engine: `syntect` with pure-Rust regex (`default-fancy`). Syntaxes: syntect defaults + additional `.sublime-syntax`
  files (TOML, TypeScript/TSX, Dockerfile, PowerShell, dotenv, log) in `assets/syntaxes/` with licenses tracked
  in `assets/syntaxes/LICENSES.md`; compiled into a binary packdump at build time (`build.rs`) to avoid startup parsing.
- Lazy syntax loading: the packdump is deserialized on first use per syntax (startup budget).
- Incremental: per-line `ParseState` + `HighlightState` cache; an edit invalidates from the changed line; re-highlighting
  proceeds until states converge with the cached ones (usually a few lines). Only lines up to the viewport bottom are
  highlighted synchronously (time budget 2 ms/frame); the remainder continues on a worker with a rope snapshot.
- Theme mapping: syntect scopes → Notoir syntax tokens (spec 21) through a scope-selector table; themes never ship
  `.tmTheme` files.
- Plugins (Phase 5) can register languages with a `.sublime-syntax` file in their package, compiled at load time on a
  worker.
- Disabled in large-file mode.
- Future (optional): tree-sitter grammars behind the same `Highlighter` trait for code outline accuracy; not planned
  before Phase 6.

## Outline

Providers return `Vec<Symbol { name, kind, range, selection_range, children }>`:

| Language | Provider |
|---|---|
| Markdown | Headings (ATX and Setext), nested by level |
| JSON / YAML / TOML | Keys/tables up to `outline.depth` (default 2) |
| Code (Rust, Python, JS/TS, …) | Phase 6: lightweight regex/heuristic symbol extractor or tree-sitter; until then outline shows "No symbols" for code |
| Plugins | Phase 5: `notoir.outline.register(language, fn)` |

Outline panel (secondary sidebar by default, movable to any area):

```text
OUTLINE                         [filter] [sort] ⋯
▾ Introduction
▾ Architecture
    Editor
    Git
    MCP
  Implementation
  Roadmap
```

- Follows the active document; highlights the symbol containing the cursor (`outline.followCursor = true`).
- Click jumps (records navigation history); filter box; sort by position (default) or name; collapse/expand all.
- Recomputed on debounce (150 ms) from the parse already produced for folding/preview where possible.

## Go to Symbol

`navigate.goToSymbol` (`Mod+Shift+O`) = Quick Open with `@` prefix over the active document's outline; fuzzy filtered;
previews location while navigating; Esc restores.

## Breadcrumbs

Optional bar under the tab strip (`workbench.breadcrumbs.enabled`, default **on**; toggle in View → Appearance):

```text
notes > docs > architecture.md > Architecture > MCP > Permissions
```

- Segments: workspace-relative path parts (folder segments), file name, then symbol path at the cursor (Markdown
  headings, JSON keys `config.json > editor > minimap > enabled`, code symbols when available).
- Each segment is clickable: folder → dropdown listing siblings (files/folders) to open; file → siblings; symbol →
  dropdown of sibling symbols to jump to.
- Keyboard: `Mod+Shift+.` focuses breadcrumbs; arrows move; Enter opens dropdown.
- Files outside the workspace show the absolute path collapsed (`… > Downloads > x.md`).

## Test Cases

```gherkin
Feature: Highlighting

  Scenario: Highlighting updates incrementally
    Given a 10,000-line Rust file
    When the user types a double quote on line 5,000
    Then lines after 5,000 are re-highlighted without blocking the frame

  Scenario: TOML and TypeScript are highlighted
    When "Cargo.toml" and "app.ts" are opened
    Then both show syntax colours

  Scenario: Language mode override
    Given "notes.txt"
    When the user selects "Markdown" from the status bar language picker
    Then the document is highlighted as Markdown and the Markdown contextual controls appear

Feature: Outline

  Scenario: Markdown outline
    Given a document with headings "Introduction", "Architecture" (with "Editor", "Git", "MCP"), "Roadmap"
    Then the Outline shows that nested structure
    When the user clicks "MCP"
    Then the editor jumps to the "MCP" heading

Feature: Breadcrumbs

  Scenario: JSON breadcrumb path
    Given the cursor is inside the value of editor.minimap.enabled in "config.json"
    Then breadcrumbs read "config.json > editor > minimap > enabled"

  Scenario: Folder segment dropdown
    When the user clicks the "docs" breadcrumb segment
    Then a dropdown lists the entries of "docs"
```
