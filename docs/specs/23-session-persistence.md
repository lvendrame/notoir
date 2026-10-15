# 23 — Session & State Persistence

**Status:** Partial — SQLite state store and migrations (M2); per-workspace and no-workspace sessions (layout tree, tabs with selections/scroll/pin/wrap/zoom, panels, Explorer expanded folders, zoom) written from the IO thread every 2 s/focus loss/exit, restore on launch and on opening a workspace, missing files in one notice, window geometry (M8). Later: lazy loading of inactive tabs (all restored tabs load in the background now), search panel state (M11), bookmarks (M15), navigation history

## Purpose

Define what transient state Notoir remembers, where it is stored, the storage technology choice, migrations and
corruption handling. Settings (spec 20) and recovery (spec 09) are separate stores.

## What Is Remembered

Per workspace (keyed by canonical root path) and for the "no workspace" session:

- Open documents (paths, Scratchpad ids, Untitled recovery entry ids) and their views
- Editor groups, layout tree, split fractions, active group, active tab per group, pinned tabs, MRU order
- Per-view cursor/selections, scroll position, folds, per-view zoom/wrap overrides, preview split mode/ratio
- Panel layout: areas, placement, visibility, sizes, active panels, primary side, maximized state
- Explorer expanded directories (capped at 500 paths), Explorer scroll
- Search panel last query and toggles
- Bookmarks per file path
- Navigation history (optional, `workbench.restoreNavigationHistory = false` by default)

Global:

- Window geometry per monitor configuration (position, size, maximized, full screen), last workspace per window
- Recent files/workspaces + pins (spec 08)
- Scratchpad metadata (spec 08; content files in `<data>/scratchpads/`)
- Command Palette recently used
- Global zoom level
- MCP client registrations, permission grants and activity log (spec 29/32)
- Plugin install approvals (spec 30)
- Dismissed one-time notices, first-run completion

Never stored in `.notoir/` (spec 07).

## Storage Choice (AD-6)

**SQLite via `rusqlite` (bundled)** at `<data>/state.db`, WAL mode, `synchronous = NORMAL`.

Rationale:

| Criterion | SQLite | Single JSON/TOML state file | Embedded KV (redb, sled) |
|---|---|---|---|
| Corruption resistance | Journaled, atomic transactions; survives crashes mid-write | Needs careful temp+rename; whole-file rewrite on every change | Good (redb), sled less mature |
| Concurrent windows (processes) | Built-in locking, multiple readers/one writer | Last writer wins, races between processes | redb single-process lock |
| Migrations | `PRAGMA user_version` + SQL migrations, well understood | Ad-hoc schema versioning | Ad-hoc |
| Partial updates (activity log, recent list) | Cheap row updates | Rewrite whole file | Fine |
| Inspectability / debugging | Any SQLite tool | Text editor | Custom tooling |
| Footprint | ~1 MB code; fast open | Minimal | Small |

Multi-process access (one process per window, AD-10) and append-heavy data (MCP activity) decide it for SQLite.
Session blobs (layout trees, view states) are stored as versioned JSON in a column to keep the schema small and evolve
with serde defaults.

Recovery content is **not** in SQLite (spec 09) so a damaged DB can never lose unsaved text.

## Schema (v1)

