local json = require 'json'
local Preview = require 'Preview'

local Export = {}

Export.MAX_PHOTOS = 500
Export.MAX_PIXELS = 65535
Export.MAX_MEGAPIXELS = 1000

local FORMATS = { 'JPEG', 'TIFF', 'PSD', 'DNG', 'ORIGINAL' }
local COLOR_SPACES = { 'sRGB', 'AdobeRGB', 'ProPhotoRGB' }
local TIFF_COMPRESSIONS = { 'compressionMethod_None', 'compressionMethod_LZW', 'compressionMethod_ZIP' }
local SIZE_TYPES = { 'longEdge', 'shortEdge', 'dimensions', 'megapixels' }
local SHARPEN_MEDIA = { 'screen', 'matte', 'glossy' }
local METADATA = { 'all', 'copyrightOnly', 'copyrightAndContactOnly', 'allExceptCameraInfo' }
local CONFLICTS = { 'rename', 'overwrite', 'skip' }

local DEFAULT_NAME = '{{image_name}}'

local function fail(message)
  error({ code = 'bad_request', message = message }, 0)
end

local function isInteger(n)
  return type(n) == 'number' and n == math.floor(n) and math.abs(n) < 2 ^ 53
end

local function absent(value)
  return value == nil or value == json.null
end

local function quoted(list)
  local parts = {}
  for i, item in ipairs(list) do
    parts[i] = '"' .. tostring(item) .. '"'
  end
  return table.concat(parts, ', ')
end

local function oneOf(value, allowed, name, default)
  if absent(value) then
    return default
  end
  for _, item in ipairs(allowed) do
    if value == item then
      return value
    end
  end
  fail('"' .. name .. '" must be one of ' .. quoted(allowed) .. '.')
end

local function boolean(value, name)
  if absent(value) then
    return false
  end
  if type(value) ~= 'boolean' then
    fail('"' .. name .. '" must be true or false.')
  end
  return value
end

local function pixels(value, name)
  if not isInteger(value) or value < 1 or value > Export.MAX_PIXELS then
    fail('"' .. name .. '" must be an integer from 1 to ' .. Export.MAX_PIXELS .. '.')
  end
  return value
end

local function parseSize(size)
  if absent(size) then
    return nil
  end
  if type(size) ~= 'table' then
    fail('"size" must be an object like {"type":"longEdge","value":2048}.')
  end
  local sizeType = oneOf(size.type, SIZE_TYPES, 'size.type')
  if sizeType == nil then
    fail('"size.type" must be one of ' .. quoted(SIZE_TYPES) .. '.')
  end
  if sizeType == 'dimensions' then
    return {
      type = sizeType,
      width = pixels(size.width, 'size.width'),
      height = pixels(size.height, 'size.height'),
    }
  end
  if sizeType == 'megapixels' then
    local value = size.value
    if type(value) ~= 'number' or value ~= value or value <= 0 or value > Export.MAX_MEGAPIXELS then
      fail('"size.value" must be a number above 0 and at most ' .. Export.MAX_MEGAPIXELS .. ' for megapixels.')
    end
    return { type = sizeType, value = value }
  end
  return { type = sizeType, value = pixels(size.value, 'size.value') }
end

local function parseSharpen(sharpen)
  if absent(sharpen) then
    return nil
  end
  if type(sharpen) ~= 'table' then
    fail('"sharpen" must be an object like {"media":"screen","level":2}.')
  end
  local media = oneOf(sharpen.media, SHARPEN_MEDIA, 'sharpen.media')
  if media == nil then
    fail('"sharpen.media" must be one of ' .. quoted(SHARPEN_MEDIA) .. '.')
  end
  local level = sharpen.level
  if absent(level) then
    level = 2
  elseif level ~= 1 and level ~= 2 and level ~= 3 then
    fail('"sharpen.level" must be 1 (low), 2 (standard) or 3 (high).')
  end
  return { media = media, level = level }
end

local function parseName(name)
  if absent(name) then
    return DEFAULT_NAME
  end
  if type(name) ~= 'string' or name == '' then
    fail('"name" must be a non-empty file name template such as "' .. DEFAULT_NAME .. '".')
  end
  if name:find('{{custom_token}}', 1, true) then
    fail('"{{custom_token}}" is not supported in "name".')
  end
  if name:find('/', 1, true) then
    fail('"name" must not contain "/". Use "subfolder" to export into a subfolder.')
  end
  return name
end

local function parseSubfolder(subfolder)
  if absent(subfolder) then
    return nil
  end
  if type(subfolder) ~= 'string' or subfolder == '' or subfolder:sub(1, 1) == '/' or subfolder:find('..', 1, true) then
    fail('"subfolder" must be a relative folder name without "..".')
  end
  return subfolder
end

