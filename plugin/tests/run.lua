local pluginDir = 'plugin/lrcli.lrdevplugin/'
package.path = pluginDir .. '?.lua;' .. package.path

local json = require 'json'
local Dispatch = require 'Dispatch'
local Version = require 'Version'

local passed, failed = 0, 0

local function test(name, fn)
  local ok, err = pcall(fn)
  if ok then
    passed = passed + 1
  else
    failed = failed + 1
    print('FAIL ' .. name .. '\n  ' .. tostring(err))
  end
end

local function eq(actual, expected, label)
  if actual ~= expected then
    error((label or 'value') .. ': expected ' .. tostring(expected) .. ', got ' .. tostring(actual), 2)
  end
end

local function roundTrip(v)
  local decoded, err = json.decode(json.encode(v))
  if err then
    error(err, 2)
  end
  return decoded
end

test('json encodes scalars', function()
  eq(json.encode(1), '1')
  eq(json.encode(-3), '-3')
  eq(json.encode(0.5), '0.5')
  eq(json.encode(1e20), '1e+20')
  eq(json.encode(true), 'true')
  eq(json.encode(json.null), 'null')
  eq(json.encode('a'), '"a"')
end)

test('json encodes integers without a decimal point', function()
  eq(json.encode(7.0), '7')
  eq(json.encode({ id = 1234 }), '{"id":1234}')
end)

test('json rejects NaN and infinity', function()
  eq(pcall(json.encode, 0 / 0), false)
  eq(pcall(json.encode, math.huge), false)
  eq(pcall(json.encode, -math.huge), false)
end)

test('json encodes empty array marker and empty object', function()
  eq(json.encode(json.array({})), '[]')
  eq(json.encode({}), '{}')
  eq(json.encode({ 1, 2, 3 }), '[1,2,3]')
  eq(json.encode({ list = json.array() }), '{"list":[]}')
end)

test('json decodes empty array to array marker', function()
  local v = json.decode('{"a":[]}')
  eq(json.encode(v), '{"a":[]}')
end)

test('json escapes control characters and quotes', function()
  eq(json.encode('a"b\\c\n\t\1'), '"a\\"b\\\\c\\n\\t\\u0001"')
  eq(roundTrip('a"b\\c\n\t\1'), 'a"b\\c\n\t\1')
end)

test('json decodes unicode escapes and surrogate pairs', function()
  eq(json.decode('"\\u00e9"'), '\xC3\xA9')
  eq(json.decode('"\\u20ac"'), '\xE2\x82\xAC')
  eq(json.decode('"\\ud83d\\ude00"'), '\xF0\x9F\x98\x80')
  eq(json.decode('"\\u0041"'), 'A')
end)

test('json passes through raw UTF-8', function()
  eq(roundTrip('caf\xC3\xA9 \xF0\x9F\x98\x80'), 'caf\xC3\xA9 \xF0\x9F\x98\x80')
end)

test('json rejects lone surrogates', function()
  local v, err = json.decode('"\\ud83d"')
  eq(v, nil)
  eq(type(err), 'string')
  eq(json.decode('"\\ude00"'), nil)
end)

test('json decodes numbers', function()
  eq(json.decode('0'), 0)
  eq(json.decode('-12'), -12)
  eq(json.decode('1.5'), 1.5)
  eq(json.decode('1e3'), 1000)
  eq(json.decode('-2.5E-1'), -0.25)
end)

test('json round-trips nested structures', function()
  local v = roundTrip({ a = { 1, 2, { b = 'x' } }, n = json.null, t = true, f = false })
  eq(v.a[3].b, 'x')
  eq(v.n, json.null)
  eq(v.t, true)
  eq(v.f, false)
end)

test('json decode errors are returned, not thrown', function()
  local bad = { '', '{', '[1,]', '{"a":}', '{"a" 1}', 'nope', '01', '1.', '"abc', '[1] x', '{"a":1,}', '"\\q"', '"a\nb"', "{'a':1}" }
  for _, input in ipairs(bad) do
    local v, err = json.decode(input)
    eq(v, nil, 'decode(' .. input .. ')')
    eq(type(err), 'string', 'error for ' .. input)
  end
  local v, err = json.decode(nil)
  eq(v, nil)
  eq(type(err), 'string')
end)

test('json decode rejects excessive nesting', function()
  local v = json.decode(string.rep('[', 1000) .. string.rep(']', 1000))
  eq(v, nil)
end)

local TOKEN = 'abcdef0123456789abcdef0123456789'

local function newDispatch()
  return Dispatch.new({
    token = TOKEN,
    protocol = 1,
    handlers = {
      hello = function()
        return { protocol = 1, plugin = { version = '0.1.0' } }
      end,
      echo = function(args)
        return { echoed = args.value }
      end,
      boom = function()
        error('kaput')
      end,
    },
  })
