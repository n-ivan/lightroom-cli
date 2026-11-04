local DevelopPanels = require 'DevelopPanels'
local DevelopRanges = require 'DevelopRanges'
local json = require 'json'
local PanelSettings = require 'PanelSettings'
local Photos = require 'Photos'
local SearchCriteria = require 'SearchCriteria'
local Targets = require 'Targets'
local Writes = require 'Writes'

local LrApplication = import 'LrApplication'
local LrTasks = import 'LrTasks'

local CatalogHandlers = {}

local SUMMARY_RAW_KEYS = { 'path', 'rating', 'pickStatus', 'colorNameForLabel', 'dateTimeOriginalISO8601', 'dimensions' }
local SUMMARY_FORMATTED_KEYS = { 'fileName' }
local INFO_RAW_KEYS = { 'gps', 'fileFormat' }
local INFO_FORMATTED_KEYS = {
  'title', 'caption', 'keywordTags', 'cameraModel', 'lens', 'isoSpeedRating', 'shutterSpeed', 'aperture',
  'focalLength', 'exposureBias',
}

local function concat(...)
  local out = {}
  for _, list in ipairs({ ... }) do
    for _, key in ipairs(list) do
      out[#out + 1] = key
    end
  end
  return out
end

local function fetchMetadata(catalog, photos, rawKeys, formattedKeys)
  local raw = catalog:batchGetRawMetadata(photos, rawKeys)
  local formatted = catalog:batchGetFormattedMetadata(photos, formattedKeys)
  local metas = {}
  for i, photo in ipairs(photos) do
    local meta = { id = photo.localIdentifier }
    for key, value in pairs(raw[photo] or {}) do
      meta[key] = value
    end
    for key, value in pairs(formatted[photo] or {}) do
      meta[key] = value
    end
    meta.captureTime = meta.dateTimeOriginalISO8601
    metas[i] = meta
  end
  return metas
end

local function summaries(catalog, photos)
  local list = json.array({})
  for i, meta in ipairs(fetchMetadata(catalog, photos, SUMMARY_RAW_KEYS, SUMMARY_FORMATTED_KEYS)) do
    list[i] = Photos.summary(meta)
  end
  return list
end

local function photoIds(photos)
  local ids = json.array({})
  for i, photo in ipairs(photos) do
    ids[i] = photo.localIdentifier
  end
  return ids
end

local function selectPhotos(catalog, photos)
  local rest = {}
  for i = 2, #photos do
    rest[#rest + 1] = photos[i]
  end
  catalog:setSelectedPhotos(photos[1], rest)
end

local function badRequest(message)
  error({ code = 'bad_request', message = message }, 0)
end

local function settingsPhoto(catalog, args)
  local ids = Targets.parseIds(args.photos)
  if ids and #ids > 1 then
    badRequest('"photos" must list at most one photo for settings.')
  end
  if not ids and args.useSelection ~= true then
    return nil
  end
  local photos = Targets.resolve(catalog, ids)
  if #photos > 1 then
    badRequest('Select a single photo (or pass --photo <id>) to show its settings.')
  end
  return photos[1]
end

function CatalogHandlers.new()
  local handlers = {}

  function handlers.search(args)
    local catalog = LrApplication.activeCatalog()
    local limit, offset = SearchCriteria.paging(args)
    local searchDesc = SearchCriteria.build(args.filters)
    local found
    if searchDesc then
      found = catalog:findPhotos({ searchDesc = searchDesc, sort = 'captureTime', ascending = true })
    else
      found = catalog:getAllPhotos()
    end
    local page = {}
    for i = offset + 1, math.min(offset + limit, #found) do
      page[#page + 1] = found[i]
    end
    return { total = #found, offset = offset, limit = limit, photos = summaries(catalog, page) }
  end

  function handlers.select(args)
    local catalog = LrApplication.activeCatalog()
    local ids = Targets.parseIds(args.photos)
    if not ids or #ids == 0 then
      error({ code = 'bad_request', message = '"photos" is required and must list at least one photo id.' }, 0)
    end
    local photos = Targets.lookup(catalog, ids)

    selectPhotos(catalog, photos)
    local sourceChanged = false
    if not Targets.sameIds(Targets.selection(catalog), ids) then
      catalog:setActiveSources(catalog.kAllPhotos)
      LrTasks.sleep(0.2)
      selectPhotos(catalog, photos)
      sourceChanged = true
      if not Targets.sameIds(Targets.selection(catalog), ids) then
        error({
          code = 'select_failed',
          message = 'Lightroom did not select all requested photos. A Library filter may be hiding them; clear it (Library > Enable Filters) and retry.',
        }, 0)
      end
    end
    return { selected = photoIds(photos), sourceChanged = sourceChanged }
  end

  function handlers.selection()
    local catalog = LrApplication.activeCatalog()
    return { photos = summaries(catalog, Targets.selection(catalog)) }
  end

  function handlers.info(args)
    local catalog = LrApplication.activeCatalog()
    local photos, target = Targets.resolve(catalog, Targets.parseIds(args.photos))
    local metas = fetchMetadata(
      catalog,
      photos,
      concat(SUMMARY_RAW_KEYS, INFO_RAW_KEYS),
      concat(SUMMARY_FORMATTED_KEYS, INFO_FORMATTED_KEYS)
    )
    local list = json.array({})
    for i, meta in ipairs(metas) do
      list[i] = Photos.info(meta, args.develop == true and photos[i]:getDevelopSettings() or nil)
    end
    return { target = target, photos = list }
  end

  function handlers.settings(args)
    local panels = DevelopPanels
    if args.panel ~= nil and args.panel ~= json.null then
      if type(args.panel) ~= 'string' then
        badRequest('"panel" must be a string.')
      end
      local panel, message = PanelSettings.findPanel(DevelopPanels, args.panel)
      if not panel then
        badRequest(message)
      end
      panels = { panel }
    end
    local photo = settingsPhoto(LrApplication.activeCatalog(), args)
    if not photo then
      return { panels = PanelSettings.listing(panels, DevelopRanges, nil, nil), fileType = json.null }
    end
    if photo:getRawMetadata('isVideo') then
      badRequest('Videos have no develop settings.')
    end
    local fileType = Writes.rangesFor(DevelopRanges, photo:getRawMetadata('fileFormat')) == DevelopRanges.raw
        and 'raw'
      or 'nonRaw'
    return {
      photo = photo.localIdentifier,
      panels = PanelSettings.listing(panels, DevelopRanges, fileType, photo:getDevelopSettings()),
      fileType = fileType,
    }
  end

  return handlers
end

return CatalogHandlers
