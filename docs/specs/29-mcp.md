# 29 — MCP Server

**Status:** Done — M20: transport (rmcp Streamable HTTP on 127.0.0.1, `mcp.port` or the next free port, Host/Origin allow-lists, bearer tokens stored as SHA-256 hashes, per-client rate limit), `notoir mcp-bridge`, lifecycle and `mcp.autoStart` (restricted), client identity (Add Client, one-time token, config snippets, revoke), status bar `MCP ●N` with popover, Connections, Permissions, Activity panel (`mcp_activity` with retention). M21: the full tool catalogue plus `editor_get_selection`; documents load into memory without a tab and get a background tab when edited; `document_write` / `document_apply_edits` as one undoable `ChangeOrigin::Mcp` transaction with `expected_revision` → `revision_conflict`; fading edit highlight (`mcp.highlightEdits`); `filesystem_write` atomic, then external-change handling with a banner naming the client; `filesystem.outsideWorkspace` for absolute paths of `filesystem_*`; `mcp.maxReadBytes`; activity targets with `(+a −b)` line counts. Later: resources (`notoir://document/{id}`), `git_commit` / `git_push` (Phase 6), plugin-contributed tools (spec 30). Deviations: activity times are UTC; open SSE streams of a revoked client end with their session keep-alive

## Purpose

Notoir acts primarily as an **MCP server**, exposing its workspace, documents, editor and (read-only) Git state to MCP
clients (Claude Desktop, Claude Code, local agents) under an explicit permission model. This spec covers transport,
client identity, the tool catalogue, document-vs-filesystem semantics, execution model, the activity log and UI.
Permission semantics are in spec 32.

## Server Lifecycle

- Off by default. Tools → MCP › Start Server / Stop Server; `mcp.autoStart` (default false, restricted).
- Status bar `MCP ●2` (number of connected clients) appears only while the server runs.
- Notification on start: "MCP server running at http://127.0.0.1:47823/mcp".
- Stopping disconnects all clients; in-flight requests are rejected with a clear error.

## Transport (AD-8)

- `rmcp` server with **Streamable HTTP** transport, `axum` on `127.0.0.1` only (never `0.0.0.0`), port `mcp.port`
  (default 47823; if busy, next free port, shown in UI).
- Security: Host header allow-list (`127.0.0.1:<port>`, `localhost:<port>`), Origin allow-list (absent or loopback
  only) to block DNS rebinding and browser requests; **bearer token required** on every request
  (`Authorization: Bearer <token>`).
- **stdio bridge**: `notoir mcp-bridge --token <token> [--port <p>]` — a subcommand of the same binary that speaks MCP
  over stdio and proxies to the running app's HTTP endpoint. Used by stdio-only clients (e.g. Claude Desktop config).
  If Notoir is not running, the bridge returns an MCP error "Notoir is not running" (it never launches a hidden
  instance).
- One MCP session per client connection; the server is one per Notoir process (each window process can run its own on
  a different port; the Connections UI shows which).

## Client Identity

- Tools → MCP › Connections → "Add Client…": user names the client ("Claude Desktop"), Notoir generates a random 256-bit
  token, shows copyable config snippets (Claude Code `claude mcp add --transport http notoir http://127.0.0.1:47823/mcp
  --header "Authorization: Bearer …"`, Claude Desktop JSON using the bridge), and stores only the token hash
  (`mcp_client` table, spec 23). The token is shown once.
- Each request is attributed to a `ClientId`. The client's self-reported `clientInfo.name` from `initialize` is
  displayed alongside but never used for authorization.
- Revoke: immediately rejects the token and drops its sessions.
- Unauthenticated requests get 401 and an activity entry "rejected: missing/invalid token".

## Execution Model

```text
rmcp handler (tokio)  ──McpRequest{client, tool, args, reply: oneshot}──►  UI thread queue
                                                                              │
                                     permission gate (spec 32) ── Ask ──► permission prompt (non-blocking UI;
                                                                              │                request waits, timeout 120 s)
                                                                              ▼
                                                     command handlers / domain services (same as UI)
                                                                              │
                         ◄──────────── McpResponse (JSON) via oneshot ────────┘
```

