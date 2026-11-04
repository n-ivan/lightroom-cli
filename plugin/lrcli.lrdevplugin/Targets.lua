local json = require 'json'

local Targets = {}

local function fail(code, message)
  error({ code = code, message = message }, 0)
end

local function isInteger(n)
  return type(n) == 'number' and n == math.floor(n) and math.abs(n) < 2 ^ 53
end

-- Returns nil when no ids were given, otherwise a de-duplicated list.
function Targets.parseIds(value)
  if value == nil or value == json.null then
    return nil
  end
  if type(value) ~= 'table' then
    fail('bad_request', '"photos" must be an array of integer photo ids.')
  end
  local ids, seen = {}, {}
  for _, id in ipairs(value) do
    if not isInteger(id) then
      fail('bad_request', '"photos" must be an array of integer photo ids.')
    end
    if not seen[id] then
      seen[id] = true
      ids[#ids + 1] = id
    end
  end
  return ids
end

-- The SDK documents no lookup by localIdentifier, so index the whole catalog.
function Targets.index(catalog)
  local byId = {}
  for _, photo in ipairs(catalog:getAllPhotos()) do
    byId[photo.localIdentifier] = photo
  end
  return byId
end

function Targets.lookup(catalog, ids)
  local byId = Targets.index(catalog)
  local photos, missing = {}, {}
  for _, id in ipairs(ids) do
    local photo = byId[id]
    if photo then
      photos[#photos + 1] = photo
    else
      missing[#missing + 1] = tostring(id)
    end
  end
  if #missing > 0 then
    fail('photo_not_found', 'Photos not found in the catalog: ' .. table.concat(missing, ', ')
      .. '. Use `lrcli search` to find valid ids.')
  end
  return photos
end

-- getTargetPhotos() returns the whole filmstrip when nothing is selected, so
-- emptiness has to be checked with getTargetPhoto().
function Targets.selection(catalog)
  if catalog:getTargetPhoto() == nil then
    return {}
  end
  return catalog:getTargetPhotos()
end

function Targets.resolve(catalog, ids)
  if ids and #ids > 0 then
    return Targets.lookup(catalog, ids), 'ids'
  end
  local photos = Targets.selection(catalog)
  if #photos == 0 then
    fail('no_photo_selected', 'Nothing is selected in Lightroom. Pass --photo <id> (see `lrcli search`).')
  end
  return photos, 'selection'
end

function Targets.describe(catalog, photos)
  local raw = catalog:batchGetRawMetadata(photos, { 'isVideo' })
  local formatted = catalog:batchGetFormattedMetadata(photos, { 'fileName' })
  local infos = {}
  for i, photo in ipairs(photos) do
    infos[i] = {
      id = photo.localIdentifier,
      photo = photo,
      filename = (formatted[photo] or {}).fileName or '',
      isVideo = (raw[photo] or {}).isVideo == true,
    }
  end
  return infos
end

function Targets.sameIds(photos, ids)
  if #photos ~= #ids then
    return false
  end
  local wanted = {}
  for _, id in ipairs(ids) do
    wanted[id] = true
  end
  for _, photo in ipairs(photos) do
    if not wanted[photo.localIdentifier] then
      return false
    end
  end
  return true
end

return Targets
