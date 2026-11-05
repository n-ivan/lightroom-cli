local json = require 'json'
local Photos = require 'Photos'

local Writes = {}

local WRITE_TIMEOUT_SECONDS = 10
local MAX_SUGGESTIONS = 8
local MAX_KEYWORD_DEPTH = 50
local FLAG_VALUES = { picked = 1, none = 0, rejected = -1 }
local FLAG_NAMES = { [1] = 'picked', [0] = 'none', [-1] = 'rejected' }
-- setRawMetadata gives any other string a white label, so only these may be sent.
local LABELS = { red = true, yellow = true, green = true, blue = true, purple = true, none = true }

local function fail(code, message)
  error({ code = code, message = message }, 0)
end

local function badRequest(message)
  fail('bad_request', message)
end

local function isInteger(n)
  return type(n) == 'number' and n == math.floor(n) and math.abs(n) < 2 ^ 53
end

local function isFinite(n)
  return type(n) == 'number' and n == n and n ~= math.huge and n ~= -math.huge
end

local function isObject(value)
  return type(value) == 'table' and getmetatable(value) == nil
end

local function sortedKeys(t)
  local keys = {}
  for key in pairs(t) do
    keys[#keys + 1] = key
  end
  table.sort(keys)
  return keys
end

local function describeValue(value)
  if type(value) == 'string' then
    return '"' .. value .. '"'
  end
  return tostring(value)
end

function Writes.suggest(key, keyTypes)
  local wanted = key:lower()
  local prefix, contains = {}, {}
  for _, candidate in ipairs(sortedKeys(keyTypes)) do
    local lower = candidate:lower()
    if lower:sub(1, #wanted) == wanted or wanted:sub(1, #lower) == lower then
      prefix[#prefix + 1] = candidate
    elseif lower:find(wanted, 1, true) then
      contains[#contains + 1] = candidate
    end
  end
  for _, candidate in ipairs(contains) do
    prefix[#prefix + 1] = candidate
  end
  while #prefix > MAX_SUGGESTIONS do
    prefix[#prefix] = nil
  end
  return prefix
end

local function checkKey(key, keyTypes)
  local kind = keyTypes[key]
  if not kind then
    local close = Writes.suggest(key, keyTypes)
    local hint = #close > 0 and (' Did you mean: ' .. table.concat(close, ', ') .. '?') or ''
    badRequest('Unknown develop setting "' .. key .. '".' .. hint
      .. ' Run `lrcli settings` to list setting names and ranges.')
  end
  if kind == 'table' then
    badRequest(key .. ' is a structured setting and cannot be set from the command line.')
  end
  return kind
end

local function checkValue(key, kind, value)
  local ok
  if kind == 'number' then
    ok = isFinite(value)
  else
    ok = type(value) == kind
  end
  if not ok then
    badRequest(key .. ' expects a ' .. kind .. ', got ' .. describeValue(value) .. '.')
  end
end

local function objectArg(args, name)
  local value = args[name]
  if value == nil or value == json.null then
    return {}
  end
  if not isObject(value) then
    badRequest('"' .. name .. '" must be a JSON object mapping develop setting names to values.')
  end
  return value
end

-- The SDK's documented key list is out of date (it lacks e.g. the ColorGrade*
-- and Perspective* keys LrC 15 returns), so keys a photo actually has count too.
function Writes.knownKeyTypes(documented, settingsList)
  local types = {}
  for key, kind in pairs(documented) do
    types[key] = kind
  end
  for _, settings in ipairs(settingsList) do
    for key, value in pairs(settings) do
      local kind = type(value)
      if types[key] == nil and (kind == 'number' or kind == 'string' or kind == 'boolean') then
        types[key] = kind
      end
    end
  end
  return types
end

function Writes.parseApply(args, keyTypes)
  local set, add = objectArg(args, 'set'), objectArg(args, 'add')
  local touched = {}
  for _, key in ipairs(sortedKeys(set)) do
    checkValue(key, checkKey(key, keyTypes), set[key])
    touched[#touched + 1] = key
  end
  for _, key in ipairs(sortedKeys(add)) do
    local kind = checkKey(key, keyTypes)
    if kind ~= 'number' then
      badRequest(key .. ' is a ' .. kind .. ' setting; relative changes (+= and -=) only work on numbers.')
    end
    checkValue(key, kind, add[key])
    if set[key] ~= nil then
      badRequest(key .. ' is given both as an absolute value and as a relative change. Use one.')
    end
    touched[#touched + 1] = key
  end
  if #touched == 0 then
    badRequest('Nothing to apply. Give at least one setting, for example Exposure2012=0.5 or Contrast2012+=10.')
  end
  table.sort(touched)
  return { set = set, add = add, touched = touched }
end

-- Relative changes are computed here because the SDK does not document how
-- applyDevelopSettings treats them.
function Writes.applyValues(current, parsed)
  local values, before = {}, {}
  for _, key in ipairs(parsed.touched) do
    local now = current[key]
    before[key] = now == nil and json.null or now
    if parsed.set[key] ~= nil then
      values[key] = parsed.set[key]
    elseif type(now) == 'number' then
      values[key] = now + parsed.add[key]
    else
      return nil, nil, key .. ' has no numeric value on this photo, so it cannot be changed relatively. Use '
        .. key .. '=<value> instead.'
    end
  end
  return values, before
end

-- DNG can hold non-raw data, but Lightroom reports its own per-photo range
-- anyway, so treating every DNG as raw only makes white balance bounds looser.
function Writes.rangesFor(all, fileFormat)
  if fileFormat == 'RAW' or fileFormat == 'DNG' then
    return all.raw
  end
  return all.nonRaw
end

-- applyDevelopSettings stores out-of-range values unclamped, and a photo holding
-- them renders as if none of its develop edits applied.
-- On non-raw files getRange reports a range for Temperature/Tint and the write
-- reads back, but the value doesn't stick; those files use the Incremental keys.
local NON_RAW_ALTERNATIVES = { Temperature = 'IncrementalTemperature', Tint = 'IncrementalTint' }

function Writes.rangeError(values, ranges)
  for _, key in ipairs(sortedKeys(values)) do
    if ranges[NON_RAW_ALTERNATIVES[key] or ''] then
      return key .. ' only applies to raw files; use ' .. NON_RAW_ALTERNATIVES[key] .. ' (' .. table.concat(ranges[NON_RAW_ALTERNATIVES[key]], '..') .. ') for this photo.'
    end
    local value, range = values[key], ranges[key]
    if type(value) == 'number' and range and (value < range[1] or value > range[2]) then
      return key .. '=' .. tostring(value) .. " is outside Lightroom's range " .. range[1] .. '..' .. range[2] .. '.'
    end
  end
  return nil
end

function Writes.pick(settings, keys)
  local out = {}
  for _, key in ipairs(keys) do
    local value = settings[key]
    out[key] = value == nil and json.null or value
  end
  return out
end

local function deepEqual(a, b)
  if type(a) ~= type(b) then
    return false
  end
  if type(a) ~= 'table' then
    return a == b
  end
  for k, v in pairs(a) do
    if not deepEqual(v, b[k]) then
      return false
    end
  end
  for k in pairs(b) do
    if a[k] == nil then
      return false
    end
  end
  return true
end

function Writes.changedSettings(before, after)
  local cleanBefore, cleanAfter = Photos.sanitize(before) or {}, Photos.sanitize(after) or {}
  local changedBefore, changedAfter = {}, {}
  local seen = {}
  for _, source in ipairs({ cleanBefore, cleanAfter }) do
    for key in pairs(source) do
      if not seen[key] then
        seen[key] = true
        if not deepEqual(cleanBefore[key], cleanAfter[key]) then
          changedBefore[key] = cleanBefore[key] == nil and json.null or cleanBefore[key]
          changedAfter[key] = cleanAfter[key] == nil and json.null or cleanAfter[key]
        end
      end
    end
  end
  return changedBefore, changedAfter
end

function Writes.gate(catalog, name, fn, protectedCall)
  local failure
  local ok, status = protectedCall(catalog.withWriteAccessDo, catalog, name, function()
    local callOk, err = protectedCall(fn)
    if not callOk then
      failure = err
      -- Rethrowing is what makes Lightroom roll the gate back.
      error(err, 0)
    end
  end, { timeout = WRITE_TIMEOUT_SECONDS })
  if failure ~= nil then
    error(failure, 0)
  end
  if not ok then
    error(status, 0)
  end
  if status ~= 'executed' then
    fail('write_busy', "Lightroom couldn't get write access (it may be busy or showing a dialog). Retry, or check Lightroom.")
  end
end

function Writes.parseRating(args)
  if not isInteger(args.rating) or args.rating < 0 or args.rating > 5 then
    badRequest('"rating" must be an integer from 0 to 5.')
  end
  return args.rating
end

-- "either nil or number of stars": no stars is nil, not 0.
function Writes.ratingValue(rating)
  if rating == 0 then
    return nil
  end
  return rating
end

function Writes.parseFlag(args)
  local value = FLAG_VALUES[args.flag]
  if type(args.flag) ~= 'string' or value == nil then
    badRequest('"flag" must be one of picked, rejected, none.')
  end
  return args.flag, value
end

function Writes.flagName(pickStatus)
  return FLAG_NAMES[pickStatus or 0] or 'none'
end

function Writes.parseLabel(args)
  if type(args.label) ~= 'string' or not LABELS[args.label] then
    badRequest('"label" must be one of red, yellow, green, blue, purple, none.')
  end
  return args.label
end

function Writes.labelName(raw)
  local lower = type(raw) == 'string' and raw:lower() or 'none'
  return LABELS[lower] and lower or 'none'
end

function Writes.parseKeywordPath(text)
  if type(text) ~= 'string' then
    badRequest('Keywords must be strings.')
  end
  local parts = {}
  for part in (text .. '>'):gmatch('([^>]*)>') do
    local trimmed = part:match('^%s*(.-)%s*$')
    if trimmed == '' then
      badRequest('Keyword "' .. text .. '" has an empty name. Use "Name" or "Parent > Child".')
    end
    parts[#parts + 1] = trimmed
  end
  return parts
end

function Writes.keywordPath(keyword)
  local names = {}
  local node = keyword
  while node and #names < MAX_KEYWORD_DEPTH do
    table.insert(names, 1, node:getName())
    node = node:getParent()
  end
  return table.concat(names, ' > ')
end

local function ambiguous(name, matches)
  local paths = {}
  for i, keyword in ipairs(matches) do
    paths[i] = Writes.keywordPath(keyword)
  end
  badRequest('Keyword "' .. name .. '" matches more than one keyword: ' .. table.concat(paths, '; ')
    .. '. Give the full path, for example "' .. paths[1] .. '".')
end

local function findByName(nodes, lowerName, found)
  for _, keyword in ipairs(nodes) do
    if keyword:getName():lower() == lowerName then
      found[#found + 1] = keyword
    end
    findByName(keyword:getChildren(), lowerName, found)
  end
end

local function matchLevel(nodes, name)
  local exact, loose = {}, {}
  for _, keyword in ipairs(nodes) do
    local candidate = keyword:getName()
    if candidate == name then
      exact[#exact + 1] = keyword
    elseif candidate:lower() == name:lower() then
      loose[#loose + 1] = keyword
    end
  end
  local matches = #exact > 0 and exact or loose
  if #matches > 1 then
    ambiguous(name, matches)
  end
  return matches[1]
end

local function slice(list, from)
  local out = {}
  for i = from, #list do
    out[#out + 1] = list[i]
  end
  return out
end

-- The SDK documents no lookup by name, so walk the tree. A bare name matches
-- anywhere in it; a path starts at the top level.
function Writes.planKeyword(roots, text)
  local components = Writes.parseKeywordPath(text)
  local plan
  if #components == 1 then
    local found = {}
    findByName(roots, components[1]:lower(), found)
    if #found > 1 then
      ambiguous(components[1], found)
    end
    plan = { keyword = found[1], rest = found[1] and {} or components }
  else
    local parent
    local nodes = roots
    plan = { rest = {} }
    for i, name in ipairs(components) do
      if i > 1 then
        nodes = parent:getChildren()
      end
      local keyword = matchLevel(nodes, name)
      if not keyword then
        plan.parent = parent
        plan.rest = slice(components, i)
        break
      end
      parent = keyword
    end
    if #plan.rest == 0 then
      plan.keyword = parent
    end
  end
  local base = plan.keyword and Writes.keywordPath(plan.keyword)
    or plan.parent and (Writes.keywordPath(plan.parent) .. ' > ')
    or ''
  plan.path = plan.keyword and base or (base .. table.concat(plan.rest, ' > '))
  return plan
end

-- Must run inside a write gate. The shared plan caches what it created so every
-- photo reuses the same keyword.
function Writes.ensureKeyword(catalog, plan)
  if plan.keyword then
    return plan.keyword
  end
  local parent = plan.parent
  for _, name in ipairs(plan.rest) do
    parent = catalog:createKeyword(name, {}, true, parent, true)
    if not parent then
      fail('keyword_failed', 'Lightroom would not create the keyword "' .. plan.path .. '".')
    end
  end
  plan.keyword = parent
  return parent
end

function Writes.keywordIdSet(keywords)
  local ids = {}
  for _, keyword in ipairs(keywords or {}) do
    ids[keyword.localIdentifier] = true
  end
  return ids
end

function Writes.tagWork(addPlans, removePlans, photoKeywordIds)
  local adds, removes = {}, {}
  for _, plan in ipairs(addPlans) do
    if not (plan.keyword and photoKeywordIds[plan.keyword.localIdentifier]) then
      adds[#adds + 1] = plan
    end
  end
  for _, plan in ipairs(removePlans) do
    if photoKeywordIds[plan.keyword.localIdentifier] then
      removes[#removes + 1] = plan
    end
  end
  return adds, removes
end

local function stringList(args, name)
  local list = args[name]
  if list == nil or list == json.null then
    return {}
  end
  if type(list) ~= 'table' then
    badRequest('"' .. name .. '" must be an array of keyword names.')
  end
  local out = {}
  for i, item in ipairs(list) do
    if type(item) ~= 'string' then
      badRequest('"' .. name .. '" must be an array of keyword names.')
    end
    out[i] = item
  end
  return out
end

function Writes.parseTag(args)
  local add, remove = stringList(args, 'add'), stringList(args, 'remove')
  if #add == 0 and #remove == 0 then
    badRequest('Nothing to do. Give keywords to add or remove.')
  end
  return add, remove
end

function Writes.parsePreset(args)
  for _, field in ipairs({ 'name', 'uuid' }) do
    local value = args[field]
    if value ~= nil and (type(value) ~= 'string' or value == '') then
      badRequest('"' .. field .. '" must be a non-empty string.')
    end
  end
  if args.name == nil and args.uuid == nil then
    badRequest('Give a preset "name" or "uuid". Run `lrcli presets` to list them.')
  end
  local amount = args.amount
  if amount == json.null then
    amount = nil
  end
  if amount ~= nil and (not isInteger(amount) or amount < 0 or amount > 200) then
    badRequest('"amount" must be an integer from 0 to 200.')
  end
  return { name = args.name, uuid = args.uuid, amount = amount }
end

function Writes.findPreset(folders, parsed)
  local matches = {}
  for _, folder in ipairs(folders) do
    for _, preset in ipairs(folder.presets) do
      local hit
      if parsed.uuid then
        hit = preset.uuid:lower() == parsed.uuid:lower()
      else
        hit = preset.name:lower() == parsed.name:lower()
      end
      if hit then
        matches[#matches + 1] = { folder = folder.name, name = preset.name, uuid = preset.uuid, preset = preset.preset }
      end
    end
  end
  local wanted = parsed.uuid or parsed.name
  if #matches == 0 then
    badRequest('No develop preset matches "' .. wanted .. '". Run `lrcli presets` to list them.')
  end
  if #matches > 1 then
    local labels = {}
    for i, match in ipairs(matches) do
      labels[i] = match.folder .. '/' .. match.name .. ' (' .. match.uuid .. ')'
    end
    badRequest('More than one develop preset is named "' .. wanted .. '": ' .. table.concat(labels, '; ')
      .. '. Pass the uuid of the one you want.')
  end
  return matches[1]
end

-- Decoded JSON arrays carry json.lua's marker metatable; handing such a table to
-- applyDevelopSettings made withWriteAccessDo hang on commit, so the SDK only
-- ever gets plain copies.
function Writes.plain(value)
  if type(value) ~= 'table' then
    return value
  end
  local copy = {}
  for k, v in pairs(value) do
    copy[k] = Writes.plain(v)
  end
  return copy
end

local TOLERANCE = 1e-6

local function sameValue(a, b)
  if type(a) == 'number' and type(b) == 'number' then
    return math.abs(a - b) <= TOLERANCE
  end
  if type(a) ~= 'table' or type(b) ~= 'table' then
    return a == b
  end
  for k, v in pairs(a) do
    if not sameValue(v, b[k]) then
      return false
    end
  end
  for k in pairs(b) do
    if a[k] == nil then
      return false
    end
  end
  return true
end

local function parseExpectation(raw)
  if not isObject(raw) then
    badRequest('"expectAfter" must be an object.')
  end
  local item = { keywordsPresent = stringList(raw, 'keywordsPresent'), keywordsAbsent = stringList(raw, 'keywordsAbsent') }
  if raw.develop ~= nil then
    if not isObject(raw.develop) then
      badRequest('"expectAfter.develop" must be an object.')
    end
    item.develop = raw.develop
  end
  if raw.rating ~= nil then
    item.rating = Writes.parseRating(raw)
  end
  if raw.flag ~= nil then
    item.flag = Writes.parseFlag(raw)
  end
  if raw.label ~= nil then
    item.label = Writes.parseLabel(raw)
  end
  return item
end

-- Before-values were read from getDevelopSettings, so they are restored as-is
-- (including table values such as Look) instead of being validated like input.
function Writes.parseRestore(args)
  local list = args.changes
  if type(list) ~= 'table' or #list == 0 then
    badRequest('"changes" must be a non-empty array.')
  end
  local changes = {}
  for i, raw in ipairs(list) do
    if not isObject(raw) or not isInteger(raw.id) then
      badRequest('Every change needs an integer "id".')
    end
    local change = { id = raw.id, keywordsAdded = stringList(raw, 'keywordsAdded'), keywordsRemoved = stringList(raw, 'keywordsRemoved') }
    if raw.develop ~= nil and raw.develop ~= json.null then
      if not isObject(raw.develop) then
        badRequest('"develop" must be an object.')
      end
      change.develop = {}
      for key, value in pairs(raw.develop) do
        if type(key) ~= 'string' then
          badRequest('Develop setting names must be strings.')
        end
        if value ~= json.null then
          change.develop[key] = Writes.plain(value)
        end
      end
    end
    if raw.rating ~= nil then
      change.rating = Writes.parseRating(raw)
    end
    if raw.flag ~= nil then
      local _, pickStatus = Writes.parseFlag(raw)
      change.pickStatus = pickStatus
    end
    if raw.label ~= nil then
      change.label = Writes.parseLabel(raw)
    end
    if raw.expectAfter ~= nil and raw.expectAfter ~= json.null then
      change.expectAfter = parseExpectation(raw.expectAfter)
    end
    changes[i] = change
  end
  return changes
end

-- Reads only the fields `expected` asks about, so photos are not queried for
-- state nobody checks.
function Writes.stateOf(photo, expected)
  local state = {}
  if expected.develop then
    state.develop = Photos.sanitize(photo:getDevelopSettings()) or {}
  end
  if expected.rating ~= nil then
    state.rating = photo:getRawMetadata('rating') or 0
  end
  if expected.flag ~= nil then
    state.flag = Writes.flagName(photo:getRawMetadata('pickStatus'))
  end
  if expected.label ~= nil then
    state.label = Writes.labelName(photo:getRawMetadata('colorNameForLabel'))
  end
  if #expected.keywordsPresent + #expected.keywordsAbsent > 0 then
    state.keywordPaths = {}
    for _, keyword in ipairs(photo:getRawMetadata('keywords') or {}) do
      state.keywordPaths[Writes.keywordPath(keyword):lower()] = true
    end
  end
  return state
end

local function show(value)
  if value == nil or value == json.null then
    return 'absent'
  end
  if type(value) == 'table' then
    return json.encode(Photos.sanitize(value))
  end
  return tostring(value)
end

-- Describes every field of `state` that differs from what lrcli left behind.
function Writes.conflicts(state, expected)
  local found = {}
  if expected.develop then
    for _, key in ipairs(sortedKeys(expected.develop)) do
      local want = expected.develop[key]
      if want == json.null then
        want = nil
      end
      if not sameValue(state.develop[key], want) then
        found[#found + 1] = key .. ' is ' .. show(state.develop[key]) .. ', lrcli set ' .. show(want)
      end
    end
  end
  for _, field in ipairs({ 'rating', 'flag', 'label' }) do
    if expected[field] ~= nil and state[field] ~= expected[field] then
      found[#found + 1] = field .. ' is ' .. show(state[field]) .. ', lrcli set ' .. show(expected[field])
    end
  end
  for _, path in ipairs(expected.keywordsPresent) do
    if not state.keywordPaths[path:lower()] then
      found[#found + 1] = 'keyword "' .. path .. '" is gone, lrcli added it'
    end
  end
  for _, path in ipairs(expected.keywordsAbsent) do
    if state.keywordPaths[path:lower()] then
      found[#found + 1] = 'keyword "' .. path .. '" is back, lrcli removed it'
    end
  end
  return found
end

return Writes