- MCP never touches `AppCore` directly; all calls are executed on the UI thread through the command/document layer
  (spec 01, 05). Long operations (search, filesystem reads of large files, Git diff) start jobs and reply when done.
- Each request has a timeout (`mcp.requestTimeoutMs`, default 30 s; permission prompts extend it up to 120 s).
- Edits from MCP are transactions with `ChangeOrigin::Mcp(client)`: undoable by the user, shown in activity, and
  briefly highlighted in the editor (fading accent background on changed ranges, `mcp.highlightEdits = true`).
- Rate limiting: per-client token bucket (default 20 requests/s burst 50) to protect the UI thread.

## Document vs Filesystem Semantics

This distinction is central.

| Operation family | Reads/writes | Notes |
|---|---|---|
| `document_*` | Notoir's **in-memory document** (opens it if needed, without a tab unless requested) | Returns unsaved buffer content and `dirty`, `revision`; writes modify the buffer, mark it dirty, are undoable; saving is a separate `document_save` call |
| `filesystem_*` | The **disk** directly, inside the workspace boundary | Reads ignore buffers; writes are a higher-risk capability, go through atomic write + conflict check, and trigger external-change handling (spec 10) for open documents |
| `editor_*` | UI state: tabs, active document, selections, cursor | Changing selections/opening tabs is visible to the user |

`document_read` of a dirty open file returns the buffer; `filesystem_read` of the same path returns disk content.

Optimistic concurrency: `document_write` / `document_apply_edits` accept `expected_revision`; if the document changed
since, the call fails with `revision_conflict` and the current revision so the client can re-read.

## Tool Catalogue

Wire names use `snake_case` with an underscore namespace (`document_read`), because many clients restrict tool names
to `[a-zA-Z0-9_-]`. Conceptual dotted names from product docs map 1:1 (`document.read` → `document_read`).

| Tool | Capability | Description |
|---|---|---|
| `workspace_get` | `workspace.read` | Root, name, Git presence, open document count |
| `workspace_list_files` | `workspace.read` | Paged listing (glob filter, respects ignore rules) |
| `workspace_search` | `workspace.search` | Text/regex search; dirty buffers searched in memory; paged results |
| `document_open` | `document.read` | Load a document into Notoir memory (optional `show: bool` opens a tab → also needs `editor.control`) |
| `document_read` | `document.read` | Content (optionally a line range) + metadata (dirty, revision, language, encoding, eol) |
| `document_create` | `document.create` | Create a new file-backed document (written to disk on create) or a Scratchpad |
| `document_write` | `document.modify` | Replace full buffer content (with `expected_revision`) |
| `document_apply_edits` | `document.modify` | Apply a list of range edits (line/col or offsets) atomically as one transaction |
| `document_save` | `document.save` | Save through the normal pipeline (conflict checks included) |
| `document_rename` | `file.rename` | Rename/move a file within the workspace (updates open documents) |
| `document_delete` | `file.delete` | Move file to trash (default denied) |
| `editor_get_open_tabs` | `editor.read` | Groups, tabs, active tab, dirty flags |
| `editor_get_active_document` | `editor.read` | Active document id/path + selections |
| `editor_get_selection` | `editor.read` + `document.read` | Selected text of the active editor (or a view) with its file and the range of every selection (1-based line/column and 0-based offsets) |
| `editor_open_document` | `editor.control` | Open/focus a document in a tab, optional line |
| `editor_select_range` | `editor.control` | Set selection in a view |
| `editor_insert` | `document.modify` + `editor.read` | Insert at cursor/selection of the active view |
| `editor_replace` | `document.modify` + `editor.read` | Replace current selection |
| `settings_get` | `settings.read` | Effective settings (filtered: no secrets/paths outside workspace) |
| `settings_update` | `settings.modify` | Update User or Workspace setting (default denied; restricted keys never) |
| `git_status` | `git.read` | Branch, ahead/behind, changes |
| `git_diff` | `git.read` | Diff of a file/workspace vs index/HEAD |
| `filesystem_read` | `filesystem.read` | Raw disk read inside workspace |
| `filesystem_write` | `filesystem.write` | Raw disk write inside workspace (default Ask every time) |
| `git_commit`, `git_push` | `git.commit`, `git.push` | Phase 6 candidates; default denied when added |
| plugin-contributed tools | plugin's capability + `mcp.pluginTools` | Phase 5: plugins may expose MCP tools (spec 30) |

