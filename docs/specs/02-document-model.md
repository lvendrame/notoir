# 02 — Document Model

**Status:** Partial — document model, encodings, line endings, language detection (M1); file loading with binary/lossy/size handling, indentation detection, encoding and line-ending changes (M5); large-file mode effects on minimap/syntax/preview (M9, M12, M13)

## Purpose

Define `Document`: the in-memory representation of an open file or Scratchpad, independent of any tab or view. Covers
identity, buffer, dirty state, revisions, encoding, line endings, language detection, read-only state and large-file
mode.

## Data Model

```rust
pub struct DocumentId(u64);              // process-unique, never reused in a session

pub struct Document {
    pub id: DocumentId,
    pub origin: DocumentOrigin,
    pub buffer: TextBuffer,              // notoir-editor: rope + undo history
    pub revision: u64,                   // +1 on every applied transaction (incl. undo/redo)
    pub saved_revision: Option<u64>,     // revision matching disk content; None = never saved
    pub saved_content_hash: Option<u64>, // xxh3 of last loaded/saved bytes
    pub disk_state: DiskState,           // InSync | ModifiedExternally | DeletedExternally | Unknown
    pub encoding: TextEncoding,          // Utf8 { bom }, Utf16Le { bom }, Utf16Be { bom }, Other(&'static Encoding)
    pub line_ending: LineEnding,         // Lf | CrLf | Cr
    pub mixed_line_endings: bool,
    pub language: LanguageId,            // "plaintext", "markdown", "json", …
    pub language_overridden: bool,       // user picked language from status bar
    pub read_only: ReadOnly,             // No | FileSystem | User | LargeFile
    pub large_file: bool,
    pub save_state: SaveState,           // Idle | Saving(job) | Failed(err)
}

pub enum DocumentOrigin {
    File { path: PathBuf },              // canonical absolute path
    Scratchpad { scratch_id: ScratchId, title: String }, // "Scratch 1"
    Untitled { n: u32 },                 // File → New File before first save: "Untitled-1"
    Virtual { kind: VirtualKind },       // Settings tab, compare result, Git revision (read-only)
}
```

`is_dirty() = saved_revision != Some(revision)` for File/Untitled; Scratchpads are never "dirty" in the save-prompt
sense because they are persisted automatically (spec 08), but they show a subtle "unsaved to file" marker.

## Identity Rules

- At most one `Document` per canonical file path. Opening an already-open path focuses or creates a view on the existing
  document; it never loads a second copy.
- Path canonicalization: absolute, `.`/`..` resolved, symlinks **not** resolved for display but resolved for identity
  (two symlinks to one file share a document). On Windows, comparison is case-insensitive and uses long-path form; on
  macOS case-insensitivity follows the volume (detected via `pathconf`-equivalent check), default case-insensitive; on
  Linux case-sensitive.
- `DocumentRegistry` keeps a reference count of views per document. When the last view closes, the document is closed
  (after the dirty prompt) unless it is held by another consumer (e.g. an MCP `document.open` with `keepAlive`,
  spec 29) — such holds are listed in the MCP activity panel.

## Loading

1. Read bytes off the UI thread (inline if ≤ 1 MiB, otherwise a worker; the tab shows a loading shimmer).
2. Detect encoding: BOM → UTF-8/UTF-16; else valid UTF-8 → UTF-8; else `chardetng` guess; `files.encoding` setting may
   force a default for guess-only files.
3. Decode with `encoding_rs`. If decoding is lossy (replacement chars introduced), open **read-only** with a banner:
   "This file was opened with encoding X and contains invalid characters. [Reopen with Encoding…]". Saving a lossy
   decode is never allowed without explicit "Save with Encoding…" choice.
4. Detect line endings: count `\r\n`, `\n`, `\r`. Majority wins; ties → platform default (`CRLF` Windows, `LF` else).
   If more than one style appears, set `mixed_line_endings` and show a non-blocking info bar: "Mixed line endings —
   file will be saved with LF. [Keep LF] [Use CRLF]".