-- directoryExists is injected so this stays free of SDK calls.
function Export.parseArgs(args, directoryExists)
  local dest = args.dest
  if type(dest) ~= 'string' or dest:sub(1, 1) ~= '/' then
    fail('"dest" is required and must be an absolute directory path.')
  end
  if not directoryExists(dest) then
    fail('"dest" does not exist or is not a directory: ' .. dest)
  end

  local format = oneOf(args.format, FORMATS, 'format')
  if format == nil then
    fail('"format" is required and must be one of ' .. quoted(FORMATS) .. '.')
  end

  local quality = args.quality
  if not absent(quality) then
    if format ~= 'JPEG' then
      fail('"quality" applies only to the JPEG format.')
    end
    if not isInteger(quality) or quality < 0 or quality > 100 then
      fail('"quality" must be an integer from 0 to 100.')
    end
  elseif format == 'JPEG' then
    quality = 90
  else
    quality = nil
  end

  local bitDepth = args.bitDepth
  if not absent(bitDepth) then
    if format ~= 'TIFF' and format ~= 'PSD' then
      fail('"bitDepth" applies only to the TIFF and PSD formats.')
    end
    if bitDepth ~= 8 and bitDepth ~= 16 then
      fail('"bitDepth" must be 8 or 16.')
    end
  else
    bitDepth = nil
  end

  local tiffCompression = args.tiffCompression
  if not absent(tiffCompression) then
    if format ~= 'TIFF' then
      fail('"tiffCompression" applies only to the TIFF format.')
    end
    tiffCompression = oneOf(tiffCompression, TIFF_COMPRESSIONS, 'tiffCompression')
  else
    tiffCompression = nil
  end

  return {
    dest = dest,
    format = format,
    quality = quality,
    size = parseSize(args.size),
    colorSpace = oneOf(args.colorSpace, COLOR_SPACES, 'colorSpace', 'sRGB'),
    bitDepth = bitDepth,
    tiffCompression = tiffCompression,
    sharpen = parseSharpen(args.sharpen),
    metadata = oneOf(args.metadata, METADATA, 'metadata', 'all'),
    removeLocation = boolean(args.removeLocation, 'removeLocation'),
    name = parseName(args.name),
    onConflict = oneOf(args.onConflict, CONFLICTS, 'onConflict', 'rename'),
    subfolder = parseSubfolder(args.subfolder),
    addToCatalog = boolean(args.addToCatalog, 'addToCatalog'),
  }
end

function Export.checkCount(count)
  if count > Export.MAX_PHOTOS then
    fail('Too many photos (' .. count .. '); export at most ' .. Export.MAX_PHOTOS .. ' per call.')
  end
end

-- LR_size_resizeType 'longEdge' and 'shortEdge' are constrained by
-- LR_size_maxHeight, so both maxima carry the same value. Never 'ask' for
-- LR_collisionHandling: Lightroom would open a dialog and block the export.
function Export.exportSettings(parsed)
  local settings = {
    LR_export_destinationType = 'specificFolder',
    LR_export_destinationPathPrefix = parsed.dest,
    LR_export_useSubfolder = parsed.subfolder ~= nil,
    LR_reimportExportedPhoto = parsed.addToCatalog,
    LR_collisionHandling = parsed.onConflict,
    LR_format = parsed.format,
    LR_export_colorSpace = parsed.colorSpace,
    LR_size_doConstrain = parsed.size ~= nil,
    LR_size_doNotEnlarge = true,
    LR_size_units = 'pixels',
    LR_outputSharpeningOn = parsed.sharpen ~= nil,
    LR_embeddedMetadataOption = parsed.metadata,
    LR_removeLocationMetadata = parsed.removeLocation,
    LR_useWatermark = false,
    LR_includeVideoFiles = false,
    LR_renamingTokensOn = true,
    LR_tokens = parsed.name,
    LR_extensionCase = 'lowercase',
  }
  if parsed.subfolder then
    settings.LR_export_destinationPathSuffix = parsed.subfolder
  end
  if parsed.quality then
    settings.LR_jpeg_quality = parsed.quality / 100
    settings.LR_jpeg_useLimitSize = false
  end
  if parsed.bitDepth then
    settings.LR_export_bitDepth = parsed.bitDepth
  end
  if parsed.tiffCompression then
    -- The SDK guide spells this LRtiff_compressionMethod; Lightroom ignores
    -- that and only honours LR_tiff_compressionMethod (verified on LrC 15.6).
    settings.LR_tiff_compressionMethod = parsed.tiffCompression
  end
  if parsed.sharpen then
    settings.LR_outputSharpeningMedia = parsed.sharpen.media
    settings.LR_outputSharpeningLevel = parsed.sharpen.level
  end
  local size = parsed.size
  if size then
    settings.LR_size_resizeType = size.type
    if size.type == 'megapixels' then
      settings.LR_size_megapixels = size.value
    elseif size.type == 'dimensions' then
      settings.LR_size_maxWidth = size.width
      settings.LR_size_maxHeight = size.height
    else
      settings.LR_size_maxWidth = size.value
      settings.LR_size_maxHeight = size.value
    end
  end
  return settings
end

-- With LR_collisionHandling = 'skip' Lightroom produces no rendition at all for
-- a photo whose target file exists; real failures come back with a message.
function Export.entries(infos, results, parsed)
  local entries = Preview.exportEntries(infos, results)
  if parsed.onConflict == 'skip' then
    for i, info in ipairs(infos) do
      if not info.skipReason and results[info.id] == nil then
        entries[i].error = json.null
        entries[i].skipped = true
      end
    end
  end
  return entries
end

function Export.reply(target, parsed, entries)
  return { target = target, dest = parsed.dest, photos = entries }
end

return Export
