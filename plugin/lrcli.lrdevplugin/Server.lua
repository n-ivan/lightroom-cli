local Broker = require 'Broker'
local Dispatch = require 'Dispatch'
local Handlers = require 'Handlers'
local Log = require 'Log'
local PortsFile = require 'PortsFile'
local Version = require 'Version'
local json = require 'json'

local LrApplication = import 'LrApplication'
local LrDate = import 'LrDate'
local LrFunctionContext = import 'LrFunctionContext'
local LrSocket = import 'LrSocket'
local LrTasks = import 'LrTasks'

local TICK_SECONDS = 0.05
local SPAWN_RETRY_SECONDS = 15
local SUPERSEDE_CHECK_SECONDS = 1
-- The broker pings every 30 s, so this much silence means it is gone or wedged.
local STALE_SECONDS = 90

local Server = {}

local current

local function now()
  return LrDate.currentTime()
end

local function encodeReply(reply)
  local ok, line = pcall(json.encode, reply)
  if ok then
    return line
  end
  Log.error('could not encode reply: ' .. tostring(line))
  return json.encode({
    id = reply.id,
    ok = false,
    error = 'internal_error',
    message = 'The reply could not be encoded as JSON: ' .. tostring(line),
  })
end

local function closeQuietly(socket)
  if socket then
    LrTasks.pcall(function()
      socket:close()
    end)
  end
end

