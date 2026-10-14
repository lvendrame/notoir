# 30 — Lua Plugins

## Purpose

Define the Lua plugin system: package format, manifest and permissions, runtime isolation, the stable `notoir` API,
contribution points, lifecycle, errors, and the Plugin Manager. Plugins never see egui or Rust internals.

## Package Format

```text
<plugins-dir>/word-count/
├── plugin.toml          # manifest (required)
├── main.lua             # entry (manifest `main`)
├── lib/…                # additional modules (require-able inside the package only)
├── locales/en.json      # optional translations (spec 22)
├── syntaxes/*.sublime-syntax   # optional language definitions
└── README.md
```

`<plugins-dir>` = `<data>/plugins/` (spec 31), configurable via `plugins.directory` (User scope only). Workspace
`.notoir/plugins.toml` may **recommend** plugins (ids + versions) but never installs or enables them automatically.

## Manifest

```toml
[plugin]
id = "word-count"                 # [a-z0-9-]{2,64}, unique
name = "Word Count"
version = "1.0.0"                 # semver
api = "1"                         # Notoir plugin API major version
main = "main.lua"
authors = ["Jane <jane@example.com>"]
description = "Counts words in the active document."
license = "MIT"

[permissions]
workspace_read = true             # read files inside the open workspace via notoir.fs
workspace_write = false           # write/create/rename files inside the workspace
documents_read = true             # read open document buffers (default true when any command needs editor text)
documents_write = false           # modify document buffers
network = false                   # notoir.http (with optional allow-list)
network_hosts = []                # e.g. ["api.example.com"]
process = false                   # notoir.process.run (asks user each time unless always-allowed)
clipboard = false
mcp_tools = false                 # expose MCP tools (still gated by MCP permissions)
settings_write = false            # change Notoir settings (own plugin settings are always writable)

[contributes]
commands = [{ id = "count", title = "%commands.count%", category = "Plugins" }]
keybindings = [{ command = "count", key = "Mod+Alt+W", when = "editorFocus" }]
menus = [{ command = "count", location = "editor/context", group = "plugins" }]
status_items = [{ id = "words", alignment = "right", priority = 50 }]
settings = [{ key = "includeCode", type = "bool", default = false, title = "Count words in code blocks" }]
languages = [{ id = "org", extensions = [".org"], syntax = "syntaxes/org.sublime-syntax" }]
panels = [{ id = "stats", title = "Word Stats", area = "secondary" }]
activation = ["onCommand:count", "onLanguage:markdown", "onStartup"]
```

Contributions are declarative so menus, palette entries and keybindings exist **before** the Lua code runs (lazy
activation keeps startup fast).

## Permissions & Approval

- Installation (copying into the plugins dir via Plugin Manager "Install from Folder…" / "Install from .zip…") shows:

```text
"Word Count" 1.0.0 requests:

  ✓ Read open documents
  ✓ Read files in the workspace
  ✕ Internet access
  ✕ Run programs

[Install]  [Cancel]
```

- Approval stores `(plugin_id, version, permissions_hash)` in the state DB (spec 23). Plugins found in the directory
  without an approval record (e.g. copied manually) are **disabled** until the user reviews and enables them.
- An update that **adds** permissions requires re-approval; the plugin stays on the old approval (disabled if the code
  changed) until then.
- All checks happen centrally in the Rust API layer (`PermissionGate`, spec 32) on every call — never in Lua.
- `process` runs require a per-call confirmation unless the user ticks "Always allow for this plugin" (stored as a
  grant); `network` limited to `network_hosts` when non-empty.

## Runtime (AD-9)

- `mlua` with Lua 5.4 (vendored). One `Lua` state per plugin on a dedicated plugin host thread (one thread per active
  plugin, idle plugins parked).
- Standard library: `base` (minus `dofile`, `loadfile`, `load` with binary chunks), `string`, `table`, `math`, `utf8`,
  `coroutine`. **Not loaded**: `io`, `os` (except `os.time`, `os.clock`, `os.date` re-exposed), `package` C loaders,
  `debug`. `require` is replaced by a loader that resolves only `.lua` files inside the plugin's own directory.
- Limits: memory limit per VM (`plugins.memoryLimitMb`, default 64); instruction-count hook enforces per-call time budget
  (default 200 ms for event handlers, 5 s for commands, configurable per plugin by the user); exceeding raises an error
  in the plugin.
- Communication: API functions are Rust callbacks that send `PluginRequest`s to the UI thread and block the plugin thread
  on the reply (UI processes them at frame start; typical latency ≤ 1 frame). The UI thread never waits on Lua.
- Command invocation passes a `ctx` with a snapshot of the active document (lazy: text fetched on first `ctx.editor:text()`).

## API (v1)

