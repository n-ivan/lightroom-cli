local json = require 'json'

local DevelopActions = {}

local MODULES = { 'library', 'develop', 'map', 'book', 'slideshow', 'print', 'web' }
local SIMPLE_ACTIONS = { ['auto-tone'] = true, ['auto-wb'] = true, ['as-shot-wb'] = true, ['reset-all'] = true }

local function badRequest(message)
  error({ code = 'bad_request', message = message }, 0)
end

function DevelopActions.parseModule(args)
  local name = args.name
  for _, known in ipairs(MODULES) do
    if name == known then
      return name
    end
  end
  badRequest('"name" must be one of: ' .. table.concat(MODULES, ', ') .. '.')
end

function DevelopActions.parse(args)
  local list = args.actions
  if type(list) ~= 'table' or #list == 0 then
    badRequest('"actions" must be a non-empty array of develop actions.')
  end
  local actions = {}
  for i, item in ipairs(list) do
    local name = type(item) == 'table' and item.action
    if SIMPLE_ACTIONS[name] then
      if item.keys ~= nil and item.keys ~= json.null then
        badRequest('Action "' .. name .. '" takes no "keys".')
      end
      actions[i] = { action = name }
    elseif name == 'reset' then
      local keys = {}
      if type(item.keys) == 'table' then
        for j, key in ipairs(item.keys) do
          if type(key) ~= 'string' or key == '' then
            badRequest('Action "reset" needs "keys" to be non-empty strings.')
          end
          keys[j] = key
        end
      end
      if #keys == 0 then
        badRequest('Action "reset" needs a non-empty "keys" array of develop setting names.')
      end
      actions[i] = { action = 'reset', keys = keys }
    else
      badRequest('Unknown develop action "' .. tostring(name)
        .. '". Use auto-tone, auto-wb, as-shot-wb, reset-all or reset.')
    end
  end
  return actions
end

-- Controller parameters lack the "2012" suffix that getDevelopSettings keys
-- carry; the handler tries the name as given first.
function DevelopActions.alternateName(key)
  return key:match('^(.+)2012$')
end

return DevelopActions
