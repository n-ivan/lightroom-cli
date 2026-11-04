local json = require 'json'

local Photos = {}

local FLAGS = { [1] = 'picked', [0] = 'unflagged', [-1] = 'rejected' }
local LABELS = { red = true, yellow = true, green = true, blue = true, purple = true }
local MAX_DEPTH = 12

local function orNull(value)
  if value == nil or value == '' then
    return json.null
  end
  return value
end

local function flagName(pickStatus)
  return FLAGS[pickStatus or 0] or 'unflagged'
end

local function labelName(name)
  local lower = type(name) == 'string' and name:lower() or nil
  return LABELS[lower] and lower or json.null
end

-- Keeps only what json.encode can represent; develop settings contain
-- userdata-like and function values we cannot send.
function Photos.sanitize(value, depth)
  depth = depth or 0
  local kind = type(value)
  if kind == 'string' or kind == 'boolean' then
    return value
  end
  if kind == 'number' then
    if value ~= value or value == math.huge or value == -math.huge then
      return nil
    end
    return value
  end
  if kind ~= 'table' or depth >= MAX_DEPTH then
    return nil
  end
  local out, count, max, isSequence = {}, 0, 0, true
  for k, v in pairs(value) do
    local clean = Photos.sanitize(v, depth + 1)
    if clean ~= nil then
      if type(k) == 'string' then
        isSequence = false
        out[k] = clean
      elseif type(k) == 'number' and k >= 1 and k == math.floor(k) then
        out[k] = clean
        count = count + 1
        if k > max then
          max = k
        end
      end
    end
  end
  if count == 0 then
    return out
  end
  if not isSequence then
    for i = 1, max do
      out[i] = nil
    end
  elseif count ~= max then
    return nil
  end
  return out
end

function Photos.splitKeywords(text)
  local keywords = json.array({})
  if type(text) == 'string' then
    for part in text:gmatch('[^,]+') do
      local trimmed = part:match('^%s*(.-)%s*$')
      if trimmed ~= '' then
        keywords[#keywords + 1] = trimmed
      end
    end
  end
  return keywords
end

function Photos.summary(meta)
  local dimensions = meta.dimensions or {}
  return {
    id = meta.id,
    filename = meta.fileName or '',
    path = meta.path or '',
    captureTime = orNull(meta.captureTime),
    rating = meta.rating or 0,
    flag = flagName(meta.pickStatus),
    label = labelName(meta.colorNameForLabel),
    width = dimensions.width or json.null,
    height = dimensions.height or json.null,
  }
end

function Photos.info(meta, developSettings)
  local out = Photos.summary(meta)
  out.title = orNull(meta.title)
  out.caption = orNull(meta.caption)
  out.keywords = Photos.splitKeywords(meta.keywordTags)
  out.fileFormat = orNull(meta.fileFormat)
  out.exif = {
    camera = orNull(meta.cameraModel),
    lens = orNull(meta.lens),
    iso = orNull(meta.isoSpeedRating),
    shutter = orNull(meta.shutterSpeed),
    aperture = orNull(meta.aperture),
    focalLength = orNull(meta.focalLength),
    exposureBias = orNull(meta.exposureBias),
  }
  if meta.gps then
    out.gps = { latitude = meta.gps.latitude, longitude = meta.gps.longitude }
  else
    out.gps = json.null
  end
  if developSettings ~= nil then
    out.develop = Photos.sanitize(developSettings) or {}
  end
  return out
end

return Photos
