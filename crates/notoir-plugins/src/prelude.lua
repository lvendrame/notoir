-- The `notoir` API (docs/specs/30 §API v1), built over the host's native
-- functions in `__notoir`. Every call is checked by Notoir, never here.
local native = __notoir
__notoir = nil
local call = native.call

notoir = { _commands = {}, _events = {} }

notoir.commands = {
  register = function(spec)
    if type(spec) ~= "table" or type(spec.id) ~= "string" or type(spec.run) ~= "function" then
      error("notoir.commands.register expects { id = string, run = function }", 2)
    end
    call("commands.declared", { id = spec.id })
    notoir._commands[spec.id] = spec.run
  end,
  execute = function(id, args)
    return call("commands.execute", { id = id, args = args })
  end,
}

function notoir.notify(message, opts)
  call("notify", { message = tostring(message), level = (opts and opts.level) or "info" })
end

local Editor = {}
Editor.__index = Editor
function Editor:text() return call("editor.text", { view = self.view }) end
function Editor:selection() return call("editor.selection", { view = self.view }) end
function Editor:selections() return call("editor.selections", { view = self.view }) end
function Editor:path() return call("editor.path", { view = self.view }) end
function Editor:language() return call("editor.language", { view = self.view }) end
function Editor:replace(range, text)
  return call("editor.replace", { view = self.view, range = range, text = tostring(text) })
end
function Editor:insert(text) return call("editor.insert", { view = self.view, text = tostring(text) }) end
function Editor:set_selection(range) return call("editor.set_selection", { view = self.view, range = range }) end

notoir.editor = {
  active = function()
    local view = call("editor.active", {})
    if view == nil then return nil end
    return setmetatable({ view = view }, Editor)
  end,
}

notoir.documents = {
  open = function(path) return call("documents.open", { path = path }) end,
  list = function() return call("documents.list", {}) end,
}

notoir.fs = {
  read = function(path) return native.fs("read", path) end,
  list = function(dir) return native.fs("list", dir or ".") end,
  write = function(path, text) return native.fs("write", path, tostring(text)) end,
}

notoir.workspace = {
  root = function() return call("workspace.root", {}) end,
  find_files = function(glob) return call("workspace.find_files", { glob = glob }) end,
}

notoir.settings = {
  get = function(key) return call("settings.get", { key = key }) end,
  set = function(key, value) return call("settings.set", { key = key, value = value }) end,
}

notoir.events = {
  on = function(name, fn)
    if type(name) ~= "string" or type(fn) ~= "function" then
      error("notoir.events.on expects (name, function)", 2)
    end
    local list = notoir._events[name]
    if not list then
      list = {}
      notoir._events[name] = list
      call("events.subscribe", { name = name })
    end
    table.insert(list, fn)
  end,
}

notoir.status = {
  set = function(id, item) return call("status.set", { id = id, item = item }) end,
}
notoir.panels = {
  render = function(id, tree) return call("panels.render", { id = id, tree = tree }) end,
}
notoir.formatters = {
  register = function(language, fn) return call("formatters.register", { language = language }) end,
}
notoir.outline = {
  register = function(language, fn) return call("outline.register", { language = language }) end,
}

notoir.http = { get = function(url, opts) return native.http_get(url, opts) end }
notoir.process = {
  run = function(cmd, args, opts) return native.process_run(cmd, args or {}, opts) end,
}
notoir.clipboard = {
  get = function() return native.clipboard("get") end,
  set = function(text) return native.clipboard("set", tostring(text)) end,
}
notoir.i18n = { t = function(key, args) return native.t(key, args) end }
notoir.log = {
  info = function(m) native.log("info", tostring(m)) end,
  warn = function(m) native.log("warn", tostring(m)) end,
  error = function(m) native.log("error", tostring(m)) end,
}

function print(...)
  local parts = {}
  for i = 1, select("#", ...) do parts[#parts + 1] = tostring(select(i, ...)) end
  native.log("info", table.concat(parts, "\t"))
end

-- Host entry points.
function notoir._run_command(id, args)
  local fn = notoir._commands[id]
  if not fn then error("command " .. id .. " was declared but not registered") end
  local ctx = { editor = notoir.editor.active(), args = args }
  return fn(ctx)
end

function notoir._emit(name, payload)
  local list = notoir._events[name]
  if not list then return end
  for _, fn in ipairs(list) do fn(payload) end
end
