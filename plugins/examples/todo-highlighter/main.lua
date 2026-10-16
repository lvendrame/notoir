-- TODO Highlighter: a declarative panel, an outline provider and a language.
local found = {}

local function scan()
  found = {}
  local editor = notoir.editor.active()
  if editor then
    local n = 0
    for line in (editor:text() .. "\n"):gmatch("(.-)\n") do
      n = n + 1
      local tag, rest = line:match("(%u%u%u%u?%u?)[:%s]%s*(.*)")
      if tag == "TODO" or tag == "FIXME" then
        table.insert(found, { line = n, text = tag .. ": " .. rest })
      end
    end
  end
  local items = {}
  for _, f in ipairs(found) do
    table.insert(items, { id = f.line, text = f.line .. "  " .. f.text })
  end
  local children = { { type = "heading", text = notoir.i18n.t("count", { n = #found }) } }
  if #items > 0 then
    table.insert(children, { type = "list", items = items, on_select = "reveal" })
  else
    table.insert(children, { type = "label", text = notoir.i18n.t("none") })
  end
  table.insert(children, { type = "separator" })
  table.insert(children, { type = "button", text = notoir.i18n.t("refresh"), command = "refresh" })
  notoir.panels.render("todos", { type = "column", children = children })
end

notoir.commands.register({ id = "refresh", run = scan })
notoir.commands.register({ id = "reveal", run = function(ctx)
  local line = ctx.args and ctx.args.item
  if ctx.editor and line then
    ctx.editor:set_selection({ start = { line = line, column = 1 }, ["end"] = { line = line, column = 1 } })
  end
end })

-- `.todo` files: every `[ ]` / `[x]` item is an outline entry.
notoir.outline.register("todo", function(text)
  local symbols, n = {}, 0
  for line in (text .. "\n"):gmatch("(.-)\n") do
    n = n + 1
    local item = line:match("^%s*%- %[[ xX]%] (.*)")
    if item then table.insert(symbols, { name = item, line = n }) end
  end
  return symbols
end)

notoir.events.on("on_active_editor_change", scan)
notoir.events.on("on_file_change", scan)
scan()
