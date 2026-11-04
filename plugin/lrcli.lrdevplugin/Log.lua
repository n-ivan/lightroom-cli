local LrFileUtils = import 'LrFileUtils'
local LrPathUtils = import 'LrPathUtils'

-- LrLogger's 'logfile' location differs across LrC versions and can be blocked
-- by macOS privacy protection, so log next to the ports file instead.
local Log = {}

local MAX_RECENT = 30
local recent = {}

local function remember(line)
  recent[#recent + 1] = line
  if #recent > MAX_RECENT then
    table.remove(recent, 1)
  end
end

local function logPath()
  local dir = LrPathUtils.child(LrPathUtils.getStandardFilePath('home'), '.lrcli')
  LrFileUtils.createAllDirectories(dir)
  return LrPathUtils.child(dir, 'plugin.log')
end

local fileFailureReported = false

local function appendToFile(line)
  local ok, err = pcall(function()
    local file = assert(io.open(logPath(), 'a'))
    file:write(line, '\n')
    file:close()
  end)
  if not ok and not fileFailureReported then
    fileFailureReported = true
    remember('log file unavailable: ' .. tostring(err))
  end
end

local function write(level, message)
  local line = os.date('%Y-%m-%d %H:%M:%S') .. ' ' .. level .. ' ' .. tostring(message)
  remember(line)
  appendToFile(line)
end

function Log.info(message)
  write('INFO', message)
end

function Log.error(message)
  write('ERROR', message)
end

function Log.recent()
  return recent
end

return Log
