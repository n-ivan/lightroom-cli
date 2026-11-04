local DevelopKeys = require 'DevelopKeys'
local Targets = require 'Targets'
local json = require 'json'

local LrApplication = import 'LrApplication'
local LrApplicationView = import 'LrApplicationView'
local LrDevelopController = import 'LrDevelopController'
local LrTasks = import 'LrTasks'

-- Maintenance commands, reached through `lrcli raw`: they regenerate data the
-- SDK only exposes at runtime (see scripts/gen-develop-ranges.py).
local DevHandlers = {}

-- Controller names from the LrDevelopController "Parameters in panels" list
-- that differ from getDevelopSettings keys.
local CONTROLLER_ONLY = {
  'Exposure', 'Contrast', 'Highlights', 'Shadows', 'Whites', 'Blacks', 'Clarity', 'Brightness',
  'PresetAmount', 'ProfileAmount', 'straightenAngle',
}

local function probeNames(photo)
  local names, seen = {}, {}
  local function add(name)
    if not seen[name] then
      seen[name] = true
      names[#names + 1] = name
    end
  end
  for key, kind in pairs(DevelopKeys) do
    if kind == 'number' then
      add(key)
    end
  end
  for key, value in pairs(photo:getDevelopSettings()) do
    if type(value) == 'number' then
      add(key)
    end
  end
  for _, name in ipairs(CONTROLLER_ONLY) do
    add(name)
  end
  table.sort(names)
  return names
end

function DevHandlers.new()
  local handlers = {}

  -- getRange only works in the Develop module, on the active photo.
  function handlers.develop_ranges(args)
    local catalog = LrApplication.activeCatalog()
    local photos = Targets.resolve(catalog, Targets.parseIds(args.photos))
    local photo = photos[1]
    local previous = LrApplicationView.getCurrentModuleName()
    catalog:setSelectedPhotos(photo, {})
    LrApplicationView.switchToModule('develop')
    LrTasks.sleep(1)
    local ranges, failed = {}, json.array({})
    for _, name in ipairs(probeNames(photo)) do
      local ok, min, max = LrTasks.pcall(LrDevelopController.getRange, name)
      if ok and type(min) == 'number' and type(max) == 'number' then
        ranges[name] = json.array({ min, max })
      else
        failed[#failed + 1] = name
      end
    end
    LrApplicationView.switchToModule(previous)
    return { photo = photo.localIdentifier, ranges = ranges, failed = failed }
  end

  return handlers
end

return DevHandlers
