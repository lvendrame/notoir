# 27 — Compare (Diff)

## Purpose

Define the built-in comparison tool: sources, the diff model, the side-by-side diff view, navigation, whitespace
handling, and copying changes between sides.

## Entry Points

- Tools → Compare Files… (`compare.open`): a small picker with two slots (Left / Right), each choosing a source.
- Toolbar Compare button: same picker, Left pre-filled with the active document.
- Tab context menu: Compare With… (Right picker, Left = this tab), Select for Compare / Compare with Selected.
- Explorer context menu: Compare With…, Select for Compare / Compare with Selected.
- `compare.currentChanges` ("Compare Current Changes"): saved disk version (left, read-only) vs current buffer (right,
  editable).
- External-change banner Compare (spec 10), recovery conflict Compare (spec 09).
- Git: Compare with HEAD, Compare with Revision…, Source Control row click (spec 28).

## Sources

```rust
pub enum CompareSource {
    Document(DocumentId),            // open tab (live buffer, editable side)
    File(PathBuf),                   // read from disk (opened as a document → editable)
    Clipboard,                       // snapshot, read-only virtual doc
    SavedVersion(DocumentId),        // disk content of an open doc, read-only virtual doc
    GitHead(PathBuf),                // HEAD blob, read-only (Phase 3)
    GitRevision(PathBuf, String),    // any commit-ish, read-only (Phase 3)
    Text { title: String, text: Arc<str> }, // internal (recovery, MCP preview)
}
```

Read-only sources become `Virtual` documents (spec 02) titled e.g. `notes.md (Saved)`, `notes.md (HEAD)`,
`Clipboard`.

## Diff Model (`notoir-editor::diff`)

- Line diff via `similar` (Patience algorithm default; Myers option), producing hunks with line ranges.
- Inline (intra-line) char/word diff for modified line pairs (`similar` `TextDiff::from_chars` limited to lines ≤ 2,000
  chars).
- Options: ignore trailing whitespace, ignore all whitespace changes, ignore line ending differences (always on —
  buffers are normalized), ignore case (palette).
- Computed on a worker from rope snapshots; recomputed on edit with 200 ms debounce; results tagged with both sides'
  revisions.
- Large inputs (> 50k lines either side) diff on a worker with progress; > `diff.maxSize` (default 50 MiB) refused with
  a message.

## Diff View

Editor-area view (`ViewKind::Diff`), opening as a tab titled `left ↔ right`:

```text
┌──────────── notes.md (Saved) ────────────┬──────────── notes.md ────────────────┐
│ 12  The editor is fast.                  │ 12  The editor is fast.               │
│ 13- It supports splits.                  │ 13+ It supports true splits.         │
│                                          │ 14+ And duplicate views.             │
│ 14  Git is optional.                     │ 15  Git is optional.                  │
└──────────────────────────────────────────┴──────────────────────────────────────┘
          ▲ synchronized scrolling, aligned hunks (filler rows), overview ruler at the far right
```

- Two editor panes reusing the editor widget (gutter, syntax highlighting, wrap) with filler rows aligning hunks.
- Synchronized vertical scrolling (and horizontal when wrap off).
- Colours: `diff.removedLine`/`insertedLine` backgrounds plus `−`/`+` gutter markers (not colour alone); inline changes
  with stronger backgrounds.
- Overview ruler shows hunk positions; click to jump.
- Contextual toolbar: Previous Change (`Shift+F7`), Next Change (`F7`), Ignore Whitespace toggle,
  Swap Sides, Inline view toggle (Phase 6).
- Editable sides (documents) can be edited in place; edits update the diff live and mark the document dirty. Read-only
  sides show the lock icon in their header.
- Hunk actions (hover on the gutter between panes): `→` copy left-to-right, `←` copy right-to-left — available only
  toward an editable side; applied as an undoable transaction on the target document. "Revert Hunk" in Compare Current
  Changes copies the saved version's hunk back into the buffer.
- Header shows "3 changes" and "Identical" when no differences ("Files are identical" info bar).
- Closing the diff tab does not close the underlying documents' other views; virtual read-only docs are discarded.

## Test Cases

```gherkin
Feature: Compare

  Scenario: Compare two open tabs
    Given tabs "old.md" and "new.md"
    When the user selects "old.md" for compare and chooses "Compare with Selected" on "new.md"
    Then a diff tab "old.md ↔ new.md" opens with differences highlighted

  Scenario: Compare current changes
    Given "notes.md" has unsaved edits
    When the user runs "Compare Current Changes"
    Then the left side shows the saved disk version read-only
    And the right side shows the current buffer editable

  Scenario: Next change navigation
    Given a diff with 3 hunks and the cursor before the first
    When the user presses Next Change three times
    Then each hunk is revealed in order

  Scenario: Ignore whitespace
    Given two files differing only in trailing spaces
    When Ignore Whitespace is on
    Then the diff shows "Files are identical"

  Scenario: Copy change to an editable side
    Given a diff of saved version (left) vs buffer (right)
    When the user reverts a hunk using the copy-left-to-right action
    Then the buffer contains the saved text for that hunk
    And undo restores the buffer change

  Scenario: No copy toward read-only side
    Given the left side is Git HEAD
    Then no right-to-left copy action is offered

  Scenario: Live update while editing
    Given an open diff of two documents
    When the user edits the right side to match the left
    Then the hunk disappears
```
