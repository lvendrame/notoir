# 34 — Future Note Features (Phase 6)

**Status:** Done — M24: wiki links (`[[Note]]`, `[[Note#Heading|alias]]`, paths; parsed outside code and front matter) resolved case-insensitively by file stem, then as paths relative to the linking note or the root (a note next to the source wins over others of the same name, else a picker); Ctrl/Cmd+click or Follow Link (Alt+Enter) opens the target at its heading; a missing target asks to create the note next to the linking one; the editor underlines links (dashed for missing ones); `[[` completion from the note index (Up/Down/Enter/Tab/Esc, one undo step); the preview renders links (missing ones dashed with an icon) and follows them; renaming a note offers to update links (open notes as one undoable edit each, closed ones rewritten on disk); Backlinks panel with context snippets and unlinked mentions (searched in the background, open buffers included); tags from prose and front matter (`tags: [..]` or a list), Tags panel with counts and the notes of a tag, Quick Open `#` lists tags then their notes; Notes: Rebuild Index. The link index is an in-memory cache rebuilt from the files whenever the workspace file index is, with edited buffers folded in. M25: indexed full-text search — an SQLite FTS5 trigram index (`notoir-workspace::text_index`) in `<cache>/index/<FNV hash of the workspace key>/text.sqlite`, kept by a worker thread from the workspace file list (refreshed by the watcher; unchanged files skipped by mtime and size); the Search panel and MCP `workspace_search` use it only while it is current (no pending file changes or jobs) and for non-regex queries of 3+ characters, as a candidate filter in front of the same matcher, so results equal the streaming search; otherwise they stream, with a hint in the panel; a corrupt or outdated database is deleted and rebuilt; Search: Rebuild Search Index; `search.useIndex`. Templates: `.notoir/templates/*.md` (shared) and `<config>/templates/` (personal, any file); File → New File from Template… picks one, names the file inline in the Explorer (the template's extension is added when the name has none), creates it empty and fills it through the editor and the save pipeline (one undo step, caret at `{{cursor}}`); without a workspace an untitled document; `{{date}}` (`YYYY-MM-DD`) and `{{time}}` (`HH:MM`) in local time, `{{title}}` = the file name without extension. Deviations: SQLite FTS5 instead of tantivy; the link and tag index stays in memory (rebuilt from the files, not persisted in the cache).

## Purpose

Record note-oriented features that are **not** initial blockers but that the architecture must allow without
redesign: wiki links, backlinks, tags, indexed full-text search, templates. Plain files remain the source of truth;
any index is a rebuildable cache.

## Wiki Links

```markdown
We decided this in [[Architecture Decisions]].
See [[Architecture Decisions#MCP|the MCP section]].
```

- Resolution: case-insensitive match of the link target against file stems in the workspace (`Architecture
  Decisions.md`), then relative paths; ambiguity → picker; missing → "Create 'Architecture Decisions.md'?" on
  Ctrl/Cmd+click.
- Editor: link highlighted, Ctrl/Cmd+click follows, autocomplete after `[[` from the file index.
- Preview renders wiki links as links; missing targets styled distinctly (dashed underline + icon).
- Renaming a file offers to update wiki links referencing it (preview of changes, one undo step per document).

Architecture hooks already present: Markdown parser with source ranges (spec 24), file index (spec 15), workspace-wide
edit application (spec 15 Replace All), navigation history.

## Backlinks

Panel (secondary sidebar) listing documents linking to the active one with context snippets:

```text
BACKLINKS
Referenced by:
• Project Plan.md — "…decided in [[Architecture Decisions]]…"
• MCP Architecture.md
• TODO.md
Unlinked mentions (2) ▸
```

Requires the link index below.

## Tags

`#rust`, `#architecture`, `#meeting` in Markdown prose (not in code spans/blocks, not headings) and in front matter
`tags: [..]`. Tag panel lists tags with counts; clicking filters files; Quick Open `#` prefix searches tags (spec 15
reserves it).

## Indexed Full-Text Search

- Optional local index (e.g. `tantivy`) in `<cache>/index/<workspace-hash>/`, built and updated by a background worker
  from watcher events; disposable and rebuildable at any time ("Rebuild Index").
- The Search panel uses the index when available and falls back to the streaming grep search (spec 15).
- Link and tag indexes live in the same cache (SQLite or tantivy fields); never in `.notoir/`.

## Templates

`.notoir/templates/*.md` (shareable) and `<config>/templates/` (personal); "New File from Template…" with variables
`{{date}}`, `{{time}}`, `{{title}}`, `{{cursor}}`.

## Other Phase 6 Candidates

Enhanced Git sync for notes, custom themes UI, plugin marketplace, richer MCP extensions (resources, prompts,
`git_commit` with Ask), local document history (periodic snapshots per file, separate from recovery), Markdown export
(HTML/PDF), advanced JSON tree editing (spec 25), merge conflict UI, native macOS menu bar, OS recent-documents
integration, Mermaid/math rendering.

## Test Cases

```gherkin
Feature: Wiki links (Phase 6)

  Scenario: Follow a wiki link
    Given "Architecture Decisions.md" exists
    When the user Ctrl/Cmd+clicks "[[Architecture Decisions]]"
    Then "Architecture Decisions.md" opens

  Scenario: Index is disposable
    Given the search index is deleted while Notoir is closed
    When the workspace is opened
    Then search still works via streaming search while the index rebuilds
```
