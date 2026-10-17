# Plugin API reference

Plugin API version **1** (`api = "1"` in `plugin.toml`). All functions live in the global `notoir` table. A call that
is not allowed or fails raises a Lua error whose message starts with a code, such as
`permission_denied: reading documents` or `invalid_range: The range is outside the document`. Use `pcall` to handle
it.

## The Lua environment

- Lua 5.4 with `string`, `table`, `math`, `utf8`, `coroutine` and the basic functions.
- `os.time`, `os.clock` and `os.date` are available; the rest of `os`, `io`, `debug`, `dofile` and `loadfile` are
  not. `load` accepts text chunks only.
- `require("lib.name")` loads `lib/name.lua` from the plugin's own folder, and nothing else.
- `print(...)` writes to Plugin Output.
- Limits per plugin: 64 MB of memory (`plugins.memoryLimitMb`), 5 s per command (`plugins.commandTimeoutMs`) and
  200 ms per event handler (`plugins.eventTimeoutMs`). Waiting for Notoir to answer an API call does not count.

## Positions and ranges

Lines and columns are **1-based** and count characters. A position is `{ line = 3, column = 1 }` or
`{ offset = 42 }` (0-based character offset from the start). A range is `{ start = <position>, ["end"] = <position> }`
(`end` is a Lua keyword, hence the brackets).

Ranges returned by Notoir carry all three: `{ start = { line, column, offset }, ["end"] = { … }, text = "…" }`.

## Permissions

| Manifest key | Needed for |
|---|---|
| `documents_read` | `notoir.editor.active`, `editor:text/selection/selections/path/language`, `notoir.documents.*` |
| `documents_write` | `editor:replace/insert/set_selection` |
| `workspace_read` | `notoir.workspace.*`, `notoir.fs.read/list` |
| `workspace_write` | `notoir.fs.write` |
| `network`, `network_hosts` | `notoir.http.get` (only the listed hosts when `network_hosts` is not empty) |
| `process` | `notoir.process.run` (the user is asked every time unless they chose Always Allow) |
| `clipboard` | `notoir.clipboard.*` |
| `settings_write` | `notoir.settings.set` on settings that are not the plugin's own |

Calls without the permission raise `permission_denied`.

## Commands

### `notoir.commands.register(spec)`

Registers the code of a command declared in `[contributes] commands`. `spec` is `{ id = "count", run = function(ctx)
… end }`. Registering an undeclared id raises `not_declared`.

`ctx` has:

