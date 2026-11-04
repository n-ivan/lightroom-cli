local Log = require 'Log'

local LrFileUtils = import 'LrFileUtils'

-- LrC's require only says "Could not load toolkit script: <name>"; compiling
-- each file with loadstring recovers the actual syntax error and line, and
-- requiring each module separately shows which one fails at load time.
local function diagnoseLoadFailure()
  if type(loadstring) ~= 'function' then
    Log.error('loadstring is unavailable; cannot diagnose further')
    return
  end
  for path in LrFileUtils.files(_PLUGIN.path) do
    if path:match('%.lua$') then
      local name = path:match('([^/]+)%.lua$')
      local _, compileErr = loadstring(LrFileUtils.readFile(path), name)
      if compileErr then
        Log.error('compile error: ' .. compileErr)
      elseif name ~= 'Init' and name ~= 'Info' then
        local loaded, loadErr = pcall(require, name)
        if not loaded then
          Log.error('loading ' .. name .. ' failed: ' .. tostring(loadErr))
        end
      end
    end
  end
end

Log.info('init')
local ok, err = pcall(function()
  require('Server').start()
end)
if not ok then
  Log.error('init failed: ' .. tostring(err))
  diagnoseLoadFailure()
end
