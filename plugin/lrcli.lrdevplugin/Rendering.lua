local LrExportSession = import 'LrExportSession'

local Rendering = {}

function Rendering.photoInfos(catalog, photos, videoMessage)
  local raw = catalog:batchGetRawMetadata(photos, { 'isVideo' })
  local formatted = catalog:batchGetFormattedMetadata(photos, { 'fileName' })
  local infos = {}
  for i, photo in ipairs(photos) do
    local info = { id = photo.localIdentifier, photo = photo }
    local formattedMeta = formatted[photo] or {}
    if formattedMeta.fileName and formattedMeta.fileName ~= '' then
      info.filename = formattedMeta.fileName
    end
    if (raw[photo] or {}).isVideo == true then
      info.skipReason = videoMessage
    end
    infos[i] = info
  end
  return infos
end

-- One export session for all non-skipped photos; returns photo id to
-- { path = ... } or { message = ... }.
function Rendering.render(infos, exportSettings)
  local results = {}
  local photos = {}
  for _, info in ipairs(infos) do
    if not info.skipReason then
      photos[#photos + 1] = info.photo
    end
  end
  if #photos == 0 then
    return results
  end
  local session = LrExportSession({ photosToExport = photos, exportSettings = exportSettings })
  session:doExportOnCurrentTask()
  for _, rendition in session:renditions() do
    local ok, pathOrMessage = rendition:waitForRender()
    local id = rendition.photo.localIdentifier
    if ok then
      results[id] = { path = pathOrMessage }
    else
      results[id] = { message = tostring(pathOrMessage) }
    end
  end
  return results
end

return Rendering
