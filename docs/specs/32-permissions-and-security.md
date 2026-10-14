# 32 — Permissions & Security

**Status:** Not started — M20 (MCP), M22 (plugins)

## Purpose

Define the shared permission model for the two trust boundaries — MCP clients and Lua plugins — plus workspace trust,
and the security requirements that apply across the app. A single `PermissionGate` in `notoir-core` evaluates every
externally originated operation.

## Principals and Capabilities

```rust
pub enum Principal { McpClient(ClientId), Plugin(PluginId) }

pub enum Capability {
    // workspace / files
    WorkspaceRead, WorkspaceSearch, FileCreate, FileRename, FileDelete,
    FilesystemRead, FilesystemWrite, FilesystemOutsideWorkspace,
    // documents & editor
    DocumentRead, DocumentCreate, DocumentModify, DocumentSave, EditorRead, EditorControl,
    // settings
    SettingsRead, SettingsModify,
    // git
    GitRead, GitCommit, GitPush,
    // plugin-only
    Network, Process, Clipboard, ExposeMcpTools,
    // reserved
    ExecuteCommand(CommandId),      // running arbitrary built-in commands by id
    ModifyPluginConfig,
}

pub enum Decision { Allow, Deny, Ask }
pub enum GrantScope { Once, Session, Persistent }
pub struct Grant { principal: Principal, capability: Capability, decision: Decision, scope: GrantScope, workspace: Option<WorkspaceKey> }
```

## Defaults

MCP client defaults (applied when a client is added; editable per client):

| Group | Capability | Default |
|---|---|---|
| Workspace | Read files (`WorkspaceRead`, `DocumentRead`) | Allow |
| | Search files | Allow |
| | Open files in Notoir memory | Allow |
| | Create files | Ask |
| | Modify documents (`DocumentModify`) | Allow (undoable, visible) |
| | Save documents | Ask (session) |
| | Rename files | Ask |
| | Delete files | **Deny** |
| | Raw filesystem read | Allow |
| | Raw filesystem write | Ask every time |
| | Outside workspace | **Deny** |
| Editor | Read tabs/active document | Allow |
| | Control editor (open tabs, select) | Allow |
| Git | View status / diff | Allow |
| | Commit | **Deny** |
| | Push | **Deny** |
| Settings | Read | Allow |
| | Modify | **Deny** |
| Commands | Execute arbitrary commands | **Deny** |
| Plugins | Modify plugin configuration | **Deny** (cannot be enabled for MCP) |

Plugin capabilities derive from the approved manifest permissions (spec 30): not requested → Deny (not even Ask);
requested and approved → Allow, except `Process` (Ask per call unless "always allow") and `FilesystemOutsideWorkspace`
(not available to plugins in v1).

Risk classes for commands (spec 05): `Safe` (allowed if capability allows), `Sensitive` (Ask unless persistent Allow),
`Dangerous` (destructive or remote-affecting: delete, discard, commit, push, settings write — Deny by default for every
principal; user can grant per principal with a warning).

## Evaluation

```text
check(principal, capability, target) →
  1. principal revoked/disabled            → Deny
  2. target outside allowed scope          → Deny (path boundary, restricted settings, workspace untrusted)
  3. explicit Deny grant                   → Deny
  4. Once/Session/Persistent Allow grant   → Allow
  5. default for capability                → Allow | Ask | Deny
```

- Evaluated on the UI thread in the command layer for every MCP request and plugin API call; never cached outside the
  gate; grants changes take effect immediately (in-flight requests re-checked before execution).
- Path boundary checks canonicalize paths (resolving symlinks) and compare against the canonical workspace root;
  Windows device paths and alternate data streams (`file.txt:stream`) are rejected.

## Ask Prompt

Non-modal prompt anchored above the status bar (does not steal editor focus; keyboard reachable via `F6`):

```text
⚠ "Local Agent" (MCP) wants to save "notes.md".
   [Allow Once]  [Allow for Session]  [Always Allow]  [Deny]          details ▸
```

- Shows principal, capability in plain language (translated), target, and for writes a link "Preview changes" (diff via
  spec 27).
- Multiple pending prompts queue; identical requests are coalesced.
- Timeout (120 s) → Deny, logged.
- "Always Allow" for Dangerous capabilities requires a second confirmation.
- Prompts appear even in Zen Mode, because the requester is waiting on them.

## Permissions UI

Tools → MCP › Permissions and Plugin Manager › Permissions show the same matrix component:

```text
MCP ACCESS — Claude Desktop                       Workspace: all ▾

Workspace      ✓ Read files   ✓ Search   ✓ Open   ? Create   ✓ Modify   ✕ Delete
Editor         ✓ Read tabs    ✓ Control
Git            ✓ Status       ✕ Commit   ✕ Push
Settings       ✓ Read         ✕ Modify
                                        [Reset to Defaults]  [Revoke All]
```

States use icon + text (Allow ✓, Ask ?, Deny ✕), not colour alone. Session grants are listed separately with "Clear".

## Workspace Trust

Untrusted workspaces (default for any workspace containing `.notoir/` that the user has not trusted) ignore restricted
settings (spec 20), do not activate workspace-recommended plugins, do not auto-start MCP, and never run Sync Notes
triggers. Trust is stored per canonical path in the state DB; revocable in Settings › Workspace.

## Security Requirements

- MCP binds loopback only, enforces Host/Origin allow-lists and bearer tokens; tokens stored hashed (spec 29).
- No secrets in logs; document contents never logged by default (spec 35).
- Plugins: restricted Lua stdlib, no native modules, memory/time limits, central permission gate (spec 30).
- Markdown preview: no HTML/JS execution; remote images off by default; links with non-http schemes confirm.
- Git: no credential storage in Notoir; `GIT_TERMINAL_PROMPT=0`; no automatic pushes.
- Updates: signed manifests and artifacts (spec 31).
- File writes only through the save pipeline / fs_ops with path validation.
- `unsafe_code = "forbid"` across the workspace.
- Dependency audit (`cargo audit`, `cargo deny` for licenses and advisories) in CI.

## Test Cases

```gherkin
Feature: Permission gate

  Scenario: Delete denied by default for MCP
    Given an MCP client with default permissions
    When it calls document_delete on "old.md"
    Then the call fails with permission_denied
    And "old.md" still exists
    And the activity log records the denial

  Scenario: Ask once
    Given the client's Save capability is Ask
    When it calls document_save
    Then a prompt asks the user
    When the user clicks "Allow Once"
    Then the save happens
    And a second document_save prompts again

  Scenario: Session grant expires on restart
    Given the user chose "Allow for Session" for Create files
    When Notoir restarts
    Then the next create prompts again

  Scenario: Revocation applies immediately
    Given a client with Modify allowed
    When the user revokes the client
    Then its next request is rejected with 401

  Scenario: Symlink escape
    Given "link" in the workspace is a symlink to "/etc"
    When an MCP client reads "link/hosts"
    Then the call fails with path_outside_workspace

  Scenario: Prompt timeout denies
    Given a pending permission prompt
    When 120 seconds pass without an answer
    Then the request is denied

Feature: Plugin capabilities

  Scenario: Unrequested capability is denied without prompt
    Given a plugin that did not request clipboard
    When it calls notoir.clipboard.get
    Then the call fails with a permission error and no prompt is shown
```