```sql
CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE workspace_session (
  workspace_key TEXT PRIMARY KEY,        -- canonical root path, or '::no-workspace::<window-slot>'
  updated_at INTEGER NOT NULL,
  session_json TEXT NOT NULL,            -- { "v": 1, layout, groups, views, panels, explorer, search }
  bookmarks_json TEXT NOT NULL DEFAULT '{}'
);
CREATE TABLE window_state (slot INTEGER PRIMARY KEY, workspace_key TEXT, geometry_json TEXT NOT NULL, updated_at INTEGER NOT NULL);
CREATE TABLE recent (kind TEXT NOT NULL, path TEXT NOT NULL, display_name TEXT, last_opened INTEGER NOT NULL, pinned INTEGER NOT NULL DEFAULT 0, PRIMARY KEY (kind, path));
CREATE TABLE scratchpad (scratch_id TEXT PRIMARY KEY, title TEXT NOT NULL, language TEXT NOT NULL, created_at INTEGER, updated_at INTEGER, open INTEGER NOT NULL);
CREATE TABLE palette_recent (command_id TEXT PRIMARY KEY, used_at INTEGER NOT NULL);
CREATE TABLE mcp_client (client_id TEXT PRIMARY KEY, name TEXT NOT NULL, token_hash TEXT NOT NULL, created_at INTEGER, last_seen INTEGER, revoked INTEGER NOT NULL DEFAULT 0);
CREATE TABLE permission_grant (principal TEXT NOT NULL, capability TEXT NOT NULL, scope TEXT NOT NULL, decision TEXT NOT NULL, workspace_key TEXT, created_at INTEGER, PRIMARY KEY (principal, capability, workspace_key));
CREATE TABLE mcp_activity (id INTEGER PRIMARY KEY, at INTEGER NOT NULL, client_id TEXT, tool TEXT NOT NULL, target TEXT, outcome TEXT NOT NULL, duration_ms INTEGER);
CREATE TABLE plugin_approval (plugin_id TEXT PRIMARY KEY, version TEXT NOT NULL, permissions_hash TEXT NOT NULL, approved_at INTEGER NOT NULL, enabled INTEGER NOT NULL);
CREATE TABLE notice_dismissed (key TEXT PRIMARY KEY, at INTEGER NOT NULL);
```

## Write Policy

- Session state is written debounced (2 s after `SessionTracker` marks dirty), on window focus loss, and on exit, in a
  single transaction, from the blocking pool using a serialized snapshot (no DB access on the UI thread).
- Reads at startup happen before the first frame on the main thread only for the small window/session row needed to
  restore layout (target < 10 ms); everything else loads lazily.
- `mcp_activity` pruned to `mcp.activityRetentionDays` (default 14) on startup.

## Migrations & Corruption

- `PRAGMA user_version` holds the schema version; migrations are forward-only SQL scripts embedded in
  `notoir-storage/migrations/NNNN_name.sql`, applied in a transaction at startup, unit-tested from every historical
  version.
- Before migrating, the DB file is copied to `state.db.bak-v<old>`.
- A DB from a newer app version (user_version greater than known) opens read-only for recent/session data; writes are
  skipped with a warning (downgrade safety).
- On open failure or `PRAGMA quick_check` failure: rename to `state.db.corrupt-<timestamp>`, create a fresh DB, show a
  notice ("Notoir could not read its saved session and started fresh. Your files and unsaved recovery data are not
  affected."). Recovery (spec 09) still runs because it is file-based.
- Session JSON blobs carry `"v"`; unknown/older versions are migrated by serde defaults; unparsable blobs are discarded
  for that workspace only.

## Restore Behaviour

- On launch with `workbench.startup = restoreSession` (default): restore the last window's workspace and session; files
  missing on disk are skipped with one aggregated notice; Scratchpads always restored.
- Opening a workspace restores its last session. Restoring opens documents lazily: only active tabs load content
  immediately; other tabs load on activation (keeps startup fast).
- `--new-window` and `notoir <file>` start without restoring the previous workspace.

## Test Cases

```gherkin
Feature: Session restore

  Scenario: Layout and cursors restored per workspace
    Given workspace "notes" with two groups, three tabs, and the cursor at line 40 in "a.md"
    When Notoir quits and the user reopens workspace "notes"
    Then the two groups and three tabs are restored
    And "a.md" has its cursor at line 40

  Scenario: Different workspaces have independent sessions
    Given workspace "A" has tab "x.md" and workspace "B" has tab "y.md"
    When the user opens "A"
    Then only "x.md" is restored

  Scenario: Missing files are skipped
    Given the saved session includes "gone.md" which was deleted
    When the session is restored
    Then other tabs open and one notice lists "gone.md" as missing

Feature: Storage robustness

  Scenario: Corrupt database
    Given state.db is corrupt
    When Notoir starts
    Then it starts with a fresh session and a notice
    And the corrupt file is kept as state.db.corrupt-<timestamp>
    And recovery of unsaved documents still works

  Scenario: Migration from every previous schema version
    Given a state.db at each historical schema version
    When it is opened by the current version
    Then migration succeeds and data is preserved

  Scenario: Two windows write concurrently
    Given two Notoir windows open different workspaces
    When both update their sessions at the same time
    Then both sessions are stored without error
```
