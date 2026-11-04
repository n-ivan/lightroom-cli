local json = require 'json'

local Preview = {}

Preview.DEFAULT_SIZE = 1600
Preview.MIN_SIZE = 64
Preview.MAX_SIZE = 4096
Preview.MAX_PHOTOS = 50

local function fail(message)
  error({ code = 'bad_request', message = message }, 0)
end

local function isInteger(n)
  return type(n) == 'number' and n == math.floor(n) and math.abs(n) < 2 ^ 53
end

-- directoryExists is injected so this stays free of SDK calls.
function Preview.parseArgs(args, directoryExists)
  local size = args.size
  if size == nil or size == json.null then
    size = Preview.DEFAULT_SIZE
  elseif not isInteger(size) or size < Preview.MIN_SIZE or size > Preview.MAX_SIZE then
    fail('"size" must be an integer from ' .. Preview.MIN_SIZE .. ' to ' .. Preview.MAX_SIZE .. '.')
  end
  local mode = args.mode
  if mode == nil or mode == json.null then
    mode = 'export'
  elseif mode ~= 'export' and mode ~= 'thumbnail' then
    fail('"mode" must be "export" or "thumbnail".')
  end
  local dir = args.dir
  if type(dir) ~= 'string' or dir:sub(1, 1) ~= '/' then
    fail('"dir" is required and must be an absolute directory path.')
  end
  if not directoryExists(dir) then
    fail('"dir" does not exist or is not a directory: ' .. dir)
  end
  return { size = size, mode = mode, dir = dir }
end

function Preview.checkCount(count)
  if count > Preview.MAX_PHOTOS then
    fail('Too many photos (' .. count .. '); preview at most ' .. Preview.MAX_PHOTOS .. ' per call.')
  end
end

-- LR_size_resizeType 'longEdge' is constrained by LR_size_maxHeight, so both
-- maxima carry the same value.
function Preview.exportSettings(dir, size)
  return {
    LR_export_destinationType = 'specificFolder',
    LR_export_destinationPathPrefix = dir,
    LR_export_useSubfolder = false,
    LR_reimportExportedPhoto = false,
    LR_collisionHandling = 'overwrite',
    LR_format = 'JPEG',
    LR_export_colorSpace = 'sRGB',
    LR_jpeg_quality = 0.8,
    LR_jpeg_useLimitSize = false,
    LR_size_doConstrain = true,
    LR_size_doNotEnlarge = true,
    LR_size_resizeType = 'longEdge',
    LR_size_maxHeight = size,
    LR_size_maxWidth = size,
    LR_size_units = 'pixels',
    LR_size_resolution = 72,
    LR_size_resolutionUnits = 'inch',
    LR_outputSharpeningOn = false,
    LR_embeddedMetadataOption = 'copyrightOnly',
    LR_removeLocationMetadata = true,
    LR_useWatermark = false,
    LR_includeVideoFiles = false,
    LR_renamingTokensOn = true,
    -- Image # is unique within one export, so same-named photos (virtual copies) don't overwrite each other.
    LR_tokens = 'lrcli-{{image_name}}-{{naming_operationSequence_3Digits}}',
    LR_extensionCase = 'lowercase',
  }
end

function Preview.thumbnailPath(dir, id)
  return dir:gsub('/+$', '') .. '/lrcli-' .. id .. '-thumb.jpg'
end

function Preview.entry(mode, info, file, err)
  local entry = {
    id = info.id,
    filename = info.filename or json.null,
    file = file or json.null,
    error = err or json.null,
  }
  if mode == 'thumbnail' then
    entry.possiblyStale = true
  end
  return entry
end

-- results maps photo id to { path = ... } or { message = ... }. Photos in
-- infos with a skipReason (videos) or no result get an error entry.
function Preview.exportEntries(infos, results)
  local entries = json.array({})
  for i, info in ipairs(infos) do
    local result = results[info.id]
    if info.skipReason then
      entries[i] = Preview.entry('export', info, nil, info.skipReason)
    elseif result and result.path then
      entries[i] = Preview.entry('export', info, result.path, nil)
    else
      entries[i] = Preview.entry(
        'export',
        info,
        nil,
        (result and result.message) or 'Lightroom did not produce a rendition for this photo.'
      )
    end
  end
  return entries
end

function Preview.reply(target, parsed, entries)
  return { target = target, mode = parsed.mode, size = parsed.size, photos = entries }
end

return Preview
