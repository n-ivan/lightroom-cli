local pluginDir = 'plugin/lrcli.lrdevplugin/'
package.path = pluginDir .. '?.lua;' .. package.path

local json = require 'json'
local Dispatch = require 'Dispatch'
local Version = require 'Version'
local Photos = require 'Photos'
local SearchCriteria = require 'SearchCriteria'
local Targets = require 'Targets'
local Preview = require 'Preview'
local Writes = require 'Writes'
local DevelopActions = require 'DevelopActions'

local passed, failed = 0, 0

local function test(name, fn)
  local ok, err = pcall(fn)
  if ok then
    passed = passed + 1
  else
    failed = failed + 1
    print('FAIL ' .. name .. '\n  ' .. tostring(err))
  end
end

local function eq(actual, expected, label)
  if actual ~= expected then
    error((label or 'value') .. ': expected ' .. tostring(expected) .. ', got ' .. tostring(actual), 2)
  end
end

local function roundTrip(v)
  local decoded, err = json.decode(json.encode(v))
  if err then
    error(err, 2)
  end
  return decoded
end

test('json encodes scalars', function()
  eq(json.encode(1), '1')
  eq(json.encode(-3), '-3')
  eq(json.encode(0.5), '0.5')
  eq(json.encode(1e20), '1e+20')
  eq(json.encode(true), 'true')
  eq(json.encode(json.null), 'null')
  eq(json.encode('a'), '"a"')
end)

test('json encodes integers without a decimal point', function()
  eq(json.encode(7.0), '7')
  eq(json.encode({ id = 1234 }), '{"id":1234}')
end)

test('json rejects NaN and infinity', function()
  eq(pcall(json.encode, 0 / 0), false)
  eq(pcall(json.encode, math.huge), false)
  eq(pcall(json.encode, -math.huge), false)
end)

test('json encodes empty array marker and empty object', function()
  eq(json.encode(json.array({})), '[]')
  eq(json.encode({}), '{}')
  eq(json.encode({ 1, 2, 3 }), '[1,2,3]')
  eq(json.encode({ list = json.array() }), '{"list":[]}')
end)

test('json decodes empty array to array marker', function()
  local v = json.decode('{"a":[]}')
  eq(json.encode(v), '{"a":[]}')
end)

test('json escapes control characters and quotes', function()
  eq(json.encode('a"b\\c\n\t\1'), '"a\\"b\\\\c\\n\\t\\u0001"')
  eq(roundTrip('a"b\\c\n\t\1'), 'a"b\\c\n\t\1')
end)

test('json decodes unicode escapes and surrogate pairs', function()
  eq(json.decode('"\\u00e9"'), '\xC3\xA9')
  eq(json.decode('"\\u20ac"'), '\xE2\x82\xAC')
  eq(json.decode('"\\ud83d\\ude00"'), '\xF0\x9F\x98\x80')
  eq(json.decode('"\\u0041"'), 'A')
end)

test('json passes through raw UTF-8', function()
  eq(roundTrip('caf\xC3\xA9 \xF0\x9F\x98\x80'), 'caf\xC3\xA9 \xF0\x9F\x98\x80')
end)

test('json rejects lone surrogates', function()
  local v, err = json.decode('"\\ud83d"')
  eq(v, nil)
  eq(type(err), 'string')
  eq(json.decode('"\\ude00"'), nil)
end)

test('json decodes numbers', function()
  eq(json.decode('0'), 0)
  eq(json.decode('-12'), -12)
  eq(json.decode('1.5'), 1.5)
  eq(json.decode('1e3'), 1000)
  eq(json.decode('-2.5E-1'), -0.25)
end)

test('json round-trips nested structures', function()
  local v = roundTrip({ a = { 1, 2, { b = 'x' } }, n = json.null, t = true, f = false })
  eq(v.a[3].b, 'x')
  eq(v.n, json.null)
  eq(v.t, true)
  eq(v.f, false)
end)

test('json decode errors are returned, not thrown', function()
  local bad = { '', '{', '[1,]', '{"a":}', '{"a" 1}', 'nope', '01', '1.', '"abc', '[1] x', '{"a":1,}', '"\\q"', '"a\nb"', "{'a':1}" }
  for _, input in ipairs(bad) do
    local v, err = json.decode(input)
    eq(v, nil, 'decode(' .. input .. ')')
    eq(type(err), 'string', 'error for ' .. input)
  end
  local v, err = json.decode(nil)
  eq(v, nil)
  eq(type(err), 'string')
end)

test('json decode rejects excessive nesting', function()
  local v = json.decode(string.rep('[', 1000) .. string.rep(']', 1000))
  eq(v, nil)
end)

local TOKEN = 'abcdef0123456789abcdef0123456789'

local function newDispatch()
  return Dispatch.new({
    token = TOKEN,
    protocol = 1,
    handlers = {
      hello = function()
        return { protocol = 1, plugin = { version = '0.1.0' } }
      end,
      echo = function(args)
        return { echoed = args.value }
      end,
      boom = function()
        error('kaput')
      end,
    },
  })
end

local function send(d, request)
  return d:handle(type(request) == 'string' and request or json.encode(request))
end

local function hello(d, overrides)
  local args = { token = TOKEN, protocol = 1, client = 'test' }
  for k, v in pairs(overrides or {}) do
    args[k] = v
  end
  return send(d, { id = 1, cmd = 'hello', args = args })
end

test('dispatch hello succeeds with the right token', function()
  local reply = hello(newDispatch())
  eq(reply.ok, true)
  eq(reply.id, 1)
  eq(reply.protocol, 1)
  eq(reply.plugin.version, '0.1.0')
end)

test('dispatch rejects commands before hello', function()
  local reply = send(newDispatch(), { id = 2, cmd = 'echo', args = { value = 1 } })
  eq(reply.ok, false)
  eq(reply.error, 'unauthorized')
  eq(reply.id, 2)
end)

test('dispatch rejects a wrong or missing token', function()
  local d = newDispatch()
  eq(hello(d, { token = 'nope' }).error, 'unauthorized')
  eq(hello(d, { token = json.null }).error, 'unauthorized')
  eq(send(d, { id = 1, cmd = 'hello' }).error, 'unauthorized')
  eq(send(d, { id = 3, cmd = 'echo' }).error, 'unauthorized')
end)

test('dispatch reports protocol mismatch', function()
  local d = newDispatch()
  local reply = hello(d, { protocol = 2 })
  eq(reply.ok, false)
  eq(reply.error, 'protocol_mismatch')
  eq(send(d, { id = 2, cmd = 'echo' }).error, 'unauthorized')
end)

test('dispatch runs handlers after hello and merges the result', function()
  local d = newDispatch()
  hello(d)
  local reply = send(d, { id = 5, cmd = 'echo', args = { value = 'x' } })
  eq(reply.ok, true)
  eq(reply.id, 5)
  eq(reply.echoed, 'x')
end)

