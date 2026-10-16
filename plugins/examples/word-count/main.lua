-- Word Count: a command, a status item and a setting.
local util = require("lib.count")

local function refresh()
  local editor = notoir.editor.active()
  if not editor then
    notoir.status.set("words", nil)
    return nil
  end
  local include_code = notoir.settings.get("includeCode")
  local n = util.words(editor:text(), include_code)
  notoir.status.set("words", {
    text = notoir.i18n.t("status", { n = n }),
    tooltip = notoir.i18n.t("tooltip"),
    command = "count",
  })
  return n
end

notoir.commands.register({ id = "count", run = function()
  local n = refresh()
  if n then notoir.notify(notoir.i18n.t("result", { n = n })) end
end })

notoir.events.on("on_active_editor_change", refresh)
notoir.events.on("on_file_change", refresh)
refresh()