- `ctx.args`: the arguments the command was run with (from a panel action, a status item or `execute`), or `nil`;
- `ctx.editor`: the active [editor](#editors), or `nil` (looked up on first use).

The command's full id is `plugin.<plugin-id>.<command-id>`, e.g. `plugin.word-count.count`.

### `notoir.commands.execute(id, args)`

Runs a command after the current call returns, as if chosen from a menu. `id` is a built-in command id (see the
[Commands reference](reference/commands.md)) or one of the plugin's own full ids; `args` are the command's arguments.
Unknown ids raise `unknown_command`.

```lua
notoir.commands.execute("view.toggleWordWrap")
notoir.commands.execute("plugin.word-count.count")
```

## Notifications and logs

### `notoir.notify(message, opts)`

Shows a notification `<Plugin name>: <message>`. `opts.level` is `"info"` (default), `"warning"` or `"error"`.

### `notoir.log.info(message)`, `notoir.log.warn(message)`, `notoir.log.error(message)`

Write to **Plugin Output**.

## Editors

### `notoir.editor.active()`

The active text editor, or `nil` (also for the Settings, Welcome and Plugin Manager tabs). Needs `documents_read`.

An editor handle has these methods:

| Method | Returns | Permission |
|---|---|---|
| `editor:text()` | the whole text | `documents_read` |
| `editor:selection()` | the primary selection as a range with `text` | `documents_read` |
| `editor:selections()` | every selection (multi-cursor) | `documents_read` |
| `editor:path()` | the file path, or `nil` for untitled documents and scratchpads | `documents_read` |
| `editor:language()` | the language id: `markdown`, `json`, `plaintext`, `rust`… | `documents_read` |
| `editor:replace(range, text)` | replaces the range | `documents_write` |
| `editor:insert(text)` | inserts at the cursor | `documents_write` |
| `editor:set_selection(range)` | selects the range (a collapsed range moves the cursor) | `documents_write` |

Each `replace` or `insert` is one undoable edit, attributed to the plugin. An editor whose tab was closed raises
`no_editor`.

## Documents

### `notoir.documents.list()`

The open documents: a list of `{ document, path, title, language, dirty }` (`path` is `nil` for untitled documents and
scratchpads).

### `notoir.documents.open(path)`

Opens a workspace file in a tab without moving the focus. `path` is relative to the workspace root or absolute inside
it.

## Workspace and files

These need an open workspace and only accept paths inside it (relative to the root, or absolute); anything else raises
`permission_denied`.

| Function | Returns |
|---|---|
| `notoir.workspace.root()` | the workspace folder, or `nil` |
| `notoir.workspace.find_files(glob)` | sorted relative paths of matching files (default `"**/*"`), without ignored files |
| `notoir.fs.read(path)` | the file's text |
| `notoir.fs.list(dir)` | `{ { name = "notes.md", dir = false }, … }` sorted by name (default `"."`) |
| `notoir.fs.write(path, text)` | `true`; creates missing folders |

`notoir.fs.write` writes the file directly; if it is open in Notoir, the user sees the usual
[changed on disk](files.md#files-changed-outside-notoir) handling. Prefer editor methods for open documents.

## Settings

### `notoir.settings.get(key)`

The effective value of a setting: a boolean, number, string or list. Keys declared in the plugin's
`[contributes] settings` can be given short (`"includeCode"`); other settings by their full key (`"editor.tabSize"`).

### `notoir.settings.set(key, value)`

Writes a user setting; `nil` resets it. The plugin's own settings are always writable; other settings need
`settings_write`, and restricted ones are refused. A value of the wrong type raises `invalid_value`.

## Events

### `notoir.events.on(name, handler)`

Calls `handler(payload)` when the event happens. Several handlers per event are allowed.

| Event | Payload |
|---|---|
| `on_file_open` | `{ document, path, title, language, dirty }` |
| `on_file_change` | the same; sent after typing pauses (`plugins.changeEventDebounceMs`, 150 ms) |
| `on_file_save` | the same |
| `on_file_close` | `{ document }` |
| `on_active_editor_change` | the new active document, or `nil` |
| `on_workspace_open` | `{ root }` |
| `on_workspace_close` | `nil` |

A manifest `activation` entry `onEvent:<name>` activates the plugin on that event.

## Status items

### `notoir.status.set(id, item)`

Shows the status item declared as `[contributes] status_items = [{ id, alignment = "left" | "right", priority }]`;
`priority` orders the plugin items on that side. `item` is `{ text = "…", tooltip = "…", command = "count" }`; clicking it runs
`command`. `nil` hides it.

## Panels

### `notoir.panels.render(id, tree)`

Sets the content of a panel declared as `[contributes] panels = [{ id, title }]`. Plugin panels appear in the **Plugins**
panel of the sidebar. `tree` is a table of elements; actions run one of the plugin's commands with arguments:

| `type` | Fields | Action |
|---|---|---|
| `column`, `row` | `children` | — |
| `heading`, `label` | `text` | — |
| `button` | `text`, `command`, `args` | runs `command` with `args` |
| `checkbox` | `text`, `checked`, `command` | runs `command` with `{ checked = true/false }` |
| `text_input` | `id`, `value`, `placeholder`, `command` | on `Enter`, runs `command` with `{ value = "…" }` |
| `list` | `items` (strings or `{ id, text }`), `on_select` | runs `on_select` with `{ item = id, text = "…" }` |
| `tree` | `items` (`{ id, text, children }`), `on_select` | same as `list` |
| `separator` | — | — |
| `progress` | `value` (0–1), `text` | — |

Call `render` again whenever the content changes.

## Providers

### `notoir.formatters.register(language, fn)`

Makes **Format Document** use `fn(text, opts)` for that language. `opts` is `{ tab_size, insert_spaces }`; return the
new text. Notoir applies the difference as one undoable edit, and only if the document did not change meanwhile.

### `notoir.outline.register(language, fn)`

Provides the [Outline](navigation.md#outline), breadcrumbs and Go to Symbol for that language. `fn(text)` returns a list
of `{ name, line, end_line, children }` (`end_line` and `children` optional).

An unknown language id raises `unknown_language`. Languages a plugin declares in `[contributes] languages` (with an
optional `.sublime-syntax` grammar) can be used.

## Network, programs and clipboard

### `notoir.http.get(url, opts)`

`GET` over `http` or `https`, with a 30-second timeout. Returns `{ status = 200, body = "…" }`. Needs `network`; with
`network_hosts`, only those hosts.

### `notoir.process.run(command, args, opts)`

Runs a program with a list of arguments (no shell), in `opts.cwd` (inside the workspace; default the workspace root),
for up to 30 seconds. Returns `{ code, stdout, stderr, timed_out }`. Needs `process`; Notoir asks the user each time
unless they chose **Always Allow** for the plugin.

### `notoir.clipboard.get()`, `notoir.clipboard.set(text)`

Read and write the clipboard text. Needs `clipboard`.

## Translations

### `notoir.i18n.t(key, args)`

The translation of `key` from the plugin's `locales/<language>.json` in the display language, falling back to English
and then to the key. `{{name}}` placeholders are replaced from `args`: `notoir.i18n.t("status", { n = 12 })`.

## Manifest reference

```toml
[plugin]
id = "word-count"          # required: a-z, 0-9 and -, 2–64 characters
name = "Word Count"        # required
version = "1.0.0"          # required, semver
api = "1"                  # required
main = "main.lua"          # default main.lua
authors = ["Jane <jane@example.com>"]
description = "Counts words in the active document."
license = "MIT"

[permissions]              # all false by default
documents_read = true
documents_write = false
workspace_read = false
workspace_write = false
network = false
network_hosts = []         # e.g. ["api.example.com"]
process = false
clipboard = false
settings_write = false
mcp_tools = false          # reserved

[contributes]
commands = [{ id = "count", title = "%commands.count%", category = "Plugins" }]
keybindings = [{ command = "count", key = "Mod+Alt+W", when = "editorFocus" }]
menus = [{ command = "count", location = "plugins" }, { command = "count", location = "editor/context" }]
status_items = [{ id = "words", alignment = "right", priority = 50 }]
settings = [{ key = "includeCode", type = "bool", default = false, title = "%settings.includeCode%" }]
languages = [{ id = "org", name = "Org", extensions = [".org"], syntax = "syntaxes/org.sublime-syntax" }]
panels = [{ id = "stats", title = "Word Stats" }]
activation = ["onCommand:count", "onLanguage:markdown"]
```

Unknown keys are errors: the plugin is listed as *Invalid* with the line of the problem. Keybinding `when` expressions
use the [same syntax as user shortcuts](keybindings.md#the-keybindings-file); an invalid one is reported in Plugin
Output.
