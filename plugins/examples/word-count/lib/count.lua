local M = {}

-- Words in `text`; fenced code blocks count only when `include_code`.
function M.words(text, include_code)
  local n, in_code = 0, false
  for line in (text .. "\n"):gmatch("(.-)\n") do
    if line:match("^%s*```") then
      in_code = not in_code
    elseif include_code or not in_code then
      for _ in line:gmatch("%S+") do n = n + 1 end
    end
  end
  return n
end

return M
