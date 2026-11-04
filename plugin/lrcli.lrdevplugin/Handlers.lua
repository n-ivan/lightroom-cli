local CatalogHandlers = require 'CatalogHandlers'

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

  for name, handler in pairs(CatalogHandlers.new()) do
    handlers[name] = handler
  end

  return handlers
end

return Handlers