test('dispatch resetAuth requires a new hello', function()
  local d = newDispatch()
  hello(d)
  d:resetAuth()
  eq(send(d, { id = 2, cmd = 'echo' }).error, 'unauthorized')
  hello(d)
  eq(send(d, { id = 3, cmd = 'echo' }).ok, true)
end)

test('dispatch answers unknown commands', function()
  local d = newDispatch()
  hello(d)
  local reply = send(d, { id = 9, cmd = 'nope' })
  eq(reply.error, 'unknown_command')
  eq(reply.id, 9)
end)

test('dispatch turns handler errors into internal_error', function()
  local d = newDispatch()
  hello(d)
  local reply = send(d, { id = 4, cmd = 'boom' })
  eq(reply.ok, false)
  eq(reply.error, 'internal_error')
  eq(reply.id, 4)
  assert(reply.message:find('kaput', 1, true), 'message should include the Lua error')
  eq(send(d, { id = 5, cmd = 'echo' }).ok, true)
end)

test('dispatch answers unparseable input with a null id', function()
  local d = newDispatch()
  for _, line in ipairs({ 'not json', '', '[1,2]', '"str"', '42', 'null' }) do
    local reply = send(d, line)
    eq(reply.error, 'bad_request', line)
    eq(reply.id, json.null, line)
  end
end)

test('dispatch rejects missing or invalid id and cmd', function()
  local d = newDispatch()
  hello(d)
  eq(send(d, { cmd = 'echo' }).error, 'bad_request')
  eq(send(d, { cmd = 'echo' }).id, json.null)
  eq(send(d, { id = 'a', cmd = 'echo' }).error, 'bad_request')
  eq(send(d, { id = 1.5, cmd = 'echo' }).error, 'bad_request')
  local noCmd = send(d, { id = 3 })
  eq(noCmd.error, 'bad_request')
  eq(noCmd.id, 3)
  eq(send(d, { id = 3, cmd = 7 }).error, 'bad_request')
  eq(send(d, '{"id":3,"cmd":"echo","args":5}').error, 'bad_request')
end)

test('dispatch turns structured handler errors into error replies', function()
  local d = Dispatch.new({
    token = TOKEN,
    protocol = 1,
    handlers = {
      hello = function()
        return {}
      end,
      missing = function()
        error({ code = 'photo_not_found', message = 'nope' }, 0)
      end,
      odd = function()
        error({ code = 5 })
      end,
    },
  })
  hello(d)
  local reply = send(d, { id = 2, cmd = 'missing' })
  eq(reply.ok, false)
  eq(reply.error, 'photo_not_found')
  eq(reply.message, 'nope')
  eq(reply.id, 2)
  eq(send(d, { id = 3, cmd = 'odd' }).error, 'internal_error')
end)

local function badRequest(fn, ...)
  local ok, err = pcall(fn, ...)
  eq(ok, false)
  eq(type(err), 'table', 'error')
  return err
end

local function crit(filters)
  return SearchCriteria.build(filters)
end

test('search: no filters means no searchDesc', function()
  eq(crit(nil), nil)
  eq(crit({}), nil)
  eq(crit(json.null), nil)
end)

