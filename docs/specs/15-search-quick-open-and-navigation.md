# 15 — Workspace Search, Quick Open & Navigation

## Purpose

Define workspace-wide search, Quick Open (fuzzy file finder), Go to Symbol, and navigation history (Back/Forward).
In-document find/replace is in spec 11.

## Workspace Search

`search.workspace` (`Mod+Shift+F`) opens the Search panel (primary sidebar by default):

```text
SEARCH                                              ⋯  ×
[ MCP                                  ] Aa ab .*
[ replace…                             ] [Replace All]
files to include  [ docs/**            ]
files to exclude  [ **/archive/**      ]
──────────────────────────────────────────────────────
24 results in 6 files                    [collapse all]
▾ docs/mcp.md                                       2
    14  Notoir exposes an **MCP** server…
    48  **MCP** configuration…
▾ README.md                                         1
    21  **MCP** integration…
```

- Scope: workspace root, honouring `.gitignore`, `files.exclude`, `search.exclude`; binary files and files larger than
  `search.maxFileSize` (default 5 MiB) skipped. Without a workspace, searches all open documents.
- Engine: `ignore` parallel walker + `grep-searcher` on the blocking pool; results streamed in batches every 50 ms,
  capped at `search.maxResults` (default 20,000) with "Results truncated" notice.
- Dirty open documents are searched in their **in-memory** content (snapshots), not disk content.
- Typing restarts the search after a 300 ms debounce, cancelling the previous job (generation rule, spec 01).
- Clicking a result opens/focuses the file, selects the match, and records a navigation entry.
- Replace All across files: preview count first ("Replace 24 occurrences across 6 files?"); applies to open documents as
  transactions (undoable per document) and to closed files through the save pipeline with conflict checks; files changed
  are listed in a notification with Undo for open ones.
- Result list virtualized; context menu on file rows: Open, Open to the Side, Copy Path, Dismiss; on match rows:
  Replace, Dismiss.
- "Find in Folder…" from the Explorer prefills `files to include`.
- Future: indexed search (spec 34) plugs in behind the same panel.

## Quick Open

`navigate.quickOpen` (`Mod+P`): palette-style overlay at the top centre of the window.

- Lists workspace files (from the background index) + open documents + Scratchpads + recent files.
- Empty query: open editors (MRU) then recent files.
- Fuzzy matching with `nucleo-matcher` on relative path; scoring favours file-name matches, contiguous matches, and
  recently opened files. `arch` → `architecture.md`, `architecture-old.md`, `docs/architecture/design.md`.
- Each row: file icon, file name (matched characters bold), dimmed relative directory, Git decoration letter.
- `file:line[:col]` suffix opens at that location (`README.md:120`).
- Prefixes (handled by providers registered on the same overlay):
  - `>` → Command Palette (spec 16)
  - `:` → Go to Line
  - `@` → Go to Symbol in current document (Phase 2; uses outline provider, spec 26)
  - `#` → reserved for workspace symbols/tags (Phase 6)
  - `?` → help listing prefixes
- Keys: ↑/↓, Enter open, `Mod+Enter` open to the side, `Esc` close.
- Index: built on workspace open on the blocking pool; kept current by the watcher; up to `search.quickOpenMaxFiles`
  (default 200,000). Quick Open never blocks: shows partial results with an "Indexing…" hint.

## Navigation History

- A `NavigationHistory` (per window) records `Location { doc: DocRef (path or scratch id), position, view_hint }` when:
  - the active document changes,
  - the cursor jumps more than `navigation.minLineDistance` (default 10) lines via a command (go to line, search result,
    outline click, bookmark, definition-like jumps, Quick Open with line),
  - not on ordinary arrow-key movement or typing.
- Back (`navigate.back`) / Forward (`navigate.forward`) move through entries, reopening closed files if needed; entries
  for deleted files are skipped.
- Max 50 entries; consecutive entries within the same few lines are merged.
- Mouse back/forward buttons (X1/X2) trigger Back/Forward.

Example: `README.md:92 → config.json:44 → architecture.md "MCP" heading`; Back returns to `config.json:44`, then
`README.md:92`.

## Navigate Menu Commands

Quick Open, Go to Line, Go to Symbol, Next/Previous Tab, Next/Previous Editor Group, Back, Forward, Next/Previous Change
(diff or Git change in the gutter), Next/Previous Bookmark, Next/Previous Search Result (iterates the Search panel's
results or the find widget's matches, whichever was used last).

## Test Cases

```gherkin
Feature: Workspace search

  Scenario: Results stream without blocking typing
    Given a workspace with 50,000 files
    When the user searches for "MCP"
    Then results appear incrementally
    And the editor remains responsive to typing during the search

  Scenario: Dirty buffers are searched in memory
    Given "notes.md" has an unsaved line containing "zebra"
    When the user searches the workspace for "zebra"
    Then "notes.md" is listed with that line

  Scenario: Gitignored files are excluded
    Given "build/out.txt" is gitignored and contains "MCP"
    When the user searches for "MCP"
    Then "build/out.txt" is not in the results

  Scenario: Clicking a result jumps to the match
    When the user clicks the result "docs/mcp.md:48"
    Then "docs/mcp.md" is active with the match on line 48 selected

Feature: Quick Open

  Scenario: Fuzzy file search
    Given the workspace contains "architecture.md", "architecture-old.md", "docs/architecture/design.md", "README.md"
    When the user types "arch" in Quick Open
    Then the first three results are the architecture files
    And "README.md" is not listed

  Scenario: Open at line
    When the user types "README.md:120" and presses Enter
    Then "README.md" opens with the cursor on line 120

  Scenario: Command prefix
    When the user types ">" in Quick Open
    Then the overlay switches to the Command Palette

Feature: Navigation history

  Scenario: Back and forward across files
    Given the user went to README.md line 92, then config.json line 44, then architecture.md line 10
    When the user runs Back twice
    Then the cursor is in README.md on line 92
    When the user runs Forward
    Then the cursor is in config.json on line 44
```
