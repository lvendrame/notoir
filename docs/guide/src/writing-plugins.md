# Writing a plugin

A plugin is a folder with a manifest (`plugin.toml`) and Lua 5.4 code. This chapter builds a small plugin step by step,
then walks through the [example plugins](plugins.md#example-plugins). Every function is described in the
[Plugin API reference](plugin-api.md).

## 1. A first command

Create a folder `hello` with two files.

`plugin.toml`:

```toml
[plugin]
id = "hello"                 # a-z, 0-9 and -, 2–64 characters
name = "Hello"
version = "0.1.0"
api = "1"
description = "Says hello."

[contributes]
commands = [{ id = "greet", title = "Say Hello" }]
activation = ["onCommand:greet"]
```

`main.lua`:

```lua
notoir.commands.register({ id = "greet", run = function(ctx)
  notoir.notify("Hello from Lua!")
end })
```

Install it with **Plugins › Install Plugin from Folder…** and pick the `hello` folder. The command appears in the
[command palette](command-palette.md) as **Plugins: Say Hello**. Run it: a notification says *Hello: Hello from Lua!*

Notoir copies the folder into its plugins folder. While you develop, edit the copy there (**Plugins › Open Plugins
Folder**) and use **Plugins › Reload Plugins**, or **Reload this plugin** in **Plugin Developer Tools**.

## 2. Declaring before running

Everything a plugin contributes is declared in `[contributes]`, so Notoir can show its commands, menu items and
shortcuts *before* the Lua code runs. The code only runs when one of the `activation` events happens:

| Event | Activates when |
|---|---|
| `onCommand:<id>` | one of the plugin's commands is run |
| `onLanguage:<language>` | a document of that language is opened or becomes active (`markdown`, `plaintext`, `json`…) |
| `onEvent:<name>` | an [event](plugin-api.md#events) happens, such as `onEvent:on_file_save` |
| `onStartup` | Notoir starts. Avoid it: it slows down every start, and Developer Tools warns about it. |

## 3. Reading the document

Let's count the words of the current document. Reading documents needs a permission:

```toml
[permissions]
documents_read = true
```

```lua
notoir.commands.register({ id = "greet", run = function(ctx)
  local editor = ctx.editor            -- the active editor, or nil
  if not editor then return end
  local n = 0
  for _ in editor:text():gmatch("%S+") do n = n + 1 end
  notoir.notify(n .. " words")
end })
```

`ctx.editor` is looked up the first time you use it, so a command that never touches it needs no permission.
Permissions are checked by Notoir on every call: without `documents_read`, `editor:text()` raises a
`permission_denied` error. After you add a permission, the plugin needs a new review in the Plugin Manager.

## 4. A key, a menu item and a status item

```toml
[contributes]
commands = [{ id = "greet", title = "Say Hello" }]
keybindings = [{ command = "greet", key = "Mod+Alt+H", when = "editorFocus" }]
menus = [
  { command = "greet", location = "plugins" },          # the Plugins menu
  { command = "greet", location = "editor/context" },   # the editor's right-click menu
]
status_items = [{ id = "words", alignment = "right", priority = 50 }]
activation = ["onCommand:greet", "onLanguage:markdown"]
```

```lua
local function refresh()
  local editor = notoir.editor.active()
  if not editor then
    notoir.status.set("words", nil)       -- hide the item
    return
  end
  local n = 0
  for _ in editor:text():gmatch("%S+") do n = n + 1 end
  notoir.status.set("words", { text = n .. " words", tooltip = "Words in this document", command = "greet" })
end

notoir.events.on("on_active_editor_change", refresh)
notoir.events.on("on_file_change", refresh)
refresh()
```

Clicking the status item runs the `greet` command. `on_file_change` arrives shortly after typing pauses
(`plugins.changeEventDebounceMs`), not on every key.

## 5. A setting

```toml
[contributes]
settings = [{ key = "includeCode", type = "bool", default = false, title = "Count words in code blocks" }]
```

```lua
local include_code = notoir.settings.get("includeCode")
```

The setting appears in **Settings › Plugins** and in **Settings** next to the plugin in the Plugin Manager, stored as
`plugins.hello.includeCode`. Types are `bool`, `int`, `float`, `string`, `enum` (with `values = [...]`) and `list`.

## 6. More files with `require`

Split code into modules inside the plugin folder:

```lua
-- lib/count.lua
local M = {}
function M.words(text)
  local n = 0
  for _ in text:gmatch("%S+") do n = n + 1 end
  return n
end
return M
```

```lua
-- main.lua
local count = require("lib.count")
```

`require` only loads `.lua` files from the plugin's own folder.

## Translations

Put translations in `locales/<language>.json` (`en.json`, `pt-BR.json`…, the same codes as the
[display languages](languages.md)). Nested keys are joined with dots:

```json
{
  "commands": { "greet": "Say Hello" },
  "status": "{{n}} words"
}
```

Use `%key%` in the manifest, and `notoir.i18n.t` in Lua:

```toml
commands = [{ id = "greet", title = "%commands.greet%" }]
```

```lua
notoir.status.set("words", { text = notoir.i18n.t("status", { n = n }) })
```

Missing keys fall back to English, then to the key itself, and are listed in Developer Tools.

## Debugging

- `print(...)` and `notoir.log.info / warn / error` write to **Plugins › Plugin Output**.
- An error in your code shows a notification with **View Error** and the Lua traceback in Plugin Output. Three errors
  within 60 seconds disable the plugin.
- **Plugin Developer Tools** shows the plugin's state, what activated it, how many times it called each API and
  missing translation keys.
- A command may run for 5 seconds and an event handler for 200 ms (`plugins.commandTimeoutMs`,
  `plugins.eventTimeoutMs`); each plugin may use 64 MB of memory (`plugins.memoryLimitMb`). Going over disables the
  plugin.

## The examples

The repository's [`plugins/examples`](https://github.com/lvendrame/notoir/tree/main/plugins/examples) folder has three
complete plugins to copy from:

- **word-count**: a command with a shortcut and menu items, a status item, a setting, `require` and translations
  (English and Brazilian Portuguese). The steps above are a simplified version of it.
- **todo-highlighter**: a side panel built from a declarative UI tree, an outline provider, and a new `.todo` language
  with a `.sublime-syntax` grammar.
- **markdown-table-formatter**: a formatter used by **Format Document**.

### A side panel (todo-highlighter)

```toml
[contributes]
commands = [{ id = "refresh", title = "Refresh" }, { id = "reveal", title = "Reveal" }]
panels = [{ id = "todos", title = "TODOs", area = "secondary" }]
```

```lua
notoir.panels.render("todos", { type = "column", children = {
  { type = "heading", text = "2 TODOs" },
  { type = "list", items = { { id = 3, text = "3  TODO: write tests" } }, on_select = "reveal" },
  { type = "separator" },
  { type = "button", text = "Refresh", command = "refresh" },
} })

notoir.commands.register({ id = "reveal", run = function(ctx)
  local line = ctx.args and ctx.args.item      -- the id of the clicked list item
  if ctx.editor and line then
    ctx.editor:set_selection({ start = { line = line, column = 1 }, ["end"] = { line = line, column = 1 } })
  end
end })
```

Plugin panels are shown in the **Plugins** panel. See [Panels](plugin-api.md#panels) for every element.

### A language and an outline (todo-highlighter)

```toml
[contributes]
languages = [{ id = "todo", name = "TODO list", extensions = [".todo"], syntax = "syntaxes/todo.sublime-syntax" }]
```

```lua
notoir.outline.register("todo", function(text)
  local symbols, n = {}, 0
  for line in (text .. "\n"):gmatch("(.-)\n") do
    n = n + 1
    local item = line:match("^%s*%- %[[ xX]%] (.*)")
    if item then table.insert(symbols, { name = item, line = n }) end
  end
  return symbols
end)
```

### A formatter (markdown-table-formatter)

```lua
notoir.formatters.register("markdown", function(text, opts)
  -- opts.tab_size, opts.insert_spaces
  return reformat(text)          -- the new text; Notoir applies only the lines that changed
end)
```

A formatter needs no permission: it receives the text and returns new text. If it fails, the document is left
unchanged and a warning names the plugin.
