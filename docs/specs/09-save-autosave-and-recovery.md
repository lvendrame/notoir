# 09 — Save, Autosave & Recovery

## Purpose

Define three independent systems: **Save** (user writes to the real file), **Autosave** (optional automatic Save), and
**Recovery** (always-on snapshots of unsaved state into Notoir's own storage). Recovery is the first line of data-loss
prevention and is one of the earliest subsystems built (milestone M8).

## Save

Commands: `file.save`, `file.saveAs`, `file.saveAll`, `file.revert`, `file.saveWithEncoding`.

### Save pipeline

1. Snapshot the rope (revision R) on the UI thread; mark document `Saving`.
2. Worker: convert `\n` to the document's line ending, encode (BOM per document), compute hash.
3. **Conflict check**: stat + hash the file on disk; if it differs from `saved_content_hash` (changed externally since
   load/last save) → abort and return `Conflict` (handled by spec 10: "File changed on disk since you opened it.
   [Compare] [Overwrite] [Cancel]").
4. Atomic write (`files.atomicSave = true`, default):
   - write to `.<name>.notoir-tmp-<rand>` in the **same directory**,
   - `fsync` the temp file,
   - copy permissions/mode (Unix) and attributes (Windows: hidden/readonly cleared appropriately),
   - rename over the original (`ReplaceFileW` semantics on Windows to preserve ACLs/creation time),
   - `fsync` the directory (Unix).
   - If the directory is not writable or the target is a hard-linked / symlinked file where rename would break the link,
     fall back to in-place truncate+write after writing a backup copy to the recovery store.
5. Register the expected (mtime, size, hash) with the watcher's self-write suppression (spec 10).
6. Result on UI thread: if the document's revision is still R → `saved_revision = R`; otherwise `saved_revision = R`
   too (document stays dirty because revision > R). Clear recovery snapshot for revision ≤ R.
7. Failure (permission denied, disk full, path gone, read-only FS): document stays dirty; notification "Could not save
   'notes.md'. Permission denied. [Save As…] [Retry] [Show Details]". Nothing is lost: the buffer and recovery snapshot
   remain.

### Other save rules

- Save on an Untitled or Scratchpad document runs Save As.
- Save As to a path that is open in another document asks to replace; the other document is closed (prompting if dirty).
- Save All saves every dirty file-backed document; Untitled ones prompt Save As sequentially; failures are aggregated
  into one notification.
- Revert File (`file.revert`) reloads from disk after confirmation ("Discard unsaved changes to 'X'?"). It is undoable
  (the revert is a transaction).
- Trim trailing whitespace / insert final newline on save (`files.trimTrailingWhitespaceOnSave`,
  `files.insertFinalNewline`, both default false) are applied as a transaction before step 1.
- Read-only documents cannot be saved in place; Save offers Save As.
- Optional "format on save" hook (Phase 5, plugin formatters) runs before step 1 with a timeout.

## Unsaved-Changes Prompts

On closing a dirty view's last view, closing a window, or quitting:

```text
Do you want to save the changes you made to "notes.md"?
Your changes will be lost if you don't save them.        (wording is accurate: recovery is cleared on Don't Save)

[Save]  [Don't Save]  [Cancel]
```

Multiple dirty documents on quit: one dialog listing them with checkboxes → [Save Selected] [Don't Save] [Cancel].

`app.hotExit` (default **true**): on quit/window close, instead of prompting, dirty documents are kept in the recovery
store and restored silently on next launch (like Scratchpads). With hot exit off, the prompts above appear.

## Autosave

Setting `files.autoSave`:

| Mode | Behaviour |
|---|---|
| `off` (default) | Never automatic. |
| `afterDelay` | Save a dirty file-backed document `files.autoSaveDelay` ms (default 1000) after its last edit. |
| `onFocusChange` | Save when the active view changes to another document/group or a panel takes focus. |
| `onWindowFocusLoss` | Save all dirty documents when the window loses focus. |

Rules: autosave never saves Untitled documents (no path), never saves documents in `Conflict`/externally-modified
state, never saves read-only or lossy-decoded documents, and does not show success notifications. Failures show one
non-repeating warning per document until it succeeds. File → Auto Save › sets the mode at User scope; workspace scope
can override (spec 20).

## Recovery

### Goals

- Every unsaved change survives a crash, power loss or kill within a bounded window (target ≤ 2 s of typing).
- Recovery never silently overwrites an original file.
- Recovery storage is independent of the state DB, so DB corruption cannot lose recovery data (AD-6).

### Storage layout

```text
<data>/recovery/
└── <session-id>/                      # one per running window process (UUID v4)
    ├── session.lock                   # held with File::try_lock while the process runs
    ├── manifest.json                  # atomically replaced; lists entries
    └── docs/
        ├── <entry-id>.json            # metadata
        └── <entry-id>.txt             # full content, UTF-8, '\n' line endings
```

`manifest.json`:

```json
{
  "format": 1,
  "session_id": "6f0c…",
  "app_version": "0.3.0",
  "pid": 41231,
  "workspace": "/Users/me/notes",
  "updated_at": "2026-10-14T14:43:02Z",
  "clean_exit": false,
  "entries": ["e1", "e2"]
}
```

`<entry-id>.json`:

```json
{
  "format": 1,
  "kind": "file | untitled | scratchpad",
  "path": "/Users/me/notes/a.md",
  "scratch_id": null,
  "title": "a.md",
  "revision": 182,
  "base_disk_hash": "xxh3:9f…",
  "base_disk_mtime": "2026-10-14T14:20:11Z",
  "encoding": "utf-8",
  "bom": false,
  "line_ending": "lf",
  "language": "markdown",
  "content_hash": "xxh3:a1…",
  "snapshot_at": "2026-10-14T14:43:02Z"
}
```

Full-content snapshots (not diffs) — simpler and corruption-resistant. Each write is temp-file + fsync + rename;
content written before metadata; metadata lists `content_hash` so a torn pair is detected.

### Scheduling

- After any edit to a dirty document: snapshot 1.5 s after the last edit (debounce), but at least every 10 s during
  continuous typing; also on window focus loss and on app quit (hot exit).
- Only dirty file documents and all Untitled documents are snapshotted. Scratchpads use their own store (spec 08) and
  are included in the manifest by reference so restore order is preserved.
- When a document becomes clean (saved or reverted) or is closed with Don't Save, its entry is deleted.
- Work runs on the blocking pool with a rope snapshot; one in-flight snapshot per document; newer requests supersede.
- Disk budget: warn if recovery exceeds `recovery.maxSizeMb` (default 1024 MiB); never delete unsaved data to stay
  under budget.

### Startup detection

1. Create own session dir and take its lock.
2. Scan other session dirs. A dir whose `session.lock` can be acquired belongs to a dead process (crash or hot exit).
3. Hot-exit sessions (manifest has `"clean_exit": true`) for the same workspace restore silently into the new session.
4. Crash sessions trigger the recovery prompt (non-modal panel over the editor area):

```text
Notoir detected unsaved documents from your previous session.

  a.md            /Users/me/notes            2 minutes before crash
  Untitled-2                                 14 lines
  Scratch 4

[Restore All]  [Review…]  [Discard]
```

- **Restore All**: each entry opens as a dirty document with the recovered content. For file entries, if the disk file
  changed since `base_disk_hash`, the document opens in conflict state with an info bar: "This file changed on disk
  after your unsaved edits. [Compare] [Keep Recovered] [Use Disk Version]". The disk file is never written until the user
  saves.
- **Review…**: list with per-entry actions: Restore, Compare with Disk, Save As…, Discard.
- **Discard**: asks for confirmation ("Permanently discard 3 unsaved documents?"), then moves the session dir to
  `<data>/recovery/.discarded/` where it is kept for 7 days before deletion.
- Entries from sessions belonging to a different workspace are offered when that workspace is opened, plus via
  Help → Recover Unsaved Documents… at any time.
- Corrupt entries (hash mismatch, unparsable) are listed as "damaged" with "Open Raw Content" so partial text is still
  recoverable; they are never silently dropped.
- Session dirs from incompatible future formats are left untouched with a log warning.

### Suspend/resume

On resume (detected via large frame-time gap), the watcher rescans open files and recovery flushes immediately.

## Test Cases

```gherkin
Feature: Save

  Scenario: Atomic save preserves file on write failure
    Given "notes.md" is open and dirty
    And the disk is full
    When the user saves
    Then the original "notes.md" on disk is unchanged
    And the document is still dirty
    And an error notification says the file could not be saved

  Scenario: Save refuses to overwrite an externally changed file
    Given "notes.md" was changed on disk by another program after it was opened
    And the document is dirty
    When the user saves
    Then the file is not written
    And the user is offered Compare, Overwrite, and Cancel

  Scenario: Save preserves Unix permissions
    Given "run.sh" has mode 0755
    When it is edited and saved
    Then its mode is still 0755

  Scenario: Typing during save
    Given a save of revision 10 is in progress
    When the user types a character (revision 11)
    And the save completes
    Then the document is still dirty

Feature: Autosave

  Scenario: After delay
    Given autosave is "afterDelay" with 1000 ms
    When the user types and waits 1.2 seconds
    Then the file on disk contains the typed text
    And no notification is shown

  Scenario: Autosave never saves untitled documents
    Given autosave is "afterDelay"
    When the user types into "Untitled-1" and waits
    Then no save dialog appears and nothing is written outside the recovery store

Feature: Recovery

  Scenario: Crash with unsaved files
    Given "a.md" has unsaved edits and "Untitled-1" contains text
    And recovery snapshots have been written
    When the process is killed
    And Notoir is launched again
    Then the recovery prompt lists "a.md" and "Untitled-1"
    When the user chooses "Restore All"
    Then both documents open dirty with the recovered content
    And "a.md" on disk is unchanged

  Scenario: Crash with scratchpads
    Given "Scratch 2" contains "phone 555"
    When the process is killed and relaunched
    Then "Scratch 2" is open with "phone 555"

  Scenario: Conflicting recovery snapshot
    Given a recovered entry for "a.md" whose disk version changed after the snapshot
    When the user restores it
    Then the document opens in conflict state with Compare / Keep Recovered / Use Disk Version

  Scenario: Discarded recovery is kept briefly
    When the user discards recovery and confirms
    Then the session data is moved to the discarded area, not deleted immediately

  Scenario: Hot exit
    Given hot exit is on and "a.md" is dirty
    When the user quits
    Then no save prompt appears
    And on next launch "a.md" opens dirty with the same content and no recovery prompt

  Scenario: Saving clears recovery
    Given "a.md" has a recovery entry
    When the user saves "a.md"
    Then the recovery entry for "a.md" is deleted
```
