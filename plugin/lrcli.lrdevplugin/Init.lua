local Log = require 'Log'

Log.info('init')
local ok, err = pcall(function()
  require('Server').start()
end)
if not ok then
  Log.error('init failed: ' .. tostring(err))
end
