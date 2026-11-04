local json = require 'json'
local DevelopKeys = require 'DevelopKeys'
local DevelopRanges = require 'DevelopRanges'
local Targets = require 'Targets'
local Writes = require 'Writes'

local LrApplication = import 'LrApplication'
local LrTasks = import 'LrTasks'
local LrUndo = import 'LrUndo'

local WriteHandlers = {}

local SETTLE_SECONDS = 0.3

local function entry(info, fields)
  fields.id = info.id
  fields.filename = info.filename
  return fields
end

local function jsonList(list)
  return json.array(list)
end

-- Plans every photo first, then writes all of them in one gate so Lightroom
-- records a single undo step and a failure rolls everything back.
local function execute(args, catalog, spec)
  local photos, target = Targets.resolve(catalog, Targets.parseIds(args.photos))
  local infos = Targets.describe(catalog, photos)
  local pending = 0
  for _, info in ipairs(infos) do
    if spec.develop and info.isVideo then
      info.error = 'Videos have no develop settings to edit.'
    else
      info.error = spec.prepare(info)
      if info.needsWrite then
        pending = pending + 1
      end
    end
  end
  if pending > 0 then
    Writes.gate(catalog, spec.gate, function()
      for _, info in ipairs(infos) do
        if info.needsWrite then
          spec.write(info)
        end
      end
    end, LrTasks.pcall)
  end
  local entries = json.array({})
  for i, info in ipairs(infos) do
    if info.error then
      entries[i] = entry(info, { error = info.error })
    else
      entries[i] = entry(info, spec.finish(info))
    end
  end
  return { target = target, photos = entries }
end

local function metadataCommand(args, catalog, options)
  local wanted = options.wanted
  return execute(args, catalog, {
    gate = options.gate,
    prepare = function(info)
      info.before = options.read(info.photo)
      info.needsWrite = info.before ~= wanted
    end,
    write = function(info)
      info.photo:setRawMetadata(options.key, options.store)
    end,
    finish = function(info)
      return { before = info.before, after = options.read(info.photo) }
    end,
  })
end

