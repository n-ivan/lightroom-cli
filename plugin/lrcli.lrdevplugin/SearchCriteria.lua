local json = require 'json'

local SearchCriteria = {}

-- Mapping onto catalog:findPhotos searchDesc criteria (LrCatalog API reference).
-- Date criteria have no >= or <=, so inclusive bounds shift by a day.
local SDK = {
  combine = 'intersect',
  rating = { criteria = 'rating' },
  flag = { criteria = 'pick', operation = '==', values = { picked = 1, unflagged = 0, rejected = -1 } },
  label = {
    criteria = 'labelColor',
    operation = '==',
    values = { red = 1, yellow = 2, green = 3, blue = 4, purple = 5 },
  },
  keyword = { criteria = 'keywords', operation = 'words' },
  text = { criteria = 'all', operation = 'all' },
  filename = { criteria = 'filename', operation = 'any' },
  folder = { criteria = 'folder', operation = 'any' },
  collection = { criteria = 'collection', operation = 'any' },
  camera = { criteria = 'camera', operation = 'any' },
  since = { criteria = 'captureTime', operation = '>' },
  ['until'] = { criteria = 'captureTime', operation = '<' },
  between = { criteria = 'captureTime', operation = 'in' },
}

local FILTER_NAMES = {
  'rating', 'flag', 'label', 'keywords', 'text', 'filename', 'folder', 'collection', 'since', 'until', 'camera',
}
local RATING_OPS = { '==', '!=', '>', '<', '>=', '<=' }

SearchCriteria.DEFAULT_LIMIT = 50
SearchCriteria.MAX_LIMIT = 500

local function shiftDay(date, days)
  local year, month, day = date:match('^(%d+)-(%d+)-(%d+)$')
  local time = os.time({ year = tonumber(year), month = tonumber(month), day = tonumber(day) + days, hour = 12 })
  return os.date('%Y-%m-%d', time)
end

local function fail(message)
  error({ code = 'bad_request', message = message }, 0)
end

local function isInteger(n)
  return type(n) == 'number' and n == math.floor(n)
end

local function contains(list, value)
  for _, item in ipairs(list) do
    if item == value then
      return true
    end
  end
  return false
end

local function sortedKeys(map)
  local keys = {}
  for k in pairs(map) do
    keys[#keys + 1] = k
  end
  table.sort(keys, function(a, b)
    return map[a] < map[b]
  end)
  return keys
end

local function nonEmptyString(name, value)
  if type(value) ~= 'string' or value == '' then
    fail('Filter "' .. name .. '" must be a non-empty string.')
  end
  return value
end

local function enumValue(name, value, values)
  if values[value] == nil then
    fail('Filter "' .. name .. '" must be one of: ' .. table.concat(sortedKeys(values), ', ') .. '.')
  end
  return values[value]
end

local function validDate(name, value)
  local y, m, d = tostring(value):match('^(%d%d%d%d)-(%d%d)-(%d%d)$')
  y, m, d = tonumber(y), tonumber(m), tonumber(d)
  local leap = y and (y % 4 == 0 and (y % 100 ~= 0 or y % 400 == 0))
  local days = m and ({ 31, leap and 29 or 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31 })[m]
  if not (days and d >= 1 and d <= days) then
    fail('Filter "' .. name .. '" must be a date formatted YYYY-MM-DD, for example 2024-05-31.')
  end
  return value
end

local function ratingCriterion(filter)
  if type(filter) ~= 'table' or not contains(RATING_OPS, filter.op) then
    fail('Filter "rating" must be {op, value} with op one of: ' .. table.concat(RATING_OPS, ', ') .. '.')
  end
  if not isInteger(filter.value) or filter.value < 0 or filter.value > 5 then
    fail('Filter "rating" value must be an integer from 0 to 5.')
  end
  return { criteria = SDK.rating.criteria, operation = filter.op, value = filter.value }
end

local function keywordCriteria(keywords)
  if type(keywords) ~= 'table' or #keywords == 0 then
    fail('Filter "keywords" must be a non-empty array of strings.')
  end
  local out = {}
  for _, keyword in ipairs(keywords) do
    nonEmptyString('keywords', keyword)
    out[#out + 1] = { criteria = SDK.keyword.criteria, operation = SDK.keyword.operation, value = keyword }
  end
  return out
end

local function textCriterion(name, value)
  local sdk = SDK[name]
  return { criteria = sdk.criteria, operation = sdk.operation, value = nonEmptyString(name, value) }
end

local function dateCriteria(filters)
  local since, untilDate = filters.since, filters['until']
  if since ~= nil then
    validDate('since', since)
  end
  if untilDate ~= nil then
    validDate('until', untilDate)
  end
  if since and untilDate then
    if since > untilDate then
      fail('Filter "since" (' .. since .. ') is later than "until" (' .. untilDate .. ').')
    end
    return { criteria = SDK.between.criteria, operation = SDK.between.operation, value = since, value2 = untilDate }
  end
  if since then
    return { criteria = SDK.since.criteria, operation = SDK.since.operation, value = shiftDay(since, -1) }
  end
  if untilDate then
    return { criteria = SDK['until'].criteria, operation = SDK['until'].operation, value = shiftDay(untilDate, 1) }
  end
end

local function cleanFilters(filters)
  if filters == nil or filters == json.null then
    return {}
  end
  if type(filters) ~= 'table' or #filters > 0 then
    fail('"filters" must be an object. Allowed filters: ' .. table.concat(FILTER_NAMES, ', ') .. '.')
  end
  local out = {}
  for name, value in pairs(filters) do
    if not contains(FILTER_NAMES, name) then
      fail('Unknown filter "' .. tostring(name) .. '". Allowed filters: ' .. table.concat(FILTER_NAMES, ', ') .. '.')
    end
    if value ~= json.null then
      out[name] = value
    end
  end
  return out
end

-- Returns nil when there are no filters (the caller then lists all photos).
function SearchCriteria.build(rawFilters)
  local filters = cleanFilters(rawFilters)
  local list = {}
  local function add(criterion)
    list[#list + 1] = criterion
  end

  if filters.rating ~= nil then
    add(ratingCriterion(filters.rating))
  end
  for _, name in ipairs({ 'flag', 'label' }) do
    if filters[name] ~= nil then
      local sdk = SDK[name]
      add({ criteria = sdk.criteria, operation = sdk.operation, value = enumValue(name, filters[name], sdk.values) })
    end
  end
  if filters.keywords ~= nil then
    for _, criterion in ipairs(keywordCriteria(filters.keywords)) do
      add(criterion)
    end
  end
  for _, name in ipairs({ 'text', 'filename', 'folder', 'collection', 'camera' }) do
    if filters[name] ~= nil then
      add(textCriterion(name, filters[name]))
    end
  end
  local dates = dateCriteria(filters)
  if dates then
    add(dates)
  end

  if #list == 0 then
    return nil
  end
  list.combine = SDK.combine
  return list
end

function SearchCriteria.paging(args)
  local limit, offset = args.limit, args.offset
  if limit == nil or limit == json.null then
    limit = SearchCriteria.DEFAULT_LIMIT
  end
  if offset == nil or offset == json.null then
    offset = 0
  end
  if not isInteger(limit) or limit < 1 or limit > SearchCriteria.MAX_LIMIT then
    fail('"limit" must be an integer from 1 to ' .. SearchCriteria.MAX_LIMIT .. '.')
  end
  if not isInteger(offset) or offset < 0 then
    fail('"offset" must be an integer of 0 or more.')
  end
  return limit, offset
end

return SearchCriteria
