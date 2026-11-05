local json = require 'json'
local DevelopActions = require 'DevelopActions'
local Targets = require 'Targets'
local Writes = require 'Writes'

local LrApplication = import 'LrApplication'
local LrApplicationView = import 'LrApplicationView'
local LrDevelopController = import 'LrDevelopController'
local LrTasks = import 'LrTasks'

local DevelopHandlers = {}

local MODULE_SWITCH_SECONDS = 1
local PHOTO_SWITCH_SECONDS = 0.3
local SETTLE_POLL_SECONDS = 0.1
local SETTLE_MAX_SECONDS = 2

local function messageOf(err)
  return type(err) == 'table' and err.message or tostring(err)
end

local function hasChanged(before, after)
  return next((Writes.changedSettings(before, after))) ~= nil
end

local function resetKey(key)
  local ok, err = LrTasks.pcall(LrDevelopController.resetToDefault, key)
  if ok then
    return
  end
  local alternate = DevelopActions.alternateName(key)
  if not alternate or not LrTasks.pcall(LrDevelopController.resetToDefault, alternate) then
    error(err, 0)
  end
end

-- The controller has no "as shot" white balance. Resetting Temperature and
-- Tint restores the camera values but leaves the label on Custom, and
-- setValue('WhiteBalance', 'As Shot') does nothing, so the label is written
-- through the catalog.
local function asShotWhiteBalance(catalog, photo)
  LrDevelopController.resetToDefault('Temperature')
  LrDevelopController.resetToDefault('Tint')
  Writes.gate(catalog, 'lrcli develop', function()
    photo:applyDevelopSettings({ WhiteBalance = 'As Shot' }, 'lrcli develop')
  end, LrTasks.pcall)
end

local function runAction(catalog, photo, action)
  if action.action == 'auto-tone' then
    LrDevelopController.setAutoTone()
  elseif action.action == 'auto-wb' then
    LrDevelopController.setAutoWhiteBalance()
  elseif action.action == 'as-shot-wb' then
    asShotWhiteBalance(catalog, photo)
  elseif action.action == 'reset-all' then
    LrDevelopController.resetAllDevelopAdjustments()
  else
    for _, key in ipairs(action.keys) do
      resetKey(key)
    end
  end
end

-- Settings can lag behind a controller call; an action may also change nothing,
-- so give up after a bounded wait.
local function settledSettings(photo, before)
  local waited = 0
  local after = photo:getDevelopSettings()
  while not hasChanged(before, after) and waited < SETTLE_MAX_SECONDS do
    LrTasks.sleep(SETTLE_POLL_SECONDS)
    waited = waited + SETTLE_POLL_SECONDS
    after = photo:getDevelopSettings()
  end
  return after
end

local function editPhoto(catalog, info, actions)
  local photo = info.photo
  local before = photo:getDevelopSettings()
  catalog:setSelectedPhotos(photo, {})
  if LrApplicationView.getCurrentModuleName() == 'develop' then
    LrTasks.sleep(PHOTO_SWITCH_SECONDS)
  else
    LrApplicationView.switchToModule('develop')
    LrTasks.sleep(MODULE_SWITCH_SECONDS)
  end
  for _, action in ipairs(actions) do
    runAction(catalog, photo, action)
  end
  local changedBefore, changedAfter = Writes.changedSettings(before, settledSettings(photo, before))
  return { before = changedBefore, after = changedAfter }
end

local function restoreView(catalog, previousModule, previousActive, previousOthers)
  if previousActive then
    catalog:setSelectedPhotos(previousActive, previousOthers)
  end
  if previousModule and LrApplicationView.getCurrentModuleName() ~= previousModule then
    LrApplicationView.switchToModule(previousModule)
  end
end

function DevelopHandlers.new()
  local handlers = {}

  function handlers.module(args)
    local name = DevelopActions.parseModule(args)
    local previous = LrApplicationView.getCurrentModuleName()
    LrApplicationView.switchToModule(name)
    return { previous = previous, current = LrApplicationView.getCurrentModuleName() }
  end

  function handlers.develop(args)
    local actions = DevelopActions.parse(args)
    local catalog = LrApplication.activeCatalog()
    local photos, target = Targets.resolve(catalog, Targets.parseIds(args.photos))
    local infos = Targets.describe(catalog, photos)

    local previousModule = LrApplicationView.getCurrentModuleName()
    local previousActive = catalog:getTargetPhoto()
    local previousOthers = {}
    for _, selected in ipairs(Targets.selection(catalog)) do
      if selected ~= previousActive then
        previousOthers[#previousOthers + 1] = selected
      end
    end

    local entries = json.array({})
    local ok, err = LrTasks.pcall(function()
      for i, info in ipairs(infos) do
        local fields
        if info.isVideo then
          fields = { error = 'Videos have no develop settings to edit.' }
        else
          local done, result = LrTasks.pcall(editPhoto, catalog, info, actions)
          fields = done and result or { error = messageOf(result) }
        end
        fields.id, fields.filename = info.id, info.filename
        entries[i] = fields
      end
    end)
    local restored, restoreErr = LrTasks.pcall(restoreView, catalog, previousModule, previousActive, previousOthers)
    if not ok then
      error(err, 0)
    end
    if not restored then
      error({
        code = 'restore_failed',
        message = 'The edits ran, but Lightroom could not return to the previous module or selection: '
          .. messageOf(restoreErr),
      }, 0)
    end
    return { target = target, photos = entries }
  end

  return handlers
end

return DevelopHandlers
