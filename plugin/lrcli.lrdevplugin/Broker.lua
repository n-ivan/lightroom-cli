local Log = require 'Log'
local PortsFile = require 'PortsFile'
local json = require 'json'

local LrPathUtils = import 'LrPathUtils'
local LrTasks = import 'LrTasks'

local Broker = {}

local function configPath()
  return LrPathUtils.child(PortsFile.dir(), 'config.json')
end

local function readBrokerPath()
  local file = io.open(configPath(), 'r')
  if not file then
    return nil, configPath() .. ' is missing. Run `lrcli plugin install`.'
  end
  local text = file:read('*a')
  file:close()
  local config, err = json.decode(text)
  if type(config) ~= 'table' or type(config.brokerPath) ~= 'string' then
    return nil, 'invalid ' .. configPath() .. (err and (': ' .. err) or '') .. '. Run `lrcli plugin install --force`.'
  end
  return config.brokerPath
end

function Broker.spawn()
  local path, err = readBrokerPath()
  if not path then
    Log.error('cannot start broker: ' .. err)
    return
  end
  local status = LrTasks.execute(PortsFile.shellQuote(path) .. ' broker >/dev/null 2>&1 &')
  if status == 0 then
    Log.info('spawned broker ' .. path)
  else
    Log.error('spawning broker ' .. path .. ' exited with status ' .. tostring(status))
  end
end

return Broker
