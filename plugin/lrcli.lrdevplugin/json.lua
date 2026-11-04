local json = {}

json.null = setmetatable({}, {
  __tostring = function()
    return 'null'
  end,
})

local arrayMarker = {}

function json.array(t)
  return setmetatable(t or {}, arrayMarker)
end

local escapes = {
  ['"'] = '\\"',
  ['\\'] = '\\\\',
  ['\b'] = '\\b',
  ['\f'] = '\\f',
  ['\n'] = '\\n',
  ['\r'] = '\\r',
  ['\t'] = '\\t',
}

local function escapeChar(c)
  return escapes[c] or string.format('\\u%04x', c:byte())
end

local function encodeString(s)
  return '"' .. s:gsub('[%c"\\]', escapeChar) .. '"'
end

local function encodeNumber(n)
  if n ~= n or n == math.huge or n == -math.huge then
    error('cannot encode NaN or infinity')
  end
  if n == math.floor(n) and math.abs(n) < 2 ^ 53 then
    return string.format('%d', n)
  end
  return string.format('%.17g', n)
end

local function isArray(t)
  if getmetatable(t) == arrayMarker then
    return true
  end
  local count, max = 0, 0
  for k in pairs(t) do
    if type(k) ~= 'number' or k < 1 or k ~= math.floor(k) then
      return false
    end
    count = count + 1
    if k > max then
      max = k
    end
  end
  return count > 0 and count == max
end

