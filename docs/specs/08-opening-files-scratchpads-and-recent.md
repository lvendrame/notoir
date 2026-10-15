# 08 — Opening Files, Scratchpads & Recent

**Status:** Partial — Open File, New File, Scratchpads (create, persist, keep, Save As), recent files with pins, Open Recent, New Window, CLI and single-instance forwarding (M2, M5); Open Workspace, folder drops and CLI folders, recent workspaces with pins (M7); Quick Open recent (M9), OS recent-documents integration (M11)

## Purpose

Define the ways content enters Notoir — Open File, Open Workspace, New File, New Scratchpad, New Window, drag and drop,
CLI/OS file association — plus Scratchpad lifecycle and the Open Recent list.

## Open File

- File → Open File… (`file.open`): native multi-select file dialog (`rfd`), starting in the active document's folder,
  else the workspace root, else the last dialog folder.
- No workspace required. Files outside the workspace open normally; their tab tooltip shows the full path.
- Opening a path already open focuses the existing view in the active group, or creates a view in the active group if
  the document is only open elsewhere (`workbench.openExistingIn = activeGroup | existingGroup`, default
  `existingGroup` → focus where it already is).
- Drag and drop from the OS: files open as tabs (dock targets per spec 03); a folder opens as workspace (with the
  same prompt as spec 07).
- CLI: `notoir [paths…] [--new-window] [--goto file:line[:col]] [--wait]`. If an instance is running, the
  single-instance socket forwards the request to the most recently focused window (AD-10). `--wait` blocks until the
  opened file's tabs close (for use as `$EDITOR`/`git commit` editor).
- OS file association / "Open With" on all platforms routes to the same entry point (macOS `application:openFiles`
  via winit event, Windows/Linux argv).

## New File

`file.newUntitled`: creates an `Untitled-N` document (language plaintext, platform default line ending, UTF-8). It is
dirty as soon as non-empty; closing prompts Save / Don't Save / Cancel. First save opens Save As. Untitled documents
are covered by recovery snapshots (spec 09).

## Scratchpads

Scratchpads are named, persistent, unsaved-to-file notes — the "notepad" use case.

- `file.newScratchpad`: creates "Scratch N" (N = lowest unused positive integer), language from
  `scratchpad.defaultLanguage` (default `markdown`).
- Content is persisted continuously to the Scratchpad store in app data (`<data>/scratchpads/<scratch_id>.txt` +
  metadata row in the state DB), debounced 500 ms after edits and on exit. This is **not** autosave to a user file and
  works even with autosave Off.
- They survive restarts and crashes; they reopen in their previous tab positions.
- Rename: double-click tab title or "Rename Scratchpad…" — changes title only.
- Save As… converts it into a file-backed document (the Scratchpad entry is then removed from the store after the file
  write succeeds).
- Closing a non-empty Scratchpad tab asks: "Keep Scratchpad" (closes the tab, content stays in the store and is listed
  under File → Open Recent → Scratchpads) / "Save As…" / "Delete" / Cancel. Closing an empty Scratchpad deletes it
  silently.
- Scratchpads are global (not per workspace) and listed in Quick Open under a "Scratchpads" section.

## New Window

`file.newWindow` launches a new Notoir process with an empty session (AD-10). Each window persists its own session
(spec 23).

## Open Recent

```text
File
 ├─ Open Recent
 │    ├─ 📌 Personal Notes            (pinned workspaces)
 │    ├─ StringArtIt                  (recent workspaces, max 10 shown)
 │    ├─ ──────────
 │    ├─ 📌 todo.md                   (pinned files)
 │    ├─ notes/meeting-2026-10-01.md  (recent files, max 10 shown)
 │    ├─ ──────────
 │    ├─ Scratchpads ›                (kept Scratchpads not open in a tab)
 │    ├─ More… (opens Quick Open filtered to recent)
 │    └─ Clear Recently Opened
```

Pins render as the lucide `pin` icon, not emoji.

- Model: `RecentEntry { kind: File | Workspace, path, display_name, last_opened: timestamp, pinned: bool }` stored in
  the state DB (spec 23), max 50 entries per kind (pinned entries do not count against the limit).
- Recorded on successful open (and Save As for files). Moved to top on reopen.
- Entries whose path no longer exists are shown disabled with "(missing)" and removed on click after a notice
  ("'X' no longer exists. Remove from list?").
- Pin/unpin from the empty-state Recent list or via the Recent list in Quick Open (context menu / `Mod+Enter`).
- Clear Recently Opened removes unpinned entries after confirmation; pinned entries stay.
- Shared across windows: each window reads the list when its menu opens; writes are atomic DB transactions.
- OS integration (later milestone): macOS Dock recent documents, Windows Jump List, Linux `recently-used.xbel`.

## Test Cases

```gherkin
Feature: Open file without workspace

  Scenario: Open a single file
    Given no workspace is open
    When the user opens "/tmp/todo.txt"
    Then "todo.txt" opens in a tab and is editable
    And the Explorer shows "No folder opened" with an "Open Workspace…" button

Feature: Scratchpads

  Scenario: New scratchpad numbering
    Given "Scratch 1" and "Scratch 3" exist
    When the user creates a new Scratchpad
    Then it is titled "Scratch 2"

  Scenario: Scratchpad survives restart without autosave
    Given autosave is Off
    And the user typed "buy milk" into "Scratch 1"
    When the user quits and relaunches Notoir
    Then "Scratch 1" is open and contains "buy milk"
    And no save prompt was shown on quit

  Scenario: Save As converts a Scratchpad into a file
    Given "Scratch 1" contains "idea"
    When the user chooses Save As and saves to "idea.md"
    Then the tab title is "idea.md" and the file contains "idea"
    And "Scratch 1" no longer exists in the Scratchpad store

  Scenario: Closing a non-empty scratchpad offers to keep it
    Given "Scratch 1" contains text
    When the user closes its tab and chooses "Keep Scratchpad"
    Then the tab closes
    And "Scratch 1" is listed under Open Recent → Scratchpads

Feature: Recent

  Scenario: Pinned workspaces stay on top
    Given "Personal Notes" is pinned and "StringArtIt" was opened more recently
    Then Open Recent lists "Personal Notes" before "StringArtIt"

  Scenario: Clear keeps pinned
    Given one pinned and three unpinned recent workspaces
    When the user chooses "Clear Recently Opened" and confirms
    Then only the pinned workspace remains

  Scenario: Missing recent file
    Given recent file "gone.md" was deleted from disk
    When the user selects it in Open Recent
    Then a notice offers to remove it from the list
```
