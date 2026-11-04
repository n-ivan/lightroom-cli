local LrDialogs = import 'LrDialogs'
local LrTasks = import 'LrTasks'
local Log = require 'Log'

LrTasks.startAsyncTask(function()
  local lines = Log.recent()
  local text = #lines > 0 and table.concat(lines, '\n') or 'No log entries yet.'
  LrDialogs.message('lrcli status', text, 'info')
end)
