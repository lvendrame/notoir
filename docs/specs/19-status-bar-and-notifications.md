# 19 — Status Bar & Notifications

**Status:** Partial — status bar shell, notifications (M3); document readouts (M4); encoding/line-ending/language pickers from the status bar (M5); Go to Line and indentation pickers (M9, M10)

## Purpose

Define the status bar items and their click actions, background-activity indication, and the notification (toast)
system including error presentation.

## Status Bar

24 px tall, `--bg-app`, 12 px text; numeric readouts in JetBrains Mono (spec 21).

```text
⎇ main ↑1 ↓2  ● 2 modified  ⟳ Indexing…        Ln 52, Col 18 │ Spaces: 4 │ UTF-8 │ LF │ Markdown │ 125% │ MCP ●2  🔔
└──────── left: workspace/global ─────────┘   └──────────────── right: active document ────────────────┘
```

| Item | Shown when | Click |
|---|---|---|
| Branch `⎇ main` | workspace is a Git repo (Phase 3) | Branch picker (switch/create) |
| Sync `↑1 ↓2` | repo has upstream | Sync menu: Pull, Push, Sync, Fetch |
| `● 2 modified` | ≥ 1 dirty document | Picker of dirty documents; "Save All" action |
| Background activity (spinner + label) | jobs running > 300 ms (indexing, search, Git network ops, plugin loading) | Popover listing jobs with Cancel |
| `Ln 52, Col 18` (+ `(12 selected)`) | text view active | Go to Line |
| Indentation `Spaces: 4` / `Tab Size: 4` | text view | Picker: Indent Using Spaces/Tabs, Change Tab Size, Detect, Convert Indentation |
| Encoding `UTF-8` (`UTF-8 BOM`) | file-backed text view | Reopen with Encoding / Save with Encoding |
| Line ending `LF` / `CRLF` | text view | Picker LF / CRLF / CR |
| Language `Markdown` | text view | Language mode picker; Word Wrap toggle; "Configure file association for '.ext'" |
| Zoom `125%` | zoom ≠ 100 % | Zoom popover |
| Read-only lock | document read-only | Toggle user read-only (if not FS read-only) |
| Large file `⚡` | large-file mode | Explanation popover |
| Recovery state | only when snapshots are failing: amber `shield-alert` icon (healthy state shows nothing) | Details + "Open Recovery Folder" |
| `MCP ●2` | MCP server running (Phase 4) | MCP Connections/Activity popover |
| Bell | always | Notification centre |

- Items are commands with args; right-click on the status bar lists items with checkboxes to hide/show each
  (`statusBar.items.<id>.visible`).
- Plugins may add items (Phase 5) on left or right with priority ordering.
- Autosave/recovery state is subtle: no flashing on routine snapshots.
- `View → Appearance → Status Bar` hides the bar.

## Notifications

```rust
pub struct Notification {
    pub id: NotificationId,
    pub severity: Severity,             // Info | Success | Warning | Error
    pub message: Translatable,          // key + args, rendered at display time
    pub detail: Option<String>,         // technical detail (untranslated diagnostics allowed), behind "Show Details"
    pub actions: Vec<NotificationAction>, // label key + CommandInvocation
    pub source: NotificationSource,     // App | Git | Mcp | Plugin(id) | Recovery
    pub sticky: bool,                   // errors with actions stay until dismissed
    pub dedupe_key: Option<String>,     // identical keys collapse with a ×N counter
}
```

- Toasts stack bottom-right above the status bar, max 3 visible, width 360 px, `--shadow-float`.
- Auto-dismiss: Info/Success 4 s, Warning 8 s, Errors sticky. Hovering pauses the timer.
- All notifications go to the Notification Centre (bell) for the session; errors also go to the log (spec 35).
- No success toasts for routine actions (manual save shows no toast; the tab dot clears). Success toasts only for
  completed long or remote operations (Git push complete, plugin installed, MCP server started).
- Zen Mode: only Warning/Error toasts.
- Examples:
  - "Could not save 'notes.md'. Permission denied." [Save As…] [Retry] [Show Details]
  - "Plugin 'Example' encountered an error and was disabled." [View Error] [Re-enable]
  - "Push failed. Authentication required." [Show Git Output] [Retry]
  - "MCP client 'Claude Desktop' wants to delete 'old.md'." → this is a permission prompt, not a toast (spec 32).
- Notifications are rendered with translated text at display time so a language switch re-renders them.

## Test Cases

```gherkin
Feature: Status bar

  Scenario: Clicking the cursor position opens Go to Line
    When the user clicks "Ln 52, Col 18"
    Then the Go to Line input opens

  Scenario: Changing line endings from the status bar
    Given a document with LF line endings
    When the user clicks "LF" and chooses "CRLF"
    Then the status bar shows "CRLF" and the document is dirty

  Scenario: Zoom item visibility
    Given zoom is 100 %
    Then no zoom item is shown
    When the user zooms in
    Then "110%" is shown

Feature: Notifications

  Scenario: Routine save shows no toast
    When the user saves a document successfully
    Then no notification appears

  Scenario: Save failure is sticky with actions
    When a save fails with permission denied
    Then an error toast with "Save As…", "Retry", and "Show Details" stays until dismissed

  Scenario: Duplicate notifications collapse
    Given the same autosave failure occurs three times
    Then one toast shows the message with "×3"
```