local function encodeValue(v, seen)
  local t = type(v)
  if v == json.null or v == nil then
    return 'null'
  elseif t == 'boolean' then
    return tostring(v)
  elseif t == 'number' then
    return encodeNumber(v)
  elseif t == 'string' then
    return encodeString(v)
  elseif t == 'table' then
    if seen[v] then
      error('cannot encode circular table')
    end
    seen[v] = true
    local parts = {}
    if isArray(v) then
      for i = 1, #v do
        parts[i] = encodeValue(v[i], seen)
      end
      seen[v] = nil
      return '[' .. table.concat(parts, ',') .. ']'
    end
    local keys = {}
    for k in pairs(v) do
      if type(k) ~= 'string' then
        error('object keys must be strings')
      end
      keys[#keys + 1] = k
    end
    table.sort(keys)
    for i, k in ipairs(keys) do
      parts[i] = encodeString(k) .. ':' .. encodeValue(v[k], seen)
    end
    seen[v] = nil
    return '{' .. table.concat(parts, ',') .. '}'
  end
  error('cannot encode value of type ' .. t)
end

function json.encode(v)
  return encodeValue(v, {})
end

local function utf8Encode(cp)
  if cp < 0x80 then
    return string.char(cp)
  elseif cp < 0x800 then
    return string.char(0xC0 + math.floor(cp / 0x40), 0x80 + cp % 0x40)
  elseif cp < 0x10000 then
    return string.char(
      0xE0 + math.floor(cp / 0x1000),
      0x80 + math.floor(cp / 0x40) % 0x40,
      0x80 + cp % 0x40
    )
  end
  return string.char(
    0xF0 + math.floor(cp / 0x40000),
    0x80 + math.floor(cp / 0x1000) % 0x40,
    0x80 + math.floor(cp / 0x40) % 0x40,
    0x80 + cp % 0x40
  )
end

local function fail(str, pos, msg)
  error({ json = msg .. ' at position ' .. pos }, 0)
end

local function skipSpace(str, pos)
  return str:find('[^ \t\r\n]', pos) or #str + 1
end

local decodeValue

local function decodeString(str, pos)
  local out = {}
  local i = pos + 1
  while true do
    local j = str:find('["\\%c]', i)
    if not j then
      fail(str, pos, 'unterminated string')
    end
    out[#out + 1] = str:sub(i, j - 1)
    local c = str:sub(j, j)
    if c == '"' then
      return table.concat(out), j + 1
    elseif c ~= '\\' then
      fail(str, j, 'control character in string')
    end
    local e = str:sub(j + 1, j + 1)
    local simple = { ['"'] = '"', ['\\'] = '\\', ['/'] = '/', b = '\b', f = '\f', n = '\n', r = '\r', t = '\t' }
    if simple[e] then
      out[#out + 1] = simple[e]
      i = j + 2
    elseif e == 'u' then
      local hex = str:match('^%x%x%x%x', j + 2)
      if not hex then
        fail(str, j, 'invalid \\u escape')
      end
      local cp = tonumber(hex, 16)
      i = j + 6
      if cp >= 0xD800 and cp <= 0xDBFF then
        local low = str:match('^\\u(%x%x%x%x)', i)
        local lowCp = low and tonumber(low, 16)
        if not lowCp or lowCp < 0xDC00 or lowCp > 0xDFFF then
          fail(str, j, 'unpaired surrogate')
        end
        cp = 0x10000 + (cp - 0xD800) * 0x400 + (lowCp - 0xDC00)
        i = i + 6
      elseif cp >= 0xDC00 and cp <= 0xDFFF then
        fail(str, j, 'unpaired surrogate')
      end
      out[#out + 1] = utf8Encode(cp)
    else
      fail(str, j, 'invalid escape')
    end
  end
end

local function decodeNumber(str, pos)
  local int = str:match('^-?%d+', pos)
  if not int or str:match('^-?0%d', pos) then
    fail(str, pos, 'invalid number')
  end
  local last = pos + #int
  local frac = str:match('^%.%d+', last)
  if frac then
    last = last + #frac
  end
  local exp = str:match('^[eE][+-]?%d+', last)
  if exp then
    last = last + #exp
  end
  return tonumber(str:sub(pos, last - 1)), last
end

local function decodeLiteral(str, pos, word, value)
  if str:sub(pos, pos + #word - 1) ~= word then
    fail(str, pos, 'unexpected character')
  end
  return value, pos + #word
end

local function decodeArray(str, pos, depth)
  local arr = json.array({})
  pos = skipSpace(str, pos + 1)
  if str:sub(pos, pos) == ']' then
    return arr, pos + 1
  end
  while true do
    local v
    v, pos = decodeValue(str, pos, depth)
    arr[#arr + 1] = v
    pos = skipSpace(str, pos)
    local c = str:sub(pos, pos)
    if c == ']' then
      return arr, pos + 1
    elseif c ~= ',' then
      fail(str, pos, "expected ',' or ']'")
    end
    pos = skipSpace(str, pos + 1)
  end
end

local function decodeObject(str, pos, depth)
  local obj = {}
  pos = skipSpace(str, pos + 1)
  if str:sub(pos, pos) == '}' then
    return obj, pos + 1
  end
  while true do
    if str:sub(pos, pos) ~= '"' then
      fail(str, pos, 'expected string key')
    end
    local key, v
    key, pos = decodeString(str, pos)
    pos = skipSpace(str, pos)
    if str:sub(pos, pos) ~= ':' then
      fail(str, pos, "expected ':'")
    end
    v, pos = decodeValue(str, skipSpace(str, pos + 1), depth)
    obj[key] = v
    pos = skipSpace(str, pos)
    local c = str:sub(pos, pos)
    if c == '}' then
      return obj, pos + 1
    elseif c ~= ',' then
      fail(str, pos, "expected ',' or '}'")
    end
    pos = skipSpace(str, pos + 1)
  end
end

decodeValue = function(str, pos, depth)
  depth = (depth or 0) + 1
  if depth > 200 then
    fail(str, pos, 'nesting too deep')
  end
  local c = str:sub(pos, pos)
  if c == '{' then
    return decodeObject(str, pos, depth)
  elseif c == '[' then
    return decodeArray(str, pos, depth)
  elseif c == '"' then
    return decodeString(str, pos)
  elseif c == 't' then
    return decodeLiteral(str, pos, 'true', true)
  elseif c == 'f' then
    return decodeLiteral(str, pos, 'false', false)
  elseif c == 'n' then
    return decodeLiteral(str, pos, 'null', json.null)
  elseif c == '-' or c:match('%d') then
    return decodeNumber(str, pos)
  elseif c == '' then
    fail(str, pos, 'unexpected end of input')
  end
  fail(str, pos, 'unexpected character')
end

function json.decode(str)
  if type(str) ~= 'string' then
    return nil, 'input must be a string'
  end
  local ok, value, pos = pcall(function()
    local v, p = decodeValue(str, skipSpace(str, 1), 0)
    p = skipSpace(str, p)
    if p <= #str then
      fail(str, p, 'trailing characters')
    end
    return v, p
  end)
  if ok then
    return value
  end
  if type(value) == 'table' and value.json then
    return nil, value.json
  end
  return nil, tostring(value)
end

return json
