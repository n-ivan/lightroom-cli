local json = require 'json'
local Preview = require 'Preview'
local Rendering = require 'Rendering'
local Targets = require 'Targets'

local LrApplication = import 'LrApplication'
local LrDate = import 'LrDate'
local LrFileUtils = import 'LrFileUtils'
local LrTasks = import 'LrTasks'

local THUMBNAIL_TIMEOUT_SECONDS = 20
-- The first delivery can be a placeholder; give a better one time to arrive.
local THUMBNAIL_SETTLE_SECONDS = 0.5
local POLL_SECONDS = 0.05

local PreviewHandler = {}

local function exportPreviews(infos, parsed)
  local results = Rendering.render(infos, Preview.exportSettings(parsed.dir, parsed.size))
  return Preview.exportEntries(infos, results)
end

local function fetchThumbnail(photo, size)
  local state = {}
  -- The request object must stay referenced until its callback has fired.
  state.request = photo:requestJpegThumbnail(size, size, function(data, err)
    if data then
      state.data = data
      state.firstAt = state.firstAt or LrDate.currentTime()
    elseif not state.data then
      state.err = err or 'Lightroom returned no thumbnail.'
    end
  end)
  local startedAt = LrDate.currentTime()
  while LrDate.currentTime() - startedAt < THUMBNAIL_TIMEOUT_SECONDS do
    if state.data and LrDate.currentTime() - state.firstAt >= THUMBNAIL_SETTLE_SECONDS then
      break
    end
    if state.err then
      break
    end
    LrTasks.sleep(POLL_SECONDS)
  end
  if state.data then
    return state.data
  end
  return nil, state.err or ('No thumbnail within ' .. THUMBNAIL_TIMEOUT_SECONDS .. ' s.')
end

local function writeFile(path, data)
  local handle, openError = io.open(path, 'wb')
  if not handle then
    return nil, 'Cannot write ' .. path .. ': ' .. tostring(openError)
  end
  local ok, writeError = handle:write(data)
  handle:close()
  if not ok then
    return nil, 'Cannot write ' .. path .. ': ' .. tostring(writeError)
  end
  return path
end

local function thumbnailPreviews(infos, parsed)
  local entries = json.array({})
  for i, info in ipairs(infos) do
    local path, err
    if info.skipReason then
      err = info.skipReason
    else
      local data
      data, err = fetchThumbnail(info.photo, parsed.size)
      if data then
        path, err = writeFile(Preview.thumbnailPath(parsed.dir, info.id), data)
      end
    end
    entries[i] = Preview.entry('thumbnail', info, path, err)
  end
  return entries
end

function PreviewHandler.new()
  local handlers = {}

  function handlers.preview(args)
    local parsed = Preview.parseArgs(args, function(dir)
      return LrFileUtils.exists(dir) == 'directory'
    end)
    local catalog = LrApplication.activeCatalog()
    local photos, target = Targets.resolve(catalog, Targets.parseIds(args.photos))
    Preview.checkCount(#photos)
    local infos = Rendering.photoInfos(catalog, photos, 'Videos cannot be previewed.')
    local entries
    if parsed.mode == 'export' then
      entries = exportPreviews(infos, parsed)
    else
      entries = thumbnailPreviews(infos, parsed)
    end
    return Preview.reply(target, parsed, entries)
  end

  return handlers
end

return PreviewHandler
