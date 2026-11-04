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

  return handlers
end

return Handlers
