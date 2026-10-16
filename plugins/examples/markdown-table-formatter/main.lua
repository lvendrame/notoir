-- Markdown Table Formatter: a formatter (Format Document) for Markdown.
local function cells(line)
  local inner = line:match("^%s*|(.*)|%s*$")
  if not inner then return nil end
  local out = {}
  for cell in (inner .. "|"):gmatch("(.-)|") do
    table.insert(out, (cell:gsub("^%s+", ""):gsub("%s+$", "")))
  end
  return out
end

local function is_rule(row)
  for _, c in ipairs(row) do
    if not c:match("^:?%-+:?$") then return false end
  end
  return #row > 0
end

local function format_table(rows)
  local widths = {}
  for _, row in ipairs(rows) do
    for i, c in ipairs(row) do
      if not is_rule(row) then widths[i] = math.max(widths[i] or 3, utf8.len(c) or #c) end
    end
  end
  local out = {}
  for _, row in ipairs(rows) do
    local parts = {}
    for i = 1, #widths do
      local c = row[i] or ""
      if is_rule(row) then
        local left, right = c:sub(1, 1) == ":", c:sub(-1) == ":"
        local dashes = string.rep("-", widths[i] - (left and 1 or 0) - (right and 1 or 0))
        c = (left and ":" or "") .. dashes .. (right and ":" or "")
      else
        c = c .. string.rep(" ", widths[i] - (utf8.len(c) or #c))
      end
      table.insert(parts, c)
    end
    table.insert(out, "| " .. table.concat(parts, " | ") .. " |")
  end
  return out
end

notoir.formatters.register("markdown", function(text)
  local lines, out, block = {}, {}, {}
  for line in (text .. "\n"):gmatch("(.-)\n") do table.insert(lines, line) end
  local trailing = text:sub(-1) == "\n"
  if trailing then table.remove(lines) end
  local function flush()
    if #block > 0 then
      for _, l in ipairs(format_table(block)) do table.insert(out, l) end
      block = {}
    end
  end
  for _, line in ipairs(lines) do
    local row = cells(line)
    if row then table.insert(block, row) else flush(); table.insert(out, line) end
  end
  flush()
  return table.concat(out, "\n") .. (trailing and "\n" or "")
end)
