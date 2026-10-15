# 25 — JSON

**Status:** Done — M14 (hand-written span parser for JSON/JSONC/JSON Lines; Text/Split/Tree with virtualized tree, filter, context menu, both-way selection sync; error banner over the greyed last valid tree + editor squiggle; Format (keeps comments) / Minify / Sort Keys / Go to Parent. Deferred: JSON Tree secondary-sidebar panel, worker parse for >1 MB (small docs parse per change, large ones debounce 100 ms on the UI thread), `json.format.indent`, tree editing (Phase 6), breadcrumbs/outline (M15).)

## Purpose

Define JSON support: validation, formatting, structural navigation, and the Tree view in three modes (Text / Tree /
Text|Tree) with bidirectional selection sync. The raw text is always the source of truth; the tree is a view over it.

## Parsing

- Span-preserving parser in `notoir-editor::json` producing `JsonNode { kind, key, key_range, value_range, children }`
  with char ranges into the rope (parsers may work on bytes internally and convert). Candidate: `jsonc-parser` crate; otherwise a hand-written recursive-descent parser
  (JSON is small enough to own; owning it gives error recovery and exact ranges).
- Dialects: strict JSON for `.json`; JSONC (comments, trailing commas) for `.jsonc`, `tsconfig.json`, `.vscode/*.json`
  and `files.associations`. JSON Lines (`.jsonl`, `.ndjson`): each line parsed separately; tree shows one root per line.
- Parsed on change with debounce (100 ms small / worker for > 1 MB). Results tagged with document revision.
- Error recovery: on syntax error the tree keeps the last valid structure greyed out with a banner "Invalid JSON at line
  12, column 5: expected ',' or '}'" (click to jump). Errors also underline in the editor (squiggle in `danger`).
- Large-file mode: tree disabled beyond `json.tree.maxSize` (default 20 MiB) with an explanation.

## Display Modes

Per view: `Text`, `JsonTree`, `JsonSplit`. Toolbar contextual control `[ Text │ Split │ Tree ]` plus a Format button.
The JSON Tree panel (secondary sidebar) is an alternative that follows the active JSON document.

## Tree View

```text
▼ {} root                       object · 3
   ▼ {} user                     object · 3
       name        "John"        string
       age         42            number
     ▼ {} address                object · 2
         country   "Portugal"
         city      "Porto"
   ▶ [] tags                     array · 12
```

- Columns: key (or `[index]`), value preview (strings quoted, colours by type using syntax tokens), type/count (dimmed).
- Virtualized rows; expand/collapse with arrows/click; `*` expands all children; filter box filters by key or value
  (substring), keeping ancestors visible.
- Long strings truncated with ellipsis; full value in tooltip.
- Selecting a node selects/highlights the corresponding text range in the editor (value range; key+value with Shift).
- With `json.syncSelection = true` (default), moving the editor cursor selects the deepest node containing it in the tree
  (debounced 100 ms).
- Context menu: Copy Value, Copy Key, Copy Path (JSONPath-like `$.user.address.city` and JS-like `user.address.city`),
  Expand All Below, Collapse All Below, Reveal in Text.

## Editing in the Tree (incremental)

| Phase | Operations |
|---|---|
| 2 | Read-only tree |
| 6 | Edit value inline (type-aware), rename key, add property, delete property, add/remove array element, reorder array elements by drag |

All tree edits are translated into **minimal text edits** on the rope (preserving surrounding formatting and comments)
and applied as transactions — undo/redo work identically to text edits.

## Formatting & Validation

- `json.format` (Format Document): pretty-print using `editor.tabSize`/`insertSpaces` (or `json.format.indent`),
  preserving key order; JSONC comments preserved; one undo step. Refused on invalid JSON with the error location.
- `json.minify` (palette).
- `json.sortKeys` (palette): recursively sorts object keys.
- Validation diagnostics: syntax only in Phase 2. JSON Schema validation (`$schema` and `json.schemas` mapping, local
  files only) is a Phase 6 candidate.

## Structural Navigation

- Breadcrumbs show the path to the node at the cursor: `config.json > editor > minimap > enabled` (spec 26).
- Outline lists top-level keys (and nested to `json.outline.depth`, default 2).
- Folding of objects/arrays (spec 12).
- `json.goToParent` (palette) moves to the parent node's opening bracket.

## Test Cases

```gherkin
Feature: JSON tree

  Scenario: Tree reflects the document
    Given the JSON '{"user":{"name":"John","age":42}}'
    When the user switches to Tree mode
    Then the tree shows "user" with children "name" = "John" and "age" = 42

  Scenario: Selecting a node selects text
    Given split mode
    When the user selects "age" in the tree
    Then the editor selects "42"

  Scenario: Cursor syncs tree selection
    Given split mode
    When the editor cursor is placed inside "Porto"
    Then the tree selects "city"

  Scenario: Invalid JSON keeps last good tree
    Given a valid document shown as a tree
    When the user deletes a closing brace
    Then the tree is greyed out with an error banner showing line and column
    And the raw text remains editable

Feature: Formatting

  Scenario: Format preserves key order and is one undo step
    Given the minified JSON '{"b":1,"a":[1,2]}'
    When the user runs Format Document
    Then the text is pretty-printed with "b" before "a"
    And one undo restores the minified text

  Scenario: Format refuses invalid JSON
    Given invalid JSON
    When the user runs Format Document
    Then the text is unchanged and the error location is shown

Feature: JSONC

  Scenario: Comments allowed in tsconfig.json
    Given "tsconfig.json" containing a // comment
    Then no syntax error is reported and the tree is shown
```
