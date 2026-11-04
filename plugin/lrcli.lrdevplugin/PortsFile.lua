local json = require 'json'

local LrFileUtils = import 'LrFileUtils'
local LrPathUtils = import 'LrPathUtils'
local LrTasks = import 'LrTasks'

local PortsFile = {}

function PortsFile.shellQuote(path)
  return "'" .. path:gsub("'", "'\\''") .. "'"
end

local function homeDir()
  return LrPathUtils.getStandardFilePath('home')
end

function PortsFile.dir()
  return LrPathUtils.child(homeDir(), '.lrcli')
end

function PortsFile.path()
  return LrPathUtils.child(PortsFile.dir(), 'ports.json')
end

function PortsFile.generateToken()
  local f = assert(io.open('/dev/urandom', 'rb'))
  local bytes = f:read(16)
  f:close()
  assert(bytes and #bytes == 16, 'could not read 16 bytes from /dev/urandom')
  return (bytes:gsub('.', function(c)
    return string.format('%02x', c:byte())
  end))
end

local function chmod(mode, path)
  local status = LrTasks.execute('chmod ' .. mode .. ' ' .. PortsFile.shellQuote(path))
  if status ~= 0 then
    error('chmod ' .. mode .. ' failed for ' .. path)
  end
end

local function moveIntoPlace(from, to)
  if os.rename then
    local ok = os.rename(from, to)
    if ok then
      return
    end
  end
  if LrFileUtils.exists(to) then
    LrFileUtils.delete(to)
  end
  if not LrFileUtils.move(from, to) then
    error('could not move ' .. from .. ' to ' .. to)
  end
end

function PortsFile.write(state)
  local dir = PortsFile.dir()
  LrFileUtils.createAllDirectories(dir)
  chmod('700', dir)

  local tempPath = PortsFile.path() .. '.tmp'
  local file = assert(io.open(tempPath, 'w'))
  file:close()
  chmod('600', tempPath)

  file = assert(io.open(tempPath, 'w'))
  file:write(json.encode({
    version = 1,
    commandPort = state.commandPort,
    replyPort = state.replyPort,
    token = state.token,
    pluginVersion = state.pluginVersion,
    instanceStartedAt = state.instanceStartedAt,
    updatedAt = os.time(),
  }))
  file:close()
  moveIntoPlace(tempPath, PortsFile.path())
end

function PortsFile.read()
  local file = io.open(PortsFile.path(), 'r')
  if not file then
    return nil
  end
  local text = file:read('*a')
  file:close()
  local ports = json.decode(text)
  if type(ports) ~= 'table' then
    return nil
  end
  return ports
end

function PortsFile.deleteIfOwned(token)
  local ports = PortsFile.read()
  if ports and ports.token == token then
    LrFileUtils.delete(PortsFile.path())
  end
end

return PortsFile