Resources (optional, Phase 4 later): `notoir://document/{id}` for open documents with change notifications;
prompts: none initially.

Tool descriptions and schemas are English, concise, and include the document-vs-filesystem distinction so models choose
correctly. Results are structured JSON (`structuredContent`) plus a short text summary.

Workspace boundary: all paths are resolved against the workspace root; `..` escapes, absolute paths outside the root,
and symlinks resolving outside are rejected (`path_outside_workspace`) unless the `filesystem.outsideWorkspace`
capability is granted (default denied, Ask each time). With no workspace open, only open documents are accessible.

Payload limits: reads above `mcp.maxReadBytes` (default 2 MiB) require ranges; listings are paged (default 500).

## Activity & Connections UI

- **Status bar** `MCP ●2` → popover: server URL, connected clients with last activity, recent 5 activity entries,
  buttons Connections…, Activity…, Stop Server.
- **MCP Connections** (Tools → MCP › Connections): table of registered clients (name, status, last seen, token created,
  permissions summary), actions Add Client…, Edit Permissions…, Revoke, Copy Config.
- **MCP Activity** panel (bottom panel tab, movable):

```text
MCP ACTIVITY                                           [filter client ▾] [clear] ⋯
14:43:02  Claude Desktop   document_read          architecture.md        ok      3 ms
14:42:51  Claude Desktop   document_open          README.md              ok      1 ms
14:41:10  Local Agent      document_apply_edits   notes.md (+3 −1)       ok      2 ms
14:40:58  Local Agent      document_delete        old.md                 denied
```

  Rows link to the target (click opens it). Arguments are summarized; **document contents are never logged** (only
  sizes/ranges). Stored in SQLite (`mcp_activity`) with retention (spec 23).
- **Permissions** (Tools → MCP › Permissions): per-client capability matrix (spec 32).
- **Settings** (Tools → MCP › Settings): opens Settings filtered to `@mcp`.

## Test Cases

```gherkin
Feature: MCP transport security

  Scenario: Requests without token are rejected
    Given the MCP server is running
    When a client calls tools/list without an Authorization header
    Then the response status is 401
    And the activity log records a rejected request

  Scenario: Non-loopback Origin rejected
    When a request arrives with Origin "https://evil.example"
    Then it is rejected with 403

  Scenario: Bridge without running app
    Given Notoir is not running
    When "notoir mcp-bridge" receives an initialize request on stdin
    Then it responds with an error saying Notoir is not running

Feature: Document vs filesystem

  Scenario: document_read returns the unsaved buffer
    Given "notes.md" on disk contains "A" and its open buffer contains "A B" (unsaved)
    When an authorized client calls document_read for "notes.md"
    Then the result content is "A B" and dirty is true
    When it calls filesystem_read for "notes.md"
    Then the result content is "A"

  Scenario: document_write is undoable and does not save
    When a client calls document_write on "notes.md"
    Then the buffer changes and the document is dirty
    And the file on disk is unchanged
    And the user can undo the change

  Scenario: Revision conflict
    Given the client read revision 5
    And the user typed so the revision is 6
    When the client calls document_apply_edits with expected_revision 5
    Then the call fails with revision_conflict and current revision 6

  Scenario: filesystem_write triggers external change handling
    Given "notes.md" is open and dirty
    And the client has filesystem.write granted
    When the client calls filesystem_write on "notes.md"
    Then the buffer is unchanged
    And the editor shows the external-change banner naming the MCP client

  Scenario: Path escape is rejected
    When a client calls document_read with path "../secrets.txt"
    Then the call fails with path_outside_workspace

Feature: Activity

  Scenario: Activity shows operations without content
    When a client calls document_write on "notes.md"
    Then the activity panel lists the call with client, tool, target and outcome
    And no document content appears in the activity entry or the log file

  Scenario: Status indicator
    Given two clients are connected
    Then the status bar shows "MCP ●2"
```