5. Normalize to `\n` in the rope (AD-13).
6. Detect language: user override → workspace `files.associations` → user `files.associations` → extension → shebang /
   first-line heuristics (`#!/usr/bin/env python`, `<?xml`, `{`) → plaintext.
7. Determine read-only: file system permissions → `ReadOnly::FileSystem`.
8. Large-file check (below).

Binary detection: if the first 8 KiB contain NUL bytes (and are not valid UTF-16), show "This file appears to be binary.
[Open Anyway] [Cancel]"; Open Anyway opens read-only.

## Encoding and Line Ending Changes

- Status bar encoding item → "Reopen with Encoding…" (reloads from disk; prompts if dirty) or "Save with Encoding…".
- Status bar line-ending item → LF / CRLF / CR. Changing it marks the document dirty (it changes saved bytes) but does
  not alter the rope. It is not yet recorded in undo history (metadata edits are a later refinement).
- BOM presence is preserved on save unless changed via "Save with Encoding…".

## Large-File Mode

Entered when any threshold is exceeded (all configurable under `files.largeFile.*`):

| Threshold | Default |
|---|---|
| Size | 20 MiB |
| Line count | 300,000 |
| Longest line | 20,000 chars |

Effects: syntax highlighting off, minimap off, outline/breadcrumb symbols off, live Markdown preview off, folding
limited to indentation, word wrap off by default (can be re-enabled), workspace-search indexing of the file skipped.
A dismissible info bar explains it: "Large file mode: some features are turned off to keep editing fast. [Learn more]".
Files above `files.largeFile.maxEditableSize` (default 512 MiB) open read-only; files above
`files.largeFile.maxOpenSize` (default 2 GiB) are refused with a clear message.

## Revisions and Snapshots

- `revision` monotonically increases; workers tag results with the revision they computed against and the UI discards
  results whose revision no longer matches when that matters (e.g. JSON tree ranges).
- `Document::snapshot() -> DocSnapshot { id, revision, rope: Rope, language, path }` is cheap and `Send`.

## Test Cases

```gherkin
Feature: Document identity

  Scenario: Opening an open file reuses the document
    Given "notes.md" is open in the left group
    When the user opens "notes.md" again from the Explorer while the right group is active
    Then a new view is created in the right group
    And both views reference the same document id
    And typing in one view is visible in the other

  Scenario: Two paths to the same file share a document
    Given "a.txt" is a symlink to "b.txt"
    When the user opens both
    Then only one document exists

Feature: Dirty state

  Scenario: Editing marks dirty and undo to saved revision clears it
    Given "notes.md" is open and saved
    When the user types "x"
    Then the document is dirty
    When the user undoes
    Then the document is not dirty

Feature: Encoding

  Scenario: UTF-16 file with BOM round-trips
    Given a UTF-16LE file with BOM containing "héllo"
    When it is opened, edited to "héllo!", and saved
    Then the file on disk is UTF-16LE with BOM containing "héllo!"

  Scenario: Lossy decode opens read-only
    Given a file whose bytes cannot be decoded without replacement characters
    When it is opened
    Then the document is read-only
    And a banner offers "Reopen with Encoding…"

Feature: Line endings

  Scenario: CRLF preserved
    Given a file with CRLF line endings
    When a line is added and the file is saved
    Then every line ending on disk is CRLF

  Scenario: Mixed line endings are reported, not silently changed
    Given a file with 10 LF and 2 CRLF line endings
    When it is opened
    Then the status bar shows "LF"
    And an info bar reports mixed line endings
    And the file is unchanged on disk until the user saves

Feature: Large files

  Scenario: Large-file mode disables expensive features
    Given a 50 MiB log file
    When it is opened
    Then syntax highlighting and the minimap are off for that document
    And an info bar explains large-file mode
```