end

local function send(d, request)
  return d:handle(type(request) == 'string' and request or json.encode(request))
end

local function hello(d, overrides)
  local args = { token = TOKEN, protocol = 1, client = 'test' }
  for k, v in pairs(overrides or {}) do
    args[k] = v
  end
  return send(d, { id = 1, cmd = 'hello', args = args })
end

test('dispatch hello succeeds with the right token', function()
  local reply = hello(newDispatch())
  eq(reply.ok, true)
  eq(reply.id, 1)
  eq(reply.protocol, 1)
  eq(reply.plugin.version, '0.1.0')
end)

test('dispatch rejects commands before hello', function()
  local reply = send(newDispatch(), { id = 2, cmd = 'echo', args = { value = 1 } })
  eq(reply.ok, false)
  eq(reply.error, 'unauthorized')
  eq(reply.id, 2)
end)

test('dispatch rejects a wrong or missing token', function()
  local d = newDispatch()
  eq(hello(d, { token = 'nope' }).error, 'unauthorized')
  eq(hello(d, { token = json.null }).error, 'unauthorized')
  eq(send(d, { id = 1, cmd = 'hello' }).error, 'unauthorized')
  eq(send(d, { id = 3, cmd = 'echo' }).error, 'unauthorized')
end)

test('dispatch reports protocol mismatch', function()
  local d = newDispatch()
  local reply = hello(d, { protocol = 2 })
  eq(reply.ok, false)
  eq(reply.error, 'protocol_mismatch')
  eq(send(d, { id = 2, cmd = 'echo' }).error, 'unauthorized')
end)

test('dispatch runs handlers after hello and merges the result', function()
  local d = newDispatch()
  hello(d)
  local reply = send(d, { id = 5, cmd = 'echo', args = { value = 'x' } })
  eq(reply.ok, true)
  eq(reply.id, 5)
  eq(reply.echoed, 'x')
end)

test('dispatch resetAuth requires a new hello', function()
  local d = newDispatch()
  hello(d)
  d:resetAuth()
  eq(send(d, { id = 2, cmd = 'echo' }).error, 'unauthorized')
  hello(d)
  eq(send(d, { id = 3, cmd = 'echo' }).ok, true)
end)

test('dispatch answers unknown commands', function()
  local d = newDispatch()
  hello(d)
  local reply = send(d, { id = 9, cmd = 'nope' })
  eq(reply.error, 'unknown_command')
  eq(reply.id, 9)
end)

test('dispatch turns handler errors into internal_error', function()
  local d = newDispatch()
  hello(d)
  local reply = send(d, { id = 4, cmd = 'boom' })
  eq(reply.ok, false)
  eq(reply.error, 'internal_error')
  eq(reply.id, 4)
  assert(reply.message:find('kaput', 1, true), 'message should include the Lua error')
  eq(send(d, { id = 5, cmd = 'echo' }).ok, true)
end)

test('dispatch answers unparseable input with a null id', function()
  local d = newDispatch()
  for _, line in ipairs({ 'not json', '', '[1,2]', '"str"', '42', 'null' }) do
    local reply = send(d, line)
    eq(reply.error, 'bad_request', line)
    eq(reply.id, json.null, line)
  end
end)

test('dispatch rejects missing or invalid id and cmd', function()
  local d = newDispatch()
  hello(d)
  eq(send(d, { cmd = 'echo' }).error, 'bad_request')
  eq(send(d, { cmd = 'echo' }).id, json.null)
  eq(send(d, { id = 'a', cmd = 'echo' }).error, 'bad_request')
  eq(send(d, { id = 1.5, cmd = 'echo' }).error, 'bad_request')
  local noCmd = send(d, { id = 3 })
  eq(noCmd.error, 'bad_request')
  eq(noCmd.id, 3)
  eq(send(d, { id = 3, cmd = 7 }).error, 'bad_request')
  eq(send(d, '{"id":3,"cmd":"echo","args":5}').error, 'bad_request')
end)

test('dispatch replies encode to JSON', function()
  local d = newDispatch()
  local line = json.encode(send(d, 'garbage'))
  assert(line:find('"id":null', 1, true), line)
  assert(line:find('"ok":false', 1, true), line)
end)

test('Version matches Info.lua', function()
  local info = dofile(pluginDir .. 'Info.lua')
  local v = info.VERSION
  eq(Version.PLUGIN, v.major .. '.' .. v.minor .. '.' .. v.revision)
  eq(Version.PROTOCOL, 1)
end)

print(string.format('lua tests: %d passed, %d failed', passed, failed))
if failed > 0 then
  os.exit(1)
end
