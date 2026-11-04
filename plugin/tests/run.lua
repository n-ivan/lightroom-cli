local pluginDir = 'plugin/lrcli.lrdevplugin/'
package.path = pluginDir .. '?.lua;' .. package.path

local json = require 'json'
local Dispatch = require 'Dispatch'
local Version = require 'Version'
local Photos = require 'Photos'
local SearchCriteria = require 'SearchCriteria'
local Targets = require 'Targets'
local Preview = require 'Preview'

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