```lua
notoir.commands.register({ id = "count", run = function(ctx) ... end })   -- id must be declared in manifest
notoir.commands.execute("file.save")                                         -- built-in commands (gated by risk/permissions)
notoir.notify(message, { level = "info" | "warning" | "error" })
notoir.editor.active()            -- → Editor handle or nil
editor:text() / editor:selection() / editor:selections() / editor:path() / editor:language()
editor:replace(range, text) / editor:insert(text) / editor:set_selection(range)   -- documents_write
notoir.documents.open(path) / notoir.documents.list()
notoir.fs.read(path) / notoir.fs.list(dir) / notoir.fs.write(path, text)          -- workspace scope only
notoir.workspace.root() / notoir.workspace.find_files(glob)
notoir.settings.get(key) / notoir.settings.set(key, value)                        -- own settings; others need settings_write
notoir.status.set("words", { text = "123 words", tooltip = "...", command = "count" })
notoir.panels.render("stats", tree)   -- declarative UI tree (see below)
notoir.formatters.register(language, function(text, opts) return new_text end)
notoir.outline.register(language, function(text) return symbols end)
notoir.events.on("on_file_save", function(ev) ... end)
notoir.http.get(url, opts)            -- network
notoir.process.run(cmd, args, opts)   -- process
notoir.clipboard.get() / set(text)    -- clipboard
notoir.mcp.register_tool({ name, description, input_schema, run })   -- mcp_tools (Phase 5 later)
notoir.i18n.t(key, args)
notoir.log.info/warn/error(msg)       -- Plugin Output channel
```

Panels use a **declarative UI tree** (no egui exposure): `{ type = "column", children = { { type = "label", text = "..." },
{ type = "button", text = "Refresh", command = "stats.refresh" }, { type = "list", items = {...}, on_select = "..." } } }`
with a fixed set of node types: column, row, label, heading, button, checkbox, text_input, list, tree, separator,
progress. Rendered by the app with Notoir components and theme.

Edits from plugins are transactions with `ChangeOrigin::Plugin(id)` — undoable, tracked in recovery.

## Lifecycle

`Discovered → (Approved) → Enabled → Activated (VM created, main.lua run) → Deactivated/Disabled/Errored`

- Activation on declared events; `onStartup` is discouraged (warning in Developer Tools).
- `Plugins → Reload Plugins` tears down VMs and re-reads manifests; contributions update live.
- Workspace events (`on_workspace_open/close`) and others per spec 06.
- Formatter hooks run with timeouts; failure skips formatting with a warning.

## Errors

- A Lua error in a callback: logged to Plugin Output with traceback; notification "Plugin 'Word Count' encountered an
  error." [View Error]. Three errors within 60 s or any memory-limit/time-limit violation → plugin disabled:
  "Plugin 'Word Count' encountered an error and was disabled. [View Error] [Re-enable]".
- Malformed manifest: plugin listed as "Invalid" in the manager with the parse error; never loaded.
- API version mismatch (`api` major ≠ supported): listed as incompatible.
- A plugin can never crash or freeze Notoir; worst case it is disabled.

## Plugin Manager

Plugins menu → Plugin Manager opens an editor tab:

```text
PLUGINS                        [ search ]            [Install from Folder…] [Install from .zip…]
Installed (3)  │  Recommended by workspace (1)  │  Updates  (Phase 6: Available / marketplace)

Word Count  1.0.0        Enabled ◉   [Disable] [Permissions] [Settings] [Uninstall]
  Counts words in the active document.
  Permissions: read documents, read workspace files
```

- Permissions view shows granted permissions and per-capability grants (process "always allow"), with revoke.
- Uninstall moves the plugin dir to trash after confirmation.
- Open Plugins Folder reveals the directory. Plugin Developer Tools (bottom panel): per-plugin logs, activation events,
  API call counts, last errors, missing translation keys, "Reload this plugin" button.
- Sample plugins in `plugins/examples/` (word-count, todo-highlighter, markdown-table-formatter) double as integration
  test fixtures.

## Test Cases

```gherkin
Feature: Plugin permissions

  Scenario: Plugin without network permission cannot use http
    Given an enabled plugin without network permission
    When it calls notoir.http.get
    Then the call raises a permission error in Lua
    And no network request is made

  Scenario: Manually copied plugin is disabled
    Given a plugin folder copied into the plugins directory without installation
    When Notoir starts
    Then the plugin is listed as "Needs review" and its code is not executed

  Scenario: Update adding permissions requires re-approval
    Given "Word Count" 1.0.0 is approved without network
    When version 1.1.0 requesting network is installed
    Then the user is shown the new permission request before it is enabled

  Scenario: Filesystem access outside workspace denied
    Given a plugin with workspace_read
    When it calls notoir.fs.read("/etc/passwd")
    Then the call raises a permission error

Feature: Plugin runtime

  Scenario: Command registered from manifest
    Given "Word Count" declares command "count"
    When the user opens the Command Palette
    Then "Plugins: Count Words" is listed before the plugin has been activated
    When the user runs it
    Then the plugin activates and shows a notification with the word count

  Scenario: Infinite loop is stopped
    Given a plugin command that loops forever
    When the user runs it
    Then the UI remains responsive
    And after the time budget the plugin errors and is reported

  Scenario: Repeated errors disable plugin
    Given a plugin whose on_file_save handler always errors
    When the user saves three times within a minute
    Then the plugin is disabled with a notification offering "View Error" and "Re-enable"

  Scenario: Plugin edits are undoable
    Given a plugin command replaces the selection with uppercase text
    When the user runs it and then presses undo
    Then the original text is restored

  Scenario: Malformed manifest
    Given a plugin.toml with a syntax error
    Then the plugin manager lists it as invalid with the parse error
```
