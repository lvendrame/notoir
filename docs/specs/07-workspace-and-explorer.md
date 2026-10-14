# 07 — Workspace & Explorer

**Status:** Not started — M7

## Purpose

Define the workspace model (any directory, no mandatory metadata), the optional `.notoir/` directory, and the Explorer
panel: tree, decorations, filter, file operations and context menus.

## Workspace Model

- Any directory can be opened as a workspace via File → Open Workspace… (folder dialog), drag-and-drop of a folder,
  Open Recent, or CLI `notoir <dir>`.
- One workspace per window. Opening another workspace in a window with one already open asks: "Open in This Window /
  New Window / Cancel" (default New Window when the current window has dirty documents).
- Opening a workspace **never** creates files. `.notoir/` is created only when the user saves a Workspace-scope
  setting, installs a workspace plugin list, or another feature needs shareable config.

```text
.notoir/
├── settings.toml     # workspace-scope settings (spec 20)
└── plugins.toml      # recommended/required plugins for this workspace (spec 30)
```

- Never stored in `.notoir/`: open tabs, cursors, scroll, layout, window position, panel sizes/visibility, recovery
  data, MCP grants. Those live in app data (spec 23), keyed by the workspace's canonical path.
- `WorkspaceState { root, name, trusted: bool, tree: ExplorerTree, settings: WorkspaceSettings, git: Option<RepoInfo> }`.
- **Workspace trust**: a workspace's `.notoir/settings.toml` and `.notoir/plugins.toml` can come from a cloned repo.
  Settings are applied, but settings marked `restricted` (e.g. plugin auto-enable, MCP auto-start, `git.autoPush`)
  are ignored until the user marks the workspace trusted ("This workspace contains settings that affect plugins/MCP/Git.
  [Trust Workspace] [Not now]").
- Close Workspace (File menu) closes all views of files in that workspace (dirty prompts), clears the Explorer, keeps
  Scratchpads and outside-workspace files open.

## Explorer Tree

```rust
pub struct ExplorerNode { pub path: PathBuf, pub name: String, pub kind: NodeKind /* File | Dir | Symlink */, pub children: ChildrenState /* NotLoaded | Loading | Loaded(Vec<NodeId>) */, pub git: Option<GitDecoration>, pub hidden: bool, pub ignored: bool }
```

- Lazy: directory children are listed when first expanded (off the UI thread); the root is listed on open. A full
  background index (for Quick Open/search, spec 15) runs separately.
- Sorting: folders first, then files, natural case-insensitive order (`file2` < `file10`), configurable
  (`explorer.sortOrder = default | mixed | modified`).
- Hidden files (dotfiles; Windows hidden attribute) shown when `explorer.showHidden` (default false). `.notoir/`
  follows `showHidden` like any other dot directory.
- Excluded patterns: `files.exclude` globs (default `**/.git`, `**/.DS_Store`, `**/Thumbs.db`) are never shown.
  `.gitignore`d files are shown dimmed by default (`explorer.dimGitIgnored = true`), hidden when
  `explorer.showGitIgnored = false`.
- Rendering is virtualized: only visible rows are laid out (target: 100k-node trees scroll at 60 fps).
- The tree updates from the file watcher (spec 10): creations, deletions and renames appear without manual refresh.
- Active file reveal: `explorer.autoReveal = true` selects and scrolls to the active document's node.
- Expanded directories are remembered per workspace (spec 23).

## Header & Toolbar

```text
EXPLORER                         [file-plus] [folder-plus] [refresh-cw] [chevrons-down-up]  ⋯
[ filter files…                                   ]
▾ my-notes
```

- Toolbar actions (icon buttons with tooltips, shown on panel hover): New File, New Folder, Refresh, Collapse All.
- `⋯` menu: Show Hidden Files (toggle), Show Git-Ignored Files (toggle), Sort Order ›, Open Editors section (toggle,
  default off), panel move/hide items (spec 17).
- Filter box (shown with `Mod+Alt+F` in Explorer or by typing while the tree has focus): fuzzy filter over the loaded
  index; matching nodes and their ancestors stay visible, matches highlighted.

## Git Decorations

Shown when the workspace (or a subfolder) is a Git repo (spec 28). Letter badge on the right **and** label colour:

| Status | Badge | Colour token |
|---|---|---|
| Modified | M | `git.modified` |
| Added (staged new) | A | `git.added` |
| Untracked | U | `git.untracked` |
| Deleted | D | `git.deleted` (shown in parent as count) |
| Conflicted | ! | `git.conflict` |
| Ignored | — | dimmed text only |
| Renamed | R | `git.renamed` |

Folders containing changes show a small dot in the strongest child colour (conflict > modified > added > untracked).

## Context Menus

File:

```text
Open
Open to the Side
──────────────
Rename…                F2
Duplicate
Delete                 Del / Cmd+Backspace
──────────────
Copy
Copy Path
Copy Relative Path
──────────────
Compare With…
Compare with Selected  (after "Select for Compare")
──────────────
Reveal in File Explorer / Finder / File Manager
──────────────
Git ›   Stage · Unstage · Discard Changes… · Compare with HEAD · File History   (only in repos)
```

Folder:

```text
New File…
New Folder…
──────────────
Rename…
Duplicate
Delete
──────────────
Copy Path
Copy Relative Path
Reveal in File Explorer / Finder / File Manager
──────────────
Find in Folder…        (workspace search scoped to folder)
```

Empty area: New File…, New Folder…, Refresh, Collapse All.

## File Operations

All go through `notoir-workspace::fs_ops` on a worker, then update the tree and any open documents:

- **New File / New Folder**: inline name editor in the tree at the target folder; validation for invalid characters
  per OS, reserved names on Windows (`CON`, `NUL`, …), and existing names. New file opens in the editor.
- **Rename**: inline editor (selects stem, not extension). Renaming an open file updates its document's path and every
  tab; renaming a folder updates all open documents below it. Case-only renames work on case-insensitive filesystems
  (two-step rename).
- **Duplicate**: `name copy.ext`, `name copy 2.ext`, …; folders copied recursively on a worker with progress.
- **Delete**: moves to OS trash (`trash` crate) after confirmation ("Move 'notes.md' to Trash?"); if trash is
  unavailable, offers permanent delete with a stronger warning. Open documents of deleted files are marked deleted on
  disk and keep their buffer (dirty) — no data loss.
- **Copy** (file): copies the file reference to the clipboard so Paste in Explorer duplicates it into the target folder;
  also puts the path as text.
- **Drag and drop** within the tree moves files (with confirmation setting `explorer.confirmDragAndDrop = true`);
  Alt/Option-drag copies. Dropping OS files into the tree copies them in.
- Every destructive or bulk operation is undoable via `explorer.undo` for the last operation where feasible (rename,
  move, create); delete relies on OS trash.

## Test Cases

```gherkin
Feature: Workspace opening

  Scenario: Opening a workspace creates no metadata
    Given a directory "notes" without a ".notoir" folder
    When the user opens it as a workspace
    Then the Explorer lists its files
    And no ".notoir" directory exists in "notes"

  Scenario: Personal UI state stays out of the workspace
    Given workspace "notes" is open with three tabs and a split
    When the user quits Notoir
    Then nothing under "notes/.notoir" has changed
    And the tabs and split are restored on next open of "notes"

  Scenario: Restricted workspace settings require trust
    Given ".notoir/settings.toml" sets "mcp.autoStart = true"
    When the untrusted workspace is opened
    Then the MCP server does not start
    And a notice offers "Trust Workspace"

Feature: Explorer

  Scenario: Hidden files toggle
    Given the workspace contains ".env" and "README.md"
    Then only "README.md" is listed
    When the user enables "Show Hidden Files"
    Then ".env" is listed

  Scenario: Renaming an open file keeps the editor
    Given "draft.md" is open and dirty
    When the user renames it to "final.md" in the Explorer
    Then the tab reads "final.md", is still dirty, and contains the unsaved text

  Scenario: Delete moves to trash and keeps open buffer
    Given "old.md" is open
    When the user deletes "old.md" from the Explorer and confirms
    Then the file is in the OS trash
    And the tab is marked deleted on disk and keeps its content

  Scenario: Invalid file name
    Given the app runs on Windows
    When the user creates a new file named "CON"
    Then the inline editor shows a validation error and no file is created

  Scenario: External file creation appears automatically
    Given the workspace is open
    When another program creates "inbox/today.md"
    Then "today.md" appears under "inbox" within one second when "inbox" is expanded
```
