# 10 — External Changes & File Watching

**Status:** Not started — M8

## Purpose

Detect and safely handle changes made to files outside Notoir (other editors, Git, scripts, sync tools, MCP
`filesystem_write`), and keep the Explorer tree current.

## Watcher

- `notify` recommended watcher (FSEvents on macOS, ReadDirectoryChangesW on Windows, inotify on Linux) wrapped by
  `notify-debouncer-full` (debounce 200 ms; rename tracking).
- Watched: workspace root recursively (excluding `files.watcherExclude`, default `**/.git/objects/**`,
  `**/node_modules/**`, `**/target/**`), plus the parent directory of every open file outside the workspace
  (watching the directory, not the file, survives atomic replace-by-rename).
- Linux inotify watch limits: on `ENOSPC`, fall back to watching open files' directories only and show a one-time
  warning with a link to docs about `fs.inotify.max_user_watches`.
- Watcher failures are logged and surfaced once; a polling fallback (`files.watcherPolling = auto|on|off`, 2 s interval
  on open files) is used for network drives where native events are unreliable.

## Change Classification

For each debounced event touching an open document's path:

1. Stat the path. Missing → `DeletedExternally` (re-check after 500 ms to tolerate write-temp-rename sequences).
2. Present: if (mtime, size) match the expected self-write record → ignore (our own save).
3. Otherwise read and hash content off-thread. Hash equal to `saved_content_hash` → ignore (touch only).
4. Hash differs → `ModifiedExternally`.

Events for paths not open only update the Explorer tree and Git status scheduler.

## Handling

| Document state | External modification | External deletion |
|---|---|---|
| Clean | Reload automatically when `files.reloadCleanOnExternalChange = true` (default). Reload is a transaction (undoable) preserving cursor/scroll by line mapping. Tab flashes the refresh indicator briefly. If false → banner like dirty case. | Tab marked deleted; content kept; document becomes dirty (unsaved relative to disk); banner: "'X' was deleted on disk. [Save] [Close]". |
| Dirty | Never reload silently. Banner in each view of that document: see below. Tab shows the externally-modified icon. | Same as clean deletion; edits kept. |

Banner (non-modal, inside the editor above the text):

```text
⟳ README.md was changed outside Notoir.            [Compare]  [Reload]  [Keep My Version]
```

- **Compare**: opens a diff (disk version left, buffer right, spec 27) with the banner still available.
- **Reload**: confirms ("Discard your unsaved changes and load the disk version?"), then reloads as an undoable
  transaction — the user can still Undo back to their version.
- **Keep My Version**: dismisses; the document stays dirty and records the new disk hash as the conflict base so the
  next Save does not re-trigger the conflict prompt (the user explicitly chose to overwrite).
- If the file reappears after deletion (e.g. Git checkout), state moves back to modified/in-sync accordingly.
- MCP-originated filesystem writes (spec 29) use this same path; the banner shows "changed by MCP client 'X'".
- Git operations that change open files (checkout, pull) trigger the same handling; clean files reload.

## Read-Only Transitions

If an open file becomes read-only (permissions), the tab shows the lock icon at next stat; editing remains allowed in
the buffer but Save routes to Save As with an explanation.

## Workspace Disappears

If the workspace root is deleted/unmounted: Explorer shows "The workspace folder is no longer available. [Locate…]
[Close Workspace]"; open documents remain with their buffers; saves fail gracefully. The watcher re-attaches if the
path reappears (polling 5 s).

## Test Cases

```gherkin
Feature: External modification

  Scenario: Clean file reloads automatically
    Given "README.md" is open and saved
    When another program rewrites "README.md"
    Then the editor shows the new content within one second
    And Undo restores the previous content

  Scenario: Dirty file is never silently reloaded
    Given "README.md" is open with unsaved edits
    When another program rewrites "README.md"
    Then the buffer still contains the unsaved edits
    And a banner offers Compare, Reload, and Keep My Version

  Scenario: Editors that save via temp file and rename
    Given "notes.md" is open and clean
    When another editor writes "notes.md.tmp" and renames it over "notes.md"
    Then the document is reloaded once
    And it is not marked as deleted

  Scenario: Own saves are ignored
    When the user saves "notes.md"
    Then no external-change banner appears

  Scenario: Touch without content change
    When another program updates only the modification time of "notes.md"
    Then nothing changes in the editor

  Scenario: External deletion keeps content
    Given "draft.md" is open
    When another program deletes "draft.md"
    Then the tab is marked deleted and the content remains
    And Save recreates the file
```
