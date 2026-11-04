local json = require 'json'

local PanelSettings = {}

-- Non-raw files accept the Incremental keys instead: getRange reports
-- Temperature/Tint for them, but the written value doesn't stick.
local RAW_ONLY_KEYS = { Temperature = true, Tint = true }
local NON_RAW_ONLY_KEYS = { IncrementalTemperature = true, IncrementalTint = true }

local function names(panels)
  local list = {}
  for _, panel in ipairs(panels) do
    list[#list + 1] = panel.id .. ' (' .. panel.title .. ')'
  end
  return table.concat(list, ', ')
end

-- Matches id or title, case-insensitively: an exact match wins, otherwise a
-- unique prefix. Returns nil and a message otherwise.
function PanelSettings.findPanel(panels, text)
  local wanted = text:lower()
  local prefix = {}
  for _, panel in ipairs(panels) do
    if panel.id:lower() == wanted or panel.title:lower() == wanted then
      return panel
    end
    if panel.id:lower():sub(1, #wanted) == wanted or panel.title:lower():sub(1, #wanted) == wanted then
      prefix[#prefix + 1] = panel
    end
  end
  if #prefix == 1 then
    return prefix[1]
  end
  if #prefix > 1 then
    return nil, 'Panel "' .. text .. '" is ambiguous. Matches: ' .. names(prefix) .. '.'
  end
  return nil, 'Unknown panel "' .. text .. '". Panels: ' .. names(panels) .. '.'
end

local function describeKey(key, fileType, ranges, current)
  local entry
  if fileType then
    local range = ranges[fileType][key]
    if not range
      or (fileType == 'raw' and NON_RAW_ONLY_KEYS[key])
      or (fileType == 'nonRaw' and RAW_ONLY_KEYS[key]) then
      return nil
    end
    entry = { key = key, min = range[1], max = range[2] }
  else
    local raw, nonRaw = ranges.raw[key], ranges.nonRaw[key]
    if raw then
      entry = { key = key, min = raw[1], max = raw[2] }
      if nonRaw and (nonRaw[1] ~= raw[1] or nonRaw[2] ~= raw[2]) then
        entry.nonRaw = { min = nonRaw[1], max = nonRaw[2] }
      end
    elseif nonRaw then
      entry = { key = key, min = nonRaw[1], max = nonRaw[2], nonRawOnly = true }
    else
      return nil
    end
  end
  if current and type(current[key]) == 'number' then
    entry.value = current[key]
  end
  return entry
end

-- `fileType` is 'raw', 'nonRaw' or nil (no photo: show both). `current` is a
-- photo's getDevelopSettings table, if any. Panels left without keys are dropped.
function PanelSettings.listing(panels, ranges, fileType, current)
  local out = json.array({})
  for _, panel in ipairs(panels) do
    local keys = json.array({})
    for _, key in ipairs(panel.keys) do
      local entry = describeKey(key, fileType, ranges, current)
      if entry then
        keys[#keys + 1] = entry
      end
    end
    if #keys > 0 then
      out[#out + 1] = { id = panel.id, title = panel.title, keys = keys }
    end
  end
  return out
end

return PanelSettings