test('search: rating', function()
  local d = crit({ rating = { op = '>=', value = 3 } })
  eq(d.combine, 'intersect')
  eq(#d, 1)
  eq(d[1].criteria, 'rating')
  eq(d[1].operation, '>=')
  eq(d[1].value, 3)
end)

test('search: rating validation', function()
  eq(badRequest(crit, { rating = { op = '~', value = 3 } }).code, 'bad_request')
  assert(badRequest(crit, { rating = { op = '==', value = 6 } }).message:find('0 to 5', 1, true))
  badRequest(crit, { rating = { op = '==', value = 1.5 } })
  badRequest(crit, { rating = 3 })
end)

test('search: flag and label map to SDK values', function()
  eq(crit({ flag = 'picked' })[1].value, 1)
  eq(crit({ flag = 'unflagged' })[1].value, 0)
  eq(crit({ flag = 'rejected' })[1].value, -1)
  eq(crit({ label = 'red' })[1].value, 1)
  eq(crit({ label = 'purple' })[1].value, 5)
  local err = badRequest(crit, { flag = 'maybe' })
  assert(err.message:find('"flag"', 1, true) and err.message:find('picked', 1, true), err.message)
  assert(badRequest(crit, { label = 'pink' }).message:find('red, yellow, green, blue, purple', 1, true))
end)

test('search: keywords become one criterion each', function()
  local d = crit({ keywords = { 'a', 'b' } })
  eq(#d, 2)
  eq(d[1].criteria, 'keywords')
  eq(d[2].value, 'b')
  badRequest(crit, { keywords = {} })
  badRequest(crit, { keywords = { '' } })
  badRequest(crit, { keywords = 'a' })
end)

test('search: text filters', function()
  eq(crit({ text = 'x' })[1].criteria, 'all')
  eq(crit({ filename = 'IMG' })[1].criteria, 'filename')
  eq(crit({ folder = '2024' })[1].criteria, 'folder')
  eq(crit({ collection = 'Best' })[1].criteria, 'collection')
  eq(crit({ camera = 'R5' })[1].criteria, 'camera')
  assert(badRequest(crit, { text = '' }).message:find('"text"', 1, true))
  badRequest(crit, { camera = 5 })
end)

test('search: dates', function()
  local since = crit({ since = '2024-02-29' })[1]
  eq(since.criteria, 'captureTime')
  eq(since.operation, '>')
  eq(since.value, '2024-02-28')
  local untilDate = crit({ ['until'] = '2024-12-31' })[1]
  eq(untilDate.operation, '<')
  eq(untilDate.value, '2025-01-01')
  eq(crit({ since = '2024-03-01' })[1].value, '2024-02-29')
  local both = crit({ since = '2024-01-01', ['until'] = '2024-01-31' })
  eq(#both, 1)
  eq(both[1].operation, 'in')
  eq(both[1].value, '2024-01-01')
  eq(both[1].value2, '2024-01-31')
  badRequest(crit, { since = '2023-02-29' })
  badRequest(crit, { since = '2024-1-1' })
  badRequest(crit, { since = 20240101 })
  badRequest(crit, { since = '2024-02-01', ['until'] = '2024-01-01' })
end)

test('search: combined filters and unknown filters', function()
  local d = crit({ rating = { op = '==', value = 5 }, flag = 'picked', text = 'x' })
  eq(#d, 3)
  eq(d.combine, 'intersect')
  assert(badRequest(crit, { colour = 'red' }).message:find('Allowed filters', 1, true))
  badRequest(crit, 'x')
end)

test('search: paging defaults and limits', function()
  local limit, offset = SearchCriteria.paging({})
  eq(limit, 50)
  eq(offset, 0)
  limit, offset = SearchCriteria.paging({ limit = 500, offset = 10 })
  eq(limit, 500)
  eq(offset, 10)
  badRequest(SearchCriteria.paging, { limit = 501 })
  badRequest(SearchCriteria.paging, { limit = 0 })
  badRequest(SearchCriteria.paging, { offset = -1 })
  badRequest(SearchCriteria.paging, { offset = 1.5 })
end)

test('photos: summary maps flags, labels and nulls', function()
  local s = Photos.summary({
    id = 7, fileName = 'a.cr3', path = '/p/a.cr3', captureTime = '2024-01-02T03:04:05Z',
    rating = 4, pickStatus = -1, colorNameForLabel = 'Red', dimensions = { width = 10, height = 20 },
  })
  eq(s.id, 7)
  eq(s.flag, 'rejected')
  eq(s.label, 'red')
  eq(s.rating, 4)
  eq(s.width, 10)
  eq(s.captureTime, '2024-01-02T03:04:05Z')
  local bare = Photos.summary({ id = 8, pickStatus = 1, colorNameForLabel = 'Custom' })
  eq(bare.flag, 'picked')
  eq(bare.label, json.null)
  eq(bare.captureTime, json.null)
  eq(bare.width, json.null)
  eq(bare.rating, 0)
  eq(Photos.summary({ id = 9 }).flag, 'unflagged')
end)

test('photos: info adds metadata, gps and optional develop', function()
  local meta = {
    id = 1, title = '', caption = 'c', keywordTags = 'a, b ,,c', fileFormat = 'RAW',
    cameraModel = 'R5', isoSpeedRating = 100, shutterSpeed = '1/250 sec', gps = { latitude = 1.5, longitude = -2 },
  }
  local i = Photos.info(meta)
  eq(i.title, json.null)
  eq(i.caption, 'c')
  eq(#i.keywords, 3)
  eq(i.keywords[2], 'b')
  eq(i.exif.camera, 'R5')
  eq(i.exif.lens, json.null)
  eq(i.exif.shutter, '1/250 sec')
  eq(i.gps.longitude, -2)
  eq(i.develop, nil)
  eq(Photos.info({ id = 2 }).gps, json.null)
  eq(json.encode(Photos.info({ id = 2 }).keywords), '[]')
  eq(Photos.info(meta, { Exposure2012 = 0.5 }).develop.Exposure2012, 0.5)
end)

test('photos: sanitize drops values json cannot encode', function()
  local clean = Photos.sanitize({
    a = 1, b = 'x', c = true, d = print, e = 0 / 0, f = { 1, 2, 3 }, g = { h = { i = 2 } }, [3] = 'stray',
  })
  eq(clean.a, 1)
  eq(clean.d, nil)
  eq(clean.e, nil)
  eq(clean.f[3], 3)
  eq(clean.g.h.i, 2)
  eq(clean[3], nil)
  eq(json.encode(Photos.sanitize({ f = { 1, 2 } })), '{"f":[1,2]}')
  local cyclic = {}
  cyclic.self = cyclic
  json.encode(Photos.sanitize(cyclic))
end)

local function stubCatalog(opts)
  local photos, all = {}, {}
  for _, id in ipairs(opts.known or {}) do
    photos[id] = { localIdentifier = id }
    all[#all + 1] = photos[id]
  end
  local selected = opts.selected or {}
  return {
    getAllPhotos = function()
      return all
    end,
    getTargetPhoto = function()
      return selected[1]
    end,
    getTargetPhotos = function()
      return #selected > 0 and selected or { photos[1], photos[2] }
    end,
  }
end

test('writes: rangeError points non-raw Temperature at IncrementalTemperature', function()
  local Writes = require 'Writes'
  local nonRaw = { Temperature = { -100, 100 }, IncrementalTemperature = { -100, 100 } }
  local raw = { Temperature = { 2000, 50000 } }
  assert(Writes.rangeError({ Temperature = 20 }, nonRaw):find('IncrementalTemperature', 1, true))
  eq(Writes.rangeError({ Temperature = 5500 }, raw), nil)
  assert(Writes.rangeError({ Temperature = 1000 }, raw):find('2000..50000', 1, true))
end)

test('writes: knownKeyTypes merges documented and live keys', function()
  local Writes = require 'Writes'
  local types = Writes.knownKeyTypes({ Exposure2012 = 'number' }, {
    { Exposure2012 = 0.5, ColorGradeMidtoneHue = 40, Look = 'x', ToneCurvePV2012 = { 0, 0 } },
  })
  eq(types.Exposure2012, 'number')
  eq(types.ColorGradeMidtoneHue, 'number')
  eq(types.Look, 'string')
  eq(types.ToneCurvePV2012, nil)
end)

test('targets: parseIds', function()
  eq(Targets.parseIds(nil), nil)
  eq(Targets.parseIds(json.null), nil)
  eq(#Targets.parseIds({ 1, 2, 2, 3 }), 3)
  eq(badRequest(Targets.parseIds, { 'a' }).code, 'bad_request')
  badRequest(Targets.parseIds, { 1.5 })
  badRequest(Targets.parseIds, 5)
end)

test('targets: explicit ids resolve all-or-nothing', function()
  local catalog = stubCatalog({ known = { 1, 2 } })
  local photos, target = Targets.resolve(catalog, { 2, 1 })
  eq(#photos, 2)
  eq(photos[1].localIdentifier, 2)
  eq(target, 'ids')
  local err = badRequest(Targets.resolve, catalog, { 1, 8, 9 })
  eq(err.code, 'photo_not_found')
  assert(err.message:find('8, 9', 1, true), err.message)
end)

test('targets: no ids uses the selection', function()
  local catalog = stubCatalog({ known = { 1, 2 }, selected = { { localIdentifier = 2 } } })
  local photos, target = Targets.resolve(catalog, nil)
  eq(#photos, 1)
  eq(target, 'selection')
  eq(Targets.resolve(catalog, {}) ~= nil, true)
end)

test('targets: nothing selected is detected despite getTargetPhotos returning the filmstrip', function()
  local catalog = stubCatalog({ known = { 1, 2 } })
  eq(#catalog:getTargetPhotos(), 2)
  local err = badRequest(Targets.resolve, catalog, nil)
  eq(err.code, 'no_photo_selected')
  assert(err.message:find('--photo <id>', 1, true))
  eq(#Targets.selection(catalog), 0)
end)

test('targets: sameIds compares as sets', function()
  eq(Targets.sameIds({ { localIdentifier = 1 }, { localIdentifier = 2 } }, { 2, 1 }), true)
  eq(Targets.sameIds({ { localIdentifier = 1 } }, { 1, 2 }), false)
  eq(Targets.sameIds({ { localIdentifier = 3 } }, { 1 }), false)
end)

test('dispatch.isPing recognises only ping requests', function()
  eq(Dispatch.isPing('{"id":1,"cmd":"ping"}'), true)
  eq(Dispatch.isPing('{"id":1,"cmd":"preview"}'), false)
  eq(Dispatch.isPing('garbage'), false)
  eq(Dispatch.isPing('[1]'), false)
end)

test('dispatch.nextIndex runs one command at a time but lets pings through', function()
  local ping = '{"id":2,"cmd":"ping"}'
  local other = '{"id":1,"cmd":"preview"}'
  eq(Dispatch.nextIndex({}, false), nil)
  eq(Dispatch.nextIndex({ other, ping }, false), 1)
  eq(Dispatch.nextIndex({}, true), nil)
  eq(Dispatch.nextIndex({ other }, true), nil)
  eq(Dispatch.nextIndex({ other, ping }, true), 2)
end)

local function previewErr(args)
  local ok, err = pcall(Preview.parseArgs, args, function(dir)
    return dir == '/tmp/ok'
  end)
  eq(ok, false)
  eq(err.code, 'bad_request')
  return err
end

test('preview: parseArgs applies defaults and validates', function()
  local parsed = Preview.parseArgs({ dir = '/tmp/ok' }, function()
    return true
  end)
  eq(parsed.size, 1600)
  eq(parsed.mode, 'export')
  eq(parsed.dir, '/tmp/ok')
  eq(Preview.parseArgs({ dir = '/tmp/ok', size = 64, mode = 'thumbnail' }, function()
    return true
  end).mode, 'thumbnail')
  previewErr({})
  previewErr({ dir = 'relative' })
  previewErr({ dir = '/tmp/missing' })
  previewErr({ dir = '/tmp/ok', size = 63 })
  previewErr({ dir = '/tmp/ok', size = 4097 })
  previewErr({ dir = '/tmp/ok', size = 100.5 })
  previewErr({ dir = '/tmp/ok', mode = 'raw' })
end)

test('preview: photo count is capped', function()
  Preview.checkCount(50)
  eq(pcall(Preview.checkCount, 51), false)
end)

test('preview: export settings constrain the long edge and keep exports local', function()
  local s = Preview.exportSettings('/tmp/d', 800)
  eq(s.LR_export_destinationPathPrefix, '/tmp/d')
  eq(s.LR_size_maxHeight, 800)
  eq(s.LR_size_maxWidth, 800)
  eq(s.LR_size_resizeType, 'longEdge')
  eq(s.LR_jpeg_quality, 0.8)
  eq(s.LR_reimportExportedPhoto, false)
  eq(s.LR_collisionHandling, 'overwrite')
  eq(s.LR_tokens, 'lrcli-{{image_name}}-{{naming_operationSequence_3Digits}}')
end)

test('preview: thumbnailPath handles trailing slashes', function()
  eq(Preview.thumbnailPath('/a/b', 7), '/a/b/lrcli-7-thumb.jpg')
  eq(Preview.thumbnailPath('/a/b/', 7), '/a/b/lrcli-7-thumb.jpg')
end)

test('preview: exportEntries keeps photo order and reports skips and misses', function()
  local infos = {
    { id = 1, filename = 'a.cr3' },
    { id = 2, filename = 'v.mov', skipReason = 'Videos cannot be previewed.' },
    { id = 3, filename = 'c.cr3' },
    { id = 4 },
  }
  local entries = Preview.exportEntries(infos, {
    [1] = { path = '/d/lrcli-a.jpg' },
    [3] = { message = 'render failed' },
  })
  eq(#entries, 4)
  eq(entries[1].file, '/d/lrcli-a.jpg')
  eq(entries[1].error, json.null)
  eq(entries[2].file, json.null)
  eq(entries[2].error, 'Videos cannot be previewed.')
  eq(entries[3].error, 'render failed')
  eq(entries[4].filename, json.null)
  assert(entries[4].error ~= json.null)
  eq(entries[1].possiblyStale, nil)
  json.encode(entries)
end)

test('preview: thumbnail entries are marked possibly stale', function()
  local entry = Preview.entry('thumbnail', { id = 1, filename = 'a' }, '/d/x.jpg', nil)
  eq(entry.possiblyStale, true)
  local reply = Preview.reply('ids', { mode = 'thumbnail', size = 800 }, { entry })
  eq(reply.target, 'ids')
  eq(reply.size, 800)
end)

test('dispatch replies encode to JSON', function()
  local d = newDispatch()
  local line = json.encode(send(d, 'garbage'))
  assert(line:find('"id":null', 1, true), line)
  assert(line:find('"ok":false', 1, true), line)
end)

local keyTypes = { Exposure2012 = 'number', Contrast2012 = 'number', WhiteBalance = 'string', EnableDetail = 'boolean', ToneCurve = 'table' }

test('writes: parseApply validates keys and types', function()
  local parsed = Writes.parseApply({ set = { Contrast2012 = 15, WhiteBalance = 'Auto' }, add = { Exposure2012 = 0.5 } }, keyTypes)
  eq(table.concat(parsed.touched, ','), 'Contrast2012,Exposure2012,WhiteBalance')
  local err = badRequest(Writes.parseApply, { set = { exposure = 1 } }, keyTypes)
  eq(err.code, 'bad_request')
  assert(err.message:find('Exposure2012', 1, true), err.message)
  badRequest(Writes.parseApply, { set = { Contrast2012 = 'x' } }, keyTypes)
  badRequest(Writes.parseApply, { set = { EnableDetail = 1 } }, keyTypes)
  badRequest(Writes.parseApply, { set = { ToneCurve = 1 } }, keyTypes)
  badRequest(Writes.parseApply, { add = { WhiteBalance = 1 } }, keyTypes)
  badRequest(Writes.parseApply, { set = { Exposure2012 = 1 }, add = { Exposure2012 = 1 } }, keyTypes)
  badRequest(Writes.parseApply, {}, keyTypes)
  badRequest(Writes.parseApply, { set = json.array({}) }, keyTypes)
end)

test('writes: suggestions match by prefix, case-insensitively', function()
  eq(table.concat(Writes.suggest('contrast', keyTypes), ','), 'Contrast2012')
  eq(#Writes.suggest('zzz', keyTypes), 0)
end)

test('writes: applyValues computes relative values and reports before', function()
  local parsed = Writes.parseApply({ set = { Contrast2012 = 15 }, add = { Exposure2012 = 0.5 } }, keyTypes)
  local values, before = Writes.applyValues({ Exposure2012 = 1, Contrast2012 = 3 }, parsed)
  eq(values.Exposure2012, 1.5)
  eq(values.Contrast2012, 15)
  eq(before.Exposure2012, 1)
  local _, _, err = Writes.applyValues({ Contrast2012 = 3 }, parsed)
  assert(err and err:find('Exposure2012', 1, true), err)
end)

test('writes: changedSettings keeps only differing keys', function()
  local before, after = Writes.changedSettings({ A = 1, B = 2, T = { 1, 2 } }, { A = 1, B = 3, T = { 1, 2 }, C = true })
  eq(json.encode(before), '{"B":2,"C":null}')
  eq(json.encode(after), '{"B":3,"C":true}')
end)

test('writes: gate maps aborted to write_busy and rethrows errors', function()
  local function catalogReturning(status)
    return {
      withWriteAccessDo = function(_, _, fn)
        fn()
        return status
      end,
    }
  end
  Writes.gate(catalogReturning('executed'), 'n', function() end, pcall)
  eq(badRequest(Writes.gate, catalogReturning('aborted'), 'n', function() end, pcall).code, 'write_busy')
  local failing = {
    withWriteAccessDo = function(_, _, fn)
      fn()
      return 'executed'
    end,
  }
  eq(badRequest(Writes.gate, failing, 'n', function()
    error({ code = 'boom', message = 'm' }, 0)
  end, pcall).code, 'boom')
end)

test('writes: rating, flag and label parsing', function()
  eq(Writes.parseRating({ rating = 3 }), 3)
  eq(Writes.ratingValue(0), nil)
  eq(Writes.ratingValue(4), 4)
  badRequest(Writes.parseRating, { rating = 6 })
  badRequest(Writes.parseRating, { rating = 1.5 })
  local name, value = Writes.parseFlag({ flag = 'rejected' })
  eq(name, 'rejected')
  eq(value, -1)
  badRequest(Writes.parseFlag, { flag = 'unflagged' })
  eq(Writes.flagName(nil), 'none')
  eq(Writes.parseLabel({ label = 'purple' }), 'purple')
  badRequest(Writes.parseLabel, { label = 'pink' })
  eq(Writes.labelName('Red'), 'red')
  eq(Writes.labelName('white'), 'none')
  eq(Writes.labelName(nil), 'none')
end)

local function keywordTree()
  local nextId = 0
  local function node(name, children)
    nextId = nextId + 1
    local kw = { localIdentifier = nextId, children = children or {} }
    function kw:getName() return name end
    function kw:getChildren() return self.children end
    function kw:getParent() return self.parent end
    for _, child in ipairs(kw.children) do
      child.parent = kw
    end
    return kw
  end
  local portrait = node('Portrait', { node('Studio') })
  local places = node('Places', { node('Studio'), node('Paris') })
  return { portrait, places, node('Misc') }, portrait, places
end

test('writes: keyword paths are split and trimmed', function()
  eq(table.concat(Writes.parseKeywordPath(' A > B '), '|'), 'A|B')
  eq(table.concat(Writes.parseKeywordPath('Solo'), '|'), 'Solo')
  badRequest(Writes.parseKeywordPath, 'A >')
  badRequest(Writes.parseKeywordPath, '')
end)

test('writes: planKeyword finds, disambiguates and plans creation', function()
  local roots, portrait, places = keywordTree()
  local plan = Writes.planKeyword(roots, 'paris')
  eq(plan.path, 'Places > Paris')
  eq(plan.keyword, places.children[2])
  local ambiguous = badRequest(Writes.planKeyword, roots, 'Studio')
  assert(ambiguous.message:find('Portrait > Studio', 1, true) and ambiguous.message:find('Places > Studio', 1, true), ambiguous.message)
  eq(Writes.planKeyword(roots, 'portrait > studio').keyword, portrait.children[1])
  local fresh = Writes.planKeyword(roots, 'Portrait > Outdoor > Dusk')
  eq(fresh.keyword, nil)
  eq(fresh.parent, portrait)
  eq(table.concat(fresh.rest, '|'), 'Outdoor|Dusk')
  eq(fresh.path, 'Portrait > Outdoor > Dusk')
  local top = Writes.planKeyword(roots, 'Brand New')
  eq(top.parent, nil)
  eq(top.path, 'Brand New')
  eq(Writes.planKeyword(roots, 'Brand > New').path, 'Brand > New')
end)

test('writes: ensureKeyword creates the missing chain once', function()
  local created = {}
  local catalog = {
    createKeyword = function(_, name, _, includeOnExport, parent, returnExisting)
      created[#created + 1] = name
      return { localIdentifier = #created + 100, name = name, parent = parent }
    end,
  }
  local roots, portrait = keywordTree()
  local plan = Writes.planKeyword(roots, 'Portrait > A > B')
  local keyword = Writes.ensureKeyword(catalog, plan)
  eq(Writes.ensureKeyword(catalog, plan), keyword)
  eq(table.concat(created, ','), 'A,B')
  eq(keyword.parent.parent, portrait)
end)

test('writes: tagWork skips what the photo already has', function()
  local roots, _, places = keywordTree()
  local paris = Writes.planKeyword(roots, 'Paris')
  local misc = Writes.planKeyword(roots, 'Misc')
  local fresh = Writes.planKeyword(roots, 'Fresh')
  local have = Writes.keywordIdSet({ places.children[2] })
  local adds, removes = Writes.tagWork({ paris, fresh }, { misc }, have)
  eq(#adds, 1)
  eq(adds[1], fresh)
  eq(#removes, 0)
  adds, removes = Writes.tagWork({}, { paris }, have)
  eq(#removes, 1)
end)

test('writes: findPreset resolves by name or uuid and rejects ambiguity', function()
  local folders = {
    { name = 'Color', presets = { { name = 'Warm', uuid = 'U1', preset = 'p1' }, { name = 'Cool', uuid = 'U2', preset = 'p2' } } },
    { name = 'User', presets = { { name = 'warm', uuid = 'U3', preset = 'p3' } } },
  }
  eq(Writes.findPreset(folders, { name = 'cool' }).preset, 'p2')
  eq(Writes.findPreset(folders, { uuid = 'u3' }).folder, 'User')
  local tooMany = badRequest(Writes.findPreset, folders, { name = 'WARM' })
  assert(tooMany.message:find('Color/Warm (U1)', 1, true) and tooMany.message:find('User/warm (U3)', 1, true), tooMany.message)
  badRequest(Writes.findPreset, folders, { name = 'nope' })
  badRequest(Writes.parsePreset, {})
  badRequest(Writes.parsePreset, { name = 'x', amount = 201 })
  eq(Writes.parsePreset({ name = 'x', amount = 50 }).amount, 50)
end)

test('writes: parseRestore keeps recorded values as-is and drops nulls', function()
  local changes = Writes.parseRestore({
    changes = { { id = 1, develop = { Exposure2012 = 0.5, Contrast2012 = json.null }, rating = 0, flag = 'picked', label = 'red', keywordsAdded = { 'A' } } },
  }, keyTypes)
  eq(changes[1].develop.Exposure2012, 0.5)
  eq(changes[1].develop.Contrast2012, nil)
  eq(changes[1].rating, 0)
  eq(changes[1].pickStatus, 1)
  eq(#changes[1].keywordsAdded, 1)
  badRequest(Writes.parseRestore, { changes = {} }, keyTypes)
  badRequest(Writes.parseRestore, { changes = { { id = 'x' } } }, keyTypes)
  local look = Writes.parseRestore({ changes = { { id = 1, develop = { Look = { Name = 'Adobe Color' } } } } })
  eq(look[1].develop.Look.Name, 'Adobe Color')
  local curve = Writes.parseRestore(json.decode('{"changes":[{"id":1,"develop":{"ToneCurvePV2012":[0,0,255,255]}}]}'))
  eq(getmetatable(curve[1].develop.ToneCurvePV2012), nil)
  eq(curve[1].develop.ToneCurvePV2012[3], 255)
end)

local function writeFixture(opts)
  local log = {}
  local photos = {}
  local function newPhoto(id, props)
    local photo = { localIdentifier = id, develop = props.develop or {}, meta = props.meta or {}, keywords = props.keywords or {} }
    function photo:getDevelopSettings() return self.develop end
    function photo:applyDevelopSettings(values, name)
      log[#log + 1] = 'apply:' .. id .. ':' .. name
      for k, v in pairs(values) do self.develop[k] = v end
    end
    function photo:getRawMetadata(key)
      if key == 'keywords' then return self.keywords end
      return self.meta[key]
    end
    function photo:setRawMetadata(key, value)
      log[#log + 1] = 'set:' .. id .. ':' .. key .. '=' .. tostring(value)
      self.meta[key] = value
    end
    function photo:addKeyword(kw) log[#log + 1] = 'addKeyword:' .. id .. ':' .. kw:getName() self.keywords[#self.keywords + 1] = kw end
    function photo:removeKeyword(kw) log[#log + 1] = 'removeKeyword:' .. id .. ':' .. kw:getName() end
    photos[id] = photo
    return photo
  end
  newPhoto(1, { develop = { Exposure2012 = 1, Contrast2012 = 0 }, meta = { rating = 2, pickStatus = 0 } })
  newPhoto(2, { develop = {}, meta = { isVideo = true } })
  local roots = keywordTree()
  local catalog = {}
  function catalog:getAllPhotos() return { photos[1], photos[2] } end
  function catalog:getTargetPhoto() return nil end
  function catalog:batchGetRawMetadata(list)
    local out = {}
    for _, p in ipairs(list) do out[p] = { isVideo = p.meta.isVideo } end
    return out
  end
  function catalog:batchGetFormattedMetadata(list)
    local out = {}
    for _, p in ipairs(list) do out[p] = { fileName = 'img' .. p.localIdentifier .. '.jpg' } end
    return out
  end
  function catalog:withWriteAccessDo(name, fn, params)
    log[#log + 1] = 'gate:' .. name .. ':' .. tostring(params.timeout)
    fn()
    return opts and opts.status or 'executed'
  end
  function catalog:getKeywords() return roots end
  function catalog:createKeyword(name, _, _, parent)
    log[#log + 1] = 'create:' .. name
    local kw = { localIdentifier = 900, parent = parent }
    function kw:getName() return name end
    function kw:getChildren() return {} end
    function kw:getParent() return self.parent end
    return kw
  end
  return catalog, photos, log
end

local function loadWriteHandlers(catalog, modules)
  _G.import = function(name)
    if name == 'LrApplication' then
      return { activeCatalog = function() return catalog end, developPresetFolders = function() return {} end }
    end
    if modules and modules[name] then
      return modules[name]
    end
    return { pcall = pcall }
  end
  package.loaded.WriteHandlers = nil
  package.loaded.DevelopKeys = nil
  return require('WriteHandlers').new()
end

test('handlers: apply writes all photos in one gate and reports before/after', function()
  local catalog, photos, log = writeFixture()
  local handlers = loadWriteHandlers(catalog)
  local reply = handlers.apply({ photos = { 1 }, set = { Contrast2012 = 15 }, add = { Exposure2012 = 0.5 } })
  eq(table.concat(log, ' '), 'gate:lrcli apply:10 apply:1:lrcli apply')
  local entry = reply.photos[1]
  eq(entry.filename, 'img1.jpg')
  eq(entry.before.Exposure2012, 1)
  eq(entry.after.Exposure2012, 1.5)
  eq(entry.after.Contrast2012, 15)
  eq(reply.target, 'ids')
end)

test('handlers: apply refuses videos per photo without opening a gate for them', function()
  local catalog, _, log = writeFixture()
  local handlers = loadWriteHandlers(catalog)
  local reply = handlers.apply({ photos = { 2 }, set = { Contrast2012 = 15 } })
  assert(reply.photos[1].error, 'expected an error entry')
  eq(#log, 0)
end)

test('handlers: apply reports write_busy', function()
  local catalog = writeFixture({ status = 'aborted' })
  local handlers = loadWriteHandlers(catalog)
  eq(badRequest(handlers.apply, { photos = { 1 }, set = { Contrast2012 = 1 } }).code, 'write_busy')
end)

test('handlers: rate writes nil for zero and skips unchanged photos', function()
  local catalog, photos, log = writeFixture()
  local handlers = loadWriteHandlers(catalog)
  local reply = handlers.rate({ photos = { 1 }, rating = 0 })
  eq(table.concat(log, ' '), 'gate:lrcli rate:10 set:1:rating=nil')
  eq(reply.photos[1].before, 2)
  eq(reply.photos[1].after, 0)
  log[#log] = nil
  log[#log] = nil
  handlers.rate({ photos = { 1 }, rating = 0 })
  eq(#log, 0)
end)

test('handlers: flag and label write SDK values', function()
  local catalog, _, log = writeFixture()
  local handlers = loadWriteHandlers(catalog)
  local flagged = handlers.flag({ photos = { 1 }, flag = 'rejected' })
  eq(flagged.photos[1].after, 'rejected')
  local labelled = handlers.label({ photos = { 1 }, label = 'blue' })
  eq(labelled.photos[1].before, 'none')
  eq(labelled.photos[1].after, 'blue')
  assert(table.concat(log, ' '):find('set:1:pickStatus=-1', 1, true))
  assert(table.concat(log, ' '):find('set:1:colorNameForLabel=blue', 1, true))
end)

test('handlers: tag creates missing keywords inside the gate and reports only changes', function()
  local catalog, _, log = writeFixture()
  local handlers = loadWriteHandlers(catalog)
  local reply = handlers.tag({ photos = { 1 }, add = { 'Portrait > Outdoor', 'Misc' } })
  eq(table.concat(log, ' '), 'gate:lrcli tag:10 create:Outdoor addKeyword:1:Outdoor addKeyword:1:Misc')
  eq(json.encode(reply.photos[1].added), '["Portrait > Outdoor","Misc"]')
  eq(json.encode(reply.photos[1].removed), '[]')
  eq(badRequest(handlers.tag, { photos = { 1 }, remove = { 'Nope' } }).code, 'bad_request')
  eq(badRequest(handlers.tag, { photos = { 1 }, add = { 'Studio' } }).code, 'bad_request')
end)

test('handlers: restore applies recorded values in one undo gate', function()
  local catalog, _, log = writeFixture()
  local handlers = loadWriteHandlers(catalog)
  local reply = handlers.restore({
    changes = {
      { id = 1, develop = { Exposure2012 = 0 }, rating = 5, flag = 'none', label = 'none', keywordsRemoved = { 'Misc' } },
      { id = 99, rating = 1 },
    },
  })
  eq(table.concat(log, ' '), 'gate:lrcli undo:10 apply:1:lrcli undo set:1:rating=5 set:1:pickStatus=0 set:1:colorNameForLabel=none addKeyword:1:Misc')
  eq(json.encode(reply.photos[1].restored), '["develop","rating","flag","label","keywords"]')
  assert(reply.photos[2].error, 'missing photo is reported per photo')
end)

test('writes: ranges pick raw or non-raw by file format and flag out-of-range numbers', function()
  local all = { raw = { Temperature = { 2000, 50000 } }, nonRaw = { Temperature = { -100, 100 }, Exposure2012 = { -5, 5 } } }
  eq(Writes.rangesFor(all, 'RAW'), all.raw)
  eq(Writes.rangesFor(all, 'DNG'), all.raw)
  eq(Writes.rangesFor(all, 'TIFF'), all.nonRaw)
  eq(Writes.rangesFor(all, nil), all.nonRaw)
  eq(Writes.rangeError({ Exposure2012 = 12 }, all.nonRaw), "Exposure2012=12 is outside Lightroom's range -5..5.")
  eq(Writes.rangeError({ Exposure2012 = 5, Unknown = 1e9, WhiteBalance = 'x' }, all.nonRaw), nil)
  eq(Writes.rangeError({ Temperature = 5500 }, all.raw), nil)
  assert(Writes.rangeError({ Temperature = 5500 }, all.nonRaw), 'raw temperature is out of the non-raw range')
end)

test('writes: parseUndoNative validates expectations', function()
  local parsed = Writes.parseUndoNative(json.decode(
    '{"expect":[{"id":1,"develop":{"A":1,"B":null},"rating":0,"flag":"picked","label":"red","keywordsPresent":["K"]}],'
    .. '"keep":[{"id":2,"keywordsAbsent":["J"]}]}'))
  eq(parsed.expect[1].develop.B, json.null)
  eq(parsed.expect[1].flag, 'picked')
  eq(parsed.expect[1].rating, 0)
  eq(parsed.keep[1].keywordsAbsent[1], 'J')
  eq(#Writes.parseUndoNative({ expect = { { id = 1 } } }).keep, 0)
  badRequest(Writes.parseUndoNative, {})
  badRequest(Writes.parseUndoNative, { expect = { { id = 'x' } } })
  badRequest(Writes.parseUndoNative, { expect = { { id = 1, rating = 9 } } })
end)

test('writes: mismatch compares with tolerance, absent keys, tables and keywords', function()
  local expected = Writes.parseUndoNative(json.decode(
    '{"expect":[{"id":1,"develop":{"A":0.5,"B":null,"T":[1,2]},"rating":3,"keywordsPresent":["P"],"keywordsAbsent":["Q"]}]}')).expect[1]
  local state = { develop = { A = 0.5 + 1e-9, T = { 1, 2 } }, rating = 3, keywordPaths = { p = true } }
  eq(Writes.mismatch(state, expected), nil)
  state.develop.B = 4
  eq(Writes.mismatch(state, expected), 'photo 1: B is 4, expected absent')
  state.develop.B = nil
  state.develop.A = 0.6
  eq(Writes.mismatch(state, expected), 'photo 1: A is 0.6, expected 0.5')
  state.develop.A = 0.5
  state.develop.T = { 1, 3 }
  assert(Writes.mismatch(state, expected):find('T is'), 'table difference')
  state.develop.T = { 1, 2 }
  state.rating = 2
  eq(Writes.mismatch(state, expected), 'photo 1: rating is 2, expected 3')
  state.rating = 3
  state.keywordPaths = {}
  eq(Writes.mismatch(state, expected), 'photo 1: keyword "P" is missing')
  state.keywordPaths = { p = true, q = true }
  eq(Writes.mismatch(state, expected), 'photo 1: keyword "Q" is still present')
end)

test('handlers: apply rejects out-of-range values per photo before writing', function()
  local catalog, _, log = writeFixture()
  local handlers = loadWriteHandlers(catalog)
  local reply = handlers.apply({ photos = { 1 }, set = { Exposure2012 = 12 } })
  eq(reply.photos[1].error, "Exposure2012=12 is outside Lightroom's range -5..5.")
  eq(#log, 0)
  local relative = handlers.apply({ photos = { 1 }, add = { Exposure2012 = 4.5 } })
  eq(relative.photos[1].error, "Exposure2012=5.5 is outside Lightroom's range -5..5.")
end)

local function undoFixture(afterUndo)
  local catalog, photos = writeFixture()
  local calls = {}
  local undo = {
    canUndo = function() return true end,
    undo = function() calls[#calls + 1] = 'undo' afterUndo(photos) end,
    redo = function() calls[#calls + 1] = 'redo' end,
  }
  local handlers = loadWriteHandlers(catalog, { LrUndo = undo, LrTasks = { pcall = pcall, sleep = function() end } })
  return handlers, calls
end

test('handlers: undo_native keeps the undo when expected and kept states match', function()
  local handlers, calls = undoFixture(function(photos) photos[1].develop.Exposure2012 = nil end)
  local reply = handlers.undo_native({
    expect = { { id = 1, develop = { Exposure2012 = json.null }, rating = 2 } },
    keep = { { id = 1, develop = { Contrast2012 = 0 } } },
  })
  eq(reply.native, true)
  eq(table.concat(calls, ' '), 'undo')
end)

test('handlers: undo_native redoes when the state is not reached or a kept state changed', function()
  local handlers, calls = undoFixture(function() end)
  local reply = handlers.undo_native({ expect = { { id = 1, develop = { Exposure2012 = 0 } } } })
  eq(reply.native, false)
  assert(reply.reason:find('Exposure2012 is 1'), reply.reason)
  eq(table.concat(calls, ' '), 'undo redo')

  handlers, calls = undoFixture(function(photos) photos[1].develop.Contrast2012 = 9 end)
  reply = handlers.undo_native({ expect = { { id = 1, rating = 2 } }, keep = { { id = 1, develop = { Contrast2012 = 0 } } } })
  eq(reply.native, false)
  assert(reply.reason:find('earlier step not preserved'), reply.reason)
  eq(table.concat(calls, ' '), 'undo redo')

  handlers = undoFixture(function() end)
  assert(handlers.undo_native({ expect = { { id = 77, rating = 1 } } }).reason:find('no longer in the catalog'))
end)

test('handlers: undo_native does nothing when LrUndo has nothing to undo', function()
  local catalog = writeFixture()
  local calls = {}
  local handlers = loadWriteHandlers(catalog, { LrUndo = {
    canUndo = function() return false end,
    undo = function() calls[#calls + 1] = 'undo' end,
  } })
  local reply = handlers.undo_native({ expect = { { id = 1, rating = 2 } } })
  eq(reply.native, false)
  eq(reply.reason, 'nothing to undo')
  eq(#calls, 0)
end)

local PanelSettings = require 'PanelSettings'

local testPanels = {
  { id = 'adjustPanel', title = 'Basic', keys = { 'Temperature', 'Tint', 'Exposure2012', 'IncrementalTemperature', 'Gone' } },
  { id = 'colorGradingPanel', title = 'Color Grading', keys = { 'ColorGradeBlending' } },
  { id = 'mixerPanel', title = 'Color Mixer (HSL / B&W)', keys = { 'HueAdjustmentRed' } },
  { id = 'crop', title = 'Crop', keys = { 'CropLeft' } },
}
local testRanges = {
  raw = {
    Temperature = { 2000, 50000 }, Tint = { -150, 150 }, Exposure2012 = { -5, 5 },
    ColorGradeBlending = { 0, 100 }, HueAdjustmentRed = { -100, 100 }, CropLeft = { 0, 1 },
  },
  nonRaw = {
    Temperature = { -100, 100 }, Tint = { -100, 100 }, Exposure2012 = { -5, 5 },
    IncrementalTemperature = { -100, 100 }, ColorGradeBlending = { 0, 100 },
    HueAdjustmentRed = { -100, 100 }, CropLeft = { 0, 1 },
  },
}

local function keyNames(panel)
  local names = {}
  for i, entry in ipairs(panel.keys) do
    names[i] = entry.key
  end
  return table.concat(names, ',')
end

test('settings: findPanel matches id, title and unique prefixes case-insensitively', function()
  eq(PanelSettings.findPanel(testPanels, 'basic').id, 'adjustPanel')
  eq(PanelSettings.findPanel(testPanels, 'CROP').id, 'crop')
  eq(PanelSettings.findPanel(testPanels, 'colorGradingPanel').id, 'colorGradingPanel')
  eq(PanelSettings.findPanel(testPanels, 'color g').id, 'colorGradingPanel')
  eq(PanelSettings.findPanel(testPanels, 'color m').id, 'mixerPanel')
end)

test('settings: findPanel reports ambiguous and unknown panels with the choices', function()
  local panel, message = PanelSettings.findPanel(testPanels, 'color')
  eq(panel, nil)
  assert(message:find('ambiguous', 1, true) and message:find('mixerPanel', 1, true), message)
  panel, message = PanelSettings.findPanel(testPanels, 'zzz')
  eq(panel, nil)
  assert(message:find('Unknown panel "zzz"', 1, true) and message:find('adjustPanel (Basic)', 1, true), message)
end)

test('settings: without a photo, raw ranges come with differing non-raw ranges', function()
  local out = PanelSettings.listing(testPanels, testRanges, nil, nil)
  eq(keyNames(out[1]), 'Temperature,Tint,Exposure2012,IncrementalTemperature')
  local temperature, exposure, incremental = out[1].keys[1], out[1].keys[3], out[1].keys[4]
  eq(temperature.min, 2000)
  eq(temperature.nonRaw.min, -100)
  eq(exposure.nonRaw, nil)
  eq(incremental.nonRawOnly, true)
  eq(incremental.min, -100)
  eq(temperature.value, nil)
end)

test('settings: raw photos drop the Incremental keys and show current values', function()
  local out = PanelSettings.listing(testPanels, testRanges, 'raw', { Temperature = 5500, Exposure2012 = 0.5, Tint = 'x' })
  eq(keyNames(out[1]), 'Temperature,Tint,Exposure2012')
  eq(out[1].keys[1].value, 5500)
  eq(out[1].keys[1].max, 50000)
  eq(out[1].keys[2].value, nil)
  eq(out[1].keys[3].value, 0.5)
end)

test('settings: non-raw photos swap Temperature/Tint for the Incremental keys', function()
  local out = PanelSettings.listing(testPanels, testRanges, 'nonRaw', { IncrementalTemperature = 10 })
  eq(keyNames(out[1]), 'Exposure2012,IncrementalTemperature')
  eq(out[1].keys[2].value, 10)
end)

test('settings: panels without applicable keys are dropped and arrays encode as arrays', function()
  local out = PanelSettings.listing({ testPanels[1] }, { raw = {}, nonRaw = {} }, nil, nil)
  eq(#out, 0)
  eq(json.encode(out), '[]')
end)

test('develop actions: parse validates actions and reset keys', function()
  local parsed = DevelopActions.parse({ actions = { { action = 'auto-tone' }, { action = 'reset', keys = { 'Exposure2012' } } } })
  eq(#parsed, 2)
  eq(parsed[2].keys[1], 'Exposure2012')
  badRequest(DevelopActions.parse, {})
  badRequest(DevelopActions.parse, { actions = {} })
  badRequest(DevelopActions.parse, { actions = { { action = 'sharpen' } } })
  badRequest(DevelopActions.parse, { actions = { { action = 'reset' } } })
  badRequest(DevelopActions.parse, { actions = { { action = 'reset', keys = { '' } } } })
  badRequest(DevelopActions.parse, { actions = { { action = 'auto-wb', keys = { 'Tint' } } } })
end)

test('develop actions: module names and controller name fallback', function()
  eq(DevelopActions.parseModule({ name = 'develop' }), 'develop')
  badRequest(DevelopActions.parseModule, { name = 'Develop' })
  badRequest(DevelopActions.parseModule, {})
  eq(DevelopActions.alternateName('Exposure2012'), 'Exposure')
  eq(DevelopActions.alternateName('Tint'), nil)
end)

test('Version matches Info.lua', function()
  local info = dofile(pluginDir .. 'Info.lua')
  local v = info.VERSION
  eq(Version.PLUGIN, v.major .. '.' .. v.minor .. '.' .. v.revision)
  eq(Version.PROTOCOL, 1)
end)

print(string.format('lua tests: %d passed, %d failed', passed, failed))
if failed > 0 then
  os.exit(1)
end
