local Export = require 'Export'
local Rendering = require 'Rendering'
local Targets = require 'Targets'

local LrApplication = import 'LrApplication'
local LrFileUtils = import 'LrFileUtils'

local ExportHandler = {}

function ExportHandler.new()
  local handlers = {}

  function handlers.export(args)
    local parsed = Export.parseArgs(args, function(dir)
      return LrFileUtils.exists(dir) == 'directory'
    end)
    local catalog = LrApplication.activeCatalog()
    local photos, target = Targets.resolve(catalog, Targets.parseIds(args.photos))
    Export.checkCount(#photos)
    local infos = Rendering.photoInfos(catalog, photos, 'Videos cannot be exported.')
    local results = Rendering.render(infos, Export.exportSettings(parsed))
    return Export.reply(target, parsed, Export.entries(infos, results, parsed))
  end

  return handlers
end

return ExportHandler
