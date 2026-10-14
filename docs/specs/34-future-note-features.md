# 34 — Future Note Features (Phase 6)

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