-- One long-lived broker connection per Lightroom session. Rebinding is the
-- recovery path only: doing socket work right after a client disconnect
-- deadlocked LrC 15.6, so callbacks just record events and the loop acts later.
local function newServer(context)
  local self = {
    running = true,
    startedAt = now(),
    token = PortsFile.generateToken(),
    generation = 0,
    commandQueue = {},
    replyQueue = {},
    connectionEvents = {},
  }

  self.dispatch = Dispatch.new({
    handlers = Handlers.new({
      pluginVersion = Version.PLUGIN,
      protocol = Version.PROTOCOL,
      application = LrApplication,
    }),
    token = self.token,
    protocol = Version.PROTOCOL,
    protectedCall = LrTasks.pcall,
  })

  local function resetSession()
    self.commandQueue = {}
    self.replyQueue = {}
    self.commandConnected = false
    self.replyConnected = false
    self.lastMessageAt = nil
    self.commandNeedsReconnect = false
    self.replyNeedsReconnect = false
    self.needsRebind = false
    self.dispatch:resetAuth()
  end

  local function bindPair()
    self.generation = self.generation + 1
    local generation = self.generation
    local function isCurrent()
      return self.running and generation == self.generation
    end

    local function onError(reconnectFlag)
      return function(_, err)
        if not isCurrent() then
          return
        end
        if err == 'timeout' then
          self[reconnectFlag] = true
        else
          self.socketError = tostring(err)
          self.needsRebind = true
        end
      end
    end

    local function onConnecting(field)
      return function(_, port)
        if isCurrent() and self[field] ~= port then
          self[field] = port
          self.portsDirty = true
        end
      end
    end

    local function onClosed(name)
      return function()
        if isCurrent() then
          self.socketError = name .. ' socket closed'
          self.needsRebind = true
        end
      end
    end

    -- LrSocket is one-directional, so replies need a second socket.
    self.replySocket = LrSocket.bind({
      functionContext = context,
      plugin = _PLUGIN,
      port = 0,
      mode = 'send',
      onConnecting = onConnecting('replyPort'),
      onConnected = function()
        if isCurrent() then
          self.replyConnected = true
          self.connectionEvents[#self.connectionEvents + 1] = 'reply connected'
        end
      end,
      onClosed = onClosed('reply'),
      onError = onError('replyNeedsReconnect'),
    })

    self.commandSocket = LrSocket.bind({
      functionContext = context,
      plugin = _PLUGIN,
      port = 0,
      mode = 'receive',
      onConnecting = onConnecting('commandPort'),
      onConnected = function()
        if isCurrent() then
          self.commandConnected = true
          self.lastMessageAt = now()
          self.connectionEvents[#self.connectionEvents + 1] = 'command connected'
        end
      end,
      onMessage = function(_, message)
        if isCurrent() then
          self.lastMessageAt = now()
          self.commandQueue[#self.commandQueue + 1] = message
        end
      end,
      onClosed = onClosed('command'),
      onError = onError('commandNeedsReconnect'),
    })
  end

  local function rebind(reason)
    Log.info('rebinding sockets: ' .. reason)
    -- Bump first so callbacks fired by close() are ignored as stale.
    self.generation = self.generation + 1
    closeQuietly(self.commandSocket)
    closeQuietly(self.replySocket)
    self.commandSocket, self.replySocket = nil, nil
    self.commandPort, self.replyPort = nil, nil
    resetSession()
    LrTasks.sleep(0.1)
    bindPair()
    self.spawnedGeneration = nil
  end

  local function reconnect(socket, name)
    local ok, err = LrTasks.pcall(function()
      socket:reconnect()
    end)
    if not ok then
      Log.error(name .. ' reconnect failed: ' .. tostring(err))
      self.needsRebind = true
    end
  end

  local function writePortsIfReady()
    if not (self.portsDirty and self.commandPort and self.replyPort) then
      return
    end
    self.portsDirty = false
    local ok, err = LrTasks.pcall(PortsFile.write, {
      commandPort = self.commandPort,
      replyPort = self.replyPort,
      token = self.token,
      pluginVersion = Version.PLUGIN,
      instanceStartedAt = self.startedAt,
    })
    if ok then
      self.portsWrittenGeneration = self.generation
      Log.info('listening: command ' .. self.commandPort .. ', reply ' .. self.replyPort)
    else
      Log.error('could not write ports file: ' .. tostring(err))
    end
  end

  local function spawnBrokerIfNeeded()
    if self.commandConnected or self.portsWrittenGeneration ~= self.generation then
      return
    end
    local due = self.spawnedGeneration ~= self.generation
      or now() - (self.spawnedAt or 0) >= SPAWN_RETRY_SECONDS
    if due then
      self.spawnedGeneration = self.generation
      self.spawnedAt = now()
      Broker.spawn()
    end
  end

  local function flushReplies()
    if not self.replyConnected then
      return
    end
    for _, line in ipairs(self.replyQueue) do
      self.replySocket:send(line .. '\n')
    end
    self.replyQueue = {}
  end

  local function handleOneCommand()
    local line = table.remove(self.commandQueue, 1)
    if not line then
      return
    end
    local ok, reply = LrTasks.pcall(self.dispatch.handle, self.dispatch, line)
    if ok then
      self.replyQueue[#self.replyQueue + 1] = encodeReply(reply)
    else
      Log.error('dispatch failed: ' .. tostring(reply))
    end
  end

  -- A reloaded plugin gets a fresh global environment, so the previous
  -- instance can't be found through _G; the newest instance's ports file wins.
  local function isSuperseded()
    if now() - (self.supersedeCheckedAt or 0) < SUPERSEDE_CHECK_SECONDS then
      return false
    end
    self.supersedeCheckedAt = now()
    local ok, ports = LrTasks.pcall(PortsFile.read)
    if not ok or self.portsWrittenGeneration ~= self.generation then
      return false
    end
    if ports and ports.token == self.token then
      return false
    end
    if ports and type(ports.instanceStartedAt) == 'number' and ports.instanceStartedAt > self.startedAt then
      return true
    end
    self.portsDirty = true
    return false
  end

  local function tick()
    if isSuperseded() then
      Log.info('a newer plugin instance took over; stopping this one')
      self.stop()
      return
    end
    for _, event in ipairs(self.connectionEvents) do
      Log.info(event)
    end
    self.connectionEvents = {}
    if self.needsRebind then
      rebind(self.socketError or 'unknown reason')
      self.socketError = nil
      return
    end
    if self.commandConnected and now() - self.lastMessageAt > STALE_SECONDS then
      rebind('no message from broker for ' .. STALE_SECONDS .. ' s')
      return
    end
    if self.commandNeedsReconnect then
      self.commandNeedsReconnect = false
      reconnect(self.commandSocket, 'command')
    end
    if self.replyNeedsReconnect then
      self.replyNeedsReconnect = false
      reconnect(self.replySocket, 'reply')
    end
    writePortsIfReady()
    spawnBrokerIfNeeded()
    handleOneCommand()
    flushReplies()
  end

  function self.stop()
    if not self.running then
      return
    end
    self.running = false
    self.generation = self.generation + 1
    closeQuietly(self.commandSocket)
    closeQuietly(self.replySocket)
    pcall(PortsFile.deleteIfOwned, self.token)
  end

  function self.loop()
    resetSession()
    bindPair()
    while self.running do
      tick()
      LrTasks.sleep(TICK_SECONDS)
    end
  end

  return self
end

function Server.start()
  Log.info('starting plugin ' .. Version.PLUGIN)

  LrFunctionContext.postAsyncTaskWithContext('lrcli server', function(context)
    context:addCleanupHandler(function()
      if current then
        current.stop()
      end
    end)
    local ok, err = LrTasks.pcall(function()
      current = newServer(context)
      current.loop()
    end)
    if not ok then
      Log.error('server crashed: ' .. tostring(err))
      if current then
        current.stop()
      end
    end
  end)
end

return Server
