local Log = require 'Log'

local function statusText()
  local lines = Log.recent()
  if #lines == 0 then
    return 'No log entries: Init.lua has not run in this session.'
  end
  return table.concat(lines, '\n')
end

return {
  sectionsForTopOfDialog = function(f)
    return {
      {
        title = 'lrcli status',
        f:static_text({
          title = statusText(),
          selectable = true,
          width_in_chars = 80,
          height_in_lines = 12,
        }),
      },
    }
  end,
}