local function liveKeyTypes(photos)
  local settingsList = {}
  for _, photo in ipairs(photos) do
    if not photo:getRawMetadata('isVideo') then
      settingsList[#settingsList + 1] = photo:getDevelopSettings()
    end
  end
  return Writes.knownKeyTypes(DevelopKeys, settingsList)
end

local function presetFolders()
  local folders = {}
  for _, folder in ipairs(LrApplication.developPresetFolders()) do
    local presets = {}
    for _, preset in ipairs(folder:getDevelopPresets()) do
      presets[#presets + 1] = { name = preset:getName(), uuid = preset:getUuid(), preset = preset }
    end
    folders[#folders + 1] = { name = folder:getName(), presets = presets }
  end
  return folders
end

local function paths(plans)
  local list = json.array({})
  for i, plan in ipairs(plans) do
    list[i] = plan.path
  end
  return list
end

local function planKeywords(roots, texts, plans)
  local list, seen = {}, {}
  for _, text in ipairs(texts) do
    local plan = Writes.planKeyword(roots, text)
    local key = plan.path:lower()
    if not seen[key] then
      seen[key] = true
      list[#list + 1] = plan
    end
    plans[key] = plan
  end
  return list
end

local function applyKeywordChanges(catalog, photo, adds, removes)
  for _, plan in ipairs(removes) do
    photo:removeKeyword(plan.keyword)
  end
  for _, plan in ipairs(adds) do
    photo:addKeyword(Writes.ensureKeyword(catalog, plan))
  end
end

function WriteHandlers.new()
  local handlers = {}

  function handlers.apply(args)
    local catalog = LrApplication.activeCatalog()
    local targets = Targets.resolve(catalog, Targets.parseIds(args.photos))
    local parsed = Writes.parseApply(args, liveKeyTypes(targets))
    return execute(args, catalog, {
      gate = 'lrcli apply',
      develop = true,
      prepare = function(info)
        local values, before, err = Writes.applyValues(info.photo:getDevelopSettings(), parsed)
        if err then
          return err
        end
        local ranges = Writes.rangesFor(DevelopRanges, info.photo:getRawMetadata('fileFormat'))
        local rangeError = Writes.rangeError(values, ranges)
        if rangeError then
          return rangeError
        end
        info.values, info.before, info.needsWrite = values, before, true
      end,
      write = function(info)
        info.photo:applyDevelopSettings(info.values, 'lrcli apply')
      end,
      finish = function(info)
        return { before = info.before, after = Writes.pick(info.photo:getDevelopSettings(), parsed.touched) }
      end,
    })
  end

  function handlers.presets()
    local folders = json.array({})
    for i, folder in ipairs(presetFolders()) do
      local presets = json.array({})
      for j, preset in ipairs(folder.presets) do
        presets[j] = { name = preset.name, uuid = preset.uuid }
      end
      folders[i] = { name = folder.name, presets = presets }
    end
    return { folders = folders }
  end

  function handlers.preset(args)
    local parsed = Writes.parsePreset(args)
    local match = Writes.findPreset(presetFolders(), parsed)
    local catalog = LrApplication.activeCatalog()
    local reply = execute(args, catalog, {
      gate = 'lrcli preset ' .. match.name,
      develop = true,
      prepare = function(info)
        info.beforeAll = info.photo:getDevelopSettings()
        info.needsWrite = true
      end,
      write = function(info)
        info.photo:applyDevelopPreset(match.preset, nil, parsed.amount)
      end,
      finish = function(info)
        local before, after = Writes.changedSettings(info.beforeAll, info.photo:getDevelopSettings())
        return { before = before, after = after }
      end,
    })
    reply.preset = { name = match.name, uuid = match.uuid, folder = match.folder }
    return reply
  end

  function handlers.tag(args)
    local add, remove = Writes.parseTag(args)
    local catalog = LrApplication.activeCatalog()
    local roots = catalog:getKeywords()
    local byPath = {}
    local addPlans = planKeywords(roots, add, byPath)
    local removePlans = planKeywords(roots, remove, byPath)
    for _, plan in ipairs(removePlans) do
      if not plan.keyword then
        error({
          code = 'bad_request',
          message = 'Keyword "' .. plan.path .. '" does not exist in the catalog, so it cannot be removed.',
        }, 0)
      end
      for _, other in ipairs(addPlans) do
        if other.path:lower() == plan.path:lower() then
          error({ code = 'bad_request', message = 'Keyword "' .. plan.path .. '" is both added and removed.' }, 0)
        end
      end
    end
    return execute(args, catalog, {
      gate = 'lrcli tag',
      prepare = function(info)
        local ids = Writes.keywordIdSet(info.photo:getRawMetadata('keywords'))
        info.adds, info.removes = Writes.tagWork(addPlans, removePlans, ids)
        info.needsWrite = #info.adds + #info.removes > 0
      end,
      write = function(info)
        applyKeywordChanges(catalog, info.photo, info.adds, info.removes)
      end,
      finish = function(info)
        return { added = paths(info.adds or {}), removed = paths(info.removes or {}) }
      end,
    })
  end

  function handlers.rate(args)
    local rating = Writes.parseRating(args)
    return metadataCommand(args, LrApplication.activeCatalog(), {
      gate = 'lrcli rate',
      key = 'rating',
      wanted = rating,
      store = Writes.ratingValue(rating),
      read = function(photo)
        return photo:getRawMetadata('rating') or 0
      end,
    })
  end

  function handlers.flag(args)
    local name, pickStatus = Writes.parseFlag(args)
    return metadataCommand(args, LrApplication.activeCatalog(), {
      gate = 'lrcli flag',
      key = 'pickStatus',
      wanted = name,
      store = pickStatus,
      read = function(photo)
        return Writes.flagName(photo:getRawMetadata('pickStatus'))
      end,
    })
  end

  function handlers.label(args)
    local label = Writes.parseLabel(args)
    return metadataCommand(args, LrApplication.activeCatalog(), {
      gate = 'lrcli label',
      key = 'colorNameForLabel',
      wanted = label,
      store = label,
      read = function(photo)
        return Writes.labelName(photo:getRawMetadata('colorNameForLabel'))
      end,
    })
  end

  function handlers.restore(args)
    local changes = Writes.parseRestore(args)
    local catalog = LrApplication.activeCatalog()
    local byId = Targets.index(catalog)
    local photos = {}
    for _, change in ipairs(changes) do
      photos[#photos + 1] = byId[change.id]
    end
    local infos = Targets.describe(catalog, photos)
    local infoById = {}
    for _, info in ipairs(infos) do
      infoById[info.id] = info
    end

    local roots
    local plans = {}
    local function plan(text, warnings)
      roots = roots or catalog:getKeywords()
      local ok, result = LrTasks.pcall(Writes.planKeyword, roots, text)
      if not ok then
        warnings[#warnings + 1] = type(result) == 'table' and result.message or tostring(result)
        return nil
      end
      local key = result.path:lower()
      plans[key] = plans[key] or result
      return plans[key]
    end

    local entries = json.array({})
    local jobs = {}
    for i, change in ipairs(changes) do
      local info = infoById[change.id]
      local warnings = {}
      local job = { change = change, info = info, warnings = warnings, restored = {}, adds = {}, removes = {} }
      if not info then
        entries[i] = { id = change.id, filename = '', error = 'Photo is no longer in the catalog.' }
      else
        if change.develop and info.isVideo then
          warnings[#warnings + 1] = 'Videos have no develop settings to restore.'
          change.develop = nil
        end
        local addPlans, removePlans = {}, {}
        for _, text in ipairs(change.keywordsRemoved) do
          addPlans[#addPlans + 1] = plan(text, warnings)
        end
        for _, text in ipairs(change.keywordsAdded) do
          local found = plan(text, warnings)
          if found and found.keyword then
            removePlans[#removePlans + 1] = found
          elseif found then
            warnings[#warnings + 1] = 'Keyword "' .. found.path .. '" no longer exists.'
          end
        end
        local ids = Writes.keywordIdSet(info.photo:getRawMetadata('keywords'))
        job.adds, job.removes = Writes.tagWork(addPlans, removePlans, ids)
        jobs[#jobs + 1] = job
        entries[i] = job
      end
    end

    if #jobs > 0 then
      Writes.gate(catalog, 'lrcli undo', function()
        for _, job in ipairs(jobs) do
          local photo, change = job.info.photo, job.change
          if change.develop and next(change.develop) then
            photo:applyDevelopSettings(change.develop, 'lrcli undo')
            job.restored[#job.restored + 1] = 'develop'
          end
          if change.rating ~= nil then
            photo:setRawMetadata('rating', Writes.ratingValue(change.rating))
            job.restored[#job.restored + 1] = 'rating'
          end
          if change.pickStatus ~= nil then
            photo:setRawMetadata('pickStatus', change.pickStatus)
            job.restored[#job.restored + 1] = 'flag'
          end
          if change.label ~= nil then
            photo:setRawMetadata('colorNameForLabel', change.label)
            job.restored[#job.restored + 1] = 'label'
          end
          if #job.adds + #job.removes > 0 then
            applyKeywordChanges(catalog, photo, job.adds, job.removes)
            job.restored[#job.restored + 1] = 'keywords'
          end
        end
      end, LrTasks.pcall)
    end

    for i, item in ipairs(entries) do
      if item.change then
        entries[i] = entry(item.info, { restored = jsonList(item.restored), warnings = jsonList(item.warnings) })
      end
    end
    return { photos = entries }
  end

  -- Redoes the undo unless the catalog now matches the state the recorded step
  -- should have restored and the previous step's result is untouched: the undo
  -- stack cannot be inspected, and consecutive writes coalesce into one step.
  function handlers.undo_native(args)
    local parsed = Writes.parseUndoNative(args)
    if not LrUndo.canUndo() then
      return { native = false, reason = 'nothing to undo' }
    end
    local byId = Targets.index(LrApplication.activeCatalog())
    local function firstMismatch(label, items)
      for _, expected in ipairs(items) do
        local photo = byId[expected.id]
        if not photo then
          return label .. ': photo ' .. expected.id .. ' is no longer in the catalog'
        end
        local found = Writes.mismatch(Writes.stateOf(photo, expected), expected)
        if found then
          return label .. ': ' .. found
        end
      end
      return nil
    end
    LrUndo.undo()
    LrTasks.sleep(SETTLE_SECONDS)
    local ok, reason = LrTasks.pcall(function()
      return firstMismatch('expected state not reached', parsed.expect)
        or firstMismatch('earlier step not preserved', parsed.keep)
    end)
    if ok and reason == nil then
      return { native = true }
    end
    LrUndo.redo()
    LrTasks.sleep(SETTLE_SECONDS)
    return { native = false, reason = ok and reason or ('check failed: ' .. tostring(type(reason) == 'table' and reason.message or reason)) }
  end

  return handlers
end

return WriteHandlers
