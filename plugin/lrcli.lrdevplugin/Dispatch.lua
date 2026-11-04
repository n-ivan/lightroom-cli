local json = require 'json'

local Dispatch = {}
Dispatch.__index = Dispatch

function Dispatch.new(options)
  return setmetatable({
    handlers = options.handlers,
    token = options.token,
    protocol = options.protocol,
    -- Handlers yield (LrTasks.sleep, catalog calls), which Lua 5.1's pcall forbids.
    protectedCall = options.protectedCall or pcall,
    authenticated = false,
  }, Dispatch)
end

function Dispatch:resetAuth()
  self.authenticated = false
end

local function errorReply(id, code, message)
  return { id = id, ok = false, error = code, message = message }
end

local function isInteger(n)
  return type(n) == 'number' and n == math.floor(n) and math.abs(n) < 2 ^ 53
end

local function tokensEqual(a, b)
  if type(a) ~= 'string' or type(b) ~= 'string' or #a ~= #b then
    return false
  end
  local diff = 0
  for i = 1, #a do
    if a:byte(i) ~= b:byte(i) then
      diff = diff + 1
    end
  end
  return diff == 0
end

function Dispatch:authenticate(id, args)
  if type(args) ~= 'table' or not tokensEqual(args.token, self.token) then
    return errorReply(
      id,
      'unauthorized',
      'Invalid or missing token. Run the command again so lrcli rereads ~/.lrcli/ports.json; if it keeps failing, restart Lightroom Classic.'
    )
  end
  if args.protocol ~= self.protocol then
    return errorReply(
      id,
      'protocol_mismatch',
      'Client speaks protocol ' .. tostring(args.protocol) .. ' but the plugin speaks protocol '
        .. tostring(self.protocol) .. '. Update lrcli and the plugin to the same version, then restart Lightroom Classic.'
    )
  end
  self.authenticated = true
  return nil
end

function Dispatch:run(id, cmd, args)
  local handler = self.handlers[cmd]
  if not handler then
    return errorReply(
      id,
      'unknown_command',
      'Unknown command "' .. cmd .. '". Update lrcli and the plugin to the same version, or run `lrcli --help` for the available commands.'
    )
  end
  local ok, result = self.protectedCall(handler, args, self)
  if not ok and type(result) == 'table' and type(result.code) == 'string' then
    return errorReply(id, result.code, tostring(result.message or result.code))
  end
  if not ok then
    return errorReply(
      id,
      'internal_error',
      'The command failed inside Lightroom Classic: ' .. tostring(result)
        .. '. Check ~/.lrcli/plugin.log and report a bug if it persists.'
    )
  end
  local reply = { id = id, ok = true }
  for k, v in pairs(result or {}) do
    if k ~= 'id' and k ~= 'ok' then
      reply[k] = v
    end
  end
  return reply
end

function Dispatch:handle(line)
  local request, err = json.decode(line)
  if request == nil or type(request) ~= 'table' or getmetatable(request) ~= nil then
    return errorReply(
      json.null,
      'bad_request',
      'Request is not a JSON object' .. (err and (' (' .. err .. ')') or '')
        .. '. Send one JSON object per line with integer "id" and string "cmd".'
    )
  end
  if not isInteger(request.id) then
    return errorReply(json.null, 'bad_request', 'Missing or invalid "id". It must be an integer.')
  end
  local id = request.id
  if type(request.cmd) ~= 'string' or request.cmd == '' then
    return errorReply(id, 'bad_request', 'Missing or invalid "cmd". It must be a non-empty string.')
  end
  local args = request.args
  if args == nil or args == json.null then
    args = {}
  elseif type(args) ~= 'table' then
    return errorReply(id, 'bad_request', '"args" must be a JSON object when present.')
  end

  if request.cmd == 'hello' then
    local rejection = self:authenticate(id, args)
    if rejection then
      return rejection
    end
  elseif not self.authenticated then
    return errorReply(
      id,
      'unauthorized',
      'Send "hello" with the token from ~/.lrcli/ports.json before any other command.'
    )
  end
  return self:run(id, request.cmd, args)
end

return Dispatch
