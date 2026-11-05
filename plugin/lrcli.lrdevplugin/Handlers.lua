local CatalogHandlers = require 'CatalogHandlers'
local DevHandlers = require 'DevHandlers'
local DevelopHandlers = require 'DevelopHandlers'
local ExportHandler = require 'ExportHandler'
local PreviewHandler = require 'PreviewHandler'
local WriteHandlers = require 'WriteHandlers'

local Handlers = {}

function Handlers.new(deps)
  local handlers = {}

  function handlers.hello()
    return {
      protocol = deps.protocol,
      plugin = { version = deps.pluginVersion },
      lightroom = { version = deps.application.versionString() },
      catalog = { path = deps.application.activeCatalog():getPath() },
    }
  end

  function handlers.ping()
    return {}
  end

  for _, module in ipairs({ CatalogHandlers, PreviewHandler, WriteHandlers, DevHandlers, DevelopHandlers, ExportHandler }) do
    for name, handler in pairs(module.new()) do
      handlers[name] = handler
    end
  end

  return handlers
end

return Handlers
