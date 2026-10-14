# 35 — Logging & Error Handling

**Status:** Partial — structured logging, panic hook (M0); user errors as notifications (M3); worker panic isolation (M5)

## Purpose

Define structured logging (what, where, privacy), error types and how errors reach users, and panic handling. Notoir
never panics for recoverable errors and never hides important errors only in logs.

## Logging

- `tracing` + `tracing-subscriber` with a rolling file appender (`tracing-appender`, daily rotation, keep 7 files,
  max 20 MiB each) in the logs directory (spec 31), plus stderr in debug builds.
- Format: one JSON object per line (`timestamp`, `level`, `target`, `span`, `message`, fields) for machine readability;
  Help → View Logs opens the current log file in a read-only Notoir tab (language `log`, with level highlighting) and
  offers "Reveal Log Folder".
- Levels: default `info`; `logging.level` setting and `NOTOIR_LOG` env var (`EnvFilter` syntax) override.
- Covered events: startup (version, OS, GPU backend, paths, timings), shutdown, crashes/panics, workspace loading and
  scan stats, watcher failures, save failures (path + error kind), recovery writes/restores/failures, Git operations
  (command, duration, exit code; stderr at debug), MCP server lifecycle and requests (client, tool, target path,
  outcome, duration), plugin lifecycle and errors (with Lua traceback), settings parse errors, migrations, update checks.
- **Privacy**: document contents, clipboard contents, search queries, commit messages and MCP argument payloads are
  never logged at `info`/`warn`/`error`. Paths are logged (needed for support). `logging.redactPaths = true` replaces
  home-directory prefixes with `~` and hashes other path components for users who share logs publicly.
- Log viewer shows a one-line privacy notice: "Logs do not contain document contents."

## Error Model

- Library crates define typed errors with `thiserror` per domain: `SaveError`, `LoadError`, `RecoveryError`,
  `SettingsError`, `GitError`, `McpError`, `PluginError`, `FsOpError`, `StorageError` — each variant carrying structured
  data (path, OS error kind, git exit code, line/column).
- The app maps errors to `UserError { key: &'static str /* errors:… */, args, detail: Option<String>, actions }`
  in one place per domain (`apps/notoir/src/errors/*.rs`), so translation never depends on error message strings
  (spec 22).
- `anyhow` only at binary edges (startup, CLI).
- Every `Result` from a user-initiated action ends in either success or a visible `UserError` notification/info bar —
  enforced in code review and by a clippy-friendly convention: handlers return `CommandOutcome`, never `()`.

Examples (English source strings):

| Situation | Message | Actions |
|---|---|---|
| Save permission denied | Could not save "notes.md". Permission denied. | Save As…, Retry, Show Details |
| Disk full | Could not save "notes.md". The disk is full. | Retry, Show Details |
| Plugin failure | Plugin "Example" encountered an error and was disabled. | View Error, Re-enable |
| Git auth | Push failed. Authentication required. | Show Git Output, Retry |
| Settings parse | Your settings file has an error on line 12. Previous settings are still in effect. | Open settings.toml |
| MCP port busy | MCP server could not start: port 47823 is in use. Using 47824. | (info) |

## Panics

- `panic = "unwind"` (default). A panic hook logs the panic with backtrace, flushes recovery snapshots synchronously
  for all dirty documents (best effort, using only already-captured rope snapshots), writes a crash marker, and lets the
  process exit. Next launch shows the recovery prompt (spec 09) plus "Notoir closed unexpectedly. [Open Log] [Report
  Issue]".
- Worker threads catch panics (`catch_unwind` at job boundaries) and convert them into failed `JobResult`s; a panicking
  job never brings down the app.
- Plugin host threads catch panics in API glue and disable the plugin.
- `debug_assert!` for invariants in hot paths; `unwrap`/`expect` only where the invariant is local and documented;
  clippy `unwrap_used` warn in non-test code of library crates.

## Report Issue

Help → Report Issue opens the issue tracker URL with a prefilled template containing app version, OS, GPU backend and
locale — never logs or document content unless the user attaches them manually.

## Test Cases

```gherkin
Feature: Logging privacy

  Scenario: Document content never logged
    Given logging level is info
    When the user edits, saves, searches, and an MCP client reads a document
    Then the log file contains none of the document text, search query, or MCP payload

Feature: Error surfacing

  Scenario: Save error reaches the user
    When saving fails because the file is read-only
    Then an error notification with Save As… is shown
    And the log contains the save error with the path and error kind

Feature: Panic resilience

  Scenario: Worker panic does not crash the app
    Given a workspace search job panics
    Then the Search panel shows "Search failed" with Show Details
    And the editor keeps working

  Scenario: Main-thread panic preserves unsaved work
    Given "a.md" has unsaved edits
    When a panic occurs on the UI thread
    Then the next launch offers to restore "a.md"
```
